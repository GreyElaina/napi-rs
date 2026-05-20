use std::cell::RefCell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr;
use std::rc::Rc;
use std::{cell::Cell, panic::UnwindSafe};

use crate::{
  bindgen_prelude::{CallbackDecoder, EnvRecord, FromJs, IntoJs, Local, Scope, Unknown},
  blocking_work,
  blocking_work::{BlockingWorkCancelHandle, BlockingWorkStatus},
  check_status, sys, Env, JsError, Result, Value, ValueType,
};

use super::Object;

pub struct BlockingWork<'env, 'scope, Execute> {
  scope: &'scope mut Scope<'env, 'scope>,
  execute: Execute,
  abort_signal: Option<AbortSignal>,
}

impl<'env, 'scope> Scope<'env, 'scope> {
  pub fn blocking<Execute>(
    &'scope mut self,
    execute: Execute,
  ) -> BlockingWork<'env, 'scope, Execute> {
    BlockingWork {
      scope: self,
      execute,
      abort_signal: None,
    }
  }
}

impl<'env, 'scope, Execute> BlockingWork<'env, 'scope, Execute> {
  pub fn signal(mut self, signal: AbortSignal) -> Self {
    self.abort_signal = Some(signal);
    self
  }

  pub fn optional_signal(mut self, signal: Option<AbortSignal>) -> Self {
    self.abort_signal = signal;
    self
  }

  pub fn promise<Output, Complete, JsValue: 'static>(
    self,
    complete: Complete,
  ) -> Result<super::Promise<'env, JsValue>>
  where
    Execute: FnOnce() -> Result<Output> + Send + 'static,
    Output: Send + Sized + 'static,
    for<'js_scope> JsValue: IntoJs<'js_scope>,
    Complete: for<'callback, 'complete_scope> FnOnce(
        &mut Scope<'callback, 'complete_scope>,
        Output,
      ) -> Result<JsValue>
      + 'static,
  {
    let abort_status = self
      .abort_signal
      .as_ref()
      .map(|signal| signal.status.clone());
    let raw_env = self.scope.env().raw();
    let async_promise =
      blocking_work::run(self.scope.env_mut(), self.execute, complete, abort_status)?;

    if let Some(signal) = self.abort_signal {
      signal.blocking_work.set(Some(async_promise.cancel_handle));
    }

    Ok(unsafe { super::Promise::from_raw(raw_env, async_promise.raw_promise()) })
  }
}

type AbortCallback = Rc<RefCell<Vec<Box<dyn Fn()>>>>;

/// <https://developer.mozilla.org/zh-CN/docs/Web/API/AbortController>
pub struct AbortSignal {
  blocking_work: Rc<Cell<Option<BlockingWorkCancelHandle>>>,
  status: Rc<Cell<BlockingWorkStatus>>,
  abort: AbortCallback,
}

impl AbortSignal {
  pub fn on_abort<F: Fn() + 'static>(&self, cb: F) {
    self.abort.borrow_mut().push(Box::new(cb));
  }
}

impl UnwindSafe for AbortSignal {}
impl std::panic::RefUnwindSafe for AbortSignal {}

#[repr(transparent)]
struct AbortSignalStack(Vec<AbortSignal>);

impl<'env, 'scope> FromJs<'env, 'scope> for AbortSignal {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> crate::Result<Self> {
    let env = scope.env().raw();
    let mut signal = Object(
      Value {
        env,
        value: value.raw(),
        value_type: ValueType::Object,
      },
      PhantomData,
    );
    let blocking_work_inner: Rc<Cell<Option<BlockingWorkCancelHandle>>> = Rc::new(Cell::new(None));
    let task_status = Rc::new(Cell::new(BlockingWorkStatus::Pending));
    let abort_cbs = Rc::new(RefCell::new(vec![]));
    let abort_signal = AbortSignal {
      blocking_work: blocking_work_inner.clone(),
      status: task_status.clone(),
      abort: abort_cbs.clone(),
    };

    let mut stack;
    let mut maybe_stack = ptr::null_mut();
    let unwrap_status = unsafe { sys::napi_remove_wrap(env, signal.0.value, &mut maybe_stack) };
    if unwrap_status == sys::Status::napi_ok {
      stack = unsafe { Box::from_raw(maybe_stack as *mut AbortSignalStack) };
      stack.0.push(abort_signal);
    } else {
      stack = Box::new(AbortSignalStack(vec![abort_signal]));
    }
    let mut signal_ref = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_wrap(
          env,
          signal.0.value,
          Box::into_raw(stack).cast(),
          Some(blocking_abort_signal_finalize),
          ptr::null_mut(),
          &mut signal_ref,
        )
      },
      "Wrap AbortSignal failed"
    )?;
    let on_abort = scope.create_function::<(), Unknown>("onabort", on_abort)?;
    unsafe { signal.set_inner("onabort", on_abort.value)? };

    Ok(AbortSignal {
      blocking_work: blocking_work_inner,
      status: task_status,
      abort: abort_cbs,
    })
  }
}

unsafe extern "C" fn on_abort(
  env: sys::napi_env,
  callback_info: sys::napi_callback_info,
) -> sys::napi_value {
  match unsafe { EnvRecord::enter_scope(env, |scope| on_abort_impl(*scope.env(), callback_info)) } {
    Err(err) => {
      let js_err = JsError::from(err);
      unsafe { js_err.throw_into(env) };
      ptr::null_mut()
    }
    Ok(undefined) => undefined,
  }
}

fn on_abort_impl(
  env_wrapper: Env<'_>,
  callback_info: sys::napi_callback_info,
) -> Result<sys::napi_value> {
  let mut decoder = CallbackDecoder::<0>::new(env_wrapper, callback_info, None)?;
  decoder.with_frame(|mut frame| {
    let env = frame.raw_env();
    let this = frame.raw_this();
    let mut abort_stack = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_unwrap(env, this, &mut abort_stack) },
      "Unwrap blocking work abort stack from AbortSignal failed"
    )?;
    let abort_controller_stack =
      unsafe { Box::leak(Box::from_raw(abort_stack as *mut AbortSignalStack)) };
    for abort_controller in abort_controller_stack.0.iter() {
      // call abort callback
      for cb in abort_controller.abort.borrow().iter() {
        cb();
      }

      // Work completed, return now.
      if abort_controller.status.get() == BlockingWorkStatus::Completed {
        return Ok(ptr::null_mut());
      }
      if let Some(blocking_work) = abort_controller.blocking_work.get() {
        // The work is already completed, so there may be nothing left to cancel.
        if blocking_work.cancel(&env_wrapper).is_err() {
          abort_controller.status.set(BlockingWorkStatus::Pending);
        } else {
          // abort function must be called from JavaScript main thread, so Relaxed Ordering is ok.
          abort_controller.status.set(BlockingWorkStatus::Cancelled);
        }
      }
    }
    frame.return_value(())
  })
}

unsafe extern "C" fn blocking_abort_signal_finalize(
  _env: sys::napi_env,
  finalize_data: *mut c_void,
  _finalize_hint: *mut c_void,
) {
  drop(unsafe { Box::from_raw(finalize_data as *mut AbortSignalStack) });
}
