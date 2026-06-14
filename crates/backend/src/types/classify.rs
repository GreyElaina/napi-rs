use proc_macro2::{Ident, Span, TokenStream};
use syn::Type;

use crate::{BindgenResult, Diagnostic};

use super::inspect::resolve_class_type;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassInputKind {
  Ref,
  ClassRef,
  Borrow,
  BorrowMut,
}

impl ClassInputKind {
  pub fn from_ident(ident: &Ident) -> Option<Self> {
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

  pub fn is_mut(self) -> bool {
    matches!(self, Self::BorrowMut)
  }

  pub fn is_reference(self) -> bool {
    matches!(self, Self::Ref | Self::ClassRef)
  }
}

pub struct ClassInput<'a> {
  kind: ClassInputKind,
  inner: &'a Type,
}

impl<'a> ClassInput<'a> {
  pub fn new(kind: ClassInputKind, inner: &'a Type) -> Self {
    Self { kind, inner }
  }

  pub fn kind(&self) -> ClassInputKind {
    self.kind
  }

  pub fn inner(&self) -> &'a Type {
    self.inner
  }

  pub fn is_mut(&self) -> bool {
    self.kind.is_mut()
  }

  pub fn class_type(&self, parent: Option<&Ident>) -> Option<TokenStream> {
    resolve_class_type(self.inner, parent)
  }

  pub fn class_type_or_error(
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialKind {
  AbortSignal,
  External,
  ClassInitializer,
  ReturnThis,
  JsArgSlice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedTag {
  ClassInput(ClassInputKind),
  Optional,
  Callback,
  Special(SpecialKind),
  BorrowedRef,
  Generic,
}
