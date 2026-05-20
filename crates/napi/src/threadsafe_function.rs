#![allow(clippy::single_component_path_imports)]

use std::marker::PhantomData;
use std::os::raw::c_void;
use std::ptr::{self, null_mut};
use std::sync::{
  self,
  atomic::{AtomicBool, AtomicPtr, Ordering},
  Arc, RwLock, RwLockWriteGuard,
};

use futures::channel::oneshot::channel;

use crate::{
  bindgen_runtime::{FromJs, IntoJsArgs, Local, Scope, TypeName, Unknown, ValidateNapiValue},
  check_status, sys, Env, Error, JsError, Result, Status,
};

#[deprecated(since = "2.17.0", note = "Please use `ThreadsafeFunction` instead")]
pub type ThreadSafeCallContext<'env, T> = ThreadsafeCallContext<'env, T>;

/// ThreadSafeFunction Context object
/// the `value` is the value passed to `call` method
pub struct ThreadsafeCallContext<'env, T: 'static> {
  pub env: Env<'env>,
  pub value: T,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreadsafeFunctionCallMode {
  NonBlocking,
  Blocking,
}

impl From<ThreadsafeFunctionCallMode> for sys::napi_threadsafe_function_call_mode {
  fn from(value: ThreadsafeFunctionCallMode) -> Self {
    match value {
      ThreadsafeFunctionCallMode::Blocking => sys::ThreadsafeFunctionCallMode::blocking,
      ThreadsafeFunctionCallMode::NonBlocking => sys::ThreadsafeFunctionCallMode::nonblocking,
    }
  }
}

pub struct ThreadsafeFunctionHandle {
  raw: AtomicPtr<sys::napi_threadsafe_function__>,
  aborted: RwLock<bool>,
  referred: AtomicBool,
}

impl ThreadsafeFunctionHandle {
  /// create a Arc to hold the `ThreadsafeFunctionHandle`
  ///
  /// # Safety
  ///
  /// `raw` must be either null or a valid `napi_threadsafe_function` handle
  /// whose release is owned by the returned `ThreadsafeFunctionHandle`.
  pub(crate) unsafe fn from_handle(raw: sys::napi_threadsafe_function) -> Arc<Self> {
    Arc::new(Self {
      raw: AtomicPtr::new(raw),
      aborted: RwLock::new(false),
      referred: AtomicBool::new(true),
    })
  }

  /// Lock `aborted` with read access, call `f` with the value of `aborted`, then unlock it
  pub(crate) fn with_read_aborted<RT, F>(&self, f: F) -> RT
  where
    F: FnOnce(bool) -> RT,
  {
    let aborted_guard = self
      .aborted
      .read()
      .expect("Threadsafe Function aborted lock failed");
    f(*aborted_guard)
  }

  /// Lock `aborted` with write access, call `f` with the `RwLockWriteGuard`, then unlock it
  pub(crate) fn with_write_aborted<RT, F>(&self, f: F) -> RT
  where
    F: FnOnce(RwLockWriteGuard<bool>) -> RT,
  {
    let aborted_guard = self
      .aborted
      .write()
      .expect("Threadsafe Function aborted lock failed");
    f(aborted_guard)
  }

  #[allow(clippy::arc_with_non_send_sync)]
  pub(crate) fn null() -> Arc<Self> {
    // SAFETY: a null handle represents the pre-initialized state and is never released.
    unsafe { Self::from_handle(null_mut()) }
  }

  pub(crate) fn native_handle(&self) -> sys::napi_threadsafe_function {
    self.raw.load(Ordering::SeqCst)
  }

  pub(crate) fn set_handle(&self, raw: sys::napi_threadsafe_function) {
    self.raw.store(raw, Ordering::SeqCst)
  }
}

impl Drop for ThreadsafeFunctionHandle {
  fn drop(&mut self) {
    self.with_read_aborted(|aborted| {
      if !aborted {
        let raw = self.native_handle();
        // if ThreadsafeFunction::create failed, the raw will be null and we don't need to release it
        if !raw.is_null() {
          let release_status = unsafe {
            sys::napi_release_threadsafe_function(
              self.native_handle(),
              sys::ThreadsafeFunctionReleaseMode::release,
            )
          };
          assert!(
            release_status == sys::Status::napi_ok,
            "Threadsafe Function release failed {}",
            Status::from(release_status)
          );
        }
      }
    })
  }
}

#[repr(u8)]
pub enum ThreadsafeFunctionCallVariant {
  Direct,
  WithCallback,
}

type ThreadsafeFunctionJsCallback<Return> =
  Box<dyn for<'env> FnOnce(Result<Return>, Env<'env>) -> Result<()> + Send>;

pub struct ThreadsafeFunctionCallJsBackData<T, Return = Unknown<'static>> {
  pub data: T,
  pub call_variant: ThreadsafeFunctionCallVariant,
  pub callback: ThreadsafeFunctionJsCallback<Return>,
}

/// Communicate with the addon's main thread by invoking a JavaScript function from other threads.
///
/// ## Example
/// An example of using `ThreadsafeFunction`:
///
/// ```rust
/// use std::thread;
/// use std::sync::Arc;
///
/// use napi::{
///     threadsafe_function::{
///         ThreadSafeCallContext, ThreadsafeFunctionCallMode, ThreadsafeFunctionReleaseMode,
///     },
/// };
/// use napi_derive::napi;
///
/// #[napi]
/// pub fn call_threadsafe_function(callback: Arc<ThreadsafeFunction<(u32, bool, String), ()>>) {
///   let tsfn_cloned = tsfn.clone();
///
///   thread::spawn(move || {
///       let output: Vec<u32> = vec![0, 1, 2, 3];
///       // It's okay to call a threadsafe function multiple times.
///       tsfn.call(Ok((1, false, "NAPI-RS".into())), ThreadsafeFunctionCallMode::Blocking);
///       tsfn.call(Ok((2, true, "NAPI-RS".into())), ThreadsafeFunctionCallMode::NonBlocking);
///   });
///
///   thread::spawn(move || {
///       tsfn_cloned.call((3, false, "NAPI-RS".into())), ThreadsafeFunctionCallMode::NonBlocking);
///   });
/// }
/// ```
pub struct ThreadsafeFunction<
  T: 'static,
  Return: 'static + for<'env, 'scope> FromJs<'env, 'scope> = UnknownReturnValue,
  CallJsBackArgs: 'static = T,
  ErrorStatus: AsRef<str> + From<Status> + Send + 'static = Status,
  const CalleeHandled: bool = true,
  const Weak: bool = false,
  const MaxQueueSize: usize = 0,
> {
  handle: Arc<ThreadsafeFunctionHandle>,
  _phantom: PhantomData<(T, CallJsBackArgs, Return, ErrorStatus)>,
}

unsafe impl<
    T: Send + 'static,
    Return: for<'env, 'scope> FromJs<'env, 'scope>,
    CallJsBackArgs: 'static,
    ErrorStatus: AsRef<str> + From<Status> + Send + 'static,
    const CalleeHandled: bool,
    const Weak: bool,
    const MaxQueueSize: usize,
  > Send
  for ThreadsafeFunction<
    T,
    Return,
    CallJsBackArgs,
    ErrorStatus,
    { CalleeHandled },
    { Weak },
    { MaxQueueSize },
  >
{
}

unsafe impl<
    T: Send + 'static,
    Return: for<'env, 'scope> FromJs<'env, 'scope>,
    CallJsBackArgs: 'static,
    ErrorStatus: AsRef<str> + From<Status> + Send + 'static,
    const CalleeHandled: bool,
    const Weak: bool,
    const MaxQueueSize: usize,
  > Sync
  for ThreadsafeFunction<
    T,
    Return,
    CallJsBackArgs,
    ErrorStatus,
    { CalleeHandled },
    { Weak },
    { MaxQueueSize },
  >
{
}

impl<
    'env,
    'scope,
    T: 'static,
    Return: for<'value_env, 'value_scope> FromJs<'value_env, 'value_scope>,
    ErrorStatus: AsRef<str> + From<Status> + Send + 'static,
    const CalleeHandled: bool,
    const Weak: bool,
    const MaxQueueSize: usize,
  > FromJs<'env, 'scope>
  for ThreadsafeFunction<T, Return, T, ErrorStatus, { CalleeHandled }, { Weak }, { MaxQueueSize }>
where
  for<'value_scope> T: IntoJsArgs<'value_scope>,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Self::create(scope.env().raw(), value.raw(), |ctx| Ok(ctx.value))
  }
}

impl<
    T: 'static,
    Return: for<'value_env, 'value_scope> FromJs<'value_env, 'value_scope>,
    ErrorStatus: AsRef<str> + From<Status> + Send + 'static,
    const CalleeHandled: bool,
    const Weak: bool,
    const MaxQueueSize: usize,
  > TypeName
  for ThreadsafeFunction<T, Return, T, ErrorStatus, { CalleeHandled }, { Weak }, { MaxQueueSize }>
{
  fn type_name() -> &'static str {
    "ThreadsafeFunction"
  }

  fn value_type() -> crate::ValueType {
    crate::ValueType::Function
  }
}

impl<
    T: 'static,
    Return: for<'env, 'scope> FromJs<'env, 'scope>,
    ErrorStatus: AsRef<str> + From<Status> + Send + 'static,
    const CalleeHandled: bool,
    const Weak: bool,
    const MaxQueueSize: usize,
  > ValidateNapiValue
  for ThreadsafeFunction<T, Return, T, ErrorStatus, { CalleeHandled }, { Weak }, { MaxQueueSize }>
{
}

impl<
    T: 'static,
    Return: for<'env, 'scope> FromJs<'env, 'scope>,
    CallJsBackArgs: 'static,
    ErrorStatus: AsRef<str> + From<Status> + Send + 'static,
    const CalleeHandled: bool,
    const Weak: bool,
    const MaxQueueSize: usize,
  >
  ThreadsafeFunction<
    T,
    Return,
    CallJsBackArgs,
    ErrorStatus,
    { CalleeHandled },
    { Weak },
    { MaxQueueSize },
  >
{
  // See [napi_create_threadsafe_function](https://nodejs.org/api/n-api.html#n_api_napi_create_threadsafe_function)
  // for more information.
  pub(crate) fn create<
    NewArgs: 'static,
    R: Send + 'static + for<'scope> FnMut(ThreadsafeCallContext<'scope, T>) -> Result<NewArgs>,
  >(
    env: sys::napi_env,
    func: sys::napi_value,
    callback: R,
  ) -> Result<
    ThreadsafeFunction<
      T,
      Return,
      NewArgs,
      ErrorStatus,
      { CalleeHandled },
      { Weak },
      { MaxQueueSize },
    >,
  >
  where
    for<'scope> NewArgs: IntoJsArgs<'scope>,
  {
    let mut async_resource_name = ptr::null_mut();
    static THREAD_SAFE_FUNCTION_ASYNC_RESOURCE_NAME: &str = "napi_rs_threadsafe_function";

    #[cfg(feature = "napi10")]
    {
      let mut copied = false;
      check_status!(
        unsafe {
          sys::node_api_create_external_string_latin1(
            env,
            THREAD_SAFE_FUNCTION_ASYNC_RESOURCE_NAME.as_ptr().cast(),
            27,
            None,
            ptr::null_mut(),
            &mut async_resource_name,
            &mut copied,
          )
        },
        "Create external string latin1 in ThreadsafeFunction::create failed"
      )?;
    }

    #[cfg(not(feature = "napi10"))]
    {
      check_status!(
        unsafe {
          sys::napi_create_string_utf8(
            env,
            THREAD_SAFE_FUNCTION_ASYNC_RESOURCE_NAME.as_ptr().cast(),
            27,
            &mut async_resource_name,
          )
        },
        "Create string utf8 in ThreadsafeFunction::create failed"
      )?;
    }

    let mut raw_tsfn = ptr::null_mut();
    let callback_ptr = Box::into_raw(Box::new(callback));
    let handle = ThreadsafeFunctionHandle::null();
    check_status!(
      unsafe {
        sys::napi_create_threadsafe_function(
          env,
          func,
          ptr::null_mut(),
          async_resource_name,
          MaxQueueSize,
          1,
          Arc::downgrade(&handle).into_raw().cast_mut().cast(), // pass handler to thread_finalize_cb
          Some(thread_finalize_cb::<T, NewArgs, R>),
          callback_ptr.cast(),
          Some(call_js_cb::<T, Return, NewArgs, ErrorStatus, R, CalleeHandled>),
          &mut raw_tsfn,
        )
      },
      "Create threadsafe function in ThreadsafeFunction::create failed"
    )?;
    handle.set_handle(raw_tsfn);

    // Weak ThreadsafeFunction will not prevent the event loop from exiting
    if Weak {
      check_status!(
        unsafe { sys::napi_unref_threadsafe_function(env, raw_tsfn) },
        "Unref threadsafe function failed in Weak mode"
      )?;
    }

    Ok(ThreadsafeFunction {
      handle,
      _phantom: PhantomData,
    })
  }

  #[deprecated(
    since = "2.17.0",
    note = "Please use `ThreadsafeFunction::clone` instead of manually increasing the reference count"
  )]
  /// See [napi_ref_threadsafe_function](https://nodejs.org/api/n-api.html#n_api_napi_ref_threadsafe_function)
  /// for more information.
  ///
  /// "ref" is a keyword so that we use "refer" here.
  pub fn refer(&mut self, env: &Env) -> Result<()> {
    self.handle.with_read_aborted(|aborted| {
      if !aborted && !self.handle.referred.load(Ordering::Relaxed) {
        check_status!(unsafe {
          sys::napi_ref_threadsafe_function(env.0, self.handle.native_handle())
        })?;
        self.handle.referred.store(true, Ordering::Relaxed);
      }
      Ok(())
    })
  }

  #[deprecated(
    since = "2.17.0",
    note = "Please use `ThreadsafeFunction::clone` instead of manually decreasing the reference count"
  )]
  /// See [napi_unref_threadsafe_function](https://nodejs.org/api/n-api.html#n_api_napi_unref_threadsafe_function)
  /// for more information.
  pub fn unref(&mut self, env: &Env) -> Result<()> {
    self.handle.with_read_aborted(|aborted| {
      if !aborted && self.handle.referred.load(Ordering::Relaxed) {
        check_status!(unsafe {
          sys::napi_unref_threadsafe_function(env.0, self.handle.native_handle())
        })?;
        self.handle.referred.store(false, Ordering::Relaxed);
      }
      Ok(())
    })
  }

  pub fn aborted(&self) -> bool {
    self.handle.with_read_aborted(|aborted| aborted)
  }

  #[deprecated(
    since = "2.17.0",
    note = "Drop all references to the ThreadsafeFunction will automatically release it"
  )]
  pub fn abort(self) -> Result<()> {
    self.handle.with_write_aborted(|mut aborted_guard| {
      if !*aborted_guard {
        check_status!(unsafe {
          sys::napi_release_threadsafe_function(
            self.handle.native_handle(),
            sys::ThreadsafeFunctionReleaseMode::abort,
          )
        })?;
        *aborted_guard = true;
      }
      Ok(())
    })
  }
}

impl<
    T: 'static,
    Return: for<'env, 'scope> FromJs<'env, 'scope>,
    CallJsBackArgs: 'static,
    ErrorStatus: AsRef<str> + From<Status> + Send + 'static,
    const Weak: bool,
    const MaxQueueSize: usize,
  > ThreadsafeFunction<T, Return, CallJsBackArgs, ErrorStatus, true, { Weak }, { MaxQueueSize }>
{
  /// See [napi_call_threadsafe_function](https://nodejs.org/api/n-api.html#n_api_napi_call_threadsafe_function)
  /// for more information.
  pub fn call(&self, value: Result<T, ErrorStatus>, mode: ThreadsafeFunctionCallMode) -> Status {
    self.handle.with_read_aborted(|aborted| {
      if aborted {
        return Status::Closing;
      }

      unsafe {
        sys::napi_call_threadsafe_function(
          self.handle.native_handle(),
          Box::into_raw(Box::new(value.map(|data| {
            ThreadsafeFunctionCallJsBackData {
              data,
              call_variant: ThreadsafeFunctionCallVariant::Direct,
              callback: Box::new(|_d: Result<Return>, _| Ok(())),
            }
          })))
          .cast(),
          mode.into(),
        )
      }
      .into()
    })
  }

  /// Call the ThreadsafeFunction, and handle the return value with a callback
  pub fn call_with_return_value<
    F: Send + 'static + for<'env> FnOnce(Result<Return>, Env<'env>) -> Result<()>,
  >(
    &self,
    value: Result<T, ErrorStatus>,
    mode: ThreadsafeFunctionCallMode,
    cb: F,
  ) -> Status {
    self.handle.with_read_aborted(|aborted| {
      if aborted {
        return Status::Closing;
      }

      unsafe {
        sys::napi_call_threadsafe_function(
          self.handle.native_handle(),
          Box::into_raw(Box::new(value.map(|data| {
            ThreadsafeFunctionCallJsBackData {
              data,
              call_variant: ThreadsafeFunctionCallVariant::WithCallback,
              callback: Box::new(move |d: Result<Return>, env: Env| cb(d, env)),
            }
          })))
          .cast(),
          mode.into(),
        )
      }
      .into()
    })
  }

  /// Call the ThreadsafeFunction, and handle the return value with in `async` way
  pub async fn call_async(&self, value: Result<T, ErrorStatus>) -> Result<Return>
  where
    Return: Send,
  {
    let (sender, receiver) = channel::<Result<Return>>();

    self.handle.with_read_aborted(|aborted| {
      if aborted {
        return Err(crate::Error::from_status(Status::Closing));
      }

      check_status!(
        unsafe {
          sys::napi_call_threadsafe_function(
            self.handle.native_handle(),
            Box::into_raw(Box::new(value.map(|data| {
              ThreadsafeFunctionCallJsBackData {
                data,
                call_variant: ThreadsafeFunctionCallVariant::WithCallback,
                callback: Box::new(move |d: Result<Return>, _| {
                  sender
                    .send(d)
                    // The only reason for send to return Err is if the receiver isn't listening
                    // Not hiding the error would result in a napi_fatal_error call, it's safe to ignore it instead.
                    .or(Ok(()))
                }),
              }
            })))
            .cast(),
            ThreadsafeFunctionCallMode::NonBlocking.into(),
          )
        },
        "Threadsafe function call_async failed"
      )
    })?;
    receiver.await.map_err(|_| {
      crate::Error::new(
        Status::GenericFailure,
        "Receive value from threadsafe function sender failed",
      )
    })?
  }

  /// Call the ThreadsafeFunction the same way `call_async` does, with explicit
  /// "catch the JavaScript throw" semantics.
  ///
  /// Provided so callers can use the same method name regardless of the `CalleeHandled` value.
  pub async fn call_async_catch(&self, value: Result<T, ErrorStatus>) -> Result<Return>
  where
    Return: Send,
  {
    self.call_async(value).await
  }
}

impl<
    T: 'static,
    Return: for<'env, 'scope> FromJs<'env, 'scope>,
    CallJsBackArgs: 'static,
    ErrorStatus: AsRef<str> + From<Status> + Send + 'static,
    const Weak: bool,
    const MaxQueueSize: usize,
  > ThreadsafeFunction<T, Return, CallJsBackArgs, ErrorStatus, false, { Weak }, { MaxQueueSize }>
{
  /// See [napi_call_threadsafe_function](https://nodejs.org/api/n-api.html#n_api_napi_call_threadsafe_function)
  /// for more information.
  pub fn call(&self, value: T, mode: ThreadsafeFunctionCallMode) -> Status {
    self.handle.with_read_aborted(|aborted| {
      if aborted {
        return Status::Closing;
      }

      unsafe {
        sys::napi_call_threadsafe_function(
          self.handle.native_handle(),
          Box::into_raw(Box::new(ThreadsafeFunctionCallJsBackData {
            data: value,
            call_variant: ThreadsafeFunctionCallVariant::Direct,
            callback: Box::new(|_d: Result<Return>, _: Env| Ok(())),
          }))
          .cast(),
          mode.into(),
        )
      }
      .into()
    })
  }

  /// Call the ThreadsafeFunction, and handle the return value with a callback
  pub fn call_with_return_value<
    F: Send + 'static + for<'env> FnOnce(Result<Return>, Env<'env>) -> Result<()>,
  >(
    &self,
    value: T,
    mode: ThreadsafeFunctionCallMode,
    cb: F,
  ) -> Status {
    self.handle.with_read_aborted(|aborted| {
      if aborted {
        return Status::Closing;
      }

      unsafe {
        sys::napi_call_threadsafe_function(
          self.handle.native_handle(),
          Box::into_raw(Box::new(ThreadsafeFunctionCallJsBackData {
            data: value,
            call_variant: ThreadsafeFunctionCallVariant::WithCallback,
            callback: Box::new(cb),
          }))
          .cast(),
          mode.into(),
        )
      }
      .into()
    })
  }

  /// Call the ThreadsafeFunction in an `async` way and return the JavaScript
  /// callback's resolved value.
  ///
  /// **Warning:** if the JavaScript callback throws, this method will route
  /// the captured exception through `napi_fatal_exception`, which terminates
  /// the host process. Use [`call_async_catch`](Self::call_async_catch)
  /// if you need to handle JavaScript-thrown errors as `Err(napi::Error)`.
  pub async fn call_async(&self, value: T) -> Result<Return>
  where
    Return: Send,
  {
    let (sender, receiver) = channel::<Return>();

    self.handle.with_read_aborted(|aborted| {
      if aborted {
        return Err(crate::Error::from_status(Status::Closing));
      }

      check_status!(unsafe {
        sys::napi_call_threadsafe_function(
          self.handle.native_handle(),
          Box::into_raw(Box::new(ThreadsafeFunctionCallJsBackData {
            data: value,
            call_variant: ThreadsafeFunctionCallVariant::WithCallback,
            callback: Box::new(move |d, _| {
              d.and_then(|d| {
                sender
                  .send(d)
                  // The only reason for send to return Err is if the receiver isn't listening
                  // Not hiding the error would result in a napi_fatal_error call, it's safe to ignore it instead.
                  .or(Ok(()))
              })
            }),
          }))
          .cast(),
          ThreadsafeFunctionCallMode::NonBlocking.into(),
        )
      })
    })?;

    receiver
      .await
      .map_err(|err| crate::Error::new(Status::GenericFailure, format!("{err}")))
  }

  /// Call the ThreadsafeFunction in an `async` way and catch JavaScript-thrown
  /// errors as `Err(napi::Error)` instead of crashing the host process.
  ///
  /// The returned `Err` carries `status == Status::PendingException` when it
  /// originated from a JS throw. The error is copied into Rust-owned status,
  /// message, and cause data before it leaves the callback frame.
  pub async fn call_async_catch(&self, value: T) -> Result<Return>
  where
    Return: Send,
  {
    let (sender, receiver) = channel::<Result<Return>>();

    self.handle.with_read_aborted(|aborted| {
      if aborted {
        return Err(crate::Error::from_status(Status::Closing));
      }

      check_status!(
        unsafe {
          sys::napi_call_threadsafe_function(
            self.handle.native_handle(),
            Box::into_raw(Box::new(ThreadsafeFunctionCallJsBackData {
              data: value,
              call_variant: ThreadsafeFunctionCallVariant::WithCallback,
              callback: Box::new(move |d: Result<Return>, _| {
                sender
                  .send(d)
                  // The only reason for send to return Err is if the receiver isn't listening
                  // Not hiding the error would result in a napi_fatal_error call, it's safe to ignore it instead.
                  .or(Ok(()))
              }),
            }))
            .cast(),
            ThreadsafeFunctionCallMode::NonBlocking.into(),
          )
        },
        "Threadsafe function call_async_catch failed"
      )
    })?;
    receiver.await.map_err(|_| {
      crate::Error::new(
        Status::GenericFailure,
        "Receive value from threadsafe function sender failed",
      )
    })?
  }
}

unsafe extern "C" fn thread_finalize_cb<T: 'static, V: 'static, R>(
  #[allow(unused_variables)] env: sys::napi_env,
  finalize_data: *mut c_void,
  finalize_hint: *mut c_void,
) where
  for<'scope> V: IntoJsArgs<'scope>,
  R: 'static + for<'scope> FnMut(ThreadsafeCallContext<'scope, T>) -> Result<V>,
{
  let handle_option: Option<Arc<ThreadsafeFunctionHandle>> =
    unsafe { sync::Weak::from_raw(finalize_data.cast()).upgrade() };

  if let Some(handle) = handle_option {
    handle.with_write_aborted(|mut aborted_guard| {
      if !*aborted_guard {
        *aborted_guard = true;
      }
    });
  }

  crate::run_unwind_boundary("dropping threadsafe function callback", || {
    drop(unsafe { Box::<R>::from_raw(finalize_hint.cast()) });
  });
}

unsafe extern "C" fn call_js_cb<
  T: 'static,
  Return: 'static + for<'env, 'scope> FromJs<'env, 'scope>,
  V: 'static,
  ErrorStatus: AsRef<str> + From<Status> + Send + 'static,
  R,
  const CalleeHandled: bool,
>(
  raw_env: sys::napi_env,
  js_callback: sys::napi_value,
  context: *mut c_void,
  data: *mut c_void,
) where
  for<'scope> V: IntoJsArgs<'scope>,
  R: 'static + for<'scope> FnMut(ThreadsafeCallContext<'scope, T>) -> Result<V>,
{
  if data.is_null() {
    return;
  }

  // env and/or callback can be null when shutting down
  if raw_env.is_null() || js_callback.is_null() {
    unsafe {
      drop_call_js_cb_data::<T, Return, ErrorStatus, CalleeHandled>(data);
    }
    return;
  }

  let result = crate::catch_unwind_boundary("running threadsafe function callback", || unsafe {
    crate::bindgen_prelude::EnvRecord::enter_scope(raw_env, |scope| {
      let mut env_wrapper = *scope.env();
      let callback: &mut R = Box::leak(Box::from_raw(context.cast()));
      let val = if CalleeHandled {
        *Box::<Result<ThreadsafeFunctionCallJsBackData<T, Return>, ErrorStatus>>::from_raw(
          data.cast(),
        )
      } else {
        Ok(*Box::<ThreadsafeFunctionCallJsBackData<T, Return>>::from_raw(data.cast()))
      };

      let mut recv = ptr::null_mut();
      sys::napi_get_undefined(raw_env, &mut recv);

      // Follow async callback conventions: https://nodejs.org/en/knowledge/errors/what-are-the-error-conventions/
      // Check if the Result is okay, if so, pass a null as the first (error) argument automatically.
      // If the Result is an error, pass that as the first argument.
      let ret = val.and_then(|v| {
        (callback)(ThreadsafeCallContext {
          env: env_wrapper,
          value: v.data,
        })
        .and_then(|ret| {
          env_wrapper.with_scope(|scope| {
            let mut args = ret.into_js_args(scope)?;
            if CalleeHandled {
              let mut js_null = ptr::null_mut();
              sys::napi_get_null(raw_env, &mut js_null);
              args.insert_front(js_null);
            }
            let mut return_value = ptr::null_mut();
            let mut status = sys::napi_call_function(
              raw_env,
              recv,
              js_callback,
              args.len(),
              args.as_slice().as_ptr(),
              &mut return_value,
            );
            if let ThreadsafeFunctionCallVariant::WithCallback = v.call_variant {
              // throw Error in JavaScript callback
              let callback_arg =
                threadsafe_callback_return(scope, raw_env, &mut status, return_value);
              let callback_env = *scope.env();
              if let Err(err) = (v.callback)(callback_arg, callback_env) {
                sys::napi_fatal_exception(raw_env, JsError::from(err).into_value(raw_env));
              }
            }
            Ok(status)
          })
        })
        .map_err(|err| {
          let status = err.status.into();
          err.with_status(status)
        })
      });

      let status = match ret {
        Ok(status) => status,
        Err(e) if !CalleeHandled => {
          sys::napi_fatal_exception(raw_env, JsError::from(e).into_value(raw_env))
        }
        Err(e) => sys::napi_call_function(
          raw_env,
          recv,
          js_callback,
          1,
          [JsError::from(e).into_value(raw_env)].as_mut_ptr(),
          ptr::null_mut(),
        ),
      };
      handle_call_js_cb_status(status, raw_env);
      Ok(())
    })
  });

  match result {
    Some(Ok(())) => {}
    Some(Err(error)) => {
      unsafe { sys::napi_fatal_exception(raw_env, JsError::from(error).into_value(raw_env)) };
    }
    None => {}
  }
}

fn threadsafe_callback_return<'env, 'scope, Return>(
  scope: &mut Scope<'env, 'scope>,
  raw_env: sys::napi_env,
  status: &mut sys::napi_status,
  return_value: sys::napi_value,
) -> Result<Return>
where
  Return: FromJs<'env, 'scope>,
{
  if *status == sys::Status::napi_pending_exception {
    let mut exception = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_and_clear_last_exception(raw_env, &mut exception) },
      "Get pending exception from threadsafe function callback failed"
    )?;
    let raw_status = *status;
    *status = sys::Status::napi_ok;

    let mut error = callback_error_from_exception(scope, exception)?;
    error.status = Status::from(raw_status);
    Err(error)
  } else if *status == sys::Status::napi_ok {
    callback_value_from_return(scope, return_value)
  } else {
    Err(Error::new(
      Status::from(*status),
      "Call JavaScript callback failed in threadsafe function".to_owned(),
    ))
  }
}

fn callback_error_from_exception<'env, 'scope>(
  scope: &mut Scope<'env, 'scope>,
  exception: sys::napi_value,
) -> Result<Error> {
  let value = unsafe { Local::from_raw(exception) };
  Error::from_js(scope, value)
}

fn callback_value_from_return<'env, 'scope, Return>(
  scope: &mut Scope<'env, 'scope>,
  return_value: sys::napi_value,
) -> Result<Return>
where
  Return: FromJs<'env, 'scope>,
{
  let value = unsafe { Local::from_raw(return_value) };
  Return::from_js(scope, value)
}

unsafe fn drop_call_js_cb_data<
  T: 'static,
  Return: 'static,
  ErrorStatus: AsRef<str> + From<Status> + Send + 'static,
  const CalleeHandled: bool,
>(
  data: *mut c_void,
) {
  if CalleeHandled {
    unsafe {
      drop(Box::<
        Result<ThreadsafeFunctionCallJsBackData<T, Return>, ErrorStatus>,
      >::from_raw(data.cast()));
    }
  } else {
    unsafe {
      drop(Box::<ThreadsafeFunctionCallJsBackData<T, Return>>::from_raw(data.cast()));
    }
  }
}

fn handle_call_js_cb_status(status: sys::napi_status, raw_env: sys::napi_env) {
  if status == sys::Status::napi_ok {
    return;
  }
  if status == sys::Status::napi_pending_exception {
    let mut error_result = ptr::null_mut();
    if unsafe { sys::napi_get_and_clear_last_exception(raw_env, &mut error_result) }
      != sys::Status::napi_ok
    {
      eprintln!("napi-rs: failed to clear pending exception in threadsafe function callback");
      return;
    }

    // When shutting down, napi_fatal_exception sometimes returns another exception
    let stat = unsafe { sys::napi_fatal_exception(raw_env, error_result) };
    if stat != sys::Status::napi_ok && stat != sys::Status::napi_pending_exception {
      eprintln!("napi-rs: failed to raise fatal exception in threadsafe function callback");
    }
  } else {
    // During environment shutdown (e.g. Ctrl+C in a worker thread), any NAPI call
    // can fail. Bail out gracefully instead of panicking if we can't construct the
    // error object — there's nothing useful we can do in a half-torn-down env.
    let error_code: Status = status.into();
    let mut error_code_value = ptr::null_mut();
    if unsafe {
      sys::napi_create_string_utf8(
        raw_env,
        error_code.as_ref().as_ptr().cast(),
        error_code.as_ref().len() as isize,
        &mut error_code_value,
      )
    } != sys::Status::napi_ok
    {
      return;
    }
    const ERROR_MSG: &str = "Call JavaScript callback failed in threadsafe function";
    let mut error_msg_value = ptr::null_mut();
    if unsafe {
      sys::napi_create_string_utf8(
        raw_env,
        ERROR_MSG.as_ptr().cast(),
        ERROR_MSG.len() as isize,
        &mut error_msg_value,
      )
    } != sys::Status::napi_ok
    {
      return;
    }
    let mut error_value = ptr::null_mut();
    if unsafe {
      sys::napi_create_error(raw_env, error_code_value, error_msg_value, &mut error_value)
    } != sys::Status::napi_ok
    {
      return;
    }
    // When shutting down, napi_fatal_exception sometimes returns another exception
    let stat = unsafe { sys::napi_fatal_exception(raw_env, error_value) };
    assert!(stat == sys::Status::napi_ok || stat == sys::Status::napi_pending_exception);
  }
}

/// This is a placeholder type that is used to indicate that the return value of a threadsafe function is unknown.
/// Use this type when you don't care about the return value of a threadsafe function.
///
/// And you can't get the value of it as well because it's just a placeholder.
pub struct UnknownReturnValue;

impl TypeName for UnknownReturnValue {
  fn type_name() -> &'static str {
    "UnknownReturnValue"
  }

  fn value_type() -> crate::ValueType {
    crate::ValueType::Unknown
  }
}

impl ValidateNapiValue for UnknownReturnValue {}

impl<'env, 'scope> FromJs<'env, 'scope> for UnknownReturnValue {
  fn from_js(_: &mut Scope<'env, 'scope>, _: Local<'scope, Unknown<'scope>>) -> Result<Self> {
    Ok(UnknownReturnValue)
  }
}
