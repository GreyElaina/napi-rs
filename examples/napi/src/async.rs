use napi::bindgen_prelude::*;

#[napi]
async fn read_file_async(path: String) -> Result<Buffer> {
  let content = std::fs::read(path).map_err(|e| {
    Error::new(
      Status::GenericFailure,
      format!("failed to read file, {}", e),
    )
  })?;
  Ok(content.into())
}

#[napi]
async fn async_read_file(path: String) -> Result<Buffer> {
  tokio::fs::read(path).await.map(|v| v.into()).map_err(|e| {
    Error::new(
      Status::GenericFailure,
      format!("failed to read file, {}", e),
    )
  })
}

#[napi]
async fn async_multi_two(arg: u32) -> Result<u32> {
  Ok(arg * 2)
}

#[napi]
async fn panic_in_async() {
  panic!("panic in async function");
}

#[napi(constructor)]
pub struct AsyncThrowClass {}

#[napi]
impl AsyncThrowClass {
  #[napi]
  pub fn async_throw_error<'env>(
    &self,
    #[napi(env)] env: &'env Env<'env>,
  ) -> Result<Promise<'env, ()>> {
    env.spawn_promise(async move { Err(Error::new(Status::GenericFailure, "Throw async error")) })
  }
}
