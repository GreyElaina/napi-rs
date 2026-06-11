//! Tokio context adapter for `napi` async exports.

use std::sync::{Arc, Mutex};

use napi::{Env, Result};
use tokio::runtime::{Handle, Runtime};

struct TokioPollContext {
  runtime: Option<Runtime>,
  handle: Handle,
}

struct TokioRuntimeFactory {
  state: Mutex<TokioRuntimeFactoryState>,
  factory: Box<dyn Fn() -> Runtime + 'static>,
}

struct TokioRuntimeFactoryState {
  runtime: Option<Runtime>,
  handle: Option<Handle>,
}

impl TokioPollContext {
  fn new(runtime: Runtime) -> Self {
    let handle = runtime.handle().clone();
    Self {
      runtime: Some(runtime),
      handle,
    }
  }

  fn from_handle(handle: Handle) -> Self {
    Self {
      runtime: None,
      handle,
    }
  }

  fn enter(&self, run: &mut dyn FnMut()) {
    let _guard = self.handle.enter();
    run();
  }
}

impl TokioRuntimeFactory {
  fn new(factory: impl Fn() -> Runtime + 'static) -> Self {
    Self {
      state: Mutex::new(TokioRuntimeFactoryState {
        runtime: None,
        handle: None,
      }),
      factory: Box::new(factory),
    }
  }

  fn enter(&self, run: &mut dyn FnMut()) {
    let handle = {
      let mut state = self.state.lock().expect("Mutex poisoned");
      if state.runtime.is_none() {
        let runtime = (self.factory)();
        state.handle = Some(runtime.handle().clone());
        state.runtime = Some(runtime);
      }
      state
        .handle
        .as_ref()
        .expect("Tokio runtime handle is missing")
        .clone()
    };
    let _guard = handle.enter();
    run();
  }
}

impl Drop for TokioRuntimeFactory {
  fn drop(&mut self) {
    let runtime = {
      let mut state = self.state.lock().expect("Mutex poisoned");
      state.handle = None;
      state.runtime.take()
    };
    if let Some(runtime) = runtime {
      runtime.shutdown_background();
    }
  }
}

impl Drop for TokioPollContext {
  fn drop(&mut self) {
    if let Some(runtime) = self.runtime.take() {
      runtime.shutdown_background();
    }
  }
}

/// Install an owned Tokio runtime as the poll context for the current Node env.
///
/// The runtime is held by the env async driver and shut down without blocking
/// env teardown. Futures are still polled by napi's libuv-driven executor; this
/// only provides Tokio's thread-local reactor/timer context while polling.
pub fn install(env: &Env<'_>, runtime: Runtime) -> Result<()> {
  let context = Arc::new(TokioPollContext::new(runtime));
  env.set_async_poll_context(move |run| context.enter(run))
}

/// Install a lazily-created owned Tokio runtime as the poll context.
///
/// The runtime is created when async work is first polled, retained for the env,
/// and shut down without blocking when the env record tears down.
pub fn install_factory(env: &Env<'_>, factory: impl Fn() -> Runtime + 'static) -> Result<()> {
  let context = Arc::new(TokioRuntimeFactory::new(factory));
  env.set_async_poll_context(move |run| context.enter(run))
}

/// Install a caller-owned Tokio handle as the poll context for the current Node env.
///
/// The caller remains responsible for keeping the runtime behind the handle alive.
pub fn install_handle(env: &Env<'_>, handle: Handle) -> Result<()> {
  let context = Arc::new(TokioPollContext::from_handle(handle));
  env.set_async_poll_context(move |run| context.enter(run))
}

/// Install the Tokio runtime handle active on the current thread.
pub fn install_current(env: &Env<'_>) -> Result<()> {
  let handle = Handle::try_current().map_err(|error| {
    napi::Error::new(
      napi::Status::GenericFailure,
      format!("Tokio runtime is not available on the current thread: {error}"),
    )
  })?;
  install_handle(env, handle)
}
