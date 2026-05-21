#[cfg(feature = "napi6")]
use std::ptr;

use serde::Serialize;
use serde_json::{Map, Number, Value};

use crate::{
  bindgen_runtime::{EnvRecord, Object},
  check_status, sys, type_of, Error, Result, Ser, Status, ValueType,
};

#[cfg(feature = "napi6")]
use super::BigInt;
use super::{FromJs, IntoJs, Local, Scope, Unknown};

fn serialize_to_unknown<'scope, T>(
  scope: &mut Scope<'_, 'scope>,
  value: &T,
) -> Result<Local<'scope, Unknown<'scope>>>
where
  T: ?Sized + Serialize,
{
  let value = value.serialize(Ser::new(scope.env()))?;
  Ok(unsafe { Unknown::from_raw_unchecked(value.env, value.value) }.into_local())
}

impl<'env, 'scope> FromJs<'env, 'scope> for Value {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let env = scope.env().raw();
    let ty = type_of!(env, value.raw())?;
    let val = match ty {
      ValueType::Boolean => Value::Bool(bool::from_js(scope, value)?),
      ValueType::Number => Value::Number(Number::from_js(scope, value)?),
      ValueType::String => Value::String(String::from_js(scope, value)?),
      ValueType::Object => {
        let mut is_arr = false;
        check_status!(
          unsafe { sys::napi_is_array(env, value.raw(), &mut is_arr) },
          "Failed to detect whether given js is an array"
        )?;

        if is_arr {
          Value::Array(Vec::<Value>::from_js(scope, value)?)
        } else {
          Value::Object(Map::<String, Value>::from_js(scope, value)?)
        }
      }
      #[cfg(feature = "napi6")]
      ValueType::BigInt => {
        let n = BigInt::from_js(scope, value)?;
        if n.sign_bit {
          let (v, lossless) = n.get_i64();
          if lossless {
            Value::Number(v.into())
          } else {
            Value::String(to_string(env, value.raw())?)
          }
        } else {
          let (_, v, lossless) = n.get_u64();
          if lossless {
            Value::Number(v.into())
          } else {
            Value::String(to_string(env, value.raw())?)
          }
        }
      }
      ValueType::Null => Value::Null,
      ValueType::Function => {
        return Err(Error::new(
          Status::InvalidArg,
          "JS functions cannot be represented as a serde_json::Value".to_owned(),
        ))
      }
      ValueType::Undefined => {
        return Err(Error::new(
          Status::InvalidArg,
          "undefined cannot be represented as a serde_json::Value".to_owned(),
        ))
      }
      ValueType::Symbol => {
        return Err(Error::new(
          Status::InvalidArg,
          "JS symbols cannot be represented as a serde_json::Value".to_owned(),
        ))
      }
      ValueType::External => {
        return Err(Error::new(
          Status::InvalidArg,
          "External JS objects cannot be represented as a serde_json::Value".to_owned(),
        ))
      }
      _ => {
        return Err(Error::new(
          Status::InvalidArg,
          "Unknown JS variables cannot be represented as a serde_json::Value".to_owned(),
        ))
      }
    };

    Ok(val)
  }
}

#[cfg(feature = "napi6")]
fn to_string(env: sys::napi_env, napi_val: sys::napi_value) -> Result<String> {
  let mut string = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_coerce_to_string(env, napi_val, &mut string) },
    "Failed to coerce to string"
  )?;
  unsafe { EnvRecord::enter_scope(env, |scope| String::from_js(scope, Local::from_raw(string))) }
}

impl<'env, 'scope> FromJs<'env, 'scope> for Map<String, Value> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let obj = unsafe { Object::from_raw(scope.env().raw(), value.raw()) };
    let mut map = Map::new();
    for key in scope.keys(&obj)?.into_iter() {
      if let Some(val) = scope.get_optional_named_property::<Value, _>(&obj, &key)? {
        map.insert(key, val);
      }
    }

    Ok(map)
  }
}

impl<'scope> IntoJs<'scope> for &'scope Value {
  type Output = Unknown<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    serialize_to_unknown(scope, self)
  }
}

impl<'scope> IntoJs<'scope> for Value {
  type Output = Unknown<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    serialize_to_unknown(scope, &self)
  }
}

impl<'scope> IntoJs<'scope> for &'scope Map<String, Value> {
  type Output = Unknown<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    serialize_to_unknown(scope, self)
  }
}

impl<'scope> IntoJs<'scope> for Map<String, Value> {
  type Output = Unknown<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    serialize_to_unknown(scope, &self)
  }
}

impl<'scope> IntoJs<'scope> for &'scope Number {
  type Output = Unknown<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    serialize_to_unknown(scope, self)
  }
}

impl<'scope> IntoJs<'scope> for Number {
  type Output = Unknown<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    serialize_to_unknown(scope, &self)
  }
}

impl<'env, 'scope> FromJs<'env, 'scope> for Number {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let n = f64::from_js(scope, value)?;
    let n = if n.trunc() == n {
      if n >= 0.0f64 && n <= u32::MAX as f64 {
        Some(Number::from(n as u32))
      } else if n < 0.0f64 && n >= i32::MIN as f64 {
        Some(Number::from(n as i32))
      } else {
        Number::from_f64(n)
      }
    } else {
      Number::from_f64(n)
    };

    let n = n.ok_or_else(|| {
      Error::new(
        Status::InvalidArg,
        "Failed to convert js number to serde_json::Number".to_owned(),
      )
    })?;

    Ok(n)
  }
}
