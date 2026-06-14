use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr;

use crate::{
  bindgen_prelude::{
    CallbackDecoder, EnvRecord, FromJs, IntoJs, JsObjectValue, Local, Result, Scope, TypeName,
    Unknown,
  },
  check_status, sys, Env, Error, JsValue, Status, Value, ValueType,
};

use super::promise_future::PromiseFuture;

#[derive(Clone, Copy)]
pub struct Promise<'env, T> {
  pub(crate) inner: sys::napi_value,
  env: sys::napi_env,
  _phantom: &'env PhantomData<T>,
}

impl<'env, T> JsValue<'env> for Promise<'env, T> {
  fn value(&self) -> Value {
    Value {
      env: self.env,
      value: self.inner,
      value_type: ValueType::Object,
    }
  }
}

impl<'env, T> JsObjectValue<'env> for Promise<'env, T> {}

impl<T: TypeName> TypeName for Promise<'_, T> {
  fn type_name() -> &'static str {
    "Promise"
  }

  fn value_type() -> crate::ValueType {
    crate::ValueType::Object
  }

  fn ts_type() -> String {
    format!("Promise<{}>", T::ts_type())
  }
}

impl<'env, 'scope, T> FromJs<'env, 'scope> for Promise<'scope, T> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    super::ensure_is_promise(scope.env().raw(), value.raw())?;
    Ok(unsafe { Promise::from_raw(scope.env().raw(), value.raw()) })
  }
}

impl<T> Promise<'_, T> {
  #[doc(hidden)]
  pub(crate) unsafe fn from_raw(env: sys::napi_env, inner: sys::napi_value) -> Self {
    Self {
      inner,
      env,
      _phantom: &PhantomData,
    }
  }
}

impl<'env, T> Promise<'env, T>
where
  for<'scope> T: IntoJs<'scope>,
{
  /// Create a new promise and resolve it with the given value
  pub fn resolve(env: &Env<'env>, value: T) -> Result<Self> {
    create_completed_promise(env, value, |env, deferred, value| unsafe {
      sys::napi_resolve_deferred(env, deferred, value)
    })
  }

  /// Create a new promise and reject it with the given error
  pub fn reject<E>(env: &Env<'env>, error: E) -> Result<Self>
  where
    for<'scope> E: IntoJs<'scope>,
  {
    create_completed_promise(env, error, |env, deferred, value| unsafe {
      sys::napi_reject_deferred(env, deferred, value)
    })
  }
}

fn create_completed_promise<'env, T, V, Complete>(
  env: &Env<'env>,
  value: V,
  complete: Complete,
) -> Result<Promise<'env, T>>
where
  for<'scope> V: IntoJs<'scope>,
  Complete: FnOnce(sys::napi_env, sys::napi_deferred, sys::napi_value) -> sys::napi_status,
{
  let mut deferred = ptr::null_mut();
  let mut promise = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_create_promise(env.0, &mut deferred, &mut promise) },
    "Failed to create promise"
  )?;
  let mut scoped_env = *env;
  let raw_value = scoped_env.with_scope(|scope| value.into_js(scope).map(|local| local.raw()))?;
  check_status!(
    complete(env.0, deferred, raw_value),
    "Failed to complete promise"
  )?;
  Ok(unsafe { Promise::from_raw(env.0, promise) })
}

impl<'env, T> Promise<'env, T>
where
  T: for<'value_env, 'value_scope> FromJs<'value_env, 'value_scope>,
{
  /// Promise.then method
  pub fn then<'then, Callback, U>(&self, cb: Callback) -> Result<Promise<'env, U>>
  where
    U: 'static,
    for<'scope> U: IntoJs<'scope>,
    Callback: 'then
      + for<'callback, 'scope> FnOnce(
        &mut Scope<'callback, 'scope>,
        CallbackContext<'scope, T>,
      ) -> Result<U>,
  {
    const THEN: &[u8; 5] = b"then\0";
    let new_promise = attach_promise_callback(
      self.env,
      self.inner,
      PromiseMethod {
        name: THEN,
        name_len: 4,
        create_error: "Create then function for Promise failed",
        call_error: "Call the Promise::then failed",
      },
      Some(raw_promise_then_callback::<T, U, Callback>),
      cb,
    )?;

    Ok(unsafe { Promise::<U>::from_raw(self.env, new_promise) })
  }

  /// Promise.catch method
  pub fn catch<'catch, E, U, Callback>(&self, cb: Callback) -> Result<Promise<'env, U>>
  where
    E: for<'value_env, 'value_scope> FromJs<'value_env, 'value_scope>,
    U: 'static,
    for<'scope> U: IntoJs<'scope>,
    Callback: 'catch
      + for<'callback, 'scope> FnOnce(
        &mut Scope<'callback, 'scope>,
        CallbackContext<'scope, E>,
      ) -> Result<U>,
  {
    const CATCH: &[u8; 6] = b"catch\0";
    let new_promise = attach_promise_callback(
      self.env,
      self.inner,
      PromiseMethod {
        name: CATCH,
        name_len: 5,
        create_error: "Create catch function for Promise failed",
        call_error: "Call the Promise::catch failed",
      },
      Some(raw_promise_catch_callback::<E, U, Callback>),
      cb,
    )?;

    Ok(unsafe { Promise::<U>::from_raw(self.env, new_promise) })
  }

  /// Promise.finally method
  pub fn finally<'finally, U, Callback>(&mut self, cb: Callback) -> Result<Promise<'env, T>>
  where
    U: 'static,
    for<'scope> U: IntoJs<'scope>,
    Callback: 'finally + for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>) -> Result<U>,
  {
    const FINALLY: &[u8; 8] = b"finally\0";
    let new_promise = attach_promise_callback(
      self.env,
      self.inner,
      PromiseMethod {
        name: FINALLY,
        name_len: 7,
        create_error: "Create finally function for Promise failed",
        call_error: "Call the Promise::finally failed",
      },
      Some(raw_promise_finally_callback::<U, Callback>),
      cb,
    )?;

    Ok(unsafe { Self::from_raw(self.env, new_promise) })
  }

  /// Convert a JavaScript promise handle into a Rust future.
  ///
  /// So you can await the promise in Rust.
  pub fn into_future(self) -> Result<PromiseFuture<T>> {
    PromiseFuture::from_promise(self)
  }
}

struct PromiseMethod {
  name: &'static [u8],
  name_len: isize,
  create_error: &'static str,
  call_error: &'static str,
}

fn attach_promise_callback<Cb>(
  env: sys::napi_env,
  promise: sys::napi_value,
  method: PromiseMethod,
  raw_callback: sys::napi_callback,
  callback: Cb,
) -> Result<sys::napi_value> {
  let mut raw_method = ptr::null_mut();
  check_status!(unsafe {
    sys::napi_get_named_property(env, promise, method.name.as_ptr().cast(), &mut raw_method)
  })?;

  let raw_state = PromiseCallbackState::into_raw(callback);
  let mut state_guard = PromiseCallbackStateGuard::new(raw_state);
  attach_promise_callback_state::<Cb>(
    env,
    promise,
    raw_method,
    method,
    raw_callback,
    &mut state_guard,
  )
}

fn attach_promise_callback_state<Cb>(
  env: sys::napi_env,
  promise: sys::napi_value,
  raw_method: sys::napi_value,
  method: PromiseMethod,
  raw_callback: sys::napi_callback,
  state_guard: &mut PromiseCallbackStateGuard<Cb>,
) -> Result<sys::napi_value> {
  let mut callback = ptr::null_mut();
  check_status!(
    unsafe {
      sys::napi_create_function(
        env,
        method.name.as_ptr().cast(),
        method.name_len,
        raw_callback,
        state_guard.raw().cast(),
        &mut callback,
      )
    },
    "{}",
    method.create_error
  )?;

  state_guard.release();

  let mut new_promise = ptr::null_mut();
  check_status!(
    unsafe {
      sys::napi_call_function(
        env,
        promise,
        raw_method,
        1,
        [callback].as_ptr(),
        &mut new_promise,
      )
    },
    "{}",
    method.call_error
  )?;

  // use `napi_wrap` to trigger the finalizer after the Promise is GCed
  // Note: we don't use `napi_add_finalizer` here because it requires `napi5`
  check_status!(
    unsafe {
      sys::napi_wrap(
        env,
        new_promise,
        state_guard.raw().cast(),
        Some(promise_callback_finalizer::<Cb>),
        ptr::null_mut(),
        ptr::null_mut(),
      )
    },
    "Wrap finalizer for Promise failed"
  )?;

  Ok(new_promise)
}

struct PromiseCallbackStateGuard<Cb> {
  raw: *mut PromiseCallbackState<Cb>,
  armed: bool,
}

impl<Cb> PromiseCallbackStateGuard<Cb> {
  fn new(raw: *mut PromiseCallbackState<Cb>) -> Self {
    Self { raw, armed: true }
  }

  fn raw(&self) -> *mut PromiseCallbackState<Cb> {
    self.raw
  }

  fn release(&mut self) {
    self.armed = false;
  }
}

impl<Cb> Drop for PromiseCallbackStateGuard<Cb> {
  fn drop(&mut self) {
    if self.armed {
      drop(unsafe { Box::from_raw(self.raw) });
    }
  }
}

struct PromiseCallbackState<Cb> {
  callback: Option<Cb>,
}

impl<Cb> PromiseCallbackState<Cb> {
  fn into_raw(callback: Cb) -> *mut Self {
    Box::into_raw(Box::new(Self {
      callback: Some(callback),
    }))
  }

  unsafe fn take(raw: *mut c_void) -> Result<Cb> {
    let state = unsafe { raw.cast::<Self>().as_mut() }.ok_or_else(|| {
      Error::new(
        Status::InvalidArg,
        "Promise callback state is null".to_owned(),
      )
    })?;

    state.callback.take().ok_or_else(|| {
      Error::new(
        Status::InvalidArg,
        "Promise callback has already been consumed".to_owned(),
      )
    })
  }
}

unsafe extern "C" fn raw_promise_then_callback<T, U, Cb>(
  env: sys::napi_env,
  cbinfo: sys::napi_callback_info,
) -> sys::napi_value
where
  T: for<'value_env, 'value_scope> FromJs<'value_env, 'value_scope>,
  U: 'static,
  for<'scope> U: IntoJs<'scope>,
  Cb: for<'callback, 'scope> FnOnce(
    &mut Scope<'callback, 'scope>,
    CallbackContext<'scope, T>,
  ) -> Result<U>,
{
  unsafe {
    EnvRecord::enter_scope(env, |scope| {
      handle_then_callback::<T, U, Cb>(*scope.env(), cbinfo)
    })
  }
  .unwrap_or_else(|err| throw_error(env, err, "Error in Promise.then"))
}

#[inline]
fn handle_then_callback<T, U, Cb>(
  env_wrapper: Env<'_>,
  cbinfo: sys::napi_callback_info,
) -> Result<sys::napi_value>
where
  T: for<'value_env, 'value_scope> FromJs<'value_env, 'value_scope>,
  U: 'static,
  for<'scope> U: IntoJs<'scope>,
  Cb: for<'callback, 'scope> FnOnce(
    &mut Scope<'callback, 'scope>,
    CallbackContext<'scope, T>,
  ) -> Result<U>,
{
  let mut decoder = CallbackDecoder::<1>::new(env_wrapper, cbinfo, Some(1))?;
  decoder.with_frame(|mut frame| {
    let cb = unsafe { PromiseCallbackState::<Cb>::take(frame.raw_data()) }?;
    let then_value = frame.arg::<T>(0)?;
    let scope = frame.scope_mut();
    let callback_env = *scope.env();
    let value = cb(
      scope,
      CallbackContext {
        env: callback_env,
        value: then_value,
      },
    )?;
    frame.return_value(value)
  })
}

unsafe extern "C" fn raw_promise_catch_callback<E, U, Cb>(
  env: sys::napi_env,
  cbinfo: sys::napi_callback_info,
) -> sys::napi_value
where
  E: for<'value_env, 'value_scope> FromJs<'value_env, 'value_scope>,
  U: 'static,
  for<'scope> U: IntoJs<'scope>,
  Cb: for<'callback, 'scope> FnOnce(
    &mut Scope<'callback, 'scope>,
    CallbackContext<'scope, E>,
  ) -> Result<U>,
{
  unsafe {
    EnvRecord::enter_scope(env, |scope| {
      handle_catch_callback::<E, U, Cb>(*scope.env(), cbinfo)
    })
  }
  .unwrap_or_else(|err| throw_error(env, err, "Error in Promise.catch"))
}

#[inline(always)]
fn handle_catch_callback<E, U, Cb>(
  env_wrapper: Env<'_>,
  cbinfo: sys::napi_callback_info,
) -> Result<sys::napi_value>
where
  E: for<'value_env, 'value_scope> FromJs<'value_env, 'value_scope>,
  U: 'static,
  for<'scope> U: IntoJs<'scope>,
  Cb: for<'callback, 'scope> FnOnce(
    &mut Scope<'callback, 'scope>,
    CallbackContext<'scope, E>,
  ) -> Result<U>,
{
  let mut decoder = CallbackDecoder::<1>::new(env_wrapper, cbinfo, Some(1))?;
  decoder.with_frame(|mut frame| {
    let cb = unsafe { PromiseCallbackState::<Cb>::take(frame.raw_data()) }?;
    let catch_value = frame.arg::<E>(0)?;
    let scope = frame.scope_mut();
    let callback_env = *scope.env();
    let value = cb(
      scope,
      CallbackContext {
        env: callback_env,
        value: catch_value,
      },
    )?;
    frame.return_value(value)
  })
}

unsafe extern "C" fn raw_promise_finally_callback<U, Cb>(
  env: sys::napi_env,
  cbinfo: sys::napi_callback_info,
) -> sys::napi_value
where
  U: 'static,
  for<'scope> U: IntoJs<'scope>,
  Cb: for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>) -> Result<U>,
{
  unsafe {
    EnvRecord::enter_scope(env, |scope| {
      handle_finally_callback::<U, Cb>(*scope.env(), cbinfo)
    })
  }
  .unwrap_or_else(|err| throw_error(env, err, "Error in Promise.finally"))
}

#[inline(always)]
fn handle_finally_callback<U, Cb>(
  env_wrapper: Env<'_>,
  cbinfo: sys::napi_callback_info,
) -> Result<sys::napi_value>
where
  U: 'static,
  for<'scope> U: IntoJs<'scope>,
  Cb: for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>) -> Result<U>,
{
  let mut decoder = CallbackDecoder::<0>::new(env_wrapper, cbinfo, None)?;
  decoder.with_frame(|mut frame| {
    let cb = unsafe { PromiseCallbackState::<Cb>::take(frame.raw_data()) }?;
    let scope = frame.scope_mut();
    let value = cb(scope)?;
    frame.return_value(value)
  })
}

pub struct CallbackContext<'env, T> {
  pub env: Env<'env>,
  pub value: T,
}

impl<'scope, T> IntoJs<'scope> for CallbackContext<'_, T>
where
  T: IntoJs<'scope>,
{
  type Output = T::Output;

  fn into_js(
    self,
    scope: &mut crate::bindgen_runtime::Scope<'_, 'scope>,
  ) -> Result<Local<'scope, Self::Output>> {
    self.value.into_js(scope)
  }
}

#[inline(never)]
fn throw_error(env: sys::napi_env, err: Error, default_msg: &str) -> sys::napi_value {
  const GENERIC_FAILURE: &str = "GenericFailure\0";
  let code = if err.status.as_ref().is_empty() {
    GENERIC_FAILURE
  } else {
    err.status.as_ref()
  };
  let mut code_string = ptr::null_mut();
  let msg = if err.reason.is_empty() {
    default_msg
  } else {
    err.reason.as_ref()
  };
  let mut msg_string = ptr::null_mut();
  let mut err = ptr::null_mut();
  unsafe {
    sys::napi_create_string_latin1(
      env,
      code.as_ptr().cast(),
      code.len() as isize,
      &mut code_string,
    );
    sys::napi_create_string_utf8(
      env,
      msg.as_ptr().cast(),
      msg.len() as isize,
      &mut msg_string,
    );
    sys::napi_create_error(env, code_string, msg_string, &mut err);
    sys::napi_throw(env, err);
  };
  ptr::null_mut()
}

unsafe extern "C" fn promise_callback_finalizer<Cb>(
  _env: sys::napi_env,
  finalize_data: *mut c_void,
  _finalize_hint: *mut c_void,
) {
  if !finalize_data.is_null() {
    drop(unsafe { Box::from_raw(finalize_data.cast::<PromiseCallbackState<Cb>>()) });
  }
}
