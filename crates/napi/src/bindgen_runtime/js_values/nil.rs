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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nullable<T> {
  Undefined,
  Null,
  Value(T),
}

impl<T> Nullable<T> {
  pub fn into_value(self) -> Option<T> {
    match self {
      Nullable::Value(v) => Some(v),
      _ => None,
    }
  }

  pub fn as_value(&self) -> Option<&T> {
    match self {
      Nullable::Value(v) => Some(v),
      _ => None,
    }
  }

  pub fn is_undefined(&self) -> bool {
    matches!(self, Nullable::Undefined)
  }

  pub fn is_null(&self) -> bool {
    matches!(self, Nullable::Null)
  }

  pub fn is_value(&self) -> bool {
    matches!(self, Nullable::Value(_))
  }
}

impl<T> From<Nullable<T>> for Option<T> {
  fn from(value: Nullable<T>) -> Self {
    value.into_value()
  }
}

impl<T: TypeName> TypeName for Nullable<T> {
  fn type_name() -> &'static str {
    "Nullable"
  }

  fn value_type() -> ValueType {
    ValueType::Unknown
  }
}

impl<T: ValidateNapiValue> ValidateNapiValue for Nullable<T> {
  unsafe fn validate(env: sys::napi_env, napi_val: sys::napi_value) -> Result<sys::napi_value> {
    let mut result = -1;
    check_status!(
      unsafe { sys::napi_typeof(env, napi_val, &mut result) },
      "Failed to detect napi value type",
    )?;

    let received_type = ValueType::from(result);
    if received_type == ValueType::Null || received_type == ValueType::Undefined {
      Ok(ptr::null_mut())
    } else if let Ok(validate_ret) = unsafe { T::validate(env, napi_val) } {
      Ok(validate_ret)
    } else {
      Err(crate::Error::new(
        crate::Status::InvalidArg,
        format!(
          "Expect value to be Nullable<{}>, but received {}",
          T::value_type(),
          received_type
        ),
      ))
    }
  }
}

impl<'env, 'scope, T> FromJs<'env, 'scope> for Nullable<T>
where
  T: FromJs<'env, 'scope>,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let mut value_type = 0;
    check_status!(
      unsafe { sys::napi_typeof(scope.env().raw(), value.raw(), &mut value_type) },
      "Failed to detect Nullable value type",
    )?;

    match value_type {
      sys::ValueType::napi_undefined => Ok(Nullable::Undefined),
      sys::ValueType::napi_null => Ok(Nullable::Null),
      _ => T::from_js(scope, value).map(Nullable::Value),
    }
  }
}

impl<'scope, T> IntoJs<'scope> for Nullable<T>
where
  T: IntoJs<'scope> + 'scope,
{
  type Output = Unknown<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    match self {
      Nullable::Undefined => {
        let local = ().into_js(scope)?;
        Ok(unsafe { Local::from_raw(local.raw()) })
      }
      Nullable::Null => {
        let local = Null.into_js(scope)?;
        Ok(unsafe { Local::from_raw(local.raw()) })
      }
      Nullable::Value(value) => {
        let local = value.into_js(scope)?;
        Ok(unsafe { Local::from_raw(local.raw()) })
      }
    }
  }
}
