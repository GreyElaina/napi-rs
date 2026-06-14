use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::marker::PhantomData;
use std::panic::UnwindSafe;
use std::ptr;
use std::rc::Rc;

use crate::env::promise::CancelHandle;
use crate::{
  bindgen_prelude::{CallbackDecoder, EnvRecord, FromJs, Local, Scope, Unknown},
  check_status, sys, Env, JsError, Result, Value, ValueType,
};

use super::Object;

type AbortCallback = Rc<RefCell<Vec<Box<dyn Fn()>>>>;

/// <https://developer.mozilla.org/zh-CN/docs/Web/API/AbortController>
pub struct AbortSignal {
  cancel: Rc<Cell<Option<CancelHandle>>>,
  abort: AbortCallback,
}

impl AbortSignal {
  pub fn on_abort<F: Fn() + 'static>(&self, cb: F) {
    self.abort.borrow_mut().push(Box::new(cb));
  }

  #[doc(hidden)]
  pub fn cancel_cell(&self) -> &Rc<Cell<Option<CancelHandle>>> {
    &self.cancel
  }
}

impl super::TypeName for AbortSignal {
  fn type_name() -> &'static str {
    "AbortSignal"
  }

  fn value_type() -> crate::ValueType {
    crate::ValueType::Object
  }

  fn ts_type() -> String {
    "AbortSignal".to_owned()
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
    let cancel_inner: Rc<Cell<Option<CancelHandle>>> = Rc::new(Cell::new(None));
    let abort_cbs = Rc::new(RefCell::new(vec![]));
    let abort_signal = AbortSignal {
      cancel: cancel_inner.clone(),
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
          Some(abort_signal_finalize),
          ptr::null_mut(),
          &mut signal_ref,
        )
      },
      "Wrap AbortSignal failed"
    )?;
    let on_abort = scope.create_function::<(), Unknown>("onabort", on_abort)?;
    unsafe { signal.set_inner("onabort", on_abort.value)? };

    Ok(AbortSignal {
      cancel: cancel_inner,
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
      "Unwrap abort signal stack from AbortSignal failed"
    )?;
    let abort_signal_stack =
      unsafe { Box::leak(Box::from_raw(abort_stack as *mut AbortSignalStack)) };
    for signal in abort_signal_stack.0.iter() {
      for cb in signal.abort.borrow().iter() {
        cb();
      }
      if let Some(handle) = signal.cancel.take() {
        handle.cancel();
      }
    }
    frame.return_value(())
  })
}

unsafe extern "C" fn abort_signal_finalize(
  _env: sys::napi_env,
  finalize_data: *mut c_void,
  _finalize_hint: *mut c_void,
) {
  drop(unsafe { Box::from_raw(finalize_data as *mut AbortSignalStack) });
}
