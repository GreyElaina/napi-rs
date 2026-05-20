use std::cell::Cell;
use std::marker::PhantomData;
use std::os::raw::c_void;
use std::panic::UnwindSafe;
use std::ptr;
use std::rc::Rc;

use crate::bindgen_runtime::{with_env, IntoJs, JsObjectValue, Scope};
use crate::{check_status, sys, Env, Error, JsError, Result, Status};

struct NapiBlockingWorkDriver<Execute, Complete, Output, JsValue> {
  execute: Option<Execute>,
  complete: Option<Complete>,
  result: ComputationResult<Output>,
  completion: PromiseCompletion,
  marker: PhantomData<fn() -> JsValue>,
}

enum ComputationResult<T> {
  Pending,
  Ready(Result<T>),
}

struct PromiseCompletion {
  deferred: Deferred,
  handle: BlockingWorkHandle,
  status: Rc<Cell<BlockingWorkStatus>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockingWorkStatus {
  Pending,
  Completed,
  Cancelled,
}

struct Deferred {
  raw: sys::napi_deferred,
}

struct BlockingWorkHandle {
  raw: sys::napi_async_work,
}

#[derive(Clone, Copy)]
pub(crate) struct BlockingWorkCancelHandle {
  raw: sys::napi_async_work,
}

struct CompletionCleanup {
  first_error: Option<Error>,
}

pub(crate) struct BlockingWorkPromise<T> {
  pub(crate) cancel_handle: BlockingWorkCancelHandle,
  raw_promise: sys::napi_value,
  _phantom: PhantomData<T>,
}

impl<T> UnwindSafe for BlockingWorkPromise<T> {}
impl<T> std::panic::RefUnwindSafe for BlockingWorkPromise<T> {}

impl<T> BlockingWorkPromise<T> {
  pub(crate) fn raw_promise(&self) -> sys::napi_value {
    self.raw_promise
  }
}

impl Deferred {
  fn create_promise(env: &Env<'_>) -> Result<(Self, sys::napi_value)> {
    let mut raw_promise = ptr::null_mut();
    let mut raw_deferred = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_promise(env.raw(), &mut raw_deferred, &mut raw_promise) },
      "Create promise failed in blocking_work::run"
    )?;
    Ok((Self { raw: raw_deferred }, raw_promise))
  }

  fn resolve(self, env: &Env<'_>, value: sys::napi_value) -> Result<()> {
    check_status!(
      unsafe { sys::napi_resolve_deferred(env.raw(), self.raw, value) },
      "Resolve promise failed"
    )
  }

  fn reject(self, env: &Env<'_>, error: Error) -> Result<()> {
    check_status!(
      unsafe {
        sys::napi_reject_deferred(
          env.raw(),
          self.raw,
          JsError::from(error).into_value(env.raw()),
        )
      },
      "Reject promise failed"
    )
  }

  fn reject_cancelled(self, env: &Env<'_>) -> Result<()> {
    const ABORT_ERROR_NAME: &str = "AbortError";
    let mut error = env.create_error(Error::new(Status::Cancelled, ABORT_ERROR_NAME.to_owned()))?;
    error.set_named_property("name", ABORT_ERROR_NAME)?;
    check_status!(
      unsafe { sys::napi_reject_deferred(env.raw(), self.raw, error.0.value) },
      "Reject AbortError failed"
    )
  }
}

impl BlockingWorkHandle {
  fn empty() -> Self {
    Self {
      raw: ptr::null_mut(),
    }
  }

  fn cancel_handle(&self) -> BlockingWorkCancelHandle {
    BlockingWorkCancelHandle { raw: self.raw }
  }

  fn delete(self, env: &Env<'_>) -> Result<()> {
    if self.raw.is_null() {
      return Ok(());
    }

    check_status!(
      unsafe { sys::napi_delete_async_work(env.raw(), self.raw) },
      "Delete blocking work failed"
    )
  }
}

impl BlockingWorkCancelHandle {
  pub(crate) fn cancel(self, env: &Env<'_>) -> Result<()> {
    check_status!(
      unsafe { sys::napi_cancel_async_work(env.raw(), self.raw) },
      "Cancel blocking work failed"
    )
  }
}

impl CompletionCleanup {
  fn new() -> Self {
    Self { first_error: None }
  }

  fn record(&mut self, result: Result<()>) {
    if let Err(error) = result {
      self.first_error.get_or_insert(error);
    }
  }

  fn finish(self) -> Result<()> {
    match self.first_error {
      Some(error) => Err(error),
      None => Ok(()),
    }
  }
}

impl<T> ComputationResult<T> {
  fn into_ready(self) -> Result<T> {
    match self {
      Self::Ready(result) => result,
      Self::Pending => Err(Error::new(
        Status::GenericFailure,
        "Blocking work completed before compute produced a result".to_owned(),
      )),
    }
  }
}

impl PromiseCompletion {
  fn cancel_handle(&self) -> BlockingWorkCancelHandle {
    self.handle.cancel_handle()
  }

  fn set_handle(&mut self, raw: sys::napi_async_work) {
    self.handle.raw = raw;
  }

  fn delete_handle(&mut self, env: &Env<'_>) -> Result<()> {
    let handle = std::mem::replace(&mut self.handle, BlockingWorkHandle::empty());
    handle.delete(env)
  }
}

pub fn run<'env, Execute, Complete, Output, JsValue>(
  env: &'env Env<'env>,
  execute: Execute,
  complete: Complete,
  abort_status: Option<Rc<Cell<BlockingWorkStatus>>>,
) -> Result<BlockingWorkPromise<JsValue>>
where
  Execute: FnOnce() -> Result<Output> + Send + 'static,
  Complete: for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>, Output) -> Result<JsValue>
    + 'static,
  Output: Send + Sized + 'static,
  JsValue: 'static,
  for<'scope> JsValue: IntoJs<'scope>,
{
  let mut undefined = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_get_undefined(env.raw(), &mut undefined) },
    "Get undefined failed in blocking_work::run"
  )?;
  let (deferred, raw_promise) = Deferred::create_promise(env)?;
  let task_status = abort_status.unwrap_or_else(|| Rc::new(Cell::new(BlockingWorkStatus::Pending)));
  let mut work = Box::new(NapiBlockingWorkDriver {
    execute: Some(execute),
    complete: Some(complete),
    result: ComputationResult::Pending,
    completion: PromiseCompletion {
      deferred,
      handle: BlockingWorkHandle::empty(),
      status: task_status.clone(),
    },
    marker: PhantomData,
  });
  let work_ptr = work.as_mut() as *mut NapiBlockingWorkDriver<Execute, Complete, Output, JsValue>;
  let mut raw_handle = ptr::null_mut();
  check_status!(
    unsafe {
      sys::napi_create_async_work(
        env.raw(),
        raw_promise,
        undefined,
        Some(execute_work::<Execute, Complete, Output, JsValue>),
        Some(complete_work::<Execute, Complete, Output, JsValue>),
        work_ptr.cast(),
        &mut raw_handle,
      )
    },
    "Create blocking work failed in blocking_work::run"
  )?;
  work.completion.set_handle(raw_handle);
  let cancel_handle = work.completion.cancel_handle();
  if let Err(error) = check_status!(
    unsafe { sys::napi_queue_async_work(env.raw(), raw_handle) },
    "Queue blocking work failed in blocking_work::run"
  ) {
    let _ = work.completion.delete_handle(env);
    return Err(error);
  }
  Box::leak(work);
  Ok(BlockingWorkPromise {
    cancel_handle,
    raw_promise,
    _phantom: PhantomData,
  })
}

// SAFETY: Node-API passes the same driver pointer to worker-thread `execute` and
// env-thread `complete`. `execute` only touches `execute` and `result`;
// completion state stays on the env thread and is consumed by `complete`.
unsafe impl<Execute, Complete, Output, JsValue> Send
  for NapiBlockingWorkDriver<Execute, Complete, Output, JsValue>
where
  Execute: Send,
  Output: Send,
{
}

/// env here is the same native environment that created the work.
/// So it actually could do nothing here, because `execute` function is called in the other thread mostly.
unsafe extern "C" fn execute_work<Execute, Complete, Output, JsValue>(
  _env: sys::napi_env,
  data: *mut c_void,
) where
  Execute: FnOnce() -> Result<Output>,
{
  let work =
    unsafe { &mut *(data as *mut NapiBlockingWorkDriver<Execute, Complete, Output, JsValue>) };
  let value = crate::catch_unwind_result("running blocking work compute", || {
    let execute = work
      .execute
      .take()
      .expect("blocking work execute callback must be available during compute");
    execute()
  })
  .and_then(|result| result);
  work.result = ComputationResult::Ready(value);
}

unsafe extern "C" fn complete_work<Execute, Complete, Output, JsValue>(
  env: sys::napi_env,
  status: sys::napi_status,
  data: *mut c_void,
) where
  Complete: for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>, Output) -> Result<JsValue>,
  JsValue: 'static,
  for<'scope> JsValue: IntoJs<'scope>,
{
  if let Err(e) = unsafe {
    with_env(env, |env_wrapper| {
      complete_impl::<Execute, Complete, Output, JsValue>(env_wrapper, status, data)
    })
  } {
    let js_err = JsError::from(e);
    unsafe { js_err.throw_into(env) };
  }
}

fn complete_impl<Execute, Complete, Output, JsValue>(
  mut env: Env<'_>,
  status: sys::napi_status,
  data: *mut c_void,
) -> Result<()>
where
  Complete: for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>, Output) -> Result<JsValue>,
  JsValue: 'static,
  for<'scope> JsValue: IntoJs<'scope>,
{
  let work = unsafe {
    Box::from_raw(data as *mut NapiBlockingWorkDriver<Execute, Complete, Output, JsValue>)
  };
  let NapiBlockingWorkDriver {
    complete,
    result,
    completion,
    ..
  } = *work;
  let PromiseCompletion {
    deferred,
    handle,
    status: work_status,
  } = completion;
  let mut cleanup = CompletionCleanup::new();

  if status == sys::Status::napi_cancelled {
    cleanup.record(deferred.reject_cancelled(&env));
  } else {
    let value = crate::catch_unwind_result("resolving blocking work result", || {
      let value = match result.into_ready() {
        Ok(output) => {
          let complete =
            complete.expect("blocking work complete callback must be available during resolve");
          env.with_scope(|scope| {
            let value = complete(scope, output)?;
            let local = value.into_js(scope)?;
            Ok(local.raw())
          })
        }
        Err(error) => Err(error),
      }?;
      Ok(value)
    })
    .and_then(|result| result);

    match check_status!(status).and_then(|_| value) {
      Ok(value) if work_status.get() != BlockingWorkStatus::Cancelled => {
        cleanup.record(deferred.resolve(&env, value));
      }
      Err(error) => {
        cleanup.record(deferred.reject(&env, error));
      }
      Ok(_) => {
        cleanup.record(deferred.reject_cancelled(&env));
      }
    }
    work_status.set(BlockingWorkStatus::Completed);
  }

  cleanup.record(handle.delete(&env));
  cleanup.finish()
}
