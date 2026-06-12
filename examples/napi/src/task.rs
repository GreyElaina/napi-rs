use std::thread::sleep;
use std::time::Duration;

use napi::bindgen_prelude::*;

#[napi]
pub async fn without_abort_controller(a: u32, b: u32) -> Result<u32> {
  sleep(Duration::from_millis(100));
  Ok(a + b)
}

#[napi]
pub async fn with_abort_controller(a: u32, b: u32, signal: AbortSignal) -> Result<u32> {
  signal.on_abort(|| {});
  sleep(Duration::from_millis(100));
  Ok(a + b)
}

#[napi]
async fn with_abort_signal_handle(signal: AbortSignal) -> Result<i32> {
  let (sender, receiver) = std::sync::mpsc::channel::<i32>();
  signal.on_abort(move || {
    let _ = sender.send(999);
  });
  receiver.recv().map_err(|e| {
    Error::new(
      Status::GenericFailure,
      format!("Channel receive error: {e}"),
    )
  })
}

#[napi]
async fn blocking_void_return() -> Result<()> {
  Ok(())
}

#[napi]
pub async fn blocking_optional_return() -> Result<Option<u32>> {
  Ok(None)
}

#[napi]
pub async fn blocking_read_file(path: String) -> Result<Buffer> {
  let data =
    std::fs::read(&path).map_err(|e| Error::new(Status::GenericFailure, format!("{e}")))?;
  Ok(Buffer::from(data))
}

#[napi]
pub async fn async_resolve_array(inner: u32) -> Result<Vec<u32>> {
  Ok((0..inner).collect())
}

#[napi]
pub fn blocking_arraybuffer<'env>(
  #[napi(env)] env: &mut Env<'env>,
  data: Vec<u8>,
) -> Result<Promise<'env, ArrayBuffer<'env>>> {
  env.spawn_promise_with(
    async move {
      sleep(Duration::from_millis(10));
      Ok(data)
    },
    |scope, result| result.and_then(|output| ArrayBuffer::from_data(scope.env(), output)),
  )
}
