use std::ptr;

use crate::{bindgen_prelude::*, check_status, sys, Result, ValueType};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct Null;
pub type Undefined = ();

impl TypeName for Null {
  fn type_name() -> &'static str {
    "null"
  }

  fn value_type() -> ValueType {
    ValueType::Null
  }
}

impl ValidateNapiValue for Null {}

impl<'env, 'scope> FromJs<'env, 'scope> for Null {
  fn from_js(_: &mut Scope<'env, 'scope>, _: Local<'scope, Unknown<'scope>>) -> Result<Self> {
    Ok(Null)
  }
}

impl<'scope> IntoJs<'scope> for Null {
  type Output = Null;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let mut ret = ptr::null_mut();

    check_status!(
      unsafe { sys::napi_get_null(env, &mut ret) },
      "Failed to create napi null value"
    )?;

    Ok(unsafe { Local::from_raw(ret) })
  }
}

impl<'scope> IntoJs<'scope> for &Null {
  type Output = Null;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    self.to_owned().into_js(scope)
  }
}

impl<'scope> IntoJs<'scope> for &mut Null {
  type Output = Null;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    self.to_owned().into_js(scope)
  }
}

impl TypeName for Undefined {
  fn type_name() -> &'static str {
    "undefined"
  }

  fn value_type() -> ValueType {
    ValueType::Undefined
  }
}

impl ValidateNapiValue for Undefined {}

impl<'env, 'scope> FromJs<'env, 'scope> for Undefined {
  fn from_js(_: &mut Scope<'env, 'scope>, _: Local<'scope, Unknown<'scope>>) -> Result<Self> {
    Ok(())
  }
}

impl<'scope> IntoJs<'scope> for Undefined {
  type Output = Undefined;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let mut ret = ptr::null_mut();

    check_status!(
      unsafe { sys::napi_get_undefined(env, &mut ret) },
      "Failed to create napi undefined value"
    )?;

    Ok(unsafe { Local::from_raw(ret) })
  }
}

impl<'scope> IntoJs<'scope> for &Undefined {
  type Output = Undefined;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    ().into_js(scope)
  }
}

impl<'scope> IntoJs<'scope> for &mut Undefined {
  type Output = Undefined;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    ().into_js(scope)
  }
}
