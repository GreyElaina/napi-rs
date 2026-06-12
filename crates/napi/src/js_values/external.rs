use std::{marker::PhantomData, ptr, sync::Arc};

use crate::{
  bindgen_prelude::{
    sys, Ext, External, FromJs, IntoJs, Local, Ref, Result, Scope, Status, TypeName, Unknown,
  },
  bindgen_runtime::js_values::value_ref::{create_reference, RefState},
  check_status, Error, JsValue, Value, ValueType,
};

/// Represent the Node-API `External` value
///
/// The difference between the `JsExternal` and `External` is that the `JsExternal` holds the raw value of `External`.
/// So that you can call `Object::set_property` with the `JsExternal` value, but can't do the same with `External`.
pub struct JsExternal<'env>(pub(crate) Value, PhantomData<&'env ()>);

impl<'env> TypeName for JsExternal<'env> {
  fn type_name() -> &'static str {
    "External"
  }

  fn value_type() -> ValueType {
    ValueType::External
  }
}


impl<'env, 'scope> FromJs<'env, 'scope> for JsExternal<'scope> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(Self(
      Value {
        env: scope.env().raw(),
        value: value.raw(),
        value_type: ValueType::External,
      },
      PhantomData,
    ))
  }
}

impl<'env> JsValue<'env> for JsExternal<'env> {
  fn value(&self) -> Value {
    self.0
  }
}

impl<'env> JsExternal<'env> {
  pub(crate) unsafe fn from_raw(env: sys::napi_env, value: sys::napi_value) -> Self {
    Self(
      Value {
        env,
        value,
        value_type: ValueType::External,
      },
      PhantomData,
    )
  }

  /// Get the value from the `JsExternal`
  ///
  /// If the underlying value is not `T`, it will return `InvalidArg` error.
  pub fn get_value<T: 'static>(&self) -> Result<&mut T> {
    self.get_static_value::<T>().map(|ext| ext.as_mut())
  }

  #[inline]
  fn get_static_value<T: 'static>(&self) -> Result<&'static mut External<T>> {
    let mut unknown_tagged_object = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_value_external(self.0.env, self.0.value, &mut unknown_tagged_object) },
      "Failed to get external value"
    )?;

    match unsafe { External::from_raw_impl(unknown_tagged_object) } {
      Some(external) => Ok(external),
      None => Err(Error::new(
        Status::InvalidArg,
        format!(
          "<{}> on `External` is not the type of wrapped object",
          std::any::type_name::<T>()
        ),
      )),
    }
  }

  fn validate_type<T: 'static>(&self) -> Result<()> {
    let mut unknown_tagged_object = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_value_external(self.0.env, self.0.value, &mut unknown_tagged_object) },
      "Failed to get external value"
    )?;

    let type_id = unknown_tagged_object as *const std::any::TypeId;
    if unsafe { *type_id } == std::any::TypeId::of::<T>() {
      Ok(())
    } else {
      Err(Error::new(
        Status::InvalidArg,
        format!(
          "<{}> on `External` is not the type of wrapped object",
          std::any::type_name::<T>()
        ),
      ))
    }
  }
}

impl<'scope, T: 'static> crate::bindgen_runtime::JsRefTarget<'scope, Ref<Ext<T>>>
  for &JsExternal<'_>
{
  fn create_ref(self, scope: &mut Scope<'_, 'scope>) -> Result<Ref<Ext<T>>> {
    scope.ensure_value_env(self.0.env, "External")?;
    self.validate_type::<T>()?;
    let raw = create_reference(scope.env().raw(), self.0.value, 1)?;
    Ok(Ref::new(
      RefState::new(raw, Arc::clone(scope.deferred_queue())),
      (),
    ))
  }
}

impl<'scope, T: 'static> crate::bindgen_runtime::JsRefTarget<'scope, Ref<Ext<T>>> for External<T> {
  fn create_ref(self, scope: &mut Scope<'_, 'scope>) -> Result<Ref<Ext<T>>> {
    let value = self.into_js(scope)?;
    let raw = create_reference(scope.env().raw(), value.raw(), 1)?;
    Ok(Ref::new(
      RefState::new(raw, Arc::clone(scope.deferred_queue())),
      (),
    ))
  }
}
