mod expand;
#[cfg(not(feature = "noop"))]
mod parser;

#[cfg(not(feature = "noop"))]
#[macro_use]
extern crate napi_derive_backend;
#[macro_use]
extern crate quote;

use std::env;

use proc_macro::TokenStream;
use syn::{parse_macro_input, ItemFn};

/// ```ignore
/// #[napi]
/// fn test(name: String) {
///   "hello" + name
/// }
/// ```
#[proc_macro_attribute]
pub fn napi(attr: TokenStream, input: TokenStream) -> TokenStream {
  match expand::expand(attr.into(), input.into()) {
    Ok(tokens) => {
      if env::var("NAPI_DEBUG_GENERATED_CODE").is_ok() {
        println!("{tokens}");
      }
      tokens.into()
    }
    Err(diagnostic) => {
      println!("`napi` macro expand failed.");

      (quote! { #diagnostic }).into()
    }
  }
}

#[proc_macro_attribute]
pub fn module_init(_: TokenStream, input: TokenStream) -> TokenStream {
  let input = parse_macro_input!(input as ItemFn);
  quote! {
    napi::ctor::declarative::ctor! {
      #[ctor(unsafe)]
      #input
    }
  }
  .into()
}
