use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::{alloc, ffi};

use async_task::{Runnable, Task};

use crate::{sys, Env, Error, Result, Status};

// ---------------------------------------------------------------------------
// libuv FFI (minimal, internal to napi crate)
// ---------------------------------------------------------------------------

#[repr(C)]
struct UvAsyncT {
  _opaque: [u8; 0],
}

type UvAsyncCb = unsafe extern "C" fn(*mut UvAsyncT);
type UvCloseCb = unsafe extern "C" fn(*mut UvAsyncT);

const UV_ASYNC: ffi::c_int = 1;

extern "C" {
  fn uv_handle_size(type_: ffi::c_int) -> usize;
  fn uv_async_init(loop_: *mut sys::uv_loop_s, handle: *mut UvAsyncT, cb: UvAsyncCb) -> ffi::c_int;
  fn uv_async_send(handle: *mut UvAsyncT) -> ffi::c_int;
  fn uv_close(handle: *mut UvAsyncT, cb: Option<UvCloseCb>);
  fn uv_ref(handle: *mut UvAsyncT);
  fn uv_unref(handle: *mut UvAsyncT);
  fn uv_handle_set_data(handle: *mut UvAsyncT, data: *mut ffi::c_void);
  fn uv_handle_get_data(handle: *const UvAsyncT) -> *mut ffi::c_void;
}

fn async_handle_layout() -> alloc::Layout {
  #[cfg(test)]
  {
    alloc::Layout::from_size_align(
      std::mem::size_of::<*mut ffi::c_void>(),
      std::mem::align_of::<*mut ffi::c_void>(),
    )
    .unwrap()
  }
  #[cfg(not(test))]
  {
    let size = unsafe { uv_handle_size(UV_ASYNC) };
    alloc::Layout::from_size_align(size, std::mem::align_of::<*mut ffi::c_void>()).unwrap()
  }
}

fn alloc_async_handle() -> *mut UvAsyncT {
  let layout = async_handle_layout();
  unsafe { alloc::alloc_zeroed(layout).cast::<UvAsyncT>() }
}

#[cfg(test)]
static TEST_DEALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
fn test_reset_dealloc_count() {
  TEST_DEALLOC_COUNT.store(0, Ordering::SeqCst);
}

#[cfg(test)]
fn test_dealloc_count() -> usize {
  TEST_DEALLOC_COUNT.load(Ordering::SeqCst)
}

unsafe fn dealloc_async_handle(handle: *mut UvAsyncT) {
  #[cfg(test)]
  TEST_DEALLOC_COUNT.fetch_add(1, Ordering::SeqCst);
  let layout = async_handle_layout();
  unsafe { alloc::dealloc(handle.cast::<u8>(), layout) };
}

// ---------------------------------------------------------------------------
// AsyncChannel — cross-thread queue + uv_async_t, lifetime controlled by Arc
// ---------------------------------------------------------------------------

pub(crate) type MainThreadClosure = Box<dyn FnOnce(&mut Env<'_>) + Send + 'static>;

pub(crate) struct AsyncChannel {
  handle: *mut UvAsyncT,
  env: sys::napi_env,
  queue: Mutex<Vec<MainThreadClosure>>,
  shutdown: AtomicBool,
  handle_closed: AtomicBool,
  /// Set after `uv_async_init` succeeds; used to choose between `uv_close` and direct dealloc.
  handle_registered: AtomicBool,
  keepalive_count: std::sync::atomic::AtomicUsize,
}

// SAFETY: The uv_async_t handle is allocated on the heap and its memory
// lifetime is controlled by this Arc. uv_async_send is documented as
// thread-safe. The queue is behind a Mutex.
unsafe impl Send for AsyncChannel {}
unsafe impl Sync for AsyncChannel {}

impl AsyncChannel {
  pub(crate) fn push(&self, closure: MainThreadClosure) -> bool {
    {
      let mut queue = self.queue.lock().unwrap();
      if self.shutdown.load(Ordering::Acquire) {
        return false;
      }
      queue.push(closure);
    }
    if !self.handle_closed.load(Ordering::Acquire) {
      unsafe { uv_async_send(self.handle) };
    }
    true
  }

  fn drain(&self, env: &mut Env<'_>) {
    let closures: Vec<_> = {
      let mut queue = self.queue.lock().unwrap();
      std::mem::take(&mut *queue)
    };
    for closure in closures {
      closure(env);
    }
  }

  fn drop_remaining(&self) {
    let remaining = {
      let mut queue = self.queue.lock().unwrap();
      std::mem::take(&mut *queue)
    };
    drop(remaining);
  }

  pub(crate) fn shutdown(&self) {
    self.shutdown.store(true, Ordering::Release);
  }

  pub(crate) fn is_shutdown(&self) -> bool {
    self.shutdown.load(Ordering::Acquire)
  }

  fn close_handle(&self) {
    if self.handle_closed.swap(true, Ordering::AcqRel) {
      return;
    }
    #[cfg(not(test))]
    unsafe {
      uv_close(self.handle, Some(on_uv_close));
    }
  }

  fn ref_keepalive(&self) {
    if self.keepalive_count.fetch_add(1, Ordering::AcqRel) == 0
      && !self.handle_closed.load(Ordering::Acquire)
    {
      unsafe { uv_ref(self.handle) };
    }
  }

  fn unref_keepalive(&self) {
    let previous = self.keepalive_count.fetch_sub(1, Ordering::AcqRel);
    debug_assert!(previous > 0, "async keepalive underflow");
    if previous == 1 && !self.handle_closed.load(Ordering::Acquire) {
      unsafe { uv_unref(self.handle) };
    }
  }
}

impl Drop for AsyncChannel {
  fn drop(&mut self) {
    if self.handle_closed.load(Ordering::Acquire) {
      return;
    }
    if self.handle_registered.load(Ordering::Acquire) {
      self.close_handle();
    } else {
      unsafe { dealloc_async_handle(self.handle) };
    }
  }
}

unsafe extern "C" fn on_uv_close(handle: *mut UvAsyncT) {
  let data = unsafe { uv_handle_get_data(handle) };
  if !data.is_null() {
    unsafe { Arc::from_raw(data as *const AsyncChannel) };
  }
  unsafe { dealloc_async_handle(handle) };
}

// ---------------------------------------------------------------------------
// Per-env async driver state (lives inside EnvRecord, !Send)
// ---------------------------------------------------------------------------

pub(crate) struct AsyncDriver {
  queue: Arc<Mutex<VecDeque<Runnable>>>,
  channel: Arc<AsyncChannel>,
  poll_context: RefCell<Option<PollContext>>,
}

struct PollContext {
  enter: Box<dyn Fn(&mut dyn FnMut()) + 'static>,
}

pub(crate) struct AsyncKeepAlive {
  channel: Arc<AsyncChannel>,
}

impl AsyncKeepAlive {
  fn new(channel: Arc<AsyncChannel>) -> Self {
    channel.ref_keepalive();
    Self { channel }
  }
}

impl Drop for AsyncKeepAlive {
  fn drop(&mut self) {
    self.channel.unref_keepalive();
  }
}

impl AsyncDriver {
  pub(crate) fn new(env: &Env<'_>) -> Result<Self> {
    let uv_loop = env.get_uv_event_loop()?;
    let handle = alloc_async_handle();

    let channel = Arc::new(AsyncChannel {
      handle,
      env: env.raw(),
      queue: Mutex::new(Vec::new()),
      shutdown: AtomicBool::new(false),
      handle_closed: AtomicBool::new(false),
      handle_registered: AtomicBool::new(false),
      keepalive_count: std::sync::atomic::AtomicUsize::new(0),
    });

    let status = unsafe { uv_async_init(uv_loop, handle, on_uv_async) };
    if status != 0 {
      return Err(Error::new(
        Status::GenericFailure,
        format!("uv_async_init failed with status {status}"),
      ));
    }
    channel
      .handle_registered
      .store(true, Ordering::Release);

    unsafe {
      uv_unref(handle);
      uv_handle_set_data(handle, Arc::into_raw(channel.clone()) as *mut ffi::c_void);
    }

    Ok(Self {
      queue: Arc::new(Mutex::new(VecDeque::new())),
      channel,
      poll_context: RefCell::new(None),
    })
  }

  pub(crate) fn channel(&self) -> &Arc<AsyncChannel> {
    &self.channel
  }

  pub(crate) fn keep_alive(&self) -> AsyncKeepAlive {
    AsyncKeepAlive::new(self.channel.clone())
  }

  pub(crate) fn set_poll_context(&self, enter: impl Fn(&mut dyn FnMut()) + 'static) {
    *self.poll_context.borrow_mut() = Some(PollContext {
      enter: Box::new(enter),
    });
  }

  pub(crate) fn spawn<T: 'static>(&self, future: impl Future<Output = T> + 'static) -> Task<T> {
    let queue = self.queue.clone();
    let channel = self.channel.clone();
    let schedule = move |runnable| {
      queue.lock().unwrap().push_back(runnable);
      if !channel.handle_closed.load(Ordering::Acquire) {
        unsafe { uv_async_send(channel.handle) };
      }
    };
    let (runnable, task) = async_task::spawn_local(future, schedule);
    runnable.schedule();
    task
  }

  pub(crate) fn tick(&self) {
    let mut tick = || self.tick_inner();
    if let Some(context) = self.poll_context.borrow().as_ref() {
      (context.enter)(&mut tick);
    } else {
      tick();
    }
  }

  fn tick_inner(&self) {
    loop {
      let runnable = self.queue.lock().unwrap().pop_front();
      let Some(runnable) = runnable else {
        break;
      };
      runnable.run();
    }
  }

  pub(crate) fn teardown(self, env: &mut Env<'_>) {
    self.channel.shutdown();
    self.channel.drain(env);
    self.tick();
    self.channel.drain(env);
    self.channel.drop_remaining();
    self.channel.close_handle();
  }
}

// ---------------------------------------------------------------------------
// uv_async callback — the main driver entry point
// ---------------------------------------------------------------------------

unsafe extern "C" fn on_uv_async(handle: *mut UvAsyncT) {
  let data = unsafe { uv_handle_get_data(handle) };
  if data.is_null() {
    return;
  }

  let channel = unsafe {
    Arc::increment_strong_count(data as *const AsyncChannel);
    Arc::from_raw(data as *const AsyncChannel)
  };

  if channel.is_shutdown() {
    return;
  }

  let env_raw = channel.env;

  crate::run_unwind_boundary("async driver tick", || {
    use crate::bindgen_runtime::EnvRecord;
    use crate::{check_status, sys};

    let result: Result<()> = (|| {
      let mut handle_scope = std::ptr::null_mut();
      check_status!(
        unsafe { sys::napi_open_handle_scope(env_raw, &mut handle_scope) },
        "Failed to open handle scope for async tick"
      )?;

      let record = EnvRecord::acquire(env_raw);
      let mut env = unsafe { Env::from_raw(env_raw) };
      if let Err(e) = record.drain_deferred_refs(&mut env) {
        eprintln!("napi-rs: failed to drain deferred refs: {e:?}");
      }

      channel.drain(&mut env);

      if let Err(e) = record.with_data(|data| {
        if let Some(driver) = data.async_driver() {
          driver.tick();
        }
      }) {
        eprintln!("napi-rs: failed to tick async driver: {e:?}");
      }

      channel.drain(&mut env);

      check_status!(
        unsafe { sys::napi_close_handle_scope(env_raw, handle_scope) },
        "Failed to close handle scope for async tick"
      )?;
      Ok(())
    })();

    if let Err(e) = result {
      eprintln!("napi-rs: async driver tick failed: {e:?}");
    }
  });
}

// ---------------------------------------------------------------------------
// Public API on Env
// ---------------------------------------------------------------------------

impl<'env> Env<'env> {
  pub fn set_async_poll_context(&self, enter: impl Fn(&mut dyn FnMut()) + 'static) -> Result<()> {
    let record = self.record();
    record.with_data_mut(|data| {
      let driver = data
        .async_driver_mut()
        .ok_or_else(|| Error::new(Status::GenericFailure, "Async driver is not available"))?;
      driver.set_poll_context(enter);
      Ok(())
    })?
  }

  pub(crate) fn async_keep_alive(&self) -> Result<AsyncKeepAlive> {
    let record = self.record();
    record.with_data(|data| {
      let driver = data
        .async_driver()
        .ok_or_else(|| Error::new(Status::GenericFailure, "Async driver is not available"))?;
      Ok(driver.keep_alive())
    })?
  }

  pub(crate) fn spawn_future<T, F>(&self, future: F) -> Result<Task<T>>
  where
    T: 'static,
    F: Future<Output = T> + 'static,
  {
    let record = self.record();
    record.with_data(|data| {
      let driver = data
        .async_driver()
        .ok_or_else(|| Error::new(Status::GenericFailure, "Async driver is not available"))?;
      Ok(driver.spawn(future))
    })?
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn test_channel(handle_registered: bool) -> AsyncChannel {
    AsyncChannel {
      handle: alloc_async_handle(),
      env: std::ptr::null_mut(),
      queue: Mutex::new(Vec::new()),
      shutdown: AtomicBool::new(false),
      handle_closed: AtomicBool::new(false),
      handle_registered: AtomicBool::new(handle_registered),
      keepalive_count: AtomicUsize::new(0),
    }
  }

  #[test]
  fn unregistered_async_channel_drop_deallocates_handle() {
    test_reset_dealloc_count();
    drop(test_channel(false));
    assert_eq!(
      test_dealloc_count(),
      1,
      "unregistered handle should be deallocated when channel is dropped"
    );
  }

  #[test]
  fn registered_async_channel_drop_does_not_sync_dealloc() {
    test_reset_dealloc_count();
    drop(test_channel(true));
    assert_eq!(
      test_dealloc_count(),
      0,
      "registered handle must go through uv_close, not synchronous dealloc in Drop"
    );
  }
}
