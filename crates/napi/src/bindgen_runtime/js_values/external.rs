use std::{
  any::TypeId,
  cell::Cell,
  ffi::c_void,
  marker::PhantomData,
  ops::{Deref, DerefMut},
  ptr,
  rc::{Rc, Weak},
};

use crate::{
  bindgen_runtime::{
    Env, EnvRecord, FromJs, IntoJs, Local, Result, Scope, TypeName, Unknown, ValidateNapiValue,
  },
  check_status, sys, Error, JsExternal, Status,
};

#[repr(C)]
pub struct External<T: 'static> {
  type_id: TypeId,
  obj: T,
  size_hint: usize,
  pub adjusted_size: i64,
}

impl<T: 'static> TypeName for &External<T> {
  fn type_name() -> &'static str {
    "External"
  }

  fn value_type() -> crate::ValueType {
    crate::ValueType::External
  }
}

impl<T: 'static> TypeName for &mut External<T> {
  fn type_name() -> &'static str {
    "External"
  }

  fn value_type() -> crate::ValueType {
    crate::ValueType::External
  }
}

impl<T: 'static> From<T> for External<T> {
  fn from(t: T) -> Self {
    External::new(t)
  }
}

impl<T: 'static> ValidateNapiValue for &External<T> {}

impl<T: 'static> ValidateNapiValue for &mut External<T> {}

impl<T: 'static> External<T> {
  pub fn new(value: T) -> Self {
    Self {
      type_id: TypeId::of::<T>(),
      obj: value,
      size_hint: 0,
      adjusted_size: 0,
    }
  }

  /// Turn a raw pointer (from napi) pointing to an External into a reference to the inner object.
  ///
  /// # Safety
  /// The `unknown_tagged_object` raw pointer must point to an `External<T>` struct.
  pub(crate) unsafe fn from_raw_impl(
    unknown_tagged_object: *mut c_void,
  ) -> Option<&'static mut Self> {
    let type_id = unknown_tagged_object as *const TypeId;
    if unsafe { *type_id } == TypeId::of::<T>() {
      let tagged_object = unknown_tagged_object as *mut External<T>;
      Some(Box::leak(unsafe { Box::from_raw(tagged_object) }))
    } else {
      None
    }
  }

  /// Turn a raw pointer (from napi) pointing to an External into a mutable reference to the inner object.
  ///
  /// # Safety
  /// The `unknown_tagged_object` raw pointer must point to an `External<T>` struct.
  pub unsafe fn inner_from_raw_mut(unknown_tagged_object: *mut c_void) -> Option<&'static mut T> {
    Self::from_raw_impl(unknown_tagged_object).map(|external| &mut external.obj)
  }

  /// Turn a raw pointer (from napi) pointing to an External into a reference inner object.
  ///
  /// # Safety
  /// The `unknown_tagged_object` raw pointer must point to an `External<T>` struct.
  pub unsafe fn inner_from_raw(unknown_tagged_object: *mut c_void) -> Option<&'static T> {
    Self::from_raw_impl(unknown_tagged_object).map(|external| &external.obj)
  }

  /// `size_hint` is a value to tell Node.js GC how much memory is used by this `External` object.
  ///
  /// If getting the exact `size_hint` is difficult, you can provide an approximate value, it's only effect to the GC.
  ///
  /// If your `External` object is not effect to GC, you can use `External::new` instead.
  pub fn new_with_size_hint(value: T, size_hint: usize) -> Self {
    Self {
      type_id: TypeId::of::<T>(),
      obj: value,
      size_hint,
      adjusted_size: 0,
    }
  }

  /// convert `External<T>` to `Unknown`
  pub fn into_unknown<'env>(self, env: &'env Env<'env>) -> Result<Unknown<'env>> {
    let mut env = *env;
    env.with_scope(|scope| {
      let external = self.into_js(scope)?;
      Ok(unsafe { Unknown::from_raw_unchecked(scope.env().raw(), external.raw()) })
    })
  }

  /// Convert `External<T>` to `JsExternal`
  pub fn into_js_external<'env>(self, env: &'env Env<'env>) -> Result<JsExternal<'env>> {
    let mut env = *env;
    env.with_scope(|scope| {
      let external = self.into_js(scope)?;
      Ok(unsafe { JsExternal::from_raw(scope.env().raw(), external.raw()) })
    })
  }

  #[allow(clippy::wrong_self_convention)]
  unsafe fn create_external_value(
    self,
    env: sys::napi_env,
  ) -> Result<(sys::napi_value, *mut External<T>)> {
    let mut napi_value = ptr::null_mut();
    let size_hint = self.size_hint as i64;
    let size_hint_ptr = Box::into_raw(Box::new(size_hint));
    let obj_ptr = Box::into_raw(Box::new(self));
    check_status!(
      unsafe {
        sys::napi_create_external(
          env,
          obj_ptr.cast(),
          Some(crate::raw_finalize::<External<T>>),
          size_hint_ptr.cast(),
          &mut napi_value,
        )
      },
      "Create external value failed"
    )?;

    #[cfg(not(target_family = "wasm"))]
    {
      let mut adjusted_external_memory_size = std::mem::MaybeUninit::new(0);

      if size_hint != 0 {
        check_status!(
          unsafe {
            sys::napi_adjust_external_memory(
              env,
              size_hint,
              adjusted_external_memory_size.as_mut_ptr(),
            )
          },
          "Adjust external memory failed"
        )?;
      };

      (Box::leak(unsafe { Box::from_raw(obj_ptr) })).adjusted_size =
        unsafe { adjusted_external_memory_size.assume_init() };
    }

    Ok((napi_value, obj_ptr))
  }
}

impl<'env, 'scope, T: 'static> FromJs<'env, 'scope> for &'scope External<T> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let mut unknown_tagged_object = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_get_value_external(scope.env().raw(), value.raw(), &mut unknown_tagged_object)
      },
      "Failed to get external value"
    )?;

    match unsafe { External::<T>::from_raw_impl(unknown_tagged_object) } {
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
}

impl<'env, 'scope, T: 'static> FromJs<'env, 'scope> for &'scope mut External<T> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let mut unknown_tagged_object = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_get_value_external(scope.env().raw(), value.raw(), &mut unknown_tagged_object)
      },
      "Failed to get external value"
    )?;

    match unsafe { External::<T>::from_raw_impl(unknown_tagged_object) } {
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
}

impl<T: 'static> AsRef<T> for External<T> {
  fn as_ref(&self) -> &T {
    &self.obj
  }
}

impl<T: 'static> AsMut<T> for External<T> {
  fn as_mut(&mut self) -> &mut T {
    &mut self.obj
  }
}

impl<T: 'static> Deref for External<T> {
  type Target = T;

  fn deref(&self) -> &Self::Target {
    self.as_ref()
  }
}

impl<T: 'static> DerefMut for External<T> {
  fn deref_mut(&mut self) -> &mut Self::Target {
    self.as_mut()
  }
}

impl<'scope, T: 'static> IntoJs<'scope> for External<T> {
  type Output = JsExternal<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> crate::Result<Local<'scope, Self::Output>> {
    let (napi_value, _) = unsafe { self.create_external_value(scope.env().raw())? };
    Ok(unsafe { Local::from_raw(napi_value) })
  }
}

/// `ExternalRef` is an explicit handle to an `External` object.
pub struct ExternalRef<T: 'static> {
  pub(crate) raw: Cell<sys::napi_ref>,
  pub(crate) record: Weak<EnvRecord>,
  pub(crate) marker: PhantomData<fn() -> T>,
}

impl<T: 'static> TypeName for ExternalRef<T> {
  fn type_name() -> &'static str {
    "External"
  }

  fn value_type() -> crate::ValueType {
    crate::ValueType::External
  }
}

impl<T: 'static> ValidateNapiValue for ExternalRef<T> {}

impl<T: 'static> Drop for ExternalRef<T> {
  fn drop(&mut self) {
    let raw = self.raw.replace(ptr::null_mut());
    if raw.is_null() {
      return;
    }
    if let Some(record) = self.record.upgrade() {
      record.deferred_refs().push(raw);
    }
  }
}

impl<T: 'static> ExternalRef<T> {
  pub fn new(env: &Env, value: T) -> Result<Self> {
    let external = External::new(value);
    let mut ref_ptr = ptr::null_mut();
    let external_value = unsafe { external.create_external_value(env.0)? };
    let napi_val = external_value.0;
    check_status!(
      unsafe { sys::napi_create_reference(env.0, napi_val, 1, &mut ref_ptr) },
      "Failed to create reference on external value"
    )?;
    Ok(ExternalRef {
      raw: Cell::new(ref_ptr),
      record: Rc::downgrade(&env.record()),
      marker: PhantomData,
    })
  }

  /// Get the raw JsExternal value from the reference
  pub fn get_value<'env>(&self, env: &'env Env<'env>) -> Result<JsExternal<'env>> {
    let record = self.owner_record()?;
    if !Rc::ptr_eq(&record, &env.record()) {
      return Err(owner_mismatch());
    }
    let mut napi_val = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_reference_value(env.0, self.raw_ref()?, &mut napi_val) },
      "Failed to get reference value on external value"
    )?;
    Ok(unsafe { JsExternal::from_raw(env.0, napi_val) })
  }

  fn owner_record(&self) -> Result<Rc<EnvRecord>> {
    self.record.upgrade().ok_or_else(|| {
      Error::new(
        Status::InvalidArg,
        "External reference owner environment is no longer available".to_owned(),
      )
    })
  }

  fn raw_ref(&self) -> Result<sys::napi_ref> {
    let raw = self.raw.get();
    if raw.is_null() {
      Err(Error::new(
        Status::InvalidArg,
        "External reference is already closed".to_owned(),
      ))
    } else {
      Ok(raw)
    }
  }
}

impl<'env, 'scope, T: 'static> FromJs<'env, 'scope> for ExternalRef<T> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> crate::Result<Self> {
    let value = JsExternal::from_js(scope, value)?;
    scope.create_ref(&value)
  }
}

impl<'scope, T: 'static> IntoJs<'scope> for ExternalRef<T> {
  type Output = JsExternal<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> crate::Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let record = self.owner_record()?;
    if !Rc::ptr_eq(&record, scope.record()) {
      return Err(owner_mismatch());
    }
    let mut value = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_reference_value(env, self.raw_ref()?, &mut value) },
      "Failed to get reference value on external value"
    )?;
    Ok(unsafe { Local::from_raw(value) })
  }
}

impl<'scope, T: 'static> IntoJs<'scope> for &ExternalRef<T> {
  type Output = JsExternal<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> crate::Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let record = self.owner_record()?;
    if !Rc::ptr_eq(&record, scope.record()) {
      return Err(owner_mismatch());
    }
    let mut value = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_reference_value(env, self.raw_ref()?, &mut value) },
      "Failed to get reference value on external value"
    )?;
    Ok(unsafe { Local::from_raw(value) })
  }
}

fn owner_mismatch() -> Error {
  Error::new(
    Status::InvalidArg,
    "External reference owner environment does not match the current environment".to_owned(),
  )
}
