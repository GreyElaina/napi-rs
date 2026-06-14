use std::ptr;

use crate::{
  bindgen_runtime::{FnArgs, FromJs, Function, IntoJs, JsObjectValue, Local, Scope, Unknown},
  check_pending_exception, check_status, sys, JsValue, Result, Value, ValueType,
};

pub struct JsGlobal<'env>(
  pub(crate) Value,
  pub(crate) std::marker::PhantomData<&'env ()>,
);

impl<'env> JsValue<'env> for JsGlobal<'env> {
  fn value(&self) -> Value {
    self.0
  }
}

impl<'env> JsObjectValue<'env> for JsGlobal<'env> {}

impl crate::bindgen_runtime::TypeName for JsGlobal<'_> {
  fn type_name() -> &'static str {
    "JsGlobal"
  }

  fn value_type() -> crate::ValueType {
    crate::ValueType::Object
  }

  fn ts_type() -> String {
    "typeof global".to_owned()
  }
}

impl<'env, 'scope> FromJs<'env, 'scope> for JsGlobal<'scope> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(JsGlobal(
      Value {
        env: scope.env().raw(),
        value: value.raw(),
        value_type: ValueType::Object,
      },
      std::marker::PhantomData,
    ))
  }
}

pub struct JsTimeout<'env>(
  pub(crate) Value,
  pub(crate) std::marker::PhantomData<&'env ()>,
);

impl<'env> JsValue<'env> for JsTimeout<'env> {
  fn value(&self) -> Value {
    self.0
  }
}

impl<'env> JsObjectValue<'env> for JsTimeout<'env> {}

impl<'env, 'scope> FromJs<'env, 'scope> for JsTimeout<'scope> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(JsTimeout(
      Value {
        env: scope.env().raw(),
        value: value.raw(),
        value_type: ValueType::Object,
      },
      std::marker::PhantomData,
    ))
  }
}

pub struct JSON<'env>(
  pub(crate) Value,
  pub(crate) std::marker::PhantomData<&'env ()>,
);

impl<'env> JsValue<'env> for JSON<'env> {
  fn value(&self) -> Value {
    self.0
  }
}

impl<'env> JsObjectValue<'env> for JSON<'env> {}

impl<'env, 'scope> FromJs<'env, 'scope> for JSON<'scope> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(JSON(
      Value {
        env: scope.env().raw(),
        value: value.raw(),
        value_type: ValueType::Object,
      },
      std::marker::PhantomData,
    ))
  }
}

impl JSON<'_> {
  pub fn stringify<'env, 'scope, V>(
    &self,
    scope: &mut Scope<'env, 'scope>,
    value: V,
  ) -> Result<std::string::String>
  where
    V: IntoJs<'scope> + 'scope,
  {
    let raw_value = value.into_js(scope)?.raw();
    let raw_string = unsafe { json_stringify_raw(self.0.env, self.0.value, raw_value) }?;
    let value = unsafe { Local::<Unknown<'scope>>::from_raw(raw_string) };
    String::from_js(scope, value)
  }
}

type SupportType<'a> = Function<'a, FnArgs<(Function<'a, (), Unknown<'a>>, f64)>, JsTimeout<'a>>;

impl<'env> JsGlobal<'env> {
  pub fn set_interval<'scope>(
    &self,
    scope: &mut Scope<'env, 'scope>,
    handler: Function<'scope, (), Unknown<'scope>>,
    interval: f64,
  ) -> Result<JsTimeout<'scope>> {
    let func: SupportType<'scope> = scope.get_named_property(self, "setInterval")?;
    scope.call(
      &func,
      FnArgs {
        data: (handler, interval),
      },
    )
  }

  pub fn clear_interval<'scope>(
    &self,
    scope: &mut Scope<'env, 'scope>,
    timer: JsTimeout<'scope>,
  ) -> Result<()> {
    let func: Function<'scope, JsTimeout<'scope>, ()> =
      scope.get_named_property(self, "clearInterval")?;
    scope.call(&func, timer)
  }

  pub fn set_timeout<'scope>(
    &self,
    scope: &mut Scope<'env, 'scope>,
    handler: Function<'scope, (), Unknown<'scope>>,
    interval: f64,
  ) -> Result<JsTimeout<'scope>> {
    let func: SupportType<'scope> = scope.get_named_property(self, "setTimeout")?;
    scope.call(
      &func,
      FnArgs {
        data: (handler, interval),
      },
    )
  }

  pub fn clear_timeout<'scope>(
    &self,
    scope: &mut Scope<'env, 'scope>,
    timer: JsTimeout<'scope>,
  ) -> Result<()> {
    let func: Function<'scope, JsTimeout<'scope>, ()> =
      scope.get_named_property(self, "clearTimeout")?;
    scope.call(&func, timer)
  }
}

unsafe fn json_stringify_raw(
  env: sys::napi_env,
  json: sys::napi_value,
  value: sys::napi_value,
) -> Result<sys::napi_value> {
  let mut stringify = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_get_named_property(env, json, c"stringify".as_ptr(), &mut stringify) },
    "Get JSON.stringify failed"
  )?;
  let mut raw_return = ptr::null_mut();
  check_pending_exception!(
    env,
    unsafe { sys::napi_call_function(env, json, stringify, 1, [value].as_ptr(), &mut raw_return) },
    "Call JSON.stringify failed"
  )?;
  Ok(raw_return)
}
