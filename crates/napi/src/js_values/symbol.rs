use std::marker::PhantomData;
use std::ptr;
use std::rc::{Rc, Weak};

use crate::{
  bindgen_runtime::{
    Env, EnvRecord, FromJs, IntoJs, Local, Scope, Symbol, TypeName, Unknown, ValidateNapiValue,
  },
  check_status, sys, Error, JsValue, Result, Status, Value, ValueType,
};

#[derive(Clone, Copy)]
/// represent `Symbol` value in JavaScript
pub struct JsSymbol<'env>(
  pub(crate) Value,
  pub(crate) std::marker::PhantomData<&'env ()>,
);

impl TypeName for JsSymbol<'_> {
  fn type_name() -> &'static str {
    "symbol"
  }

  fn value_type() -> ValueType {
    ValueType::Symbol
  }
}

impl<'env> JsValue<'env> for JsSymbol<'env> {
  fn value(&self) -> Value {
    self.0
  }
}

impl<'env, 'scope> FromJs<'env, 'scope> for JsSymbol<'scope> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(JsSymbol(
      Value {
        env: scope.env().raw(),
        value: value.raw(),
        value_type: ValueType::Symbol,
      },
      PhantomData,
    ))
  }
}

impl ValidateNapiValue for JsSymbol<'_> {}

impl<'scope, const LEAK_CHECK: bool>
  crate::bindgen_runtime::JsRefTarget<'scope, SymbolRef<LEAK_CHECK>> for &JsSymbol<'_>
{
  fn create_ref(self, scope: &mut Scope<'_, 'scope>) -> Result<SymbolRef<LEAK_CHECK>> {
    scope.ensure_value_env(self.0.env, "Symbol")?;
    let mut ref_ = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_reference(scope.env().raw(), self.0.value, 1, &mut ref_) },
      "Failed to create reference"
    )?;
    Ok(SymbolRef {
      inner: ref_,
      record: Rc::downgrade(scope.record()),
      not_send: PhantomData,
    })
  }
}

impl<'scope, const LEAK_CHECK: bool>
  crate::bindgen_runtime::JsRefTarget<'scope, SymbolRef<LEAK_CHECK>> for Symbol
{
  fn create_ref(self, scope: &mut Scope<'_, 'scope>) -> Result<SymbolRef<LEAK_CHECK>> {
    let value = self.into_js(scope)?;
    let mut ref_ = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_reference(scope.env().raw(), value.raw(), 1, &mut ref_) },
      "Failed to create reference"
    )?;
    Ok(SymbolRef {
      inner: ref_,
      record: Rc::downgrade(scope.record()),
      not_send: PhantomData,
    })
  }
}

/// A reference to a JavaScript Symbol.
///
/// You must call the `unref` method to release the reference, or the symbol under the hood will be leaked forever.
///
/// Set the `LEAK_CHECK` to `false` to disable the leak check during the `Drop`
pub struct SymbolRef<const LEAK_CHECK: bool = true> {
  pub(crate) inner: sys::napi_ref,
  record: Weak<EnvRecord>,
  not_send: PhantomData<Rc<()>>,
}

impl<const LEAK_CHECK: bool> Drop for SymbolRef<LEAK_CHECK> {
  fn drop(&mut self) {
    if LEAK_CHECK && !self.inner.is_null() {
      eprintln!("ObjectRef is not unref, it considered as a memory leak");
    }
  }
}

impl<const LEAK_CHECK: bool> SymbolRef<LEAK_CHECK> {
  /// Get the object from the reference
  pub fn get_value<'env>(&self, env: &'env Env) -> Result<JsSymbol<'env>> {
    ensure_symbol_ref_owner(&self.record, env)?;
    let mut result = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_reference_value(env.0, self.inner, &mut result) },
      "Failed to get reference value"
    )?;
    Ok(JsSymbol(
      Value {
        env: env.0,
        value: result,
        value_type: ValueType::Symbol,
      },
      PhantomData,
    ))
  }

  /// Unref the reference
  pub fn unref(mut self, env: &Env) -> Result<()> {
    ensure_symbol_ref_owner(&self.record, env)?;
    check_status!(
      unsafe { sys::napi_delete_reference(env.0, self.inner) },
      "delete Ref failed"
    )?;
    self.inner = ptr::null_mut();
    Ok(())
  }
}

impl<'env, 'scope, const LEAK_CHECK: bool> FromJs<'env, 'scope> for SymbolRef<LEAK_CHECK> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let value = JsSymbol::from_js(scope, value)?;
    scope.create_ref(&value)
  }
}

impl<'scope, const LEAK_CHECK: bool> IntoJs<'scope> for &SymbolRef<LEAK_CHECK> {
  type Output = JsSymbol<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    ensure_symbol_ref_owner_record(&self.record, scope.record())?;
    let mut result = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_reference_value(env, self.inner, &mut result) },
      "Failed to get reference value"
    )?;
    Ok(unsafe { Local::from_raw(result) })
  }
}

impl<'scope, const LEAK_CHECK: bool> IntoJs<'scope> for SymbolRef<LEAK_CHECK> {
  type Output = JsSymbol<'scope>;

  fn into_js(mut self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    ensure_symbol_ref_owner_record(&self.record, scope.record())?;
    let mut result = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_reference_value(env, self.inner, &mut result) },
      "Failed to get reference value"
    )?;
    check_status!(
      unsafe { sys::napi_delete_reference(env, self.inner) },
      "delete Ref failed"
    )?;
    self.inner = ptr::null_mut();
    drop(self);
    Ok(unsafe { Local::from_raw(result) })
  }
}

fn ensure_symbol_ref_owner(record: &Weak<EnvRecord>, env: &Env) -> Result<()> {
  let owner = record.upgrade().ok_or_else(|| {
    Error::new(
      Status::InvalidArg,
      "SymbolRef owner environment is no longer available",
    )
  })?;
  let current = env.record();
  if Rc::ptr_eq(&owner, &current) {
    Ok(())
  } else {
    Err(Error::new(
      Status::InvalidArg,
      "SymbolRef owner environment does not match the current environment",
    ))
  }
}

fn ensure_symbol_ref_owner_record(record: &Weak<EnvRecord>, current: &Rc<EnvRecord>) -> Result<()> {
  let owner = record.upgrade().ok_or_else(|| {
    Error::new(
      Status::InvalidArg,
      "SymbolRef owner environment is no longer available",
    )
  })?;
  if Rc::ptr_eq(&owner, current) {
    Ok(())
  } else {
    Err(Error::new(
      Status::InvalidArg,
      "SymbolRef owner environment does not match the current environment",
    ))
  }
}
