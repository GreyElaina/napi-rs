use proc_macro2::{Ident, Span, TokenStream};
use syn::Type;

use crate::{BindgenResult, Diagnostic};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClassInputKind {
  Ref,
  ClassRef,
  Borrow,
  BorrowMut,
}

impl ClassInputKind {
  pub(crate) fn from_ident(ident: &Ident) -> Option<Self> {
    if ident == "Ref" {
      Some(Self::Ref)
    } else if ident == "ClassRef" {
      Some(Self::ClassRef)
    } else if ident == "ClassBorrow" {
      Some(Self::Borrow)
    } else if ident == "ClassBorrowMut" {
      Some(Self::BorrowMut)
    } else {
      None
    }
  }

  pub(crate) fn is_mut(self) -> bool {
    matches!(self, Self::BorrowMut)
  }

  pub(crate) fn is_reference(self) -> bool {
    matches!(self, Self::Ref | Self::ClassRef)
  }
}

pub(crate) struct ClassInput<'a> {
  kind: ClassInputKind,
  inner: &'a Type,
}

impl<'a> ClassInput<'a> {
  pub(crate) fn new(kind: ClassInputKind, inner: &'a Type) -> Self {
    Self { kind, inner }
  }

  pub(crate) fn kind(&self) -> ClassInputKind {
    self.kind
  }

  pub(crate) fn inner(&self) -> &'a Type {
    self.inner
  }

  pub(crate) fn is_mut(&self) -> bool {
    self.kind.is_mut()
  }

  pub(crate) fn class_type(&self, parent: Option<&Ident>) -> Option<TokenStream> {
    resolve_class_type(self.inner, parent)
  }

  pub(crate) fn class_type_or_error(
    &self,
    parent: Option<&Ident>,
    span: Span,
    message: &str,
  ) -> BindgenResult<TokenStream> {
    self
      .class_type(parent)
      .ok_or_else(|| Diagnostic::span_error(span, message))
  }
}

/// Unwrap `Class<T>` to `T`. Returns `None` if the type is not a `Class<T>` wrapper.
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
  let Some(syn::GenericArgument::Type(inner)) = args.args.first() else {
    return None;
  };
  Some(inner)
}

pub(crate) fn resolve_class_type(inner: &Type, parent: Option<&Ident>) -> Option<TokenStream> {
  if inner.is_self_type() {
    return parent.map(|parent| quote! { #parent });
  }
  Some(quote! { #inner })
}

pub(crate) trait NapiTypeExt {
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
