use std::marker::PhantomData;

use crate::{
  bindgen_runtime::{JsObjectValue, TypeName},
  check_status, sys, JsValue, Result, Value, ValueType,
};

#[derive(Clone, Copy)]
pub struct JsDate<'env>(pub(crate) Value, pub(crate) PhantomData<&'env ()>);

impl TypeName for JsDate<'_> {
  fn type_name() -> &'static str {
    "Date"
  }

  fn value_type() -> crate::ValueType {
    ValueType::Object
  }
}


impl<'env> JsValue<'env> for JsDate<'env> {
  fn value(&self) -> Value {
    self.0
  }
}

impl<'env> JsObjectValue<'env> for JsDate<'env> {}

impl JsDate<'_> {
  pub(crate) fn from_raw(env: sys::napi_env, value: sys::napi_value) -> Self {
    Self(
      Value {
        env,
        value,
        value_type: ValueType::Object,
      },
      PhantomData,
    )
  }

  pub fn value_of(&self) -> Result<f64> {
    let mut timestamp: f64 = 0.0;
    check_status!(unsafe { sys::napi_get_date_value(self.0.env, self.0.value, &mut timestamp) })?;
    Ok(timestamp)
  }
}
