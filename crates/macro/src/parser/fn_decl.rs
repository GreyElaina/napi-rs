use std::collections::HashMap;
use std::sync::Mutex;

use convert_case::Case;
use napi_derive_backend::{
  to_case, BindgenResult, CallbackArg, Diagnostic, FnKind, FnSelf, InjectKind, NapiFn, NapiFnArg,
  NapiFnArgKind, PropertyDescriptor, TsOverrides,
};
use proc_macro2::{Ident, Span};
use quote::ToTokens;
use syn::{Attribute, Signature, Visibility};

use super::attrs::{find_napi_attr, ArgAttrs, FlexibleString, FnAttrs};
use super::forbidden::forbidden_js_visible_type;
use super::helpers::{extract_callback_trait_types, extract_doc_comments, extract_fn_closure_generics, extract_result_ty};
use super::{get_register_ident, GENERATOR_STRUCT};

// ---------------------------------------------------------------------------
// Flexible string accessor helpers
// ---------------------------------------------------------------------------

pub(crate) fn flex_str(fs: &Option<FlexibleString>) -> Option<&str> {
  fs.as_ref().map(|s| s.value.as_str())
}

pub(crate) fn flex_string(fs: &Option<FlexibleString>) -> Option<String> {
  fs.as_ref().map(|s| s.value.clone())
}

pub(crate) fn flex_str_and_span(fs: &Option<FlexibleString>) -> Option<(&str, Span)> {
  fs.as_ref().map(|s| (s.value.as_str(), s.span))
}

// ---------------------------------------------------------------------------
// Parameter-level attribute parsing
// ---------------------------------------------------------------------------

pub(super) struct ArgAttrParseResult {
  pub ts_arg_type: Option<String>,
  pub inject: Option<InjectKind>,
}

fn parse_arg_attributes(
  p: &mut syn::PatType,
  ts_args_type: Option<(&str, Span)>,
) -> BindgenResult<ArgAttrParseResult> {
  let arg_attrs: Option<ArgAttrs> = find_napi_attr(&mut p.attrs)?;

  let Some(arg_attrs) = arg_attrs else {
    return Ok(ArgAttrParseResult {
      ts_arg_type: None,
      inject: None,
    });
  };

  if let (Some(ref ts_arg_type), Some((fn_level, _))) = (&arg_attrs.ts_arg_type, ts_args_type) {
    return Err(Diagnostic::span_error(
      ts_arg_type.span,
      format!(
        "Found a 'ts_args_type'=\"{}\" override. Cannot use 'ts_arg_type' at the same time since they are mutually exclusive.",
        fn_level
      ),
    ));
  }

  let inject = if arg_attrs.env.is_present() {
    Some(InjectKind::Env)
  } else if arg_attrs.this.is_present() {
    Some(InjectKind::This)
  } else if arg_attrs.scope.is_present() {
    Some(InjectKind::Scope)
  } else if arg_attrs.rest.is_present() {
    Some(InjectKind::Rest)
  } else {
    None
  };

  Ok(ArgAttrParseResult {
    ts_arg_type: arg_attrs.ts_arg_type.map(|f| f.value),
    inject,
  })
}

// ---------------------------------------------------------------------------
// FnKind resolution
// ---------------------------------------------------------------------------

pub(crate) fn fn_kind(opts: &FnAttrs) -> FnKind {
  if opts.getter.is_some() {
    return FnKind::Getter;
  }
  if opts.setter.is_some() {
    return FnKind::Setter;
  }
  if opts.constructor.is_present() {
    return FnKind::Constructor;
  }
  if opts.factory.is_present() {
    return FnKind::Factory;
  }
  if opts.post_init.is_present() {
    return FnKind::PostInit;
  }
  FnKind::Normal
}

// ---------------------------------------------------------------------------
// napi_fn_from_decl
// ---------------------------------------------------------------------------

pub fn napi_fn_from_decl(
  sig: &mut Signature,
  opts: &FnAttrs,
  attrs: Vec<Attribute>,
  vis: Visibility,
  parent: Option<&Ident>,
  parent_js_name: Option<String>,
) -> BindgenResult<NapiFn> {
  let mut errors = vec![];

  let syn::Signature {
    ident,
    asyncness,
    output,
    generics,
    ..
  } = sig.clone();

  let mut fn_self = None;
  let callback_traits = extract_fn_closure_generics(&generics)?;

  let ts_args_type = flex_str_and_span(&opts.ts_args_type);

  let args = sig
    .inputs
    .iter_mut()
    .filter_map(|arg| match arg {
      syn::FnArg::Typed(ref mut p) => {
        let arg_attrs =
          parse_arg_attributes(p, ts_args_type).unwrap_or_else(|e| {
            errors.push(e);
            ArgAttrParseResult {
              ts_arg_type: None,
              inject: None,
            }
          });

        let ty_str = p.ty.to_token_stream().to_string();
        if let Some(path_arguments) = callback_traits.get(&ty_str) {
          match extract_callback_trait_types(path_arguments) {
            Ok((fn_args, fn_ret)) => Some(NapiFnArg {
              kind: NapiFnArgKind::Callback(Box::new(CallbackArg {
                pat: p.pat.clone(),
                args: fn_args,
                ret: fn_ret,
              })),
              ts_arg_type: arg_attrs.ts_arg_type,
              inject: arg_attrs.inject,
            }),
            Err(e) => {
              errors.push(e);
              None
            }
          }
        } else {
          Some(NapiFnArg {
            kind: NapiFnArgKind::PatType(Box::new(p.clone())),
            ts_arg_type: arg_attrs.ts_arg_type,
            inject: arg_attrs.inject,
          })
        }
      }
      syn::FnArg::Receiver(r) => {
        if parent.is_some() {
          assert!(fn_self.is_none());
          if r.reference.is_none() {
            errors.push(err_span!(
              r,
              "The native methods can't move values from napi. Try `&self` or `&mut self` instead."
            ));
          } else if r.mutability.is_some() {
            fn_self = Some(FnSelf::MutRef);
          } else {
            fn_self = Some(FnSelf::Ref);
          }
        } else {
          errors.push(err_span!(r, "arguments cannot be `self`"));
        }
        None
      }
    })
    .collect::<Vec<_>>();

  for arg in &args {
    if let NapiFnArgKind::PatType(pat) = &arg.kind {
      if let Some((ident, message)) = forbidden_js_visible_type(&pat.ty) {
        errors.push(Diagnostic::spanned_error(&ident, message));
      }
    }
  }

  {
    let mut found_rest = false;
    for arg in &args {
      if arg.inject == Some(InjectKind::Rest) {
        if found_rest {
          errors.push(err_span!(
            sig.ident,
            "Only one #[napi(rest)] parameter is allowed"
          ));
        }
        found_rest = true;
      } else if found_rest && arg.inject.is_none() {
        errors.push(err_span!(
          sig.ident,
          "#[napi(rest)] must be the last positional parameter"
        ));
      }
    }
  }

  if let syn::ReturnType::Type(_, ty) = &output {
    if let Some((ident, message)) = forbidden_js_visible_type(ty) {
      errors.push(Diagnostic::spanned_error(&ident, message));
    }
  }

  let (ret, is_ret_result) = match output {
    syn::ReturnType::Default => (None, false),
    syn::ReturnType::Type(_, ty) => {
      let result_ty = extract_result_ty(&ty)?;
      if let Some(result_ty) = result_ty {
        (Some(result_ty), true)
      } else {
        (Some(*ty), false)
      }
    }
  };

  Diagnostic::from_vec(errors).and_then(|_| {
    let js_name = if let Some(prop_name) = &opts.getter {
      flex_str(&opts.js_name).map(|s| s.to_owned()).unwrap_or_else(|| {
        if let Some(ident) = &prop_name.0 {
          ident.to_string()
        } else {
          to_case(ident.to_string().trim_start_matches("get_"), Case::Camel)
        }
      })
    } else if let Some(prop_name) = &opts.setter {
      flex_str(&opts.js_name).map(|s| s.to_owned()).unwrap_or_else(|| {
        if let Some(ident) = &prop_name.0 {
          ident.to_string()
        } else {
          to_case(ident.to_string().trim_start_matches("set_"), Case::Camel)
        }
      })
    } else if opts.constructor.is_present() {
      "constructor".to_owned()
    } else if opts.module_exports.is_present() {
      if opts.js_name.is_some() {
        bail_span!(sig.ident, "module_exports fn can't have js_name");
      }
      if opts.getter.is_some() || opts.setter.is_some() {
        bail_span!(sig.ident, "module_exports fn can't have getter or setter");
      }
      if opts.factory.is_present() || opts.constructor.is_present() {
        bail_span!(
          sig.ident,
          "module_exports fn can't have factory or constructor"
        );
      }
      if opts.strict.is_present() {
        bail_span!(sig.ident, "module_exports fn can't have strict");
      }
      if opts.return_if_invalid.is_present() {
        bail_span!(sig.ident, "module_exports fn can't have return_if_invalid");
      }

      if parent.is_some() {
        bail_span!(sig.ident, "module_exports fn can't inside impl block");
      }

      if !generics.params.is_empty() {
        bail_span!(sig.ident, "module_exports fn can't have generic parameters");
      }

      if opts.no_export.is_present() {
        bail_span!(
          sig.ident,
          "#[napi(no_export)] can not be used with module_exports attribute"
        );
      }

      for arg in args.iter() {
        match &arg.kind {
          NapiFnArgKind::Callback(_) => {
            bail_span!(sig.ident, "module_exports fn can't have callback arguments");
          }
          NapiFnArgKind::PatType(pat) => {
            if arg.ts_arg_type.is_some() {
              bail_span!(sig.ident, "module_exports fn can't have ts_arg_type");
            }
            if let syn::Type::Path(syn::TypePath {
              path: syn::Path { segments, .. },
              ..
            }) = &*pat.ty
            {
              if let Some(segment) = segments.last() {
                if segment.ident != "Env" && segment.ident != "Object" {
                  bail_span!(
                    sig.ident,
                    "module_exports fn can only accept Env or Object as argument"
                  );
                }
                continue;
              }
            }
            if let syn::Type::Reference(syn::TypeReference { elem, .. }) = &*pat.ty {
              if let syn::Type::Path(syn::TypePath {
                path: syn::Path { segments, .. },
                ..
              }) = &**elem
              {
                if let Some(segment) = segments.last() {
                  if segment.ident != "Env" && segment.ident != "Object" {
                    bail_span!(
                      sig.ident,
                      "module_exports fn can only accept Env or Object as argument"
                    );
                  }
                  continue;
                }
              }
            }
          }
        }
        bail_span!(
          sig.ident,
          "module_exports fn can only accept Env or Object as argument"
        );
      }

      if let syn::ReturnType::Type(_, ty) = &sig.output {
        if let syn::Type::Path(syn::TypePath {
          path: syn::Path { segments, .. },
          ..
        }) = &**ty
        {
          if let Some(segment) = segments.last() {
            if segment.ident != "Result" && segment.ident != "()" {
              bail_span!(
                sig.ident,
                "module_exports fn can only return Result<()> or (), got {}",
                segment.ident
              );
            }
            if segment.ident == "Result" {
              if let syn::PathArguments::AngleBracketed(
                syn::AngleBracketedGenericArguments { args, .. },
              ) = &segment.arguments
              {
                if args.len() != 1 {
                  bail_span!(
                    segment.ident,
                    "module_exports fn can only return Result<()> or ()"
                  );
                }
                if let syn::GenericArgument::Type(syn::Type::Tuple(syn::TypeTuple {
                  elems, ..
                })) = &args[0]
                {
                  if !elems.empty_or_trailing() {
                    bail_span!(
                      segment.ident,
                      "module_exports fn can only return Result<()> or ()"
                    );
                  }
                } else {
                  bail_span!(
                    segment.ident,
                    "module_exports fn can only return Result<()> or ()"
                  );
                }
              } else {
                bail_span!(
                  segment.ident,
                  "module_exports fn can only return Result<()> or ()"
                );
              }
            }
          }
        }
      }

      to_case(ident.to_string(), Case::Camel)
    } else {
      flex_str(&opts.js_name)
        .map(|s| s.to_owned())
        .unwrap_or_else(|| to_case(ident.to_string(), Case::Camel))
    };

    let namespace = flex_string(&opts.namespace);
    let (parent_is_generator, parent_is_async_generator) = if let Some(p) = parent {
      let generator_struct = GENERATOR_STRUCT.get_or_init(|| Mutex::new(HashMap::new()));
      let generator_struct = generator_struct
        .lock()
        .expect("Lock generator struct failed");
      let key = namespace
        .as_ref()
        .map(|n| format!("{n}::{p}"))
        .unwrap_or_else(|| p.to_string());
      *generator_struct.get(&key).unwrap_or(&(false, false))
    } else {
      (false, false)
    };

    let kind = fn_kind(opts);

    if !matches!(kind, FnKind::Normal) && parent.is_none() {
      bail_span!(
        sig.ident,
        "Only fn in impl block can be marked as factory, constructor, getter, setter or post_init"
      );
    }

    if matches!(kind, FnKind::Constructor) && asyncness.is_some() {
      bail_span!(sig.ident, "Constructor don't support asynchronous function");
    }

    if matches!(kind, FnKind::PostInit) && asyncness.is_some() {
      bail_span!(sig.ident, "post_init don't support asynchronous function");
    }

    Ok(NapiFn {
      name: ident.clone(),
      js_name,
      module_exports: opts.module_exports.is_present(),
      args,
      ret,
      is_ret_result,
      is_async: asyncness.is_some(),
      vis,
      kind,
      fn_self,
      parent: parent.cloned(),
      parent_js_name,
      comments: extract_doc_comments(&attrs),
      attrs,
      strict: opts.strict.is_present(),
      return_if_invalid: opts.return_if_invalid.is_present(),
      js_mod: namespace,
      ts: TsOverrides {
        ts_type: flex_string(&opts.ts_type),
        ts_generic_types: flex_string(&opts.ts_generic_types),
        ts_args_type: flex_string(&opts.ts_args_type),
        ts_return_type: flex_string(&opts.ts_return_type),
        skip_typescript: opts.skip_typescript.is_present(),
      },
      parent_is_generator,
      parent_is_async_generator,
      descriptor: PropertyDescriptor {
        writable: opts.writable.0,
        enumerable: opts.enumerable.0,
        configurable: opts.configurable.0,
      },
      catch_unwind: opts.catch_unwind.is_present(),
      unsafe_: sig.unsafety.is_some(),
      register_name: get_register_ident(ident.to_string().as_str()),
      no_export: opts.no_export.is_present(),
      post_init_chain: Vec::new(),
    })
  })
}
