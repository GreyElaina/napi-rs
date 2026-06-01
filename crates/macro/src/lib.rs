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
use quote::format_ident;
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
  let init_name = &input.sig.ident;
  let descriptor_name = format_ident!("__napi_module_init_{init_name}");
  quote! {
    #input

    #[cfg(not(test))]
    #[doc(hidden)]
    #[allow(non_upper_case_globals)]
    #[napi::__private::linkme::distributed_slice(napi::__private::MODULE_INIT_DESCRIPTORS)]
    #[linkme(crate = napi::__private::linkme)]
    static #descriptor_name: napi::__private::ModuleInitDescriptor =
      napi::__private::ModuleInitDescriptor {
        init: #init_name,
      };
  }
  .into()
}
