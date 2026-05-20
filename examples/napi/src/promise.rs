use napi::{bindgen_prelude::*, Error};

#[napi]
pub async fn async_plus_100(p: PromiseFuture<u32>) -> Result<u32> {
  let v = p.await?;
  Ok(v + 100)
}

#[napi]
pub fn call_then_on_promise(input: Promise<'_, u32>) -> Result<Promise<'_, String>> {
  input.then(|_, v| Ok(format!("{}", v.value)))
}

#[napi]
pub fn call_catch_on_promise(input: Promise<'_, u32>) -> Result<Promise<'_, String>> {
  input.catch(|_, e: CallbackContext<String>| Ok(e.value))
}

#[napi]
pub fn call_finally_on_promise(
  mut input: Promise<u32>,
  on_finally: FunctionRef<(), ()>,
) -> Result<Promise<u32>> {
  input.finally(move |scope| {
    let on_finally = scope.borrow_function(&on_finally)?;
    scope.call(&on_finally, ())?;
    Ok(())
  })
}

#[napi]
pub fn esm_resolve<'env, 'scope>(
  scope: &mut Scope<'env, 'scope>,
  next: Function<'scope, (), Promise<'scope, ()>>,
) -> Result<Promise<'scope, ()>> {
  scope.call(&next, ())
}

#[napi]
pub fn spawn_future_lifetime<'env>(env: &'env Env, input: u32) -> Result<Promise<'env, String>> {
  env.spawn_future(async move { Ok(format!("{}", input)) })
}

#[napi]
pub struct ClassReturnInPromise {}

#[napi]
pub fn promise_return_class_instance<'env>(
  env: &'env Env,
) -> Result<Promise<'env, ClassInitializer<ClassReturnInPromise>>> {
  env.spawn_future(async move { Ok(ClassInitializer::from(ClassReturnInPromise {})) })
}

#[napi]
pub fn create_resolved_promise<'env>(env: &'env Env, value: u32) -> Result<Promise<'env, u32>> {
  Promise::resolve(env, value)
}

#[napi]
pub fn create_rejected_promise<'env>(
  env: &'env Env,
  message: String,
) -> Result<Promise<'env, u32>> {
  Promise::reject(env, Error::from_reason(message))
}
