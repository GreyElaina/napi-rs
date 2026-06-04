use std::collections::HashMap;
use std::vec::Vec;
use std::{cell::RefCell, iter};

use super::{
  add_alias, format_js_property_name, ty_to_ts_type, NativeParentTypeDef, ToTypeDef, TypeDef,
};
use crate::{typegen::JSDoc, util::to_case, NapiImpl, NapiStruct, NapiStructField, NapiStructKind};

fn unwrap_class_wrapper(ty: &syn::Type) -> &syn::Type {
  if let syn::Type::Path(path) = ty {
    if let Some(segment) = path.path.segments.last() {
      if segment.ident == "Class" {
        if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
          if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
            return inner;
          }
        }
      }
    }
  }
  ty
}

fn is_self_ref_ident(ident: &str, inner: &syn::Type) -> bool {
  match ident {
    "Ref" => is_self_type(unwrap_class_wrapper(inner)),
    "ClassRef" => is_self_type(inner),
    _ => false,
  }
}

fn reference_self_field_type(ty: &syn::Type, owner: &str) -> Option<(String, bool)> {
  let syn::Type::Path(path) = ty else {
    return None;
  };
  let segment = path.path.segments.last()?;
  let ident_str = segment.ident.to_string();

  if ident_str == "Ref" || ident_str == "ClassRef" {
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
      return None;
    };
    let Some(syn::GenericArgument::Type(inner)) = args.args.first() else {
      return None;
    };
    if is_self_ref_ident(&ident_str, inner) {
      return Some((owner.to_owned(), false));
    }
  }

  if segment.ident != "Option" {
    return None;
  }
  let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
    return None;
  };
  let Some(syn::GenericArgument::Type(inner)) = args.args.first() else {
    return None;
  };
  let syn::Type::Path(inner_path) = inner else {
    return None;
  };
  let inner_segment = inner_path.path.segments.last()?;
  let inner_ident_str = inner_segment.ident.to_string();
  if inner_ident_str != "Ref" && inner_ident_str != "ClassRef" {
    return None;
  }
  let syn::PathArguments::AngleBracketed(reference_args) = &inner_segment.arguments else {
    return None;
  };
  let Some(syn::GenericArgument::Type(reference_inner)) = reference_args.args.first() else {
    return None;
  };
  if is_self_ref_ident(&inner_ident_str, reference_inner) {
    Some((owner.to_owned(), true))
  } else {
    None
  }
}

fn is_self_type(ty: &syn::Type) -> bool {
  let syn::Type::Path(path) = ty else {
    return false;
  };
  path.qself.is_none() && path.path.segments.len() == 1 && path.path.segments[0].ident == "Self"
}

fn native_parent_type_def(ty: &syn::Type, js_name: Option<&String>) -> Option<NativeParentTypeDef> {
  let syn::Type::Path(path) = ty else {
    return None;
  };
  if path.qself.is_some() {
    return None;
  }
  let rust_path = path
    .path
    .segments
    .iter()
    .map(|segment| segment.ident.to_string())
    .collect::<Vec<_>>();
  if rust_path.is_empty() {
    return None;
  }
  Some(NativeParentTypeDef {
    rust_path,
    js_name: js_name.cloned(),
  })
}

thread_local! {
  pub(crate) static CLASS_STRUCTS: RefCell<HashMap<String, String>> = Default::default();
}

impl ToTypeDef for NapiStruct {
  fn to_type_def(&self) -> Option<TypeDef> {
    CLASS_STRUCTS.with(|c| {
      c.borrow_mut()
        .insert(self.name.to_string(), self.js_name.clone());
    });
    add_alias(self.name.to_string(), self.js_name.to_string());

    let mut js_doc = JSDoc::new(&self.comments);
    if self.is_generator {
      let generator_doc =[
"This type extends JavaScript's `Iterator`, and so has the iterator helper",
"methods. It may extend the upcoming TypeScript `Iterator` class in the future.",
"",
"@see https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Iterator#iterator_helper_methods",
"@see https://www.typescriptlang.org/docs/handbook/release-notes/typescript-5-6.html#iterator-helper-methods", ];
      js_doc.add_block(generator_doc)
    }
    if self.is_async_generator {
      let generator_doc = [
        "This type implements JavaScript's async iterable protocol.",
        "It can be used with `for await...of` loops.",
        "",
        "@see https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Iteration_protocols#the_async_iterator_and_async_iterable_protocols",
      ];
      js_doc.add_block(generator_doc)
    }

    let native_parent = match &self.kind {
      NapiStructKind::Class(class) => class
        .parent
        .as_ref()
        .and_then(|parent| native_parent_type_def(&parent.rust_path, parent.js_name.as_ref())),
      _ => None,
    };

    Some(TypeDef {
      kind: String::from(match &self.kind {
        NapiStructKind::Transparent(_) => "type",
        NapiStructKind::Class(class) if !class.ctor => "non_constructible_class",
        NapiStructKind::Class(_) => "struct",
        NapiStructKind::Object(_) => "interface",
        NapiStructKind::StructuredEnum(_) => "type",
        NapiStructKind::Array(_) => "type",
      }),
      name: self.js_name.to_owned(),
      original_name: Some(self.name.to_string()),
      def: self.gen_ts_class(),
      extends: None,
      native_parent,
      js_mod: self.js_mod.to_owned(),
      js_doc,
    })
  }
}

impl ToTypeDef for NapiImpl {
  fn to_type_def(&self) -> Option<TypeDef> {
    if let Some(output_type) = &self.iterator_yield_type {
      let next_type = if let Some(ref ty) = self.iterator_next_type {
        ty_to_ts_type(ty, false, false, false).0
      } else {
        "void".to_owned()
      };
      let return_type = if let Some(ref ty) = self.iterator_return_type {
        ty_to_ts_type(ty, false, false, false).0
      } else {
        "void".to_owned()
      };
      Some(TypeDef {
        kind: "extends".to_owned(),
        name: self.js_name.to_owned(),
        original_name: None,
        def: format!(
          "Iterator<{}, {}, {}>",
          ty_to_ts_type(output_type, false, true, false).0,
          return_type,
          next_type,
        ),
        extends: None,
        native_parent: None,
        js_mod: self.js_mod.to_owned(),
        js_doc: JSDoc::new::<Vec<String>, String>(Vec::default()),
      })
    } else if let Some(output_type) = &self.async_iterator_yield_type {
      let yield_type = ty_to_ts_type(output_type, false, true, false).0;
      let next_type = if let Some(ref ty) = self.async_iterator_next_type {
        let ty_str = ty_to_ts_type(ty, false, false, false).0;
        // Make TNext accept undefined so `for await...of` works (it calls next() with no args)
        if ty_str == "void" || ty_str == "undefined" {
          "undefined".to_owned()
        } else {
          format!("{} | undefined", ty_str)
        }
      } else {
        "undefined".to_owned()
      };
      let return_type = if let Some(ref ty) = self.async_iterator_return_type {
        ty_to_ts_type(ty, false, false, false).0
      } else {
        "void".to_owned()
      };
      // Use "impl" kind to add the [Symbol.asyncIterator]() method to the class
      // instead of "extends AsyncGenerator" which is not valid TypeScript
      Some(TypeDef {
        kind: "impl".to_owned(),
        name: self.js_name.to_owned(),
        original_name: None,
        def: format!(
          "[Symbol.asyncIterator](): AsyncGenerator<{}, {}, {}>",
          yield_type, return_type, next_type,
        ),
        extends: None,
        native_parent: None,
        js_mod: self.js_mod.to_owned(),
        js_doc: JSDoc::new::<Vec<String>, String>(Vec::default()),
      })
    } else {
      Some(TypeDef {
        kind: "impl".to_owned(),
        name: self.js_name.to_owned(),
        original_name: None,
        def: self
          .items
          .iter()
          .filter_map(|f| {
            if f.ts.skip_typescript {
              None
            } else {
              Some(format!(
                "{}{}",
                JSDoc::new(&f.comments),
                f.to_type_def()
                  .map_or(String::default(), |type_def| type_def.def)
              ))
            }
          })
          .collect::<Vec<_>>()
          .join("\\n"),
        extends: None,
        native_parent: None,
        js_mod: self.js_mod.to_owned(),
        js_doc: JSDoc::new::<Vec<String>, String>(Vec::default()),
      })
    }
  }
}

impl NapiStruct {
  fn gen_field(&self, f: &NapiStructField) -> Option<(String, String)> {
    if f.skip_typescript {
      return None;
    }

    let mut field_str = String::from("");

    if !f.comments.is_empty() {
      field_str.push_str(&format!("{}", JSDoc::new(&f.comments)))
    }

    if !f.setter {
      field_str.push_str("readonly ")
    }

    let (arg, is_optional) = reference_self_field_type(&f.ty, &self.js_name)
      .unwrap_or_else(|| ty_to_ts_type(&f.ty, false, true, false));
    let arg = f.ts_type.as_ref().map(|ty| ty.to_string()).unwrap_or(arg);
    let js_name = format_js_property_name(&f.js_name);

    let arg = match is_optional {
      false => format!("{}: {}", &js_name, arg),
      true => match self.use_nullable {
        false => format!("{}?: {}", &js_name, arg),
        true => format!("{}: {} | null", &js_name, arg),
      },
    };
    field_str.push_str(&arg);
    Some((field_str, arg))
  }

  fn gen_ts_class(&self) -> String {
    match &self.kind {
      NapiStructKind::Transparent(transparent) => {
        ty_to_ts_type(&transparent.ty, false, false, false).0
      }
      NapiStructKind::Array(array) => {
        let def = array
          .fields
          .iter()
          .filter_map(|f| self.gen_field(f).map(|(field, _)| field))
          .collect::<Vec<_>>()
          .join(", ");
        format!("[{def}]")
      }
      NapiStructKind::Class(class) => {
        let mut ctor_args = vec![];
        let def = class
          .fields
          .iter()
          .filter(|f| f.getter)
          .filter_map(|f| {
            self.gen_field(f).map(|(field, arg)| {
              ctor_args.push(arg);
              field
            })
          })
          .collect::<Vec<_>>()
          .join("\\n");
        if class.ctor {
          format!("{}\\nconstructor({})", def, ctor_args.join(", "))
        } else {
          def
        }
      }
      NapiStructKind::Object(object) => object
        .fields
        .iter()
        .filter(|f| f.getter)
        .filter_map(|f| self.gen_field(f).map(|(field, _)| field))
        .collect::<Vec<_>>()
        .join("\\n"),
      NapiStructKind::StructuredEnum(structured_enum) => structured_enum
        .variants
        .iter()
        .map(|variant| {
          let def = iter::once(format!(
            "{}: '{}'",
            structured_enum.discriminant,
            if let Some(case) = structured_enum.discriminant_case {
              to_case(variant.name.to_string(), case)
            } else {
              variant.name.to_string()
            }
          ))
          .chain(
            variant
              .fields
              .iter()
              .filter(|f| f.getter)
              .filter_map(|f| self.gen_field(f).map(|(field, _)| field)),
          )
          .collect::<Vec<_>>()
          .join(", ");
          format!("  | {{ {def} }} ")
        })
        .collect::<Vec<_>>()
        .join("\\n"),
    }
  }
}
