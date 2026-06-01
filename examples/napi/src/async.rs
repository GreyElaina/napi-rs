use futures::prelude::*;
use napi::bindgen_prelude::*;
use napi::tokio::fs;

#[napi]
async fn read_file_async(path: String) -> Result<Buffer> {
  fs::read(path)
    .map(|r| match r {
      Ok(content) => Ok(content.into()),
      Err(e) => Err(Error::new(
        Status::GenericFailure,
        format!("failed to read file, {}", e),
      )),
    })
    .await
}

#[napi]
async fn async_multi_two(arg: u32) -> Result<u32> {
  tokio::task::spawn(async move { Ok(arg * 2) })
    .await
    .unwrap()
}

#[napi]
async fn panic_in_async() {
  panic!("panic in async function");
}

#[napi(async_runtime)]
pub fn within_async_runtime_if_available() {
  tokio::spawn(async {
    println!("within_runtime_if_available");
  });
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
    env.spawn_future(async move { Err(Error::new(Status::GenericFailure, "Throw async error")) })
  }
}
