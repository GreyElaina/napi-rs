#[macro_use]
pub mod attrs;
mod fn_decl;
mod forbidden;
mod helpers;

use std::collections::HashMap;
use std::sync::{atomic::AtomicUsize, Mutex, OnceLock};

use attrs::{
  find_napi_attr, EnumAttrs, EnumVariantAttrs, FieldAttrs, FnAttrs, ImplAttrs, StructAttrs,
  StructRegistry,
};

use convert_case::Case;
use darling::FromMeta;
use fn_decl::{flex_str, flex_string, napi_fn_from_decl};
use forbidden::{forbidden_class_field_type, forbidden_js_visible_type};
use helpers::{extract_doc_comments, extract_path_ident, get_expr, get_ty};
use napi_derive_backend::{
  rm_raw_prefix, to_case, BindgenResult, Diagnostic, FnKind, Napi, NapiArray, NapiClass, NapiConst,
  NapiEnum, NapiEnumValue, NapiEnumVariant, NapiImpl, NapiItem, NapiObject, NapiStruct,
  NapiStructField, NapiStructKind, NapiStructuredEnum, NapiStructuredEnumVariant, NapiTransparent,
  NapiType, NativeParentSpec, PropertyDescriptor,
};
use proc_macro2::{Ident, Span};
use syn::{
  AngleBracketedGenericArguments, GenericArgument, Path, PathArguments, PathSegment, Type,
  Visibility,
};

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// Stores (is_sync_generator, is_async_generator) for each struct
static GENERATOR_STRUCT: OnceLock<Mutex<HashMap<String, (bool, bool)>>> = OnceLock::new();

static REGISTER_INDEX: AtomicUsize = AtomicUsize::new(0);

fn get_register_ident(name: &str) -> Ident {
  let new_name = format!(
    "__napi_register__{}_{}",
    rm_raw_prefix(name),
    REGISTER_INDEX.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
  );
  Ident::new(&new_name, Span::call_site())
}

// ---------------------------------------------------------------------------
// convert_fields
// ---------------------------------------------------------------------------

pub fn convert_fields(
  fields: &mut syn::Fields,
  check_vis: bool,
) -> BindgenResult<(Vec<NapiStructField>, bool)> {
  let mut napi_fields = vec![];
  let is_tuple = matches!(fields, syn::Fields::Unnamed(_));
  for (i, field) in fields.iter_mut().enumerate() {
    if check_vis && !matches!(field.vis, syn::Visibility::Public(_)) {
      continue;
    }

    let field_opts: FieldAttrs =
      find_napi_attr(&mut field.attrs)?.unwrap_or_else(|| FieldAttrs::from_list(&[]).unwrap());

    let (js_name, name) = match &field.ident {
      Some(ident) => (
        flex_str(&field_opts.js_name)
          .map(|s| s.to_owned())
          .unwrap_or_else(|| to_case(syn::ext::IdentExt::unraw(ident).to_string(), Case::Camel)),
        syn::Member::Named(ident.clone()),
      ),
      None => (
        flex_str(&field_opts.js_name)
          .map(|s| s.to_owned())
          .unwrap_or_else(|| format!("field{i}")),
        syn::Member::Unnamed(i.into()),
      ),
    };

    let ignored = field_opts.skip.is_present();
    let readonly = field_opts.readonly.is_present();
    let writable = field_opts.writable.0;
    let enumerable = field_opts.enumerable.0;
    let configurable = field_opts.configurable.0;
    let skip_typescript = field_opts.skip_typescript.is_present();
    let ts_type = flex_string(&field_opts.ts_type);

    let mut ty = field.ty.clone();
    if !ignored {
      if let Some((ident, message)) = forbidden_js_visible_type(&ty) {
        return Err(Diagnostic::spanned_error(&ident, message));
      }
    }

    let has_lifetime = if let Type::Path(syn::TypePath {
      path: Path { segments, .. },
      ..
    }) = &mut ty
    {
      if let Some(PathSegment {
        arguments: PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. }),
        ..
      }) = segments.last_mut()
      {
        args.iter_mut().any(|arg| {
          if let GenericArgument::Lifetime(lifetime) = arg {
            *lifetime = syn::Lifetime::new("'static", Span::call_site());
            true
          } else {
            false
          }
        })
      } else {
        false
      }
    } else {
      false
    };

    napi_fields.push(NapiStructField {
      name,
      js_name,
      ty,
      getter: !ignored,
      setter: !(ignored || readonly),
      descriptor: PropertyDescriptor {
        writable,
        enumerable,
        configurable,
      },
      comments: extract_doc_comments(&field.attrs),
      skip_typescript,
      ts_type,
      has_lifetime,
    })
  }
  Ok((napi_fields, is_tuple))
}

// ---------------------------------------------------------------------------
// Per-item conversion functions
// ---------------------------------------------------------------------------

pub fn convert_fn(f: &mut syn::ItemFn, opts: &FnAttrs) -> BindgenResult<Napi> {
  let func = napi_fn_from_decl(&mut f.sig, opts, f.attrs.clone(), f.vis.clone(), None, None)?;

  Ok(Napi {
    item: NapiItem::Fn(func),
  })
}

pub fn convert_struct(s: &mut syn::ItemStruct, opts: &StructAttrs) -> BindgenResult<Napi> {
  let mut errors = vec![];

  let rust_struct_ident = s.ident.clone();
  let final_js_name = flex_str(&opts.js_name)
    .map(|s| s.to_owned())
    .unwrap_or_else(|| to_case(s.ident.to_string(), Case::Pascal));

  let use_nullable = opts.use_nullable.0;
  let (fields, is_tuple) = convert_fields(&mut s.fields, true)?;

  let parent = opts
    .extends
    .as_ref()
    .and_then(|p| p.segments.last().map(|seg| seg.ident.to_string()));

  StructRegistry::record(&rust_struct_ident, final_js_name.clone(), parent);

  let namespace = flex_string(&opts.namespace);
  let implement_iterator = opts.iterator.is_present();
  let implement_async_iterator = opts.async_iterator.is_present();

  if implement_iterator && implement_async_iterator {
    bail_span!(
      s,
      "Cannot use both #[napi(iterator)] and #[napi(async_iterator)] on the same struct. \
       Use #[napi(iterator)] for synchronous iteration (impl Generator) or \
       #[napi(async_iterator)] for async iteration (impl AsyncGenerator)"
    );
  }

  if (implement_iterator || implement_async_iterator)
    && s
      .fields
      .iter()
      .filter(|f| matches!(f.vis, Visibility::Public(_)))
      .filter_map(|f| f.ident.clone())
      .map(|ident| ident.to_string())
      .any(|field_name| field_name == "next" || field_name == "throw" || field_name == "return")
  {
    bail_span!(
      s,
      "Generator structs cannot have public fields named `next`, `throw`, or `return`."
    );
  }

  let generator_struct = GENERATOR_STRUCT.get_or_init(|| Mutex::new(HashMap::new()));
  let mut generator_struct = generator_struct
    .lock()
    .expect("Lock generator struct failed");
  let key = namespace
    .as_ref()
    .map(|n| format!("{n}::{rust_struct_ident}"))
    .unwrap_or_else(|| rust_struct_ident.to_string());
  generator_struct.insert(key, (implement_iterator, implement_async_iterator));
  drop(generator_struct);

  let transparent = opts
    .transparent
    .is_present()
    .then(|| -> Result<_, Diagnostic> {
      if !is_tuple || s.fields.len() != 1 {
        bail_span!(
          s,
          "#[napi(transparent)] can only be applied to a struct with a single field tuple",
        )
      }
      let first_field = s.fields.iter().next().unwrap();
      Ok(first_field.ty.clone())
    })
    .transpose()?;

  let struct_kind = if let Some(transparent) = transparent {
    NapiStructKind::Transparent(NapiTransparent {
      ty: transparent,
      object_from_js: opts.object_from_js.0,
      object_to_js: opts.object_to_js.0,
    })
  } else if opts.array.is_present() {
    if !is_tuple {
      bail_span!(s, "#[napi(array)] can only be applied to a tuple struct",)
    }
    NapiStructKind::Array(NapiArray {
      fields,
      object_from_js: opts.object_from_js.0,
      object_to_js: opts.object_to_js.0,
    })
  } else if opts.object.is_present() {
    NapiStructKind::Object(NapiObject {
      fields,
      object_from_js: opts.object_from_js.0,
      object_to_js: opts.object_to_js.0,
      is_tuple,
    })
  } else {
    if opts.custom_finalize.is_present() {
      errors.push(err_span!(
        s,
        "#[napi(custom_finalize)] is not supported by the class storage object model"
      ));
    }

    for syn::Field { ty, .. } in s.fields.iter() {
      if let Some((ident, name)) = forbidden_class_field_type(ty) {
        errors.push(err_span!(
          ident,
          "Can't assign {} to a field of napi class struct",
          name
        ));
      }
    }
    NapiStructKind::Class(NapiClass {
      fields,
      ctor: opts.constructor.is_present(),
      subclass: opts.subclass.is_present(),
      parent: opts.extends.as_ref().map(|parent_path| NativeParentSpec {
        rust_path: Type::Path(syn::TypePath {
          qself: None,
          path: parent_path.clone(),
        }),
        js_name: parent_path
          .segments
          .last()
          .and_then(|segment| StructRegistry::lookup_js_name(&segment.ident)),
      }),
      implement_iterator,
      implement_async_iterator,
      is_tuple,
      use_custom_finalize: opts.custom_finalize.is_present(),
      is_generator: implement_iterator,
      is_async_generator: implement_async_iterator,
    })
  };

  match &struct_kind {
    NapiStructKind::Transparent(_) => {}
    NapiStructKind::Class(class) if !class.ctor => {}
    _ => {
      for field in s.fields.iter() {
        if !matches!(field.vis, syn::Visibility::Public(_)) {
          errors.push(err_span!(
            field,
            "#[napi] requires all struct fields to be public to mark struct as constructor or object shape\nthis field is not public."
          ));
        }
      }
    }
  };

  if matches!(struct_kind, NapiStructKind::Class(_)) {
    if s.generics.lifetimes().next().is_some() {
      errors.push(err_span!(
        s.generics,
        "napi class must not declare lifetime parameters"
      ));
    }
    if s.generics.type_params().next().is_some() {
      errors.push(err_span!(
        s.generics,
        "napi class must not declare type parameters"
      ));
    }
    if s.generics.const_params().next().is_some() {
      errors.push(err_span!(
        s.generics,
        "napi class must not declare const parameters"
      ));
    }
  }

  if s.generics.lifetimes().size_hint().0 > 1 {
    errors.push(err_span!(
      s,
      "struct with multiple generic parameters is not supported"
    ));
  }

  let lifetime = if let Some(lifetime) = s.generics.lifetimes().next() {
    if !lifetime.bounds.is_empty() {
      bail_span!(lifetime.bounds, "unsupported self type in #[napi] impl")
    }
    Some(lifetime.lifetime.to_string())
  } else {
    None
  };

  Diagnostic::from_vec(errors).map(|()| Napi {
    item: NapiItem::Struct(NapiStruct {
      js_name: final_js_name,
      name: rust_struct_ident.clone(),
      kind: struct_kind,
      js_mod: namespace,
      use_nullable,
      register_name: get_register_ident(format!("{rust_struct_ident}_struct").as_str()),
      comments: extract_doc_comments(&s.attrs),
      has_lifetime: lifetime.is_some(),
    }),
  })
}

pub fn convert_impl(i: &mut syn::ItemImpl, impl_opts: &ImplAttrs) -> BindgenResult<Napi> {
  let struct_name = match get_ty(&mut i.self_ty) {
    syn::Type::Path(syn::TypePath {
      ref mut path,
      qself: None,
    }) => path,
    _ => {
      bail_span!(i.self_ty, "unsupported self type in #[napi] impl")
    }
  };

  let (struct_name, has_lifetime) = extract_path_ident(struct_name)?;

  let (mut struct_js_name, mut is_class) = match StructRegistry::check_for_impl(&struct_name, false)
  {
    Ok(recorded_js_name) => (recorded_js_name, true),
    Err(_) => (to_case(struct_name.to_string(), Case::UpperCamel), false),
  };

  let mut items = vec![];
  let mut iterator_yield_type = None;
  let mut iterator_next_type = None;
  let mut iterator_return_type = None;
  let mut async_iterator_yield_type = None;
  let mut async_iterator_next_type = None;
  let mut async_iterator_return_type = None;

  for item in i.items.iter_mut() {
    if let Some(method) = match item {
      syn::ImplItem::Fn(m) => Some(m),
      syn::ImplItem::Type(m) => {
        if let Some((_, t, _)) = &i.trait_ {
          if let Some(PathSegment { ident, .. }) = t.segments.last() {
            if ident == "Generator" || ident == "ScopedGenerator" {
              if let Type::Path(_) = &m.ty {
                if m.ident == "Yield" {
                  iterator_yield_type = Some(m.ty.clone());
                } else if m.ident == "Next" {
                  iterator_next_type = Some(m.ty.clone());
                } else if m.ident == "Return" {
                  iterator_return_type = Some(m.ty.clone());
                }
              }
            } else if ident == "AsyncGenerator" {
              if let Type::Path(_) = &m.ty {
                if m.ident == "Yield" {
                  async_iterator_yield_type = Some(m.ty.clone());
                } else if m.ident == "Next" {
                  async_iterator_next_type = Some(m.ty.clone());
                } else if m.ident == "Return" {
                  async_iterator_return_type = Some(m.ty.clone());
                }
              }
            }
          }
        }
        None
      }
      _ => {
        bail_span!(item, "unsupported impl item in #[napi]")
      }
    } {
      let opts: Option<FnAttrs> = find_napi_attr(&mut method.attrs)?;

      let Some(opts) = opts else {
        continue;
      };

      if opts.constructor.is_present() || opts.factory.is_present() {
        struct_js_name =
          StructRegistry::check_for_impl(&struct_name, opts.constructor.is_present())?;
        is_class = true;
      }

      let vis = method.vis.clone();
      match &vis {
        Visibility::Public(_) => {}
        _ => {
          bail_span!(method.sig.ident, "only pub method supported by #[napi].",);
        }
      }

      let func = napi_fn_from_decl(
        &mut method.sig,
        &opts,
        method.attrs.clone(),
        vis,
        Some(&struct_name),
        Some(struct_js_name.clone()),
      )?;

      if matches!(func.kind, FnKind::PostInit) {
        StructRegistry::record_post_init(&struct_name.to_string(), func.name.to_string());
      }

      items.push(func);
    }
  }

  let chain = StructRegistry::collect_post_init_chain(&struct_name.to_string());
  if !chain.is_empty() {
    for item in items.iter_mut() {
      if let FnKind::Constructor {
        ref mut post_init_chain,
      } = item.kind
      {
        *post_init_chain = chain
          .iter()
          .map(|name| Ident::new(name, Span::call_site()))
          .collect();
        break;
      }
    }
  }

  let namespace = flex_string(&impl_opts.namespace);

  Ok(Napi {
    item: NapiItem::Impl(NapiImpl {
      name: struct_name.clone(),
      js_name: struct_js_name,
      is_class,
      items,
      iterator_yield_type,
      iterator_next_type,
      iterator_return_type,
      async_iterator_yield_type,
      async_iterator_next_type,
      async_iterator_return_type,
      has_lifetime,
      js_mod: namespace,
      comments: extract_doc_comments(&i.attrs),
      register_name: get_register_ident(format!("{struct_name}_impl").as_str()),
    }),
  })
}

pub fn convert_enum(e: &mut syn::ItemEnum, opts: &EnumAttrs) -> BindgenResult<Napi> {
  match e.vis {
    Visibility::Public(_) => {}
    _ => bail_span!(e, "only public enum allowed"),
  }

  let js_name = flex_str(&opts.js_name)
    .map(|s| s.to_owned())
    .unwrap_or_else(|| e.ident.to_string());
  let is_string_enum = opts.string_enum.is_some();

  if e
    .variants
    .iter()
    .any(|v| !matches!(v.fields, syn::Fields::Unit))
  {
    if opts.object.is_present() {
      let discriminant = flex_str(&opts.discriminant).unwrap_or("type");
      let discriminant_case = flex_str(&opts.discriminant_case)
        .map(|c| {
          Ok::<Case, Diagnostic>(match c {
            "lowercase" => Case::Flat,
            "UPPERCASE" => Case::UpperFlat,
            "PascalCase" => Case::Pascal,
            "camelCase" => Case::Camel,
            "snake_case" => Case::Snake,
            "UPPER_SNAKE" => Case::UpperSnake,
            "kebab-case" => Case::Kebab,
            "UPPER-KEBAB-CASE" => Case::UpperKebab,
            _ => {
              bail_span!(e, "Unknown discriminant case. Possible values are \"lowercase\", \"UPPERCASE\", \"PascalCase\", \"camelCase\", \"snake_case\", \"UPPER_SNAKE\", \"kebab-case\", or \"UPPER-KEBAB-CASE\"")
            }
          })
        })
        .transpose()?;

      let mut errors = vec![];
      let mut variants = vec![];
      for variant in e.variants.iter_mut() {
        let (fields, is_tuple) = convert_fields(&mut variant.fields, false)?;
        for field in fields.iter() {
          if field.js_name == discriminant {
            errors.push(err_span!(
              field.name,
              r#"field's js_name("{}") and discriminator("{}") conflict"#,
              field.js_name,
              discriminant,
            ));
          }
        }
        variants.push(NapiStructuredEnumVariant {
          name: variant.ident.clone(),
          fields,
          is_tuple,
        });
      }
      let rust_struct_ident = e.ident.clone();
      return Diagnostic::from_vec(errors).map(|()| Napi {
        item: NapiItem::Struct(NapiStruct {
          name: rust_struct_ident.clone(),
          js_name,
          comments: extract_doc_comments(&e.attrs),
          js_mod: flex_string(&opts.namespace),
          use_nullable: opts.use_nullable.0,
          register_name: get_register_ident(format!("{rust_struct_ident}_struct").as_str()),
          kind: NapiStructKind::StructuredEnum(NapiStructuredEnum {
            variants,
            discriminant: discriminant.to_owned(),
            discriminant_case,
            object_from_js: opts.object_from_js.0,
            object_to_js: opts.object_to_js.0,
          }),
          has_lifetime: false,
        }),
      });
    }

    let rust_struct_ident = e.ident.clone();
    let namespace = flex_string(&opts.namespace);

    StructRegistry::record(&rust_struct_ident, js_name.clone(), None);

    let mut errors = vec![];
    if e.generics.lifetimes().next().is_some() {
      errors.push(err_span!(
        e.generics,
        "napi enum class must not declare lifetime parameters"
      ));
    }
    if e.generics.type_params().next().is_some() {
      errors.push(err_span!(
        e.generics,
        "napi enum class must not declare type parameters"
      ));
    }
    if e.generics.const_params().next().is_some() {
      errors.push(err_span!(
        e.generics,
        "napi enum class must not declare const parameters"
      ));
    }

    return Diagnostic::from_vec(errors).map(|()| Napi {
      item: NapiItem::Struct(NapiStruct {
        name: rust_struct_ident.clone(),
        js_name,
        comments: extract_doc_comments(&e.attrs),
        js_mod: namespace,
        use_nullable: opts.use_nullable.0,
        register_name: get_register_ident(format!("{rust_struct_ident}_struct").as_str()),
        kind: NapiStructKind::Class(NapiClass {
          fields: vec![],
          ctor: false,
          subclass: opts.subclass.is_present(),
          parent: opts.extends.as_ref().map(|parent_path| NativeParentSpec {
            rust_path: Type::Path(syn::TypePath {
              qself: None,
              path: parent_path.clone(),
            }),
            js_name: parent_path
              .segments
              .last()
              .and_then(|segment| StructRegistry::lookup_js_name(&segment.ident)),
          }),
          implement_iterator: false,
          implement_async_iterator: false,
          is_tuple: false,
          use_custom_finalize: false,
          is_generator: false,
          is_async_generator: false,
        }),
        has_lifetime: false,
      }),
    });
  }

  let variants = match &opts.string_enum {
    Some(string_enum_opts) => {
      let case = string_enum_opts
        .0
        .as_ref()
        .map(|c| {
          Ok::<Case, Diagnostic>(match c.value.as_str() {
            "lowercase" => Case::Flat,
            "UPPERCASE" => Case::UpperFlat,
            "PascalCase" => Case::Pascal,
            "camelCase" => Case::Camel,
            "snake_case" => Case::Snake,
            "UPPER_SNAKE" => Case::UpperSnake,
            "kebab-case" => Case::Kebab,
            "UPPER-KEBAB-CASE" => Case::UpperKebab,
            _ => {
              bail_span!(e, "Unknown string enum case. Possible values are \"lowercase\", \"UPPERCASE\", \"PascalCase\", \"camelCase\", \"snake_case\", \"UPPER_SNAKE\", \"kebab-case\", or \"UPPER-KEBAB-CASE\"")
            }
          })
        })
        .transpose()?;

      e.variants
        .iter_mut()
        .map(|v| {
          if !matches!(v.fields, syn::Fields::Unit) {
            bail_span!(
              v.fields,
              "Structured enum is not supported with string enum in #[napi]"
            )
          }
          if matches!(&v.discriminant, Some((_, _))) {
            bail_span!(
              v.fields,
              "Literal values are not supported with string enum in #[napi]"
            )
          }

          let variant_opts: Option<EnumVariantAttrs> = find_napi_attr(&mut v.attrs)?;
          let val = variant_opts
            .and_then(|va| va.value.map(|fs| fs.value))
            .unwrap_or_else(|| {
              let mut val = v.ident.to_string();
              if let Some(case) = case {
                val = to_case(val, case)
              }
              val
            });

          Ok(NapiEnumVariant {
            name: v.ident.clone(),
            val: NapiEnumValue::String(val),
            comments: extract_doc_comments(&v.attrs),
          })
        })
        .collect::<BindgenResult<Vec<NapiEnumVariant>>>()?
    }
    None => {
      let mut last_variant_val: i32 = -1;

      e.variants
        .iter()
        .map(|v| {
          let val = match &v.discriminant {
            Some((_, expr)) => {
              let mut symbol = 1;
              let mut inner_expr = get_expr(expr);
              if let syn::Expr::Unary(syn::ExprUnary {
                attrs: _,
                op: syn::UnOp::Neg(_),
                expr,
              }) = inner_expr
              {
                symbol = -1;
                inner_expr = expr;
              }

              match inner_expr {
                syn::Expr::Lit(syn::ExprLit {
                  attrs: _,
                  lit: syn::Lit::Int(int_lit),
                }) => match int_lit.base10_digits().parse::<i32>() {
                  Ok(v) => symbol * v,
                  Err(_) => {
                    bail_span!(
                      int_lit,
                      "enums with #[wasm_bindgen] can only support \
                    numbers that can be represented as i32",
                    );
                  }
                },
                _ => bail_span!(
                  expr,
                  "enums with #[wasm_bindgen] may only have \
                number literal values",
                ),
              }
            }
            None => last_variant_val + 1,
          };

          last_variant_val = val;

          Ok(NapiEnumVariant {
            name: v.ident.clone(),
            val: NapiEnumValue::Number(val),
            comments: extract_doc_comments(&v.attrs),
          })
        })
        .collect::<BindgenResult<Vec<NapiEnumVariant>>>()?
    }
  };

  Ok(Napi {
    item: NapiItem::Enum(NapiEnum {
      name: e.ident.clone(),
      js_name,
      variants,
      js_mod: flex_string(&opts.namespace),
      comments: extract_doc_comments(&e.attrs),
      skip_typescript: opts.skip_typescript.is_present(),
      register_name: get_register_ident(e.ident.to_string().as_str()),
      is_string_enum,
      object_from_js: opts.object_from_js.0,
      object_to_js: opts.object_to_js.0,
    }),
  })
}

pub fn convert_const(c: &mut syn::ItemConst, opts: &attrs::ConstAttrs) -> BindgenResult<Napi> {
  match c.vis {
    Visibility::Public(_) => Ok(Napi {
      item: NapiItem::Const(NapiConst {
        name: c.ident.clone(),
        js_name: flex_str(&opts.js_name)
          .map(|s| s.to_owned())
          .unwrap_or_else(|| c.ident.to_string()),
        type_name: *c.ty.clone(),
        value: *c.expr.clone(),
        js_mod: flex_string(&opts.namespace),
        comments: extract_doc_comments(&c.attrs),
        skip_typescript: opts.skip_typescript.is_present(),
        register_name: get_register_ident(c.ident.to_string().as_str()),
      }),
    }),
    _ => bail_span!(c, "only public const allowed"),
  }
}

pub fn convert_type(t: &mut syn::ItemType, opts: &attrs::TypeAttrs) -> BindgenResult<Napi> {
  let js_name = match flex_str(&opts.js_name) {
    Some(name) => name.to_string(),
    _ => {
      let types = t
        .generics
        .type_params()
        .map(|param| param.ident.to_string())
        .collect::<Vec<String>>()
        .join(", ");

      if !types.is_empty() {
        format!("{}<{}>", t.ident, types)
      } else {
        t.ident.to_string()
      }
    }
  };

  match t.vis {
    Visibility::Public(_) => Ok(Napi {
      item: NapiItem::Type(NapiType {
        name: t.ident.clone(),
        js_name,
        value: *t.ty.clone(),
        js_mod: flex_string(&opts.namespace),
        comments: extract_doc_comments(&t.attrs),
        skip_typescript: opts.skip_typescript.is_present(),
        ts_type: flex_string(&opts.ts_type).map(|s| s.to_owned()),
        register_name: get_register_ident(t.ident.to_string().as_str()),
      }),
    }),
    _ => bail_span!(t, "only public type allowed"),
  }
}
