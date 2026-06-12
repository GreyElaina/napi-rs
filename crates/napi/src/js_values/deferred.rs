#[cfg(feature = "async")]
use std::marker::PhantomData;
#[cfg(feature = "async")]
use std::ptr;
#[cfg(feature = "async")]
use std::sync::Arc;

#[cfg(feature = "async")]
use crate::bindgen_runtime::{IntoJs, Scope};
#[cfg(feature = "async")]
use crate::env::{AsyncChannel, AsyncKeepAlive};
#[cfg(feature = "async")]
use crate::{check_status, sys, Env, Error, Result};

// ---------------------------------------------------------------------------
// Stack trace capture helpers
// ---------------------------------------------------------------------------

#[cfg(feature = "async")]
fn capture_creation_stack(env: sys::napi_env) -> Option<String> {
  unsafe {
    let mut err_msg = ptr::null_mut();
    if sys::napi_create_string_utf8(env, c"".as_ptr().cast(), 0, &mut err_msg)
      != sys::Status::napi_ok
    {
      return None;
    }

    let mut js_error = ptr::null_mut();
    if sys::napi_create_error(env, ptr::null_mut(), err_msg, &mut js_error)
      != sys::Status::napi_ok
    {
      return None;
    }

    let mut stack_value = ptr::null_mut();
    if sys::napi_get_named_property(env, js_error, c"stack".as_ptr().cast(), &mut stack_value)
      != sys::Status::napi_ok
    {
      return None;
    }

    let mut len = 0;
    if sys::napi_get_value_string_utf8(env, stack_value, ptr::null_mut(), 0, &mut len)
      != sys::Status::napi_ok
    {
      return None;
    }

    let mut buf = vec![0u8; len + 1];
    let mut copied = 0;
    if sys::napi_get_value_string_utf8(
      env,
      stack_value,
      buf.as_mut_ptr().cast(),
      buf.len(),
      &mut copied,
    ) != sys::Status::napi_ok
    {
      return None;
    }

    buf.truncate(copied);
    let stack = String::from_utf8_lossy(&buf).into_owned();
    stack.find('\n').map(|pos| stack[pos..].to_owned())
  }
}

#[cfg(feature = "async")]
fn stitch_stack(error_value: sys::napi_value, env: sys::napi_env, frames: &str) {
  unsafe {
    let mut current_stack = ptr::null_mut();
    if sys::napi_get_named_property(env, error_value, c"stack".as_ptr().cast(), &mut current_stack)
      != sys::Status::napi_ok
    {
      return;
    }

    let mut len = 0;
    if sys::napi_get_value_string_utf8(env, current_stack, ptr::null_mut(), 0, &mut len)
      != sys::Status::napi_ok
    {
      return;
    }

    let mut buf = vec![0u8; len + 1];
    let mut copied = 0;
    if sys::napi_get_value_string_utf8(
      env,
      current_stack,
      buf.as_mut_ptr().cast(),
      buf.len(),
      &mut copied,
    ) != sys::Status::napi_ok
    {
      return;
    }

    buf.truncate(copied);
    let header = String::from_utf8_lossy(&buf);
    let full_stack = format!("{header}{frames}");

    let mut new_stack = ptr::null_mut();
    if sys::napi_create_string_utf8(
      env,
      full_stack.as_ptr().cast(),
      full_stack.len() as isize,
      &mut new_stack,
    ) != sys::Status::napi_ok
    {
      return;
    }

    sys::napi_set_named_property(env, error_value, c"stack".as_ptr().cast(), new_stack);
  }
}

// ---------------------------------------------------------------------------
// DeferredCompletion — shared resolve/reject primitive
// ---------------------------------------------------------------------------

#[cfg(feature = "async")]
struct PendingState {
  env: sys::napi_env,
  deferred: sys::napi_deferred,
  keep_alive: Option<AsyncKeepAlive>,
  creation_frames: Option<String>,
}

#[cfg(feature = "async")]
// SAFETY: The raw N-API handles are only touched on the owning env's JS
// thread — either directly in spawn_promise (local executor) or inside a
// closure dispatched through AsyncChannel.
pub(crate) struct DeferredCompletion {
  state: Option<PendingState>,
}

#[cfg(feature = "async")]
unsafe impl Send for DeferredCompletion {}

#[cfg(feature = "async")]
impl DeferredCompletion {
  pub(crate) fn new(env: &Env<'_>) -> Result<(Self, sys::napi_value)> {
    let raw_env = env.raw();

    let mut raw_promise = ptr::null_mut();
    let mut raw_deferred = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_promise(raw_env, &mut raw_deferred, &mut raw_promise) },
      "Create promise failed"
    )?;

    let keep_alive = env.async_keep_alive()?;
    let creation_frames = capture_creation_stack(raw_env);

    let completion = Self {
      state: Some(PendingState {
        env: raw_env,
        deferred: raw_deferred,
        keep_alive: Some(keep_alive),
        creation_frames,
      }),
    };
    Ok((completion, raw_promise))
  }

  pub(crate) fn settle<T>(mut self, f: impl FnOnce(&mut Scope) -> Result<T>)
  where
    for<'scope> T: IntoJs<'scope>,
  {
    let Some(PendingState { env, .. }) = &self.state else {
      return;
    };
    let env_raw = *env;

    let mut env_wrapper = unsafe { Env::from_raw(env_raw) };
    let result = env_wrapper
      .with_scope(|scope| f(scope).and_then(|value| Ok(value.into_js(scope)?.raw())));

    self.complete(result);
  }

  pub(crate) fn reject(mut self, error: Error) {
    self.complete(Err(error));
  }

  fn complete(&mut self, result: Result<sys::napi_value>) {
    let Some(PendingState {
      env,
      deferred,
      keep_alive,
      creation_frames,
    }) = self.state.take()
    else {
      return;
    };
    let frames = creation_frames.as_deref().map(str::to_owned);

    match result {
      Ok(value) => {
        let status = unsafe { sys::napi_resolve_deferred(env, deferred, value) };
        if status != sys::Status::napi_ok {
          let err = Error::new(
            crate::Status::from(status),
            "napi_resolve_deferred failed, falling back to reject",
          );
          Self::reject_with(env, deferred, frames.as_deref(), err);
        }
      }
      Err(e) => {
        Self::reject_with(env, deferred, frames.as_deref(), e);
      }
    }

    drop(keep_alive);
  }

  fn reject_with(
    env: sys::napi_env,
    deferred: sys::napi_deferred,
    creation_frames: Option<&str>,
    error: Error,
  ) {
    let error_value = unsafe { crate::JsError::from(error).into_value(env) };
    if let Some(frames) = creation_frames {
      stitch_stack(error_value, env, frames);
    }
    unsafe { sys::napi_reject_deferred(env, deferred, error_value) };
  }
}

#[cfg(feature = "async")]
impl Drop for DeferredCompletion {
  fn drop(&mut self) {
    if let Some(state) = self.state.take() {
      let error = Error::new(crate::Status::Cancelled, "AbortError".to_owned());
      Self::reject_with(state.env, state.deferred, state.creation_frames.as_deref(), error);
      drop(state.keep_alive);
    }
  }
}

// ---------------------------------------------------------------------------
// JsDeferred<Data> — cross-thread deferred promise handle
// ---------------------------------------------------------------------------

#[cfg(feature = "async")]
pub struct JsDeferred<Data> {
  completion: DeferredCompletion,
  channel: Arc<AsyncChannel>,
  _data: PhantomData<Data>,
}

#[cfg(feature = "async")]
// SAFETY: Same reasoning as DeferredCompletion.
unsafe impl<Data> Send for JsDeferred<Data> {}

#[cfg(feature = "async")]
impl<Data> JsDeferred<Data>
where
  Data: 'static,
  for<'scope> Data: IntoJs<'scope>,
{
  pub(crate) fn new(env: &Env<'_>) -> Result<(Self, sys::napi_value)> {
    let (completion, raw_promise) = DeferredCompletion::new(env)?;
    let channel = {
      let record = env.record();
      record.with_data(|data| {
        data
          .async_driver()
          .map(|driver| driver.channel().clone())
          .ok_or_else(|| {
            Error::new(
              crate::Status::GenericFailure,
              "Async driver is not available",
            )
          })
      })??
    };

    Ok((
      Self {
        completion,
        channel,
        _data: PhantomData,
      },
      raw_promise,
    ))
  }

  pub fn resolve(
    self,
    resolver: impl for<'a, 'b> FnOnce(&mut Scope<'a, 'b>) -> Result<Data> + Send + 'static,
  ) {
    let Self {
      completion,
      channel,
      ..
    } = self;

    if !channel.push(Box::new(move |_env| {
      completion.settle(|scope| resolver(scope));
    })) {
      // Channel shut down — closure dropped by push, completion dropped
      // inside it. DeferredCompletion::Drop releases keepalive + debug warns.
    }
  }

  pub fn reject(self, error: Error) {
    let Self {
      completion,
      channel,
      ..
    } = self;

    if !channel.push(Box::new(move |_env| {
      completion.reject(error);
    })) {
      // Same as resolve — teardown path.
    }
  }
}
