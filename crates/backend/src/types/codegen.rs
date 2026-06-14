use proc_macro2::TokenStream;

use super::classify::{ClassInputKind, ResolvedTag, SpecialKind};

pub trait TypeCodegen: std::fmt::Debug {
  fn tag(&self) -> ResolvedTag;
  fn emit_from_js(&self, source: TokenStream) -> TokenStream;

  fn emit_into_js(&self, value: TokenStream) -> TokenStream {
    quote! {
      napi::bindgen_prelude::IntoJs::into_js(#value, frame.scope_mut()).map(|local| local.raw())
    }
  }

  fn needs_class_context(&self) -> bool {
    false
  }

  fn needs_async_ref(&self) -> bool {
    false
  }

  fn needs_mut_ref(&self) -> bool {
    false
  }

  fn unwrap_optional(&self) -> Option<&ResolvedType> {
    None
  }

  fn as_class_input_info(&self) -> Option<(ClassInputKind, &TokenStream)> {
    None
  }

  fn special_tokens(&self) -> Option<&TokenStream> {
    None
  }
}

#[derive(Debug)]
pub struct ResolvedType {
  pub kind: Box<dyn TypeCodegen>,
  pub tokens: TokenStream,
}

impl ResolvedType {
  pub fn tag(&self) -> ResolvedTag {
    self.kind.tag()
  }
}

// ── Concrete kind types ─────────────────────────────────────────────

#[derive(Debug)]
pub struct ClassInputType {
  pub kind: ClassInputKind,
  pub class_tokens: TokenStream,
}

impl TypeCodegen for ClassInputType {
  fn tag(&self) -> ResolvedTag {
    ResolvedTag::ClassInput(self.kind)
  }

  fn emit_from_js(&self, source: TokenStream) -> TokenStream {
    let class = &self.class_tokens;
    match self.kind {
      ClassInputKind::Ref => quote! { frame.arg_reference::<#class>(#source)? },
      ClassInputKind::ClassRef => quote! { frame.arg_class_ref::<#class>(#source)? },
      ClassInputKind::Borrow => quote! { frame.arg_class::<#class>(#source)? },
      ClassInputKind::BorrowMut => quote! { frame.arg_class_mut::<#class>(#source)? },
    }
  }

  fn needs_class_context(&self) -> bool {
    true
  }

  fn needs_async_ref(&self) -> bool {
    self.kind.is_reference()
  }

  fn needs_mut_ref(&self) -> bool {
    self.kind.is_mut()
  }

  fn as_class_input_info(&self) -> Option<(ClassInputKind, &TokenStream)> {
    Some((self.kind, &self.class_tokens))
  }
}

#[derive(Debug)]
pub struct OptionalType {
  pub inner: Box<ResolvedType>,
}

impl TypeCodegen for OptionalType {
  fn tag(&self) -> ResolvedTag {
    ResolvedTag::Optional
  }

  fn emit_from_js(&self, source: TokenStream) -> TokenStream {
    let inner_conversion = self.inner.kind.emit_from_js(source.clone());
    quote! {
      {
        let value_type = napi::__private::callback_frame_arg_type(&frame, #source)?;
        if matches!(value_type, napi::bindgen_prelude::ValueType::Null | napi::bindgen_prelude::ValueType::Undefined) {
          None
        } else {
          Some(#inner_conversion)
        }
      }
    }
  }

  fn emit_into_js(&self, value: TokenStream) -> TokenStream {
    let inner_emit = self.inner.kind.emit_into_js(quote! { inner_val });
    quote! {
      match #value {
        Some(inner_val) => #inner_emit,
        None => napi::bindgen_prelude::IntoJs::into_js(
          napi::bindgen_prelude::Null,
          frame.scope_mut()
        ).map(|local| local.raw()),
      }
    }
  }

  fn needs_class_context(&self) -> bool {
    self.inner.kind.needs_class_context()
  }

  fn needs_async_ref(&self) -> bool {
    self.inner.kind.needs_async_ref()
  }

  fn unwrap_optional(&self) -> Option<&ResolvedType> {
    Some(&self.inner)
  }
}

#[derive(Debug)]
pub struct CallbackType {
  pub args: Vec<TokenStream>,
  pub ret: Option<TokenStream>,
}

impl TypeCodegen for CallbackType {
  fn tag(&self) -> ResolvedTag {
    ResolvedTag::Callback
  }

  fn emit_from_js(&self, source: TokenStream) -> TokenStream {
    quote! { frame.arg::<_>(#source)? }
  }
}

#[derive(Debug)]
pub struct SpecialType {
  pub kind: SpecialKind,
  pub tokens: TokenStream,
}

impl TypeCodegen for SpecialType {
  fn tag(&self) -> ResolvedTag {
    ResolvedTag::Special(self.kind)
  }

  fn emit_from_js(&self, source: TokenStream) -> TokenStream {
    match self.kind {
      SpecialKind::AbortSignal => quote! { frame.arg::<_>(#source)? },
      SpecialKind::External => quote! { frame.arg::<_>(#source)? },
      SpecialKind::JsArgSlice => quote! { frame.arg_slice() },
      SpecialKind::ClassInitializer | SpecialKind::ReturnThis => {
        quote! { compile_error!("ClassInitializer/ReturnThis are return-only types") }
      }
    }
  }

  fn emit_into_js(&self, value: TokenStream) -> TokenStream {
    let tokens = &self.tokens;
    match self.kind {
      SpecialKind::ReturnThis => quote! { Ok(cb.raw_this()) },
      SpecialKind::ClassInitializer => {
        quote! {
          napi::bindgen_prelude::IntoClassInitializer::<#tokens>::into_class_initializer(#value)
        }
      }
      _ => {
        quote! {
          napi::bindgen_prelude::IntoJs::into_js(#value, frame.scope_mut()).map(|local| local.raw())
        }
      }
    }
  }

  fn special_tokens(&self) -> Option<&TokenStream> {
    Some(&self.tokens)
  }
}

#[derive(Debug)]
pub struct GenericType {
  pub tokens: TokenStream,
}

impl TypeCodegen for GenericType {
  fn tag(&self) -> ResolvedTag {
    ResolvedTag::Generic
  }

  fn emit_from_js(&self, source: TokenStream) -> TokenStream {
    quote! { frame.arg::<_>(#source)? }
  }
}

#[derive(Debug)]
pub struct BorrowedRefType {
  pub mutable: bool,
  pub elem_tokens: TokenStream,
}

impl TypeCodegen for BorrowedRefType {
  fn tag(&self) -> ResolvedTag {
    ResolvedTag::BorrowedRef
  }

  fn emit_from_js(&self, source: TokenStream) -> TokenStream {
    let elem = &self.elem_tokens;
    if self.mutable {
      quote! { frame.arg::<&mut #elem>(#source)? }
    } else {
      quote! { frame.arg::<&#elem>(#source)? }
    }
  }

  fn needs_async_ref(&self) -> bool {
    true
  }

  fn needs_mut_ref(&self) -> bool {
    self.mutable
  }
}
