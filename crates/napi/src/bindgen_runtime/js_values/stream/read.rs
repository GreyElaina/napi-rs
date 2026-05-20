use std::{
  ffi::c_void,
  marker::PhantomData,
  mem,
  pin::Pin,
  ptr,
  sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
  },
  task::{Context, Poll},
};

use tokio::sync::Mutex;

use futures_core::Stream;
use tokio_stream::StreamExt;

use crate::{
  bindgen_prelude::{
    BufferSlice, FnArgs, FromJs, Function, IntoJsArgs, JsObjectValue, Local, Object, Promise,
    PromiseFuture, Scope, TypeName, Unknown, ValidateNapiValue, NAPI_AUTO_LENGTH,
  },
  bindgen_runtime::{with_env, CallbackDecoder, IntoJs},
  check_status, sys,
  threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode},
  Env, Error, JsError, JsValue, Result, Status, Value, ValueType,
};

pub struct ReadableStream<'env, T> {
  pub(crate) value: sys::napi_value,
  pub(crate) env: sys::napi_env,
  _marker: PhantomData<&'env T>,
}

impl<'env, T> JsValue<'env> for ReadableStream<'env, T> {
  fn value(&self) -> Value {
    Value {
      env: self.env,
      value: self.value,
      value_type: ValueType::Object,
    }
  }
}

impl<'env, T> JsObjectValue<'env> for ReadableStream<'env, T> {}

impl<T> TypeName for ReadableStream<'_, T> {
  fn type_name() -> &'static str {
    "ReadableStream"
  }

  fn value_type() -> ValueType {
    ValueType::Object
  }
}

impl<T> ValidateNapiValue for ReadableStream<'_, T> {
  unsafe fn validate(
    env: napi_sys::napi_env,
    napi_val: napi_sys::napi_value,
  ) -> Result<napi_sys::napi_value> {
    unsafe {
      with_env(env, |mut env_wrapper| {
        env_wrapper.with_scope(|scope| {
          let global = scope.env().get_global()?;
          let constructor: Function<'_, (), ()> =
            scope.get_named_property(&global, "ReadableStream")?;
          let mut is_instance = false;
          check_status!(
            sys::napi_instanceof(env, napi_val, constructor.value, &mut is_instance),
            "Check ReadableStream instance failed"
          )?;
          if !is_instance {
            return Err(Error::new(
              Status::InvalidArg,
              "Value is not a ReadableStream",
            ));
          }
          Ok(ptr::null_mut())
        })
      })
    }
  }
}

impl<'env, 'scope, T> FromJs<'env, 'scope> for ReadableStream<'scope, T> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(Self {
      value: value.raw(),
      env: scope.env().raw(),
      _marker: PhantomData,
    })
  }
}

impl<T> ReadableStream<'_, T> {
  fn with_stream_object<R>(
    &self,
    f: impl for<'env, 'scope> FnOnce(
      &mut Scope<'env, 'scope>,
      Local<'scope, Unknown<'scope>>,
      Object<'scope>,
    ) -> Result<R>,
  ) -> Result<R> {
    unsafe {
      with_env(self.env, |mut env| {
        env.with_scope(|scope| {
          let stream = Local::from_value(scope, self, "ReadableStream")?;
          let stream_object = Object::from_js(scope, stream)?;
          f(scope, stream, stream_object)
        })
      })
    }
  }

  /// Returns a boolean indicating whether the readable stream is locked to a reader.
  pub fn locked(&self) -> Result<bool> {
    self.with_stream_object(|scope, _, stream| scope.get_named_property(&stream, "locked"))
  }

  /// The `cancel()` method of the `ReadableStream` interface returns a Promise that resolves when the stream is canceled.
  pub fn cancel(&mut self, reason: Option<String>) -> Result<Promise<'_, ()>> {
    let promise = self.with_stream_object(|scope, stream, stream_object| {
      let cancel: Function<'_, FnArgs<(Option<String>,)>, Promise<'_, ()>> =
        scope.get_named_property(&stream_object, "cancel")?;
      let promise = scope.apply(&cancel, stream, FnArgs::from((reason,)))?;
      Ok(promise.value().value)
    })?;
    Ok(unsafe { Promise::from_raw(self.env, promise) })
  }
}

impl<T: Send + Sync + 'static + for<'env, 'scope> FromJs<'env, 'scope>> ReadableStream<'_, T> {
  pub fn read(&self) -> Result<Reader<T>> {
    let read_function = self.with_stream_object(|scope, stream, stream_object| {
      let get_reader: Function<'_, (), Object<'_>> =
        scope.get_named_property(&stream_object, "getReader")?;
      let reader = scope.apply(&get_reader, stream, ())?;
      let read: Function<'_, (), PromiseFuture<IteratorValue<T>>> =
        scope.get_named_property(&reader, "read")?;
      scope
        .bind_function(&read, reader)?
        .build_threadsafe_function()
        .callee_handled::<true>()
        .weak::<true>()
        .build()
    })?;
    Ok(Reader {
      inner: read_function,
      state: Arc::new((RwLock::new(Ok(None)), AtomicBool::new(false))),
    })
  }
}

impl<T: for<'scope> IntoJs<'scope> + Send + 'static> ReadableStream<'_, T> {
  pub fn new<S: Stream<Item = Result<T>> + Unpin + Send + 'static>(
    env: &Env,
    inner: S,
  ) -> Result<Self> {
    // Create shared state for the stream
    let state = StreamState::new(inner);
    let state_ptr = Arc::into_raw(state) as *mut c_void;

    let mut underlying_source = Object::new(env)?;

    // Create pull callback
    let mut pull_fn = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_create_function(
          env.raw(),
          c"pull".as_ptr().cast(),
          NAPI_AUTO_LENGTH,
          Some(pull_callback::<T, S>),
          state_ptr,
          &mut pull_fn,
        )
      },
      "Failed to create pull function"
    )?;
    unsafe { underlying_source.set_inner("pull", pull_fn)? };

    // Create cancel callback for cleanup
    let mut cancel_fn = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_create_function(
          env.raw(),
          c"cancel".as_ptr().cast(),
          NAPI_AUTO_LENGTH,
          Some(cancel_callback::<S>),
          state_ptr,
          &mut cancel_fn,
        )
      },
      "Failed to create cancel function"
    )?;
    unsafe { underlying_source.set_inner("cancel", cancel_fn)? };

    // Register invoke to free the Arc when underlying_source is GC'd
    register_invoke::<S>(env.raw(), underlying_source.0.value, state_ptr)?;

    unsafe {
      with_env(env.raw(), |mut env| {
        env.with_scope(|scope| {
          let global = scope.env().get_global()?;
          let constructor: Unknown = scope.get_named_property(&global, "ReadableStream")?;
          if constructor.get_type()? == ValueType::Undefined {
            return Err(Error::new(
              Status::GenericFailure,
              "ReadableStream is not supported in this Node.js version",
            ));
          }
          let constructor = Local::from_value(scope, &constructor, "ReadableStream")?;
          let constructor: Function<'_, FnArgs<(Object<'_>,)>, Unknown<'_>> =
            Function::from_js(scope, constructor)?;
          let stream = scope.new_instance(&constructor, FnArgs::from((underlying_source,)))?;
          Ok(Self {
            value: stream.value().value,
            env: scope.env().raw(),
            _marker: PhantomData,
          })
        })
      })
    }
  }

  /// Creates a new `ReadableStream` with the given `stream` and `ReadableStream` class.
  ///
  /// This is useful if the runtime only supports Node-API 4 but doesn't support the WebStream API.
  ///
  /// Node-API 4 was initially introduced in `v10.16.0` and WebStream was introduced in `v16.5.0`.
  pub fn with_readable_stream_class<S: Stream<Item = Result<T>> + Unpin + Send + 'static>(
    env: &Env,
    readable_stream_class: &Unknown,
    inner: S,
  ) -> Result<Self> {
    if readable_stream_class.get_type()? == ValueType::Undefined {
      return Err(Error::new(
        Status::GenericFailure,
        "ReadableStream is not supported in this Node.js version",
      ));
    }

    // Create shared state for the stream
    let state = StreamState::new(inner);
    let state_ptr = Arc::into_raw(state) as *mut c_void;

    let mut underlying_source = Object::new(env)?;

    // Create pull callback
    let mut pull_fn = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_create_function(
          env.raw(),
          c"pull".as_ptr().cast(),
          NAPI_AUTO_LENGTH,
          Some(pull_callback::<T, S>),
          state_ptr,
          &mut pull_fn,
        )
      },
      "Failed to create pull function"
    )?;
    unsafe { underlying_source.set_inner("pull", pull_fn)? };

    // Create cancel callback for cleanup
    let mut cancel_fn = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_create_function(
          env.raw(),
          c"cancel".as_ptr().cast(),
          NAPI_AUTO_LENGTH,
          Some(cancel_callback::<S>),
          state_ptr,
          &mut cancel_fn,
        )
      },
      "Failed to create cancel function"
    )?;
    unsafe { underlying_source.set_inner("cancel", cancel_fn)? };

    // Register invoke to free the Arc when underlying_source is GC'd
    register_invoke::<S>(env.raw(), underlying_source.0.value, state_ptr)?;

    unsafe {
      with_env(env.raw(), |mut env| {
        env.with_scope(|scope| {
          let readable_stream_class =
            Local::from_value(scope, readable_stream_class, "ReadableStream")?;
          let constructor: Function<'_, FnArgs<(Object<'_>,)>, Unknown<'_>> =
            Function::from_js(scope, readable_stream_class)?;
          let stream = scope.new_instance(&constructor, FnArgs::from((underlying_source,)))?;
          Ok(Self {
            value: stream.value().value,
            env: scope.env().raw(),
            _marker: PhantomData,
          })
        })
      })
    }
  }
}

impl<'env> ReadableStream<'env, BufferSlice<'env>> {
  /// Creates a new `ReadableStream` with the given `stream` that emits bytes.
  pub fn create_with_stream_bytes<
    B: Into<Vec<u8>>,
    S: Stream<Item = Result<B>> + Unpin + Send + 'static,
  >(
    env: &Env,
    inner: S,
  ) -> Result<Self> {
    // Create shared state for the stream
    let state = StreamState::new(inner);
    let state_ptr = Arc::into_raw(state) as *mut c_void;

    let mut underlying_source = Object::new(env)?;

    // Create pull callback
    let mut pull_fn = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_create_function(
          env.raw(),
          c"pull".as_ptr().cast(),
          NAPI_AUTO_LENGTH,
          Some(pull_callback_bytes::<B, S>),
          state_ptr,
          &mut pull_fn,
        )
      },
      "Failed to create pull function"
    )?;
    unsafe { underlying_source.set_inner("pull", pull_fn)? };

    // Create cancel callback for cleanup
    let mut cancel_fn = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_create_function(
          env.raw(),
          c"cancel".as_ptr().cast(),
          NAPI_AUTO_LENGTH,
          Some(cancel_callback::<S>),
          state_ptr,
          &mut cancel_fn,
        )
      },
      "Failed to create cancel function"
    )?;
    unsafe { underlying_source.set_inner("cancel", cancel_fn)? };

    // Register invoke to free the Arc when underlying_source is GC'd
    register_invoke::<S>(env.raw(), underlying_source.0.value, state_ptr)?;

    underlying_source.set("type", "bytes")?;
    unsafe {
      with_env(env.raw(), |mut env| {
        env.with_scope(|scope| {
          let global = scope.env().get_global()?;
          let constructor: Function<'_, FnArgs<(Object<'_>,)>, Unknown<'_>> =
            scope.get_named_property(&global, "ReadableStream")?;
          let stream = scope.new_instance(&constructor, FnArgs::from((underlying_source,)))?;
          Ok(Self {
            value: stream.value().value,
            env: scope.env().raw(),
            _marker: PhantomData,
          })
        })
      })
    }
  }

  /// create a new `ReadableStream` with the given `stream` that emits bytes and `ReadableStream` class.
  pub fn with_stream_bytes_and_readable_stream_class<
    B: Into<Vec<u8>>,
    S: Stream<Item = Result<B>> + Unpin + Send + 'static,
  >(
    env: &Env,
    readable_stream_class: &Unknown,
    inner: S,
  ) -> Result<Self> {
    if readable_stream_class.get_type()? == ValueType::Undefined {
      return Err(Error::new(
        Status::GenericFailure,
        "ReadableStream is not supported in this Node.js version",
      ));
    }

    // Create shared state for the stream
    let state = StreamState::new(inner);
    let state_ptr = Arc::into_raw(state) as *mut c_void;

    let mut underlying_source = Object::new(env)?;

    // Create pull callback
    let mut pull_fn = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_create_function(
          env.raw(),
          c"pull".as_ptr().cast(),
          NAPI_AUTO_LENGTH,
          Some(pull_callback_bytes::<B, S>),
          state_ptr,
          &mut pull_fn,
        )
      },
      "Failed to create pull function"
    )?;
    unsafe { underlying_source.set_inner("pull", pull_fn)? };

    // Create cancel callback for cleanup
    let mut cancel_fn = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_create_function(
          env.raw(),
          c"cancel".as_ptr().cast(),
          NAPI_AUTO_LENGTH,
          Some(cancel_callback::<S>),
          state_ptr,
          &mut cancel_fn,
        )
      },
      "Failed to create cancel function"
    )?;
    unsafe { underlying_source.set_inner("cancel", cancel_fn)? };

    // Register invoke to free the Arc when underlying_source is GC'd
    register_invoke::<S>(env.raw(), underlying_source.0.value, state_ptr)?;

    underlying_source.set("type", "bytes")?;
    unsafe {
      with_env(env.raw(), |mut env| {
        env.with_scope(|scope| {
          let readable_stream_class =
            Local::from_value(scope, readable_stream_class, "ReadableStream")?;
          let constructor: Function<'_, FnArgs<(Object<'_>,)>, Unknown<'_>> =
            Function::from_js(scope, readable_stream_class)?;
          let stream = scope.new_instance(&constructor, FnArgs::from((underlying_source,)))?;
          Ok(Self {
            value: stream.value().value,
            env: scope.env().raw(),
            _marker: PhantomData,
          })
        })
      })
    }
  }
}

pub struct IteratorValue<T> {
  value: Option<T>,
  done: bool,
}

impl<'env, 'scope, T> FromJs<'env, 'scope> for IteratorValue<T>
where
  T: FromJs<'env, 'scope>,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let value = Object::from_js(scope, value)?;
    let done = scope.get_named_property(&value, "done")?;
    let value = scope.get_named_property(&value, "value")?;
    Ok(Self { value, done })
  }
}

pub struct Reader<T: Send + Sync + 'static + for<'env, 'scope> FromJs<'env, 'scope>> {
  inner: ThreadsafeFunction<(), PromiseFuture<IteratorValue<T>>, (), Status, true, true>,
  state: Arc<(RwLock<Result<Option<T>>>, AtomicBool)>,
}

impl<T: Send + Sync + 'static + for<'env, 'scope> FromJs<'env, 'scope>> futures_core::Stream
  for Reader<T>
{
  type Item = Result<T>;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    if self.state.1.load(Ordering::Relaxed) {
      let mut chunk = self
        .state
        .0
        .write()
        .map_err(|_| Error::new(Status::InvalidArg, "Poisoned lock in Reader::poll_next"))?;
      let chunk = mem::replace(&mut *chunk, Ok(None))?;
      match chunk {
        Some(chunk) => return Poll::Ready(Some(Ok(chunk))),
        None => return Poll::Ready(None),
      }
    }
    let waker = cx.waker().clone();
    let state = self.state.clone();
    let state_in_catch = state.clone();
    self.inner.call_with_return_value(
      Ok(()),
      ThreadsafeFunctionCallMode::NonBlocking,
      move |iterator, _| {
        let iterator = iterator?;
        crate::spawn(async move {
          let result = iterator.await;
          let update_result = match result {
            Ok(iterator) => {
              if iterator.done {
                state.1.store(true, Ordering::Relaxed);
              }
              if let Some(val) = iterator.value {
                state
                  .0
                  .write()
                  .map(|mut chunk| {
                    *chunk = Ok(Some(val));
                  })
                  .map_err(|_| Error::new(Status::InvalidArg, "Poisoned lock in Reader::poll_next"))
              } else {
                Ok(())
              }
            }
            Err(error) => state_in_catch
              .0
              .write()
              .map(|mut chunk| {
                *chunk = Err(error);
              })
              .map_err(|_| Error::new(Status::InvalidArg, "Poisoned lock in Reader::poll_next")),
          };
          if let Err(error) = update_result {
            if let Ok(mut chunk) = state_in_catch.0.write() {
              *chunk = Err(error);
            }
          }
          waker.wake();
        });
        Ok(())
      },
    );
    let mut chunk = self
      .state
      .0
      .write()
      .map_err(|_| Error::new(Status::InvalidArg, "Poisoned lock in Reader::poll_next"))?;
    let chunk = mem::replace(&mut *chunk, Ok(None))?;
    match chunk {
      Some(chunk) => Poll::Ready(Some(Ok(chunk))),
      None => Poll::Pending,
    }
  }
}

/// Shared state for ReadableStream that coordinates between pull and cancel callbacks.
/// Uses Arc to share ownership between callbacks, Mutex to protect the stream,
/// and AtomicBool for lock-free cancellation checks.
///
/// Memory management: The Arc is freed by a invoke when the underlying_source
/// object is garbage collected. Callbacks only "borrow" the Arc using the
/// increment+from_raw pattern, never freeing it directly. This prevents
/// use-after-free if cancel_callback is invoked after pull_callback has
/// already closed the stream.
struct StreamState<S> {
  stream: Mutex<Option<Pin<Box<S>>>>,
  cancelled: AtomicBool,
}

impl<S> StreamState<S> {
  fn new(stream: S) -> Arc<Self> {
    Arc::new(Self {
      stream: Mutex::new(Some(Box::pin(stream))),
      cancelled: AtomicBool::new(false),
    })
  }
}

/// invoke callback that frees the Arc<StreamState> when the underlying_source
/// object is garbage collected. This is the only place where the Arc is freed,
/// ensuring that callbacks can safely borrow without risk of use-after-free.
unsafe extern "C" fn invoke<S>(
  _env: sys::napi_env,
  finalize_data: *mut c_void,
  _finalize_hint: *mut c_void,
) {
  if !finalize_data.is_null() {
    // Consume the Arc, dropping it and freeing memory
    drop(unsafe { Arc::from_raw(finalize_data.cast::<StreamState<S>>()) });
  }
}

/// Registers a invoke on the underlying_source object that will free the Arc<StreamState>
/// when the object is garbage collected.
fn register_invoke<S>(
  env: sys::napi_env,
  underlying_source: sys::napi_value,
  state_ptr: *mut c_void,
) -> Result<()> {
  check_status!(
    unsafe {
      sys::napi_add_finalizer(
        env,
        underlying_source,
        state_ptr,
        Some(invoke::<S>),
        ptr::null_mut(),
        ptr::null_mut(),
      )
    },
    "Failed to add invoke to underlying source"
  )
}

/// Helper struct to extract and bind controller methods from callback info.
struct PullController<T: for<'scope> IntoJs<'scope>> {
  enqueue: ControllerFunctionRef<T, ()>,
  close: ControllerFunctionRef<(), ()>,
}

struct ControllerFunctionRef<Args, Return> {
  raw: sys::napi_ref,
  marker: PhantomData<fn(Args) -> Return>,
}

unsafe impl<Args, Return> Send for ControllerFunctionRef<Args, Return> {}

impl<Args, Return> ControllerFunctionRef<Args, Return> {
  fn new(scope: &mut Scope<'_, '_>, function: &Function<'_, Args, Return>) -> Result<Self> {
    let mut raw = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_reference(scope.env().raw(), function.value, 1, &mut raw) },
      "Create stream controller function reference failed"
    )?;
    Ok(Self {
      raw,
      marker: PhantomData,
    })
  }

  fn borrow<'env, 'scope>(
    &self,
    scope: &mut Scope<'env, 'scope>,
  ) -> Result<Function<'scope, Args, Return>> {
    let mut value = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_reference_value(scope.env().raw(), self.raw, &mut value) },
      "Get stream controller function reference failed"
    )?;
    Function::from_js(scope, unsafe { Local::from_raw(value) })
  }

  fn close(mut self, env: &Env) -> Result<()> {
    let raw = std::mem::replace(&mut self.raw, ptr::null_mut());
    check_status!(
      unsafe { sys::napi_delete_reference(env.raw(), raw) },
      "Delete stream controller function reference failed"
    )
  }
}

impl<T: for<'scope> IntoJs<'scope>> PullController<T> {
  fn from_callback_info(
    env: Env<'_>,
    info: sys::napi_callback_info,
  ) -> Result<(Self, *mut c_void)> {
    let mut decoder = CallbackDecoder::<1>::new(env, info, Some(1))?;
    decoder.with_frame(|mut frame| {
      let data = frame.raw_data();
      let controller = frame.arg::<Object>(0)?;
      let scope = frame.scope_mut();
      let enqueue_function: Function<'_, T, ()> =
        scope.get_named_property(&controller, "enqueue")?;
      let enqueue_function = scope.bind_function(&enqueue_function, controller)?;
      let enqueue = ControllerFunctionRef::new(scope, &enqueue_function)?;
      let close_function: Function<'_, (), ()> = scope.get_named_property(&controller, "close")?;
      let close_function = scope.bind_function(&close_function, controller)?;
      let close = ControllerFunctionRef::new(scope, &close_function)?;
      Ok((Self { enqueue, close }, data))
    })
  }

  fn close(self, env: &Env) -> Result<()> {
    let close_enqueue = self.enqueue.close(env);
    let close_close = self.close.close(env);
    close_enqueue.and(close_close)
  }
}

unsafe extern "C" fn cancel_callback<S>(
  env: sys::napi_env,
  info: sys::napi_callback_info,
) -> sys::napi_value {
  let result = unsafe { with_env(env, |env| cancel_callback_impl::<S>(env, info)) };
  if let Err(err) = result {
    unsafe {
      let js_error: JsError = err.into();
      js_error.throw_into(env);
    }
  }
  ptr::null_mut()
}

fn cancel_callback_impl<S>(env: Env<'_>, info: sys::napi_callback_info) -> Result<()> {
  let mut decoder = CallbackDecoder::<0>::new(env, info, None)?;
  decoder.with_frame(|frame| {
    let data = frame.raw_data();
    if !data.is_null() {
      // Borrow the Arc using increment+from_raw pattern.
      // The invoke registered on underlying_source will free the Arc when GC'd.
      // This prevents use-after-free if cancel is called after stream has closed.
      let state = unsafe {
        Arc::increment_strong_count(data.cast::<StreamState<S>>());
        Arc::from_raw(data.cast::<StreamState<S>>())
      };

      // Mark as cancelled so pull callback knows to stop
      state.cancelled.store(true, Ordering::SeqCst);

      // Try to take the stream - use try_lock to avoid blocking the event loop.
      // If we can't get the lock (pull is in progress), that's fine - pull will
      // see the cancelled flag and handle cleanup.
      if let Ok(mut guard) = state.stream.try_lock() {
        drop(guard.take());
      };
      // Borrowed Arc drops here, decrementing ref count (but not freeing - invoke handles that)
    }
    Ok(())
  })
}

unsafe extern "C" fn pull_callback<
  T: for<'scope> IntoJs<'scope> + Send + 'static,
  S: Stream<Item = Result<T>> + Unpin + Send + 'static,
>(
  env: sys::napi_env,
  info: sys::napi_callback_info,
) -> sys::napi_value {
  let result = unsafe {
    with_env(env, |env_wrapper| {
      pull_callback_impl::<T, S>(env_wrapper, info)
    })
  };
  match result {
    Ok(val) => val,
    Err(err) => unsafe {
      let js_error: JsError = err.into();
      js_error.throw_into(env);
      ptr::null_mut()
    },
  }
}

fn pull_callback_impl<
  T: for<'scope> IntoJs<'scope> + for<'scope> IntoJsArgs<'scope> + Send + 'static,
  S: Stream<Item = Result<T>> + Unpin + Send + 'static,
>(
  env_wrapper: Env<'_>,
  info: sys::napi_callback_info,
) -> Result<sys::napi_value> {
  let (controller, data) = PullController::<T>::from_callback_info(env_wrapper, info)?;

  // Borrow the Arc<StreamState> using the increment+from_raw pattern.
  // The invoke registered on underlying_source will free the Arc when GC'd.
  // This prevents use-after-free if cancel is called after stream has closed.
  let state = unsafe {
    Arc::increment_strong_count(data.cast::<StreamState<S>>());
    Arc::from_raw(data.cast::<StreamState<S>>())
  };

  // Check if stream was cancelled
  if state.cancelled.load(Ordering::SeqCst) {
    controller.close(&env_wrapper)?;
    return Ok(ptr::null_mut());
  }

  let state_for_async = state.clone();

  let promise = env_wrapper.spawn_future_with_callback(
    async move {
      let mut guard = state_for_async.stream.lock().await;
      if let Some(ref mut stream) = *guard {
        stream.next().await.transpose()
      } else {
        Ok(None)
      }
    },
    move |scope, val| {
      // Use inner closure to ensure controller refs close on all paths.
      let result = {
        // Re-check cancelled flag after async work completes to prevent
        // enqueueing if cancel was called while waiting for the next item
        if state.cancelled.load(Ordering::SeqCst) {
          // Stream was cancelled while waiting - skip enqueue and close
        } else if let Some(val) = val {
          let enqueue_fn = controller.enqueue.borrow(scope)?;
          scope.call(&enqueue_fn, val)?;
        } else {
          let close_fn = controller.close.borrow(scope)?;
          scope.call(&close_fn, ())?;
          // Stream ended - take the inner stream to free resources early
          // (the Arc itself is freed by the invoke when underlying_source is GC'd)
          if let Ok(mut guard) = state.stream.try_lock() {
            let _ = guard.take();
          }
        }
        Ok::<(), Error>(())
      };
      let close_result = controller.close(scope.env());
      result.and(close_result)?;
      Ok(())
    },
  )?;
  Ok(promise.inner)
}

unsafe extern "C" fn pull_callback_bytes<
  B: Into<Vec<u8>>,
  S: Stream<Item = Result<B>> + Unpin + Send + 'static,
>(
  env: sys::napi_env,
  info: sys::napi_callback_info,
) -> sys::napi_value {
  let result = unsafe {
    with_env(env, |env_wrapper| {
      pull_callback_impl_bytes::<B, S>(env_wrapper, info)
    })
  };
  match result {
    Ok(val) => val,
    Err(err) => unsafe {
      let js_error: JsError = err.into();
      js_error.throw_into(env);
      ptr::null_mut()
    },
  }
}

fn pull_callback_impl_bytes<
  B: Into<Vec<u8>>,
  S: Stream<Item = Result<B>> + Unpin + Send + 'static,
>(
  env_wrapper: Env<'_>,
  info: sys::napi_callback_info,
) -> Result<sys::napi_value> {
  let (controller, data) = PullController::<BufferSlice>::from_callback_info(env_wrapper, info)?;

  // Borrow the Arc<StreamState> using the increment+from_raw pattern.
  // The invoke registered on underlying_source will free the Arc when GC'd.
  // This prevents use-after-free if cancel is called after stream has closed.
  let state = unsafe {
    Arc::increment_strong_count(data.cast::<StreamState<S>>());
    Arc::from_raw(data.cast::<StreamState<S>>())
  };

  // Check if stream was cancelled
  if state.cancelled.load(Ordering::SeqCst) {
    controller.close(&env_wrapper)?;
    return Ok(ptr::null_mut());
  }

  let state_for_async = state.clone();

  let promise = env_wrapper.spawn_future_with_callback(
    async move {
      let mut guard = state_for_async.stream.lock().await;
      if let Some(ref mut stream) = *guard {
        stream
          .next()
          .await
          .transpose()
          .map(|v| v.map(|v| Into::<Vec<u8>>::into(v)))
      } else {
        Ok(None)
      }
    },
    move |scope, val| {
      // Use inner closure to ensure controller refs close on all paths.
      let result = {
        // Re-check cancelled flag after async work completes to prevent
        // enqueueing if cancel was called while waiting for the next item
        if state.cancelled.load(Ordering::SeqCst) {
          // Stream was cancelled while waiting - skip enqueue and close
        } else if let Some(val) = val {
          let env = *scope.env();
          let chunk = BufferSlice::from_data(&env, val)?;
          let enqueue_fn = controller.enqueue.borrow(scope)?;
          scope.call(&enqueue_fn, chunk)?;
        } else {
          let close_fn = controller.close.borrow(scope)?;
          scope.call(&close_fn, ())?;
          // Stream ended - take the inner stream to free resources early
          // (the Arc itself is freed by the invoke when underlying_source is GC'd)
          if let Ok(mut guard) = state.stream.try_lock() {
            let _ = guard.take();
          }
        }
        Ok::<(), Error>(())
      };
      let close_result = controller.close(scope.env());
      result.and(close_result)?;
      Ok(())
    },
  )?;
  Ok(promise.inner)
}
