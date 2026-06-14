use proc_macro2::{Ident, Span, TokenStream};

use crate::BindgenResult;

mod r#const;
mod r#enum;
mod r#fn;
mod r#struct;
mod r#type;

pub use r#struct::rm_raw_prefix;

pub const PROPERTY_ATTRIBUTE_DEFAULT: i32 = 0;
pub const PROPERTY_ATTRIBUTE_WRITABLE: i32 = 1 << 0;
pub const PROPERTY_ATTRIBUTE_ENUMERABLE: i32 = 1 << 1;
pub const PROPERTY_ATTRIBUTE_CONFIGURABLE: i32 = 1 << 2;

pub trait TryToTokens {
  fn try_to_tokens(&self, tokens: &mut TokenStream) -> BindgenResult<()>;

  fn try_to_token_stream(&self) -> BindgenResult<TokenStream> {
    let mut tokens = TokenStream::default();
    self.try_to_tokens(&mut tokens)?;

    Ok(tokens)
  }
}

fn get_intermediate_ident(name: &str) -> Ident {
  let new_name = format!("{name}_c_callback");
  Ident::new(&new_name, Span::call_site())
}

fn js_mod_to_token_stream(js_mod: Option<&String>) -> TokenStream {
  js_mod
    .map(|i| {
      let i = format!("{i}\0");
      quote! { Some(#i) }
    })
    .unwrap_or_else(|| quote! { None })
}

#[cfg(feature = "type-def")]
fn kind_to_tokens(kind: &str) -> TokenStream {
  match kind {
    "const" => quote! { napi::__private::TypeDefKind::Const },
    "enum" => quote! { napi::__private::TypeDefKind::Enum },
    "string_enum" => quote! { napi::__private::TypeDefKind::StringEnum },
    "interface" => quote! { napi::__private::TypeDefKind::Interface },
    "type" => quote! { napi::__private::TypeDefKind::Type },
    "fn" => quote! { napi::__private::TypeDefKind::Fn },
    "struct" => quote! { napi::__private::TypeDefKind::Struct },
    "non_constructible_class" => quote! { napi::__private::TypeDefKind::NonConstructibleClass },
    "iterator_extends" => quote! { napi::__private::TypeDefKind::IteratorExtends },
    "impl" => quote! { napi::__private::TypeDefKind::Impl },
    _ => panic!("unknown TypeDefKind: {kind}"),
  }
}

#[cfg(feature = "type-def")]
fn emit_type_def_descriptor(
  kind: &str,
  name: &str,
  original_name: Option<&str>,
  def_body: TokenStream,
  js_mod: Option<&String>,
  js_doc: &crate::typegen::JSDoc,
  native_parent_name: Option<TokenStream>,
  iterator_info: Option<TokenStream>,
  register_name: impl std::fmt::Display,
  span: Span,
) -> TokenStream {
  let kind_token = kind_to_tokens(kind);
  let js_mod_token = js_mod_to_token_stream(js_mod);

  let js_doc_str = js_doc.to_string();
  let js_doc_token = if js_doc_str.is_empty() {
    quote! { None }
  } else {
    quote! { Some(#js_doc_str) }
  };

  let original_name_token = match original_name {
    Some(s) => quote! { Some(#s) },
    None => quote! { None },
  };

  let native_parent_token = native_parent_name.unwrap_or_else(|| quote! { None });
  let iterator_info_token = iterator_info.unwrap_or_else(|| quote! { None });

  let def_fn_name = Ident::new(&format!("__napi_typedef_{register_name}_def__"), span);
  let register_ident = Ident::new(&format!("__napi_typedef_{register_name}__"), span);

  quote! {
    #[cfg(all(not(test), feature = "type-def"))]
    #[allow(non_snake_case, clippy::all)]
    fn #def_fn_name() -> String {
      #def_body
    }

    #[cfg(all(not(test), feature = "type-def"))]
    #[allow(non_upper_case_globals)]
    #[napi::__private::linkme::distributed_slice(napi::__private::TYPE_DEF_DESCRIPTORS)]
    #[linkme(crate = napi::__private::linkme)]
    static #register_ident: napi::__private::TypeDefDescriptor =
      napi::__private::TypeDefDescriptor {
        kind: #kind_token,
        name: #name,
        original_name: #original_name_token,
        def_fn: #def_fn_name,
        js_mod: #js_mod_token,
        js_doc: #js_doc_token,
        native_parent_name: #native_parent_token,
        iterator_info: #iterator_info_token,
      };
  }
}
