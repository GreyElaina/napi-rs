use std::{
  any::TypeId,
  ffi::c_void,
  ops::{Deref, DerefMut},
  ptr,
  rc::Rc,
};

use super::value_ref::{
  create_reference, ensure_record_match, ensure_same_record, reference_value, RefState,
};
use crate::{
  bindgen_runtime::{
    Env, Ext, FromJs, IntoJs, Local, Ref, Result, Scope, TypeName, Unknown, ValidateNapiValue,
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

pub type ExternalRef<T> = Ref<Ext<T>>;

impl<T: 'static> Ref<Ext<T>> {
  pub fn new_external(env: &Env, value: T) -> Result<Self> {
    let external = External::new(value);
    let external_value = unsafe { external.create_external_value(env.0)? };
    let napi_val = external_value.0;
    let raw = create_reference(env.0, napi_val, 1)?;
    Ok(Ref::new(
      RefState::new(raw, Rc::downgrade(&env.record())),
      (),
    ))
  }

  pub fn to_local<'env>(&self, env: &'env Env<'env>) -> Result<JsExternal<'env>> {
    let record = self.state.owner_record()?;
    ensure_record_match(&record, &env.record())?;
    let result = reference_value(env.0, self.state.raw_ref()?)?;
    Ok(unsafe { JsExternal::from_raw(env.0, result) })
  }
}

impl<T: 'static> TypeName for Ref<Ext<T>> {
  fn type_name() -> &'static str {
    "External"
  }

  fn value_type() -> crate::ValueType {
    crate::ValueType::External
  }
}

impl<T: 'static> ValidateNapiValue for Ref<Ext<T>> {}

impl<'env, 'scope, T: 'static> FromJs<'env, 'scope> for Ref<Ext<T>> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> crate::Result<Self> {
    let value = JsExternal::from_js(scope, value)?;
    scope.create_ref(&value)
  }
}

impl<'scope, T: 'static> IntoJs<'scope> for Ref<Ext<T>> {
  type Output = JsExternal<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> crate::Result<Local<'scope, Self::Output>> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;
    let result = reference_value(scope.env().raw(), self.state.raw_ref()?)?;
    Ok(unsafe { Local::from_raw(result) })
  }
}

impl<'scope, T: 'static> IntoJs<'scope> for &Ref<Ext<T>> {
  type Output = JsExternal<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> crate::Result<Local<'scope, Self::Output>> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;
    let result = reference_value(scope.env().raw(), self.state.raw_ref()?)?;
    Ok(unsafe { Local::from_raw(result) })
  }
}
