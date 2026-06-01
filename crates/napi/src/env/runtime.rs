use std::future::Future;
#[cfg(not(feature = "noop"))]
use std::sync::{LazyLock, OnceLock, RwLock};

use tokio::runtime::Runtime;

#[cfg(not(feature = "noop"))]
fn create_runtime() -> Runtime {
  if IS_USER_DEFINED_RT.get().copied().unwrap_or(false) {
    if let Some(user_defined_rt) = USER_DEFINED_RT
      .get()
      .and_then(|rt| rt.write().ok().and_then(|mut rt| rt.take()))
    {
      return user_defined_rt;
    }
  }

  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .expect("Create tokio runtime failed")
}

#[cfg(not(feature = "noop"))]
static RT: LazyLock<RwLock<Option<Runtime>>> =
  LazyLock::new(|| RwLock::new(Some(create_runtime())));

#[cfg(not(feature = "noop"))]
static USER_DEFINED_RT: OnceLock<RwLock<Option<Runtime>>> = OnceLock::new();

#[cfg(not(feature = "noop"))]
static IS_USER_DEFINED_RT: OnceLock<bool> = OnceLock::new();

#[cfg(not(feature = "noop"))]
/// Create a custom Tokio runtime used by the NAPI-RS.
/// You can control the tokio runtime configuration by yourself.
/// ### Example
/// ```no_run
/// use tokio::runtime::Builder;
/// use napi::create_custom_tokio_runtime;
///
/// #[napi_derive::module_init]
/// fn init() {
///    let rt = Builder::new_multi_thread().enable_all().thread_stack_size(32 * 1024 * 1024).build().unwrap();
///    create_custom_tokio_runtime(rt);
/// }
pub fn create_custom_tokio_runtime(rt: Runtime) {
  USER_DEFINED_RT.get_or_init(move || RwLock::new(Some(rt)));
  IS_USER_DEFINED_RT.get_or_init(|| true);
}

#[cfg(feature = "noop")]
pub fn create_custom_tokio_runtime(_: Runtime) {}

#[cfg(not(feature = "noop"))]
/// Start the async runtime.
pub fn start_async_runtime() {
  if let Ok(mut rt) = RT.write() {
    if rt.is_none() {
      *rt = Some(create_runtime());
    }
  }
}

#[cfg(not(feature = "noop"))]
pub fn shutdown_async_runtime() {
  if let Some(rt) = RT.write().ok().and_then(|mut rt| rt.take()) {
    rt.shutdown_background();
  }
}

#[cfg(not(feature = "noop"))]
/// Spawns a future onto the Tokio runtime.
pub fn spawn<F>(fut: F) -> tokio::task::JoinHandle<F::Output>
where
  F: 'static + Send + Future<Output = ()>,
{
  RT.read()
    .ok()
    .and_then(|rt| rt.as_ref().map(|rt| rt.spawn(fut)))
    .expect("Access tokio runtime failed in spawn")
}

#[cfg(not(feature = "noop"))]
/// Runs a future to completion.
pub fn block_on<F: Future>(fut: F) -> F::Output {
  RT.read()
    .ok()
    .and_then(|rt| rt.as_ref().map(|rt| rt.block_on(fut)))
    .expect("Access tokio runtime failed in block_on")
}

#[cfg(feature = "noop")]
/// Runs a future to completion.
pub fn block_on<F: Future>(_: F) -> F::Output {
  unreachable!("noop feature is enabled, block_on is not available")
}

#[cfg(not(feature = "noop"))]
/// spawn_blocking on the current Tokio runtime.
pub fn spawn_blocking<F, R>(func: F) -> tokio::task::JoinHandle<R>
where
  F: FnOnce() -> R + Send + 'static,
  R: Send + 'static,
{
  RT.read()
    .ok()
    .and_then(|rt| rt.as_ref().map(|rt| rt.spawn_blocking(func)))
    .expect("Access tokio runtime failed in spawn_blocking")
}

#[cfg(not(feature = "noop"))]
/// If the feature `tokio_rt` has been enabled this will enter the runtime context and
/// then call the provided closure. Otherwise it will just call the provided closure.
pub fn within_runtime_if_available<F: FnOnce() -> T, T>(f: F) -> T {
  RT.read()
    .ok()
    .and_then(|rt| {
      rt.as_ref().map(|rt| {
        let rt_guard = rt.enter();
        let ret = f();
        drop(rt_guard);
        ret
      })
    })
    .expect("Access tokio runtime failed in within_runtime_if_available")
}

#[cfg(feature = "noop")]
pub fn within_runtime_if_available<F: FnOnce() -> T, T>(f: F) -> T {
  f()
}
