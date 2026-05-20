use std::marker::PhantomData;
use std::ptr;
use std::rc::{Rc, Weak};

use crate::{
  bindgen_runtime::{Env, EnvRecord, FromJs, IntoJs, Local, Scope, TypeName, ValidateNapiValue},
  check_status, sys, type_of, Error, JsValue, Result, Status, Value, ValueType,
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

impl ValidateNapiValue for Unknown<'_> {
  unsafe fn validate(
    _env: napi_sys::napi_env,
    _napi_val: napi_sys::napi_value,
  ) -> Result<sys::napi_value> {
    Ok(ptr::null_mut())
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

impl<'scope, const LEAK_CHECK: bool>
  crate::bindgen_runtime::JsRefTarget<'scope, UnknownRef<LEAK_CHECK>> for &Unknown<'_>
{
  fn create_ref(self, scope: &mut Scope<'_, 'scope>) -> Result<UnknownRef<LEAK_CHECK>> {
    scope.ensure_value_env(self.0.env, "Unknown")?;
    let mut ref_ = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_reference(scope.env().raw(), self.0.value, 1, &mut ref_) },
      "Failed to create reference"
    )?;
    Ok(UnknownRef {
      inner: ref_,
      record: Rc::downgrade(scope.required_record()?),
      not_send: PhantomData,
    })
  }
}

/// A reference to a unknown JavaScript value.
///
/// You must call the `unref` method to release the reference, or the object under the hood will be leaked forever.
///
/// Set the `LEAK_CHECK` to `false` to disable the leak check during the `Drop`
pub struct UnknownRef<const LEAK_CHECK: bool = true> {
  pub(crate) inner: sys::napi_ref,
  record: Weak<EnvRecord>,
  not_send: PhantomData<Rc<()>>,
}

impl<const LEAK_CHECK: bool> Drop for UnknownRef<LEAK_CHECK> {
  fn drop(&mut self) {
    if LEAK_CHECK && !self.inner.is_null() {
      eprintln!("ObjectRef is not unref, it considered as a memory leak");
    }
  }
}

impl<const LEAK_CHECK: bool> UnknownRef<LEAK_CHECK> {
  /// Get the object from the reference
  pub fn get_value<'env>(&self, env: &'env Env) -> Result<Unknown<'env>> {
    ensure_unknown_ref_owner(&self.record, env)?;
    let mut result = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_reference_value(env.0, self.inner, &mut result) },
      "Failed to get reference value"
    )?;
    Ok(unsafe { Unknown::from_raw_unchecked(env.0, result) })
  }

  /// Unref the reference
  pub fn unref(mut self, env: &Env) -> Result<()> {
    ensure_unknown_ref_owner(&self.record, env)?;
    check_status!(
      unsafe { sys::napi_reference_unref(env.0, self.inner, &mut 0) },
      "unref Ref failed"
    )?;
    check_status!(
      unsafe { sys::napi_delete_reference(env.0, self.inner) },
      "delete Ref failed"
    )?;
    self.inner = ptr::null_mut();
    Ok(())
  }
}

impl<'env, 'scope, const LEAK_CHECK: bool> FromJs<'env, 'scope> for UnknownRef<LEAK_CHECK> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let value = Unknown::from_js(scope, value)?;
    scope.create_ref(&value)
  }
}

impl<'scope, const LEAK_CHECK: bool> IntoJs<'scope> for &UnknownRef<LEAK_CHECK> {
  type Output = Unknown<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    ensure_unknown_ref_owner_record(&self.record, scope.required_record()?)?;
    let mut result = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_reference_value(env, self.inner, &mut result) },
      "Failed to get reference value"
    )?;
    Ok(unsafe { Local::from_raw(result) })
  }
}

impl<'scope, const LEAK_CHECK: bool> IntoJs<'scope> for UnknownRef<LEAK_CHECK> {
  type Output = Unknown<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    ensure_unknown_ref_owner_record(&self.record, scope.required_record()?)?;
    let mut result = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_reference_value(env, self.inner, &mut result) },
      "Failed to get reference value"
    )?;
    check_status!(
      unsafe { sys::napi_delete_reference(env, self.inner) },
      "Failed to delete reference"
    )?;
    Ok(unsafe { Local::from_raw(result) })
  }
}

fn ensure_unknown_ref_owner(record: &Weak<EnvRecord>, env: &Env) -> Result<()> {
  let owner = record.upgrade().ok_or_else(|| {
    Error::new(
      Status::InvalidArg,
      "UnknownRef owner environment is no longer available",
    )
  })?;
  let current = env.record();
  if Rc::ptr_eq(&owner, &current) {
    Ok(())
  } else {
    Err(Error::new(
      Status::InvalidArg,
      "UnknownRef owner environment does not match the current environment",
    ))
  }
}

fn ensure_unknown_ref_owner_record(
  record: &Weak<EnvRecord>,
  current: &Rc<EnvRecord>,
) -> Result<()> {
  let owner = record.upgrade().ok_or_else(|| {
    Error::new(
      Status::InvalidArg,
      "UnknownRef owner environment is no longer available",
    )
  })?;
  if Rc::ptr_eq(&owner, current) {
    Ok(())
  } else {
    Err(Error::new(
      Status::InvalidArg,
      "UnknownRef owner environment does not match the current environment",
    ))
  }
}
