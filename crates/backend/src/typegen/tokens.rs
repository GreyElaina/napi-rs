use proc_macro2::{Ident, TokenStream};
use syn::{GenericArgument, PathArguments, Type, TypePath};

use crate::types::classify::ClassInputKind;
use crate::types::inspect::NapiTypeExt;
use crate::TYPEDARRAY_SLICE_TYPES;

fn first_generic_type(arguments: &PathArguments) -> Option<&Type> {
  let PathArguments::AngleBracketed(args) = arguments else {
    return None;
  };
  match args.args.first() {
    Some(GenericArgument::Type(ty)) => Some(ty),
    _ => None,
  }
}

fn clean_type(ty: &mut Type, parent: Option<&Ident>) {
  match ty {
    Type::Reference(r) => {
      r.lifetime = None;
      clean_type(&mut r.elem, parent);
    }
    Type::Path(p) => {
      if p.qself.is_none() && p.path.segments.len() == 1 && p.path.segments[0].ident == "Self" {
        if let Some(parent) = parent {
          p.path.segments[0].ident = parent.clone();
        }
      }
      for seg in &mut p.path.segments {
        if let PathArguments::AngleBracketed(args) = &mut seg.arguments {
          args.args.iter_mut().for_each(|arg| {
            if let GenericArgument::Type(inner) = arg {
              clean_type(inner, parent);
            }
          });
          args.args = args
            .args
            .clone()
            .into_iter()
            .filter(|a| !matches!(a, GenericArgument::Lifetime(_)))
            .collect();
        }
      }
    }
    Type::Tuple(t) => t.elems.iter_mut().for_each(|e| clean_type(e, parent)),
    Type::Array(a) => clean_type(&mut a.elem, parent),
    Type::Slice(s) => clean_type(&mut s.elem, parent),
    Type::Group(g) => clean_type(&mut g.elem, parent),
    Type::Paren(p) => clean_type(&mut p.elem, parent),
    _ => {}
  }
}

fn resolve_self_type<'a>(ty: &'a Type, parent: Option<&Ident>) -> Option<TokenStream> {
  if !ty.is_self_type() {
    return None;
  }
  let parent = parent?;
  Some(quote! { <#parent as napi::bindgen_prelude::TypeName>::ts_type() })
}

pub fn ty_to_ts_type_tokens(
  ty: &Type,
  is_return_ty: bool,
  is_struct_field: bool,
  parent: Option<&Ident>,
) -> (TokenStream, bool) {
  match ty {
    Type::Reference(r) if is_return_ty => {
      if let Type::Path(p) = r.elem.as_ref() {
        if p.qself.is_none() && p.path.segments.len() == 1 {
          let ident = &p.path.segments[0].ident;
          if ident == "Self" || parent.is_some_and(|p| ident == p) {
            return (quote! { "this".to_owned() }, false);
          }
        }
      }
      ty_to_ts_type_tokens(&r.elem, is_return_ty, is_struct_field, parent)
    }
    Type::Reference(r) => ty_to_ts_type_tokens(&r.elem, is_return_ty, is_struct_field, parent),

    Type::Tuple(tuple) => {
      if tuple.elems.is_empty() {
        if is_return_ty {
          (quote! { "void".to_owned() }, false)
        } else {
          (quote! { "undefined".to_owned() }, false)
        }
      } else {
        let elem_tokens: Vec<_> = tuple
          .elems
          .iter()
          .map(|elem| ty_to_ts_type_tokens(elem, false, false, parent).0)
          .collect();
        (
          quote! {
            {
              let elems: Vec<String> = vec![#( #elem_tokens ),*];
              format!("[{}]", elems.join(", "))
            }
          },
          false,
        )
      }
    }

    Type::Path(TypePath {
      qself: None, path, ..
    }) => {
      if let Some(seg) = path.segments.last() {
        let ident_str = seg.ident.to_string();

        if ident_str == "Option" {
          if let Some(inner) = first_generic_type(&seg.arguments) {
            let (inner_tokens, _) =
              ty_to_ts_type_tokens(inner, is_return_ty, is_struct_field, parent);
            return if is_struct_field {
              (inner_tokens, true)
            } else if is_return_ty {
              (quote! { format!("{} | null", #inner_tokens) }, true)
            } else {
              (
                quote! { format!("{} | undefined | null", #inner_tokens) },
                true,
              )
            };
          }
        }

        if ident_str == "Result" {
          if let Some(inner) = first_generic_type(&seg.arguments) {
            return ty_to_ts_type_tokens(inner, is_return_ty, false, parent);
          }
        }

        if ident_str == "Nullable" {
          if let Some(inner) = first_generic_type(&seg.arguments) {
            let (inner_tokens, _) = ty_to_ts_type_tokens(inner, false, false, parent);
            return (
              quote! { format!("{} | null | undefined", #inner_tokens) },
              true,
            );
          }
        }

        if ident_str == "ClassInitializer" {
          if let Some(inner) = first_generic_type(&seg.arguments) {
            return ty_to_ts_type_tokens(inner, false, false, parent);
          }
        }

        if ClassInputKind::from_ident(&seg.ident).is_some() {
          if let Some(inner) = first_generic_type(&seg.arguments) {
            return ty_to_ts_type_tokens(inner, false, false, parent);
          }
        }

        if ident_str == "Class" {
          if let Some(inner) = first_generic_type(&seg.arguments) {
            return ty_to_ts_type_tokens(inner, false, false, parent);
          }
        }

        if ident_str == "This" {
          if let Some(inner) = first_generic_type(&seg.arguments) {
            return ty_to_ts_type_tokens(inner, is_return_ty, is_struct_field, parent);
          }
          return (quote! { "this".to_owned() }, false);
        }
      }

      if let Some(tokens) = resolve_self_type(ty, parent) {
        return (tokens, false);
      }

      let mut clean_ty = ty.clone();
      clean_type(&mut clean_ty, parent);
      (
        quote! { <#clean_ty as napi::bindgen_prelude::TypeName>::ts_type() },
        false,
      )
    }

    Type::Group(g) => ty_to_ts_type_tokens(&g.elem, is_return_ty, is_struct_field, parent),
    Type::Paren(p) => ty_to_ts_type_tokens(&p.elem, is_return_ty, is_struct_field, parent),

    Type::Array(a) => {
      let (elem_tokens, is_optional) = ty_to_ts_type_tokens(&a.elem, false, false, parent);
      (quote! { format!("{}[]", #elem_tokens) }, is_optional)
    }

    Type::Slice(syn::TypeSlice { elem, .. }) => {
      if let Type::Path(TypePath { path, .. }) = &**elem {
        if let Some(seg) = path.segments.last() {
          if let Some(js_type) = TYPEDARRAY_SLICE_TYPES.get(&&*seg.ident.to_string()) {
            let js_type_str = *js_type;
            return (quote! { #js_type_str.to_owned() }, false);
          }
        }
      }
      (quote! { "any[]".to_owned() }, false)
    }

    _ => (quote! { "any".to_owned() }, false),
  }
}

pub fn callback_to_ts_type_tokens(
  callback: &crate::CallbackArg,
  parent: Option<&Ident>,
) -> TokenStream {
  let arg_parts: Vec<_> = callback
    .args
    .iter()
    .enumerate()
    .map(|(i, arg)| {
      let (ts_tokens, is_optional) = ty_to_ts_type_tokens(arg, false, false, parent);
      let arg_name = format!("arg{i}");
      if is_optional {
        quote! { format!("{}?: {}", #arg_name, #ts_tokens) }
      } else {
        quote! { format!("{}: {}", #arg_name, #ts_tokens) }
      }
    })
    .collect();

  let ret_tokens = match &callback.ret {
    Some(ty) => ty_to_ts_type_tokens(ty, true, false, parent).0,
    None => quote! { "void".to_owned() },
  };

  quote! {
    {
      let args: Vec<String> = vec![#( #arg_parts ),*];
      format!("({}) => {}", args.join(", "), #ret_tokens)
    }
  }
}
