use std::ptr;

use crate::{
  bindgen_runtime::{
    Env, FromJs, IntoJs, Local, Result, Scope, TypeName, Unknown, ValidateNapiValue,
  },
  check_status, sys, JsSymbol,
};

pub struct Symbol {
  desc: Option<String>,
  #[cfg(feature = "napi9")]
  for_desc: Option<String>,
}

impl TypeName for Symbol {
  fn type_name() -> &'static str {
    "Symbol"
  }

  fn value_type() -> crate::ValueType {
    crate::ValueType::Symbol
  }
}

impl ValidateNapiValue for Symbol {}

impl Symbol {
  pub fn new<S: ToString>(desc: S) -> Self {
    Self {
      desc: Some(desc.to_string()),
      #[cfg(feature = "napi9")]
      for_desc: None,
    }
  }

  pub fn identity() -> Self {
    Self {
      desc: None,
      #[cfg(feature = "napi9")]
      for_desc: None,
    }
  }

  #[cfg(feature = "napi9")]
  pub fn for_desc<S: AsRef<str>>(desc: S) -> Self {
    Self {
      desc: None,
      for_desc: Some(desc.as_ref().to_owned()),
    }
  }

  /// Convert `Symbol` to `JsSymbol`
  pub fn into_js_symbol<'env>(self, env: &'env Env) -> Result<JsSymbol<'env>> {
    let mut env = *env;
    env.with_scope(|scope| {
      let symbol = self.into_js(scope)?;
      Ok(JsSymbol(
        crate::Value {
          env: scope.env().raw(),
          value: symbol.raw(),
          value_type: crate::ValueType::Symbol,
        },
        std::marker::PhantomData,
      ))
    })
  }
}

impl<'scope> IntoJs<'scope> for Symbol {
  type Output = JsSymbol<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> crate::Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let mut symbol_value = ptr::null_mut();
    #[cfg(feature = "napi9")]
    if let Some(desc) = self.for_desc {
      check_status!(
        unsafe {
          sys::node_api_symbol_for(
            env,
            desc.as_ptr().cast(),
            desc.len() as isize,
            &mut symbol_value,
          )
        },
        "Failed to call node_api_symbol_for"
      )?;
      return Ok(unsafe { Local::from_raw(symbol_value) });
    }
    check_status!(unsafe {
      sys::napi_create_symbol(
        env,
        match self.desc {
          Some(desc) => {
            let mut desc_string = ptr::null_mut();
            let desc_len = desc.len();
            check_status!(sys::napi_create_string_utf8(
              env,
              desc.as_ptr().cast(),
              desc_len as isize,
              &mut desc_string
            ))?;
            desc_string
          }
          None => ptr::null_mut(),
        },
        &mut symbol_value,
      )
    })?;
    Ok(unsafe { Local::from_raw(symbol_value) })
  }
}

impl<'env, 'scope> FromJs<'env, 'scope> for Symbol {
  fn from_js(
    _: &mut Scope<'env, 'scope>,
    _: Local<'scope, Unknown<'scope>>,
  ) -> crate::Result<Self> {
    Ok(Self {
      desc: None,
      #[cfg(feature = "napi9")]
      for_desc: None,
    })
  }
}
