use proc_macro2::Ident;
use quote::ToTokens;
use syn::{Type, TypeReference};

use super::classify::ClassInputKind;
use super::codegen::*;
use super::inspect::*;

pub fn resolve_arg_type(ty: &Type, parent: Option<&Ident>) -> ResolvedType {
  let tokens = ty.to_token_stream();

  if is_abort_signal(ty) {
    return ResolvedType {
      kind: Box::new(SpecialType {
        kind: super::classify::SpecialKind::AbortSignal,
        tokens: tokens.clone(),
      }),
      tokens,
    };
  }

  if is_js_arg_slice(ty) {
    return ResolvedType {
      kind: Box::new(SpecialType {
        kind: super::classify::SpecialKind::JsArgSlice,
        tokens: tokens.clone(),
      }),
      tokens,
    };
  }

  if let Type::Reference(TypeReference {
    mutability, elem, ..
  }) = ty
  {
    if is_typed_array_slice(elem) {
      return ResolvedType {
        kind: Box::new(BorrowedRefType {
          mutable: mutability.is_some(),
          elem_tokens: elem.to_token_stream(),
        }),
        tokens,
      };
    }

    if is_external(elem) {
      return ResolvedType {
        kind: Box::new(BorrowedRefType {
          mutable: mutability.is_some(),
          elem_tokens: elem.to_token_stream(),
        }),
        tokens,
      };
    }

    if let Type::Path(path) = elem.as_ref() {
      if let Some(seg) = path.path.segments.last() {
        if seg.ident != "str" && seg.ident != "Env" {
          let class_tokens = resolve_class_tokens(elem, parent);
          let kind = if mutability.is_some() {
            ClassInputKind::BorrowMut
          } else {
            ClassInputKind::Borrow
          };
          return ResolvedType {
            kind: Box::new(ClassInputType { kind, class_tokens }),
            tokens,
          };
        }
      }
    }

    return ResolvedType {
      kind: Box::new(GenericType {
        tokens: tokens.clone(),
      }),
      tokens,
    };
  }

  if let Some(input) = ty.as_class_input() {
    let class_tokens = resolve_class_tokens(input.inner(), parent);
    return ResolvedType {
      kind: Box::new(ClassInputType {
        kind: input.kind(),
        class_tokens,
      }),
      tokens,
    };
  }

  if let Some(input) = ty.as_optional_class_input() {
    let class_tokens = resolve_class_tokens(input.inner(), parent);
    let inner = ResolvedType {
      kind: Box::new(ClassInputType {
        kind: input.kind(),
        class_tokens,
      }),
      tokens: input.inner().to_token_stream(),
    };
    return ResolvedType {
      kind: Box::new(OptionalType {
        inner: Box::new(inner),
      }),
      tokens,
    };
  }

  if let Some(inner_ty) = ty.option_inner() {
    let inner = resolve_arg_type(inner_ty, parent);
    return ResolvedType {
      kind: Box::new(OptionalType {
        inner: Box::new(inner),
      }),
      tokens,
    };
  }

  if is_external(ty) {
    return ResolvedType {
      kind: Box::new(SpecialType {
        kind: super::classify::SpecialKind::External,
        tokens: tokens.clone(),
      }),
      tokens,
    };
  }

  ResolvedType {
    kind: Box::new(GenericType {
      tokens: tokens.clone(),
    }),
    tokens,
  }
}

pub fn resolve_return_type(ty: &Type, parent: Option<&Ident>) -> ResolvedType {
  let tokens = ty.to_token_stream();

  if let Type::Reference(TypeReference { elem, .. }) = ty {
    if let Type::Path(path) = elem.as_ref() {
      if path.qself.is_none() && path.path.segments.len() == 1 {
        let ident = &path.path.segments[0].ident;
        if ident == "Self" || parent.is_some_and(|p| ident == p) {
          return ResolvedType {
            kind: Box::new(SpecialType {
              kind: super::classify::SpecialKind::ReturnThis,
              tokens: tokens.clone(),
            }),
            tokens,
          };
        }
      }
    }
  }

  if let Some(class_tokens) = ty.as_class_initializer(parent) {
    return ResolvedType {
      kind: Box::new(SpecialType {
        kind: super::classify::SpecialKind::ClassInitializer,
        tokens: class_tokens,
      }),
      tokens,
    };
  }

  ResolvedType {
    kind: Box::new(GenericType {
      tokens: tokens.clone(),
    }),
    tokens,
  }
}

pub fn resolve_field_type(ty: &Type, parent: Option<&Ident>) -> ResolvedType {
  let tokens = ty.to_token_stream();

  if let Some(input) = ty.as_class_input() {
    let class_tokens = resolve_class_tokens(input.inner(), parent);
    return ResolvedType {
      kind: Box::new(ClassInputType {
        kind: input.kind(),
        class_tokens,
      }),
      tokens,
    };
  }

  if let Some(input) = ty.as_optional_class_input() {
    let class_tokens = resolve_class_tokens(input.inner(), parent);
    let inner = ResolvedType {
      kind: Box::new(ClassInputType {
        kind: input.kind(),
        class_tokens,
      }),
      tokens: input.inner().to_token_stream(),
    };
    return ResolvedType {
      kind: Box::new(OptionalType {
        inner: Box::new(inner),
      }),
      tokens,
    };
  }

  if let Some(inner_ty) = ty.option_inner() {
    let inner = resolve_field_type(inner_ty, parent);
    return ResolvedType {
      kind: Box::new(OptionalType {
        inner: Box::new(inner),
      }),
      tokens,
    };
  }

  ResolvedType {
    kind: Box::new(GenericType {
      tokens: tokens.clone(),
    }),
    tokens,
  }
}
