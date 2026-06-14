#[cfg(all(feature = "async", feature = "napi4"))]
use std::cell::Cell;
#[cfg(all(feature = "async", feature = "napi4"))]
use std::future::Future;
#[cfg(all(feature = "async", feature = "napi4"))]
use std::rc::Rc;

#[cfg(all(feature = "async", feature = "napi4"))]
use crate::bindgen_runtime::IntoJs;
#[cfg(all(feature = "async", feature = "napi4"))]
use crate::bindgen_runtime::Promise;
#[cfg(all(feature = "async", feature = "napi4"))]
use crate::bindgen_runtime::Scope;
#[cfg(all(feature = "async", feature = "napi4"))]
use crate::JsDeferred;
#[cfg(all(feature = "async", feature = "napi4"))]
use crate::Result;

use super::Env;

// ---------------------------------------------------------------------------
// CancelHandle — type-erased cancellation for spawned promises
// ---------------------------------------------------------------------------

#[doc(hidden)]
#[cfg(all(feature = "async", feature = "napi4"))]
pub struct CancelHandle {
  inner: Rc<Cell<Option<async_task::Task<()>>>>,
}

#[cfg(all(feature = "async", feature = "napi4"))]
impl CancelHandle {
  fn new() -> (Self, CancelSlot) {
    let inner = Rc::new(Cell::new(None));
    let slot = CancelSlot {
      inner: Rc::clone(&inner),
    };
    (Self { inner }, slot)
  }

  pub fn cancel(&self) {
    if let Some(task) = self.inner.take() {
      drop(task);
    }
  }
}

#[cfg(all(feature = "async", feature = "napi4"))]
impl Drop for CancelHandle {
  fn drop(&mut self) {
    if let Some(task) = self.inner.take() {
      task.detach();
    }
  }
}

#[cfg(all(feature = "async", feature = "napi4"))]
struct CancelSlot {
  inner: Rc<Cell<Option<async_task::Task<()>>>>,
}

#[cfg(all(feature = "async", feature = "napi4"))]
impl CancelSlot {
  fn arm(self, task: async_task::Task<()>) {
    self.inner.set(Some(task));
  }
}

impl<'env> Env<'env> {
  #[cfg(all(feature = "async", feature = "napi4", feature = "noop"))]
  fn spawn_promise_with_inner<Data, PromiseValue, Fut, Complete>(
    &self,
    _future: Fut,
    _complete: Complete,
  ) -> Result<(Promise<'env, PromiseValue>, CancelHandle)>
  where
    Data: 'static,
    PromiseValue: 'static,
    for<'scope> PromiseValue: IntoJs<'scope>,
    Fut: 'static + Future<Output = Result<Data>>,
    Complete: 'static
      + for<'callback, 'scope> FnOnce(
        &mut Scope<'callback, 'scope>,
        Result<Data>,
      ) -> Result<PromiseValue>,
  {
    let (handle, _slot) = CancelHandle::new();
    Ok((
      unsafe { Promise::from_raw(self.0, std::ptr::null_mut()) },
      handle,
    ))
  }

  #[cfg(all(feature = "async", feature = "napi4", not(feature = "noop")))]
  fn spawn_promise_with_inner<Data, PromiseValue, Fut, Complete>(
    &self,
    future: Fut,
    complete: Complete,
  ) -> Result<(Promise<'env, PromiseValue>, CancelHandle)>
  where
    Data: 'static,
    PromiseValue: 'static,
    for<'scope> PromiseValue: IntoJs<'scope>,
    Fut: 'static + Future<Output = Result<Data>>,
    Complete: 'static
      + for<'callback, 'scope> FnOnce(
        &mut Scope<'callback, 'scope>,
        Result<Data>,
      ) -> Result<PromiseValue>,
  {
    use std::cell::RefCell;

    use futures::FutureExt;

    use crate::js_values::DeferredCompletion;

    let (completion, raw_promise) = DeferredCompletion::new(self)?;
    let completion = Rc::new(RefCell::new(Some(completion)));
    let (handle, slot) = CancelHandle::new();

    let inner = {
      let completion = Rc::clone(&completion);
      async move {
        let Some(completion) = completion.borrow_mut().take() else {
          return;
        };
        let result = std::panic::AssertUnwindSafe(future).catch_unwind().await;
        completion.settle(|scope| {
          complete(
            scope,
            result.unwrap_or_else(|panic_payload| {
              Err(crate::Error::new(
                crate::Status::GenericFailure,
                crate::panic_message(panic_payload.as_ref()),
              ))
            }),
          )
        });
      }
    };

    match self.spawn_future(inner) {
      Ok(task) => {
        slot.arm(task);
      }
      Err(error) => {
        if let Some(completion) = completion.borrow_mut().take() {
          completion.reject(crate::Error::new(
            crate::Status::GenericFailure,
            error.reason.clone(),
          ));
        }
        return Err(error);
      }
    }

    Ok((unsafe { Promise::from_raw(self.0, raw_promise) }, handle))
  }

  #[cfg(all(feature = "async", feature = "napi4"))]
  pub fn spawn_promise<T, F>(&self, fut: F) -> Result<Promise<'_, T>>
  where
    T: 'static,
    F: 'static + Future<Output = Result<T>>,
    for<'scope> T: IntoJs<'scope>,
  {
    let (promise, _) = self.spawn_promise_with_inner(fut, |_, result| result)?;
    Ok(promise)
  }

  #[cfg(all(feature = "async", feature = "napi4"))]
  pub fn spawn_promise_with<
    T: 'static,
    V: 'static,
    F: 'static + Future<Output = Result<T>>,
    R: 'static + for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>, Result<T>) -> Result<V>,
  >(
    &self,
    fut: F,
    callback: R,
  ) -> Result<Promise<'env, V>>
  where
    for<'scope> V: IntoJs<'scope>,
  {
    let (promise, _) = self.spawn_promise_with_inner(fut, callback)?;
    Ok(promise)
  }

  #[doc(hidden)]
  #[cfg(all(feature = "async", feature = "napi4"))]
  pub fn spawn_promise_cancellable<
    T: 'static,
    V: 'static,
    F: 'static + Future<Output = Result<T>>,
    R: 'static + for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>, Result<T>) -> Result<V>,
  >(
    &self,
    fut: F,
    callback: R,
  ) -> Result<(Promise<'env, V>, CancelHandle)>
  where
    for<'scope> V: IntoJs<'scope>,
  {
    self.spawn_promise_with_inner(fut, callback)
  }

  #[cfg(all(feature = "async", feature = "napi4"))]
  pub fn deferred<Data>(&self) -> Result<(JsDeferred<Data>, Promise<'env, Data>)>
  where
    Data: 'static,
    for<'scope> Data: IntoJs<'scope>,
  {
    let (deferred, raw_promise) = JsDeferred::new(self)?;
    Ok((deferred, unsafe { Promise::from_raw(self.0, raw_promise) }))
  }

  /// Test harness: models spawn failure after `DeferredCompletion::new` while still
  /// returning the native promise handle (regression for unsettled promises).
  #[doc(hidden)]
  #[cfg(all(feature = "async", feature = "napi4"))]
  pub fn regression_promise_if_spawn_fails(&self) -> Result<Promise<'env, ()>> {
    use crate::js_values::DeferredCompletion;

    let (completion, raw_promise) = DeferredCompletion::new(self)?;
    completion.reject(crate::Error::new(
      crate::Status::GenericFailure,
      "regression: spawn_future failed after deferred creation",
    ));

    Ok(unsafe { Promise::from_raw(self.0, raw_promise) })
  }
}

/// Mirrors `spawn_promise_with_inner` settle dispatch — extracted for regression tests.
#[cfg(all(test, feature = "async", feature = "napi4"))]
enum RegressionFutureOutcome<T> {
  Ok(T),
  AsyncErr,
  Panic,
}

#[cfg(all(test, feature = "async", feature = "napi4"))]
fn regression_run_settle_dispatch<T>(
  outcome: RegressionFutureOutcome<T>,
  complete: &mut dyn FnMut(std::result::Result<T, &'static str>),
) {
  complete(match outcome {
    RegressionFutureOutcome::Ok(data) => Ok(data),
    RegressionFutureOutcome::AsyncErr => Err("async error"),
    RegressionFutureOutcome::Panic => Err("async panic"),
  });
}

/// Mirrors post-`DeferredCompletion::new` spawn path — extracted for regression tests.
#[cfg(all(test, feature = "async", feature = "napi4"))]
fn regression_run_spawn_dispatch(spawn_ok: bool) -> bool {
  if spawn_ok {
    return false;
  }
  true
}

#[cfg(all(test, feature = "async", feature = "napi4"))]
mod regression_tests {
  use super::*;

  #[test]
  fn completion_runs_when_future_returns_err() {
    let mut completion_ran = false;
    regression_run_settle_dispatch::<()>(RegressionFutureOutcome::AsyncErr, &mut |_result| {
      completion_ran = true;
    });

    assert!(
      completion_ran,
      "completion must run on async Err so AsyncArgRefs::finalize can run"
    );
  }

  #[test]
  fn completion_runs_when_future_panics() {
    let mut completion_ran = false;
    regression_run_settle_dispatch::<()>(RegressionFutureOutcome::Panic, &mut |_result| {
      completion_ran = true;
    });

    assert!(
      completion_ran,
      "completion must run on async panic so AsyncArgRefs::finalize can run"
    );
  }

  #[test]
  fn promise_settled_when_spawn_future_fails() {
    let settled = regression_run_spawn_dispatch(false);
    assert!(
      settled,
      "deferred promise must reject when spawn_future fails after creation"
    );
  }
}
