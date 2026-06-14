use proc_macro2::{Ident, TokenStream};
use syn::{Type, TypePath};

use crate::TYPEDARRAY_SLICE_TYPES;

use super::classify::{ClassInput, ClassInputKind};

// ── syn::Type queries ───────────────────────────────────────────────

pub fn is_abort_signal(ty: &Type) -> bool {
  match ty {
    Type::Path(TypePath { path, .. }) => path
      .segments
      .last()
      .is_some_and(|seg| seg.ident == "AbortSignal"),
    Type::Reference(r) => is_abort_signal(&r.elem),
    _ => false,
  }
}

pub fn is_external(ty: &Type) -> bool {
  matches!(
    ty,
    Type::Path(path)
      if path.path.segments.last().is_some_and(|seg| seg.ident == "External")
  )
}

pub fn is_js_arg_slice(ty: &Type) -> bool {
  matches!(
    ty,
    Type::Path(TypePath { path, .. })
      if path.segments.last().is_some_and(|seg| seg.ident == "JsArgSlice")
  )
}

pub fn is_typed_array_slice(elem: &Type) -> bool {
  if let Type::Slice(slice) = elem {
    if let Type::Path(path) = &*slice.elem {
      if let Some(seg) = path.path.segments.first() {
        return TYPEDARRAY_SLICE_TYPES.contains_key(&&*seg.ident.to_string());
      }
    }
  }
  false
}

pub fn extract_vec_element_type(ty: &Type) -> Option<&Type> {
  let Type::Path(TypePath { path, .. }) = ty else {
    return None;
  };
  let seg = path.segments.last()?;
  if seg.ident != "Vec" {
    return None;
  }
  let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
    return None;
  };
  match args.args.first() {
    Some(syn::GenericArgument::Type(elem)) => Some(elem),
    _ => None,
  }
}

fn unwrap_class_wrapper(ty: &Type) -> Option<&Type> {
  let Type::Path(path) = ty else {
    return None;
  };
  let segment = path.path.segments.last()?;
  if segment.ident != "Class" {
    return None;
  }
  let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
    return None;
  };
  match args.args.first() {
    Some(syn::GenericArgument::Type(inner)) => Some(inner),
    _ => None,
  }
}

pub fn resolve_class_type(inner: &Type, parent: Option<&Ident>) -> Option<TokenStream> {
  if inner.is_self_type() {
    return parent.map(|parent| quote! { #parent });
  }
  Some(quote! { #inner })
}

pub fn resolve_class_tokens(inner: &Type, parent: Option<&Ident>) -> TokenStream {
  resolve_class_type(inner, parent).unwrap_or_else(|| quote! { Self })
}

// ── NapiTypeExt trait ───────────────────────────────────────────────

pub trait NapiTypeExt {
  fn as_class_input(&self) -> Option<ClassInput<'_>>;
  fn as_optional_class_input(&self) -> Option<ClassInput<'_>>;
  fn as_class_initializer_inner(&self) -> Option<&Type>;
  fn as_class_initializer(&self, parent: Option<&Ident>) -> Option<TokenStream>;
  fn option_inner(&self) -> Option<&Type>;
  fn this_inner(&self) -> Option<&Type>;
  fn is_bare_this(&self) -> bool;
  fn is_self_type(&self) -> bool;
  fn needs_class_context(&self) -> bool;
}

impl NapiTypeExt for Type {
  fn as_class_input(&self) -> Option<ClassInput<'_>> {
    let Type::Path(path) = self else {
      return None;
    };
    let segment = path.path.segments.last()?;
    let kind = ClassInputKind::from_ident(&segment.ident)?;
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
      return None;
    };
    let Some(syn::GenericArgument::Type(inner)) = args.args.first() else {
      return None;
    };
    let inner = if kind == ClassInputKind::Ref {
      unwrap_class_wrapper(inner).unwrap_or(inner)
    } else {
      inner
    };
    Some(ClassInput::new(kind, inner))
  }

  fn as_optional_class_input(&self) -> Option<ClassInput<'_>> {
    self.option_inner()?.as_class_input()
  }

  fn as_class_initializer_inner(&self) -> Option<&Type> {
    let Type::Path(path) = self else {
      return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "ClassInitializer" {
      return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
      return None;
    };
    let Some(syn::GenericArgument::Type(inner)) = args.args.first() else {
      return None;
    };
    Some(inner)
  }

  fn as_class_initializer(&self, parent: Option<&Ident>) -> Option<TokenStream> {
    resolve_class_type(self.as_class_initializer_inner()?, parent)
  }

  fn option_inner(&self) -> Option<&Type> {
    let Type::Path(path) = self else {
      return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "Option" {
      return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
      return None;
    };
    let Some(syn::GenericArgument::Type(inner)) = args.args.first() else {
      return None;
    };
    Some(inner)
  }

  fn this_inner(&self) -> Option<&Type> {
    let Type::Path(path) = self else {
      return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "This" {
      return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
      return None;
    };
    let Some(syn::GenericArgument::Type(inner)) = args.args.first() else {
      return None;
    };
    Some(inner)
  }

  fn is_bare_this(&self) -> bool {
    matches!(
      self,
      Type::Path(path)
        if path.qself.is_none()
          && path.path.segments.len() == 1
          && path.path.segments[0].ident == "This"
    )
  }

  fn is_self_type(&self) -> bool {
    matches!(
      self,
      Type::Path(path)
        if path.qself.is_none()
          && path.path.segments.len() == 1
          && path.path.segments[0].ident == "Self"
    )
  }

  fn needs_class_context(&self) -> bool {
    match self {
      Type::Path(path) => path.path.segments.last().is_some_and(|segment| {
        if ClassInputKind::from_ident(&segment.ident).is_some() {
          return true;
        }
        if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
          return args
            .args
            .iter()
            .any(|arg| matches!(arg, syn::GenericArgument::Type(ty) if ty.needs_class_context()));
        }
        false
      }),
      Type::Reference(reference) => match reference.elem.as_ref() {
        Type::Slice(_) => false,
        Type::Path(path) => path.path.segments.last().is_some_and(|segment| {
          !matches!(
            segment.ident.to_string().as_str(),
            "Env" | "str" | "External"
          )
        }),
        _ => false,
      },
      Type::Group(group) => group.elem.needs_class_context(),
      Type::Paren(paren) => paren.elem.needs_class_context(),
      _ => false,
    }
  }
}
