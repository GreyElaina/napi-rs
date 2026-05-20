use std::os::raw::c_void;
use std::ptr;
use std::{
  marker::PhantomData,
  sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
  },
};

#[cfg(feature = "deferred_trace")]
use crate::{bindgen_runtime::JsObjectValue, JsValue};
use crate::{
  bindgen_runtime::{with_env, IntoJs, Object, Scope},
  check_status, sys, Env, Error, Result,
};

#[cfg(feature = "deferred_trace")]
/// A javascript error which keeps a stack trace
/// to the original caller in an asynchronous context.
/// This is required as the stack trace is lost when
/// an error is created in a different thread.
///
/// See this issue for more details:
/// https://github.com/nodejs/node-addon-api/issues/595
#[repr(transparent)]
#[derive(Clone)]
struct DeferredTrace(sys::napi_ref);

#[cfg(feature = "deferred_trace")]
impl DeferredTrace {
  fn new(raw_env: sys::napi_env) -> Result<Self> {
    let env = unsafe { Env::from_raw(raw_env) };
    let reason = env.create_string("none")?;

    let mut js_error = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_error(raw_env, ptr::null_mut(), reason.raw(), &mut js_error) },
      "Create error in DeferredTrace failed"
    )?;

    let mut result = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_reference(raw_env, js_error, 1, &mut result) },
      "Create reference in DeferredTrace failed"
    )?;

    Ok(Self(result))
  }

  fn into_rejected(self, raw_env: sys::napi_env, mut err: Error) -> Result<sys::napi_value> {
    let env = unsafe { Env::from_raw(raw_env) };
    let mut raw = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_reference_value(raw_env, self.0, &mut raw) },
      "Failed to get referenced value in DeferredTrace"
    )?;

    let mut obj = unsafe { Object::from_raw(raw_env, raw) };
    obj.set_named_property("message", &err.reason)?;
    if let Some(name) = err.js_name.as_ref() {
      obj.set_named_property("name", name)?;
    }
    obj.set_named_property(
      "code",
      env.create_string_from_std(
        err
          .js_code
          .clone()
          .unwrap_or_else(|| format!("{}", err.status)),
      )?,
    )?;
    if let Some(cause) = err.cause.take() {
      obj.set_named_property("cause", *cause)?;
    }
    let err_value = Ok(raw);
    check_status!(
      unsafe { sys::napi_delete_reference(raw_env, self.0) },
      "Failed to get referenced value in DeferredTrace"
    )?;
    err_value
  }
}

pub(crate) type EnvFinalizeCallback = Box<dyn for<'env> FnOnce(Env<'env>) + Send>;
type FinalizeCallback = Arc<Mutex<Option<EnvFinalizeCallback>>>;

struct DeferredThreadsafeFunction {
  raw: sys::napi_threadsafe_function,
  called: AtomicBool,
  released: AtomicBool,
}

// SAFETY: N-API threadsafe function handles are safe to share across threads.
unsafe impl Send for DeferredThreadsafeFunction {}
unsafe impl Sync for DeferredThreadsafeFunction {}

impl DeferredThreadsafeFunction {
  fn new(raw: sys::napi_threadsafe_function) -> Arc<Self> {
    Arc::new(Self {
      raw,
      called: AtomicBool::new(false),
      released: AtomicBool::new(false),
    })
  }

  fn raw(&self) -> sys::napi_threadsafe_function {
    self.raw
  }

  fn mark_called(&self) -> bool {
    !self.called.swap(true, Ordering::AcqRel)
  }

  fn release(&self) -> Result<()> {
    if self.released.swap(true, Ordering::AcqRel) {
      return Ok(());
    }
    check_status!(
      unsafe {
        sys::napi_release_threadsafe_function(self.raw, sys::ThreadsafeFunctionReleaseMode::release)
      },
      "Release threadsafe function in JsDeferred failed"
    )
  }
}

impl Drop for DeferredThreadsafeFunction {
  fn drop(&mut self) {
    if !self.called.load(Ordering::Acquire) {
      let status = unsafe {
        sys::napi_release_threadsafe_function(self.raw, sys::ThreadsafeFunctionReleaseMode::release)
      };
      debug_assert!(
        status == sys::Status::napi_ok,
        "Release unused threadsafe function in JsDeferred failed: {}",
        crate::Status::from(status)
      );
    }
  }
}

struct DeferredData<Resolver: Send> {
  resolver: Result<Resolver>,
  #[cfg(feature = "deferred_trace")]
  trace: DeferredTrace,
  tsfn: Arc<DeferredThreadsafeFunction>,
  finalize_callback: FinalizeCallback,
}

pub struct JsDeferred<Data, Resolver: Send> {
  tsfn: Arc<DeferredThreadsafeFunction>,
  #[cfg(feature = "deferred_trace")]
  trace: DeferredTrace,
  finalize_callback: FinalizeCallback,
  _data: PhantomData<Data>,
  _resolver: PhantomData<Resolver>,
}

impl<Data, Resolver: Send> Clone for JsDeferred<Data, Resolver> {
  fn clone(&self) -> Self {
    Self {
      tsfn: self.tsfn.clone(),
      #[cfg(feature = "deferred_trace")]
      trace: self.trace.clone(),
      finalize_callback: self.finalize_callback.clone(),
      _data: PhantomData,
      _resolver: PhantomData,
    }
  }
}

unsafe impl<Data, Resolver: Send> Send for JsDeferred<Data, Resolver> {}

impl<Data, Resolver> JsDeferred<Data, Resolver>
where
  Data: 'static,
  for<'scope> Data: IntoJs<'scope>,
  Resolver: for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>) -> Result<Data> + Send,
{
  pub(crate) fn new<'env>(env: &'env Env<'env>) -> Result<(Self, Object<'env>)> {
    let (tsfn, promise) = js_deferred_new_raw(env, Some(napi_resolve_deferred::<Data, Resolver>))?;

    let deferred = Self {
      tsfn: DeferredThreadsafeFunction::new(tsfn),
      #[cfg(feature = "deferred_trace")]
      trace: DeferredTrace::new(env.0)?,
      finalize_callback: Default::default(),
      _data: PhantomData,
      _resolver: PhantomData,
    };

    Ok((deferred, promise))
  }

  /// Consumes the deferred, and resolves the promise. The provided function will be called
  /// from the JavaScript thread, and should return the resolved value.
  pub fn resolve(self, resolver: Resolver) {
    self.call_tsfn(Ok(resolver))
  }

  /// Consumes the deferred, and rejects the promise with the provided error.
  pub fn reject(self, error: Error) {
    self.call_tsfn(Err(error))
  }
}

impl<Data, Resolver> JsDeferred<Data, Resolver>
where
  Resolver: Send,
{
  pub fn set_finalize_callback(&mut self, finalize_callback: Option<EnvFinalizeCallback>) {
    self.finalize_callback = Arc::new(Mutex::new(finalize_callback));
  }

  fn call_tsfn(self, result: Result<Resolver>) {
    let tsfn = self.tsfn.clone();
    if !tsfn.mark_called() {
      return;
    }
    let data = DeferredData {
      resolver: result,
      #[cfg(feature = "deferred_trace")]
      trace: self.trace,
      tsfn: tsfn.clone(),
      finalize_callback: self.finalize_callback.clone(),
    };

    // Call back into the JS thread via a threadsafe function. This results in napi_resolve_deferred being called.
    let raw_data = Box::into_raw(Box::from(data));
    let status = unsafe {
      sys::napi_call_threadsafe_function(
        tsfn.raw(),
        raw_data.cast(),
        sys::ThreadsafeFunctionCallMode::blocking,
      )
    };
    if status != sys::Status::napi_ok {
      unsafe { drop(Box::from_raw(raw_data)) };
      if let Err(err) = tsfn.release() {
        eprintln!("napi-rs: failed to release JsDeferred threadsafe function: {err:?}");
      }
    }
    debug_assert!(
      status == sys::Status::napi_ok,
      "Call threadsafe function in JsDeferred failed"
    );
  }
}

fn js_deferred_new_raw<'env>(
  env: &'env Env<'env>,
  resolve_deferred: sys::napi_threadsafe_function_call_js,
) -> Result<(sys::napi_threadsafe_function, Object<'env>)> {
  let mut raw_promise = ptr::null_mut();
  let mut raw_deferred = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_create_promise(env.0, &mut raw_deferred, &mut raw_promise) },
    "Create promise in JsDeferred failed"
  )?;

  // Create a threadsafe function so we can call back into the JS thread when we are done.
  let mut async_resource_name = ptr::null_mut();
  check_status!(
    unsafe {
      sys::napi_create_string_utf8(
        env.0,
        c"napi_resolve_deferred".as_ptr().cast(),
        22,
        &mut async_resource_name,
      )
    },
    "Create async resource name in JsDeferred failed"
  )?;

  let mut tsfn = ptr::null_mut();
  check_status!(
    unsafe {
      sys::napi_create_threadsafe_function(
        env.0,
        ptr::null_mut(),
        ptr::null_mut(),
        async_resource_name,
        0,
        1,
        ptr::null_mut(),
        None,
        raw_deferred.cast(),
        resolve_deferred,
        &mut tsfn,
      )
    },
    "Create threadsafe function in JsDeferred failed"
  )?;

  let promise = unsafe { Object::from_raw(env.0, raw_promise) };

  Ok((tsfn, promise))
}

unsafe extern "C" fn napi_resolve_deferred<Data, Resolver>(
  env: sys::napi_env,
  _js_callback: sys::napi_value,
  context: *mut c_void,
  data: *mut c_void,
) where
  Data: 'static,
  for<'scope> Data: IntoJs<'scope>,
  Resolver: for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>) -> Result<Data> + Send,
{
  if data.is_null() {
    return;
  }

  let deferred_data: Box<DeferredData<Resolver>> = unsafe { Box::from_raw(data.cast()) };
  if env.is_null() {
    return;
  }

  let deferred = context.cast();
  if let Err(error) = unsafe {
    with_env(env, |env_wrapper| {
      resolve_deferred_with_env(env_wrapper, deferred, *deferred_data)
    })
  } {
    eprintln!("napi-rs: failed to resolve deferred callback: {error:?}");
  }
}

fn resolve_deferred_with_env<Data, Resolver>(
  mut env_wrapper: Env<'_>,
  deferred: sys::napi_deferred,
  deferred_data: DeferredData<Resolver>,
) -> Result<()>
where
  Data: 'static,
  for<'scope> Data: IntoJs<'scope>,
  Resolver: for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>) -> Result<Data> + Send,
{
  let env = env_wrapper.raw();
  let finalize_callback = deferred_data
    .finalize_callback
    .lock()
    .expect("Mutex poisoned")
    .take();
  let result = deferred_data.resolver.and_then(|resolver| {
    env_wrapper.with_scope(|scope| {
      let value = resolver(scope)?;
      value.into_js(scope).map(|local| local.raw())
    })
  });

  let release_tsfn_result = deferred_data.tsfn.release();

  if let Err(e) = release_tsfn_result.and(result).and_then(|res| {
    check_status!(
      unsafe { sys::napi_resolve_deferred(env, deferred, res) },
      "Resolve deferred value failed"
    )
    .map(|_| {
      #[cfg(feature = "deferred_trace")]
      {
        let status = unsafe { sys::napi_delete_reference(env, deferred_data.trace.0) };
        if status != sys::Status::napi_ok && cfg!(debug_assertions) {
          eprintln!(
            "Failed to delete reference in deferred {}",
            crate::Status::from(status)
          );
        }
      }
    })
  }) {
    #[cfg(feature = "deferred_trace")]
    let error = deferred_data.trace.into_rejected(env, e);
    #[cfg(not(feature = "deferred_trace"))]
    let error = Ok::<sys::napi_value, Error>(unsafe { crate::JsError::from(e).into_value(env) });

    match error {
      Ok(error) => {
        unsafe { sys::napi_reject_deferred(env, deferred, error) };
        if let Some(finalize_callback) = finalize_callback {
          finalize_callback(env_wrapper);
        }
      }
      Err(err) => {
        if let Some(finalize_callback) = finalize_callback {
          finalize_callback(env_wrapper);
        }
        if cfg!(debug_assertions) {
          eprintln!("Failed to reject deferred: {err:?}");
          let mut err = ptr::null_mut();
          let mut err_msg = ptr::null_mut();
          unsafe {
            sys::napi_create_string_utf8(env, c"Rejection failed".as_ptr().cast(), 0, &mut err_msg);
            sys::napi_create_error(env, ptr::null_mut(), err_msg, &mut err);
            sys::napi_reject_deferred(env, deferred, err);
          }
        }
      }
    }
  } else if let Some(finalize_callback) = finalize_callback {
    finalize_callback(env_wrapper);
  }
  Ok(())
}
