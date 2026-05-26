use std::{sync::mpsc, thread::sleep, time::Duration};

use napi::bindgen_prelude::*;

#[napi]
pub fn without_abort_controller<'env>(
  #[napi(env)] env: &mut Env<'env>,
  a: u32,
  b: u32,
) -> Result<Promise<'env, u32>> {
  env.with_scope(|scope| {
    scope
      .blocking(move || {
        sleep(Duration::from_millis(100));
        Ok(a + b)
      })
      .promise(|_, output| Ok(output))
  })
}

#[napi]
pub fn with_abort_controller<'env>(
  #[napi(env)] env: &mut Env<'env>,
  a: u32,
  b: u32,
  signal: AbortSignal,
) -> Result<Promise<'env, u32>> {
  env.with_scope(|scope| {
    scope
      .blocking(move || {
        sleep(Duration::from_millis(100));
        Ok(a + b)
      })
      .signal(signal)
      .promise(|_, output| Ok(output))
  })
}

#[napi]
fn with_abort_signal_handle<'env>(
  #[napi(env)] env: &mut Env<'env>,
  signal: AbortSignal,
) -> Result<Promise<'env, i32>> {
  let (sender, receiver) = mpsc::channel::<i32>();
  signal.on_abort(move || {
    if sender.send(999).is_err() {
      return;
    }
  });
  env.with_scope(|scope| {
    scope
      .blocking(move || {
        receiver.recv().map_err(|e| {
          Error::new(
            Status::GenericFailure,
            format!("Channel receive error: {e}"),
          )
        })
      })
      .signal(signal)
      .promise(|_, output| Ok(output))
  })
}

#[napi]
fn blocking_void_return<'env>(#[napi(env)] env: &mut Env<'env>) -> Result<Promise<'env, ()>> {
  env.with_scope(|scope| scope.blocking(|| Ok(())).promise(|_, output| Ok(output)))
}

#[napi]
pub fn blocking_optional_return<'env>(
  #[napi(env)] env: &mut Env<'env>,
) -> Result<Promise<'env, Option<u32>>> {
  env.with_scope(|scope| {
    scope
      .blocking(|| Ok(()))
      .promise(|_, ()| Ok(Option::<u32>::None))
  })
}

#[napi]
pub fn blocking_read_file<'env>(
  #[napi(env)] env: &mut Env<'env>,
  path: String,
) -> Result<Promise<'env, Buffer>> {
  env.with_scope(|scope| {
    scope
      .blocking(move || {
        std::fs::read(&path).map_err(|e| Error::new(Status::GenericFailure, format!("{e}")))
      })
      .promise(|_, output| Ok(Buffer::from(output)))
  })
}

#[napi]
pub fn async_resolve_array<'env>(
  #[napi(env)] env: &mut Env<'env>,
  inner: u32,
) -> Result<Promise<'env, Vec<u32>>> {
  env.with_scope(|scope| {
    scope
      .blocking(move || Ok(inner))
      .promise(|_, output| Ok((0..output).collect::<Vec<_>>()))
  })
}

#[napi]
pub fn blocking_finally<'env>(
  #[napi(env)] env: &mut Env<'env>,
  inner: ObjectRef,
) -> Result<Promise<'env, ()>> {
  let label = "task-finally-cleanup".to_owned();
  env.with_scope(|scope| {
    scope
      .blocking(move || {
        drop(label);
        Ok(())
      })
      .promise(move |scope, ()| {
        let env = scope.env();
        let mut obj = inner.to_local(env)?;
        obj.set("resolve", true)?;
        obj.set("finally", true)?;
        inner.close(env)?;
        Ok(())
      })
  })
}

#[napi]
pub fn blocking_arraybuffer<'env>(
  #[napi(env)] env: &mut Env<'env>,
  data: Vec<u8>,
) -> Result<Promise<'env, ArrayBuffer<'env>>> {
  env.with_scope(|scope| {
    scope
      .blocking(move || {
        sleep(Duration::from_millis(10));
        Ok(data)
      })
      .promise(|scope, output| ArrayBuffer::from_data(scope.env(), output))
  })
}
