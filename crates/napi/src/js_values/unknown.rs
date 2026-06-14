use std::sync::Arc;

use crate::{
  bindgen_runtime::{
    js_values::value_ref::{
      create_reference, delete_reference, ensure_deferred_match_env, ensure_same_deferred,
      reference_value, RefState,
    },
    Env, FromJs, IntoJs, JsRefTarget, Local, Ref, Scope, TypeName, Unk,
  },
  sys, type_of, JsValue, Result, Value, ValueType,
};

#[derive(Clone, Copy)]
/// Represents a raw JavaScript value
pub struct Unknown<'env>(
  pub(crate) Value,
  pub(crate) std::marker::PhantomData<&'env ()>,
);

impl<'env> JsValue<'env> for Unknown<'env> {
  fn value(&self) -> Value {
    self.0
  }
}

impl TypeName for Unknown<'_> {
  fn type_name() -> &'static str {
    "unknown"
  }

  fn value_type() -> ValueType {
    ValueType::Unknown
  }
}

impl Unknown<'_> {
  pub fn get_type(&self) -> Result<ValueType> {
    type_of!(self.0.env, self.0.value)
  }

  /// Unknown doesn't have a type
  ///
  /// # Safety
  ///
  /// The caller must ensure that:
  /// - The `env` is a valid napi env pointer
  /// - The `napi_val` is a valid js value pointer
  pub unsafe fn from_raw_unchecked(env: sys::napi_env, value: sys::napi_value) -> Self {
    Unknown(
      Value {
        env,
        value,
        value_type: ValueType::Unknown,
      },
      std::marker::PhantomData,
    )
  }

  #[cfg(feature = "serde-json")]
  pub(crate) fn into_local<'scope>(self) -> Local<'scope, Unknown<'scope>> {
    unsafe { Local::from_raw(self.0.value) }
  }
}

pub type UnknownRef = Ref<Unk>;

impl<'scope> JsRefTarget<'scope, Ref<Unk>> for &Unknown<'_> {
  fn create_ref(self, scope: &mut Scope<'_, 'scope>) -> Result<Ref<Unk>> {
    scope.ensure_value_env(self.0.env, "Unknown")?;
    let raw = create_reference(scope.env().raw(), self.0.value, 1)?;
    Ok(Ref::new(
      RefState::new(raw, Arc::clone(scope.deferred_queue())),
      (),
    ))
  }
}

impl Ref<Unk> {
  pub fn to_local<'env>(&self, env: &'env Env) -> Result<Unknown<'env>> {
    ensure_deferred_match_env(&self.state, env)?;
    let result = reference_value(env.0, self.state.raw_ref()?)?;
    Ok(unsafe { Unknown::from_raw_unchecked(env.0, result) })
  }

  pub fn close(self, env: &Env) -> Result<()> {
    ensure_deferred_match_env(&self.state, env)?;
    delete_reference(env.0, self.state.take_raw()?)
  }
}

impl<'env, 'scope> FromJs<'env, 'scope> for Ref<Unk> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let value = Unknown::from_js(scope, value)?;
    scope.create_ref(&value)
  }
}

impl<'scope> IntoJs<'scope> for &Ref<Unk> {
  type Output = Unknown<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    ensure_same_deferred(&self.state, scope)?;
    let result = reference_value(scope.env().raw(), self.state.raw_ref()?)?;
    Ok(unsafe { Local::from_raw(result) })
  }
}

impl<'scope> IntoJs<'scope> for Ref<Unk> {
  type Output = Unknown<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    ensure_same_deferred(&self.state, scope)?;
    let raw_ref = self.state.raw_ref()?;
    let result = reference_value(scope.env().raw(), raw_ref)?;
    delete_reference(scope.env().raw(), self.state.take_raw()?)?;
    Ok(unsafe { Local::from_raw(result) })
  }
}
