use proc_macro2::{Ident, Literal, TokenStream};
use quote::ToTokens;

use crate::{codegen::js_mod_to_token_stream, BindgenResult, NapiConst, TryToTokens};

impl TryToTokens for NapiConst {
  fn try_to_tokens(&self, tokens: &mut TokenStream) -> BindgenResult<()> {
    let register = self.gen_module_register();
    (quote! {
      #register
    })
    .to_tokens(tokens);

    Ok(())
  }
}

impl NapiConst {
  fn gen_module_register(&self) -> TokenStream {
    if cfg!(test) {
      return quote! {};
    }

    let name_ident = &self.name;
    let js_name_lit = Literal::string(&format!("{}\0", self.name));
    let register_name = &self.register_name;
    let cb_name = Ident::new(
      &format!("__register__const__{register_name}_callback__"),
      self.name.span(),
    );
    let js_mod_ident = js_mod_to_token_stream(self.js_mod.as_ref());

    quote! {
      #[allow(non_snake_case)]
      #[allow(clippy::all)]
      unsafe fn #cb_name(env: napi::sys::napi_env) -> napi::Result<napi::sys::napi_value> {
        let mut env_wrapper = unsafe { napi::bindgen_prelude::Env::from_raw(env) };
        env_wrapper.with_scope(|scope| {
          napi::bindgen_prelude::IntoJs::into_js(#name_ident, scope).map(|value| value.raw())
        })
      }
      #[cfg(not(test))]
      #[allow(non_upper_case_globals)]
      #[napi::__private::linkme::distributed_slice(napi::__private::MODULE_EXPORT_DESCRIPTORS)]
      #[linkme(crate = napi::__private::linkme)]
      static #register_name: napi::__private::ModuleExportDescriptor =
        napi::__private::ModuleExportDescriptor {
          js_mod: #js_mod_ident,
          js_name: #js_name_lit,
          callback: #cb_name,
        };
    }
  }
}
