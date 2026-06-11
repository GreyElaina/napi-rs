use napi::bindgen_prelude::*;

/// Returns a promise created before a forced `spawn_future` failure.
#[napi]
pub fn regression_promise_if_spawn_fails<'env>(
  #[napi(env)] env: &'env Env<'env>,
) -> Result<Promise<'env, ()>> {
  env.regression_promise_if_spawn_fails()
}
