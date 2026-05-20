use std::fmt::{self, Display};
use std::ptr;

use crate::{
  bindgen_runtime::{EscapableHandleScope, Local, Object, Unknown},
  {check_status, sys, JsNumber, JsString, Result, ValueType},
};

#[derive(Debug, Clone, Copy)]
pub struct Value {
  pub(crate) env: sys::napi_env,
  pub(crate) value: sys::napi_value,
  pub(crate) value_type: ValueType,
}

impl Display for Value {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Value({:?})", self.value_type)
  }
}

pub trait JsValue<'env>: Sized {
  fn value(&self) -> Value;

  #[doc(hidden)]
  unsafe fn raw(&self) -> sys::napi_value {
    self.value().value
  }

  /// Convert the value to an unknown
  fn to_unknown(&self) -> Unknown<'env> {
    Unknown(
      Value {
        env: self.value().env,
        value: self.value().value,
        value_type: ValueType::Unknown,
      },
      std::marker::PhantomData,
    )
  }

  /// Coerce the value to a boolean
  fn coerce_to_bool(&self) -> Result<bool> {
    let mut new_raw_value = ptr::null_mut();
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_coerce_to_bool(env, self.value().value, &mut new_raw_value)
    })?;
    let mut result = false;
    check_status!(unsafe { sys::napi_get_value_bool(env, new_raw_value, &mut result) })?;
    Ok(result)
  }

  fn coerce_to_number(&self) -> Result<JsNumber<'_>> {
    let mut new_raw_value = ptr::null_mut();
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_coerce_to_number(env, self.value().value, &mut new_raw_value)
    })?;
    Ok(JsNumber(
      Value {
        env,
        value: new_raw_value,
        value_type: ValueType::Number,
      },
      std::marker::PhantomData,
    ))
  }

  fn coerce_to_string(&self) -> Result<JsString<'_>> {
    let mut new_raw_value = ptr::null_mut();
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_coerce_to_string(env, self.value().value, &mut new_raw_value)
    })?;
    Ok(JsString(
      Value {
        env,
        value: new_raw_value,
        value_type: ValueType::String,
      },
      std::marker::PhantomData,
    ))
  }

  fn coerce_to_object(&self) -> Result<Object<'env>> {
    let mut new_raw_value = ptr::null_mut();
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_coerce_to_object(env, self.value().value, &mut new_raw_value)
    })?;
    Ok(unsafe { Object::from_raw(env, new_raw_value) })
  }

  #[cfg(feature = "napi5")]
  fn is_date(&self) -> Result<bool> {
    let mut is_date = true;
    let env = self.value().env;
    check_status!(unsafe { sys::napi_is_date(env, self.value().value, &mut is_date) })?;
    Ok(is_date)
  }

  fn is_promise(&self) -> Result<bool> {
    let mut is_promise = true;
    let env = self.value().env;
    check_status!(unsafe { sys::napi_is_promise(env, self.value().value, &mut is_promise) })?;
    Ok(is_promise)
  }

  fn is_error(&self) -> Result<bool> {
    let mut result = false;
    let env = self.value().env;
    check_status!(unsafe { sys::napi_is_error(env, self.value().value, &mut result) })?;
    Ok(result)
  }

  fn is_typedarray(&self) -> Result<bool> {
    let mut result = false;
    let env = self.value().env;
    check_status!(unsafe { sys::napi_is_typedarray(env, self.value().value, &mut result) })?;
    Ok(result)
  }

  fn is_dataview(&self) -> Result<bool> {
    let mut result = false;
    let env = self.value().env;
    check_status!(unsafe { sys::napi_is_dataview(env, self.value().value, &mut result) })?;
    Ok(result)
  }

  fn is_array(&self) -> Result<bool> {
    let mut is_array = false;
    let env = self.value().env;
    check_status!(unsafe { sys::napi_is_array(env, self.value().value, &mut is_array) })?;
    Ok(is_array)
  }

  fn is_buffer(&self) -> Result<bool> {
    let mut is_buffer = false;
    let env = self.value().env;
    check_status!(unsafe { sys::napi_is_buffer(env, self.value().value, &mut is_buffer) })?;
    Ok(is_buffer)
  }

  fn is_arraybuffer(&self) -> Result<bool> {
    let mut is_buffer = false;
    let env = self.value().env;
    check_status!(unsafe { sys::napi_is_arraybuffer(env, self.value().value, &mut is_buffer) })?;
    Ok(is_buffer)
  }

  fn instanceof<'c, Constructor>(&self, constructor: Constructor) -> Result<bool>
  where
    Constructor: JsValue<'c>,
  {
    let mut result = false;
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_instanceof(env, self.value().value, constructor.raw(), &mut result)
    })?;
    Ok(result)
  }

  fn escape<'scope>(
    &self,
    escapable_handle_scope: EscapableHandleScope<'scope>,
  ) -> Result<Local<'scope, Unknown<'scope>>> {
    let mut result = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_escape_handle(
          escapable_handle_scope.env,
          escapable_handle_scope.scope,
          self.raw(),
          &mut result,
        )
      },
      "Failed to escape handle"
    )?;
    Ok(unsafe { Local::from_raw(result) })
  }
}
