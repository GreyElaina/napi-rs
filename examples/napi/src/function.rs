use napi::{
  bindgen_prelude::{FnArgs, Function, FunctionRef, Promise, Reference, Scope},
  threadsafe_function::{ThreadsafeFunctionCallMode, UnknownReturnValue},
  Env, Error, Result, Status,
};

use crate::class::Animal;

#[napi]
pub fn call0(#[napi(scope)] scope: &mut Scope, callback: Function<(), u32>) -> Result<u32> {
  scope.call(&callback, ())
}

#[napi]
pub fn call1(
  #[napi(scope)] scope: &mut Scope,
  callback: Function<u32, u32>,
  arg: u32,
) -> Result<u32> {
  scope.call(&callback, arg)
}

#[napi]
pub fn call2(
  #[napi(scope)] scope: &mut Scope,
  callback: Function<FnArgs<(u32, u32)>, u32>,
  arg1: u32,
  arg2: u32,
) -> Result<u32> {
  scope.call(&callback, FnArgs::from((arg1, arg2)))
}

#[napi]
pub fn call_with_tuple_arg(
  #[napi(scope)] scope: &mut Scope,
  callback: Function<(u32, u32), u32>,
  arg1: u32,
  arg2: u32,
) -> Result<u32> {
  scope.call(&callback, (arg1, arg2))
}

#[napi]
pub fn call_with_nested_function_arg<'env, 'scope>(
  #[napi(scope)] scope: &mut Scope<'env, 'scope>,
  callback: Function<'scope, Function<'scope, u32, u32>, u32>,
) -> Result<u32> {
  let inner: Function<'scope, u32, u32> =
    scope.create_function("inner", no_export_function_c_callback)?;
  scope.call(&callback, inner)
}

#[napi]
pub fn apply0(
  #[napi(scope)] scope: &mut Scope,
  ctx: Reference<Animal>,
  callback: Function<(), ()>,
) -> Result<()> {
  scope.apply(&callback, ctx, ())
}

#[napi]
pub fn apply1(
  #[napi(scope)] scope: &mut Scope,
  ctx: Reference<Animal>,
  callback: Function<String, ()>,
  name: String,
) -> Result<()> {
  scope.apply(&callback, ctx, name)
}

#[napi]
pub fn call_function(#[napi(scope)] scope: &mut Scope, cb: Function<(), u32>) -> Result<u32> {
  scope.call(&cb, ())
}

#[napi]
pub fn call_function_with_arg(
  #[napi(scope)] scope: &mut Scope,
  cb: Function<FnArgs<(u32, u32)>, u32>,
  arg0: u32,
  arg1: u32,
) -> Result<u32> {
  scope.call(&cb, FnArgs::from((arg0, arg1)))
}

#[napi(ts_return_type = "Promise<void>")]
pub fn create_reference_on_function<'env>(
  #[napi(env)] env: &'env Env,
  cb: Function<'env, (), ()>,
) -> Result<Promise<'env, ()>> {
  let tsfn = cb.build_threadsafe_function().build()?;
  env.spawn_future(async move {
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    tsfn.call((), ThreadsafeFunctionCallMode::NonBlocking);
    Ok(())
  })
}

#[napi]
pub fn call_function_with_arg_and_ctx(
  #[napi(scope)] scope: &mut Scope,
  ctx: Reference<Animal>,
  cb: Function<String, ()>,
  name: String,
) -> Result<()> {
  scope.apply(&cb, ctx, name)
}

#[napi]
pub fn reference_as_callback(
  #[napi(scope)] scope: &mut Scope,
  callback: FunctionRef<FnArgs<(u32, u32)>, u32>,
  arg0: u32,
  arg1: u32,
) -> Result<u32> {
  let callback = scope.borrow_function(&callback)?;
  scope.call(&callback, FnArgs::from((arg0, arg1)))
}

#[napi]
pub fn reference_with_tuple_arg(
  #[napi(scope)] scope: &mut Scope,
  callback: FunctionRef<(u32, u32), u32>,
  arg0: u32,
  arg1: u32,
) -> Result<u32> {
  let callback = scope.borrow_function(&callback)?;
  scope.call(&callback, (arg0, arg1))
}

#[napi]
pub fn build_threadsafe_function_from_function(
  callback: Function<FnArgs<(u32, u32)>, u32>,
) -> Result<()> {
  let tsfn = callback.build_threadsafe_function().build()?;
  let jh1 = std::thread::spawn(move || {
    tsfn.call((1, 2).into(), ThreadsafeFunctionCallMode::NonBlocking);
  });
  let tsfn_max_queue_size_1 = callback
    .build_threadsafe_function()
    .max_queue_size::<1>()
    .build()?;

  let jh2 = std::thread::spawn(move || {
    tsfn_max_queue_size_1.call((1, 2).into(), ThreadsafeFunctionCallMode::NonBlocking);
  });

  let tsfn_weak = callback
    .build_threadsafe_function()
    .weak::<true>()
    .build()?;

  let jh3 = std::thread::spawn(move || {
    tsfn_weak.call((1, 2).into(), ThreadsafeFunctionCallMode::NonBlocking);
  });

  jh1.join().unwrap();
  jh2.join().unwrap();
  jh3.join().unwrap();

  Ok(())
}

#[napi]
pub fn build_threadsafe_function_from_function_callee_handle(
  callback: Function<(), ()>,
) -> Result<()> {
  let tsfn = callback
    .build_threadsafe_function()
    .callee_handled::<true>()
    .build()?;

  std::thread::spawn(move || {
    tsfn.call(
      Err(Error::new(Status::GenericFailure, "run tsfn failed")),
      ThreadsafeFunctionCallMode::NonBlocking,
    );
  });

  Ok(())
}

#[napi]
pub fn create_function<'env>(#[napi(env)] env: &'env Env) -> Result<Function<'env, u32, u32>> {
  env.create_function("customFunction", no_export_function_c_callback)
}

#[napi(no_export)]
pub fn no_export_function(input: u32) -> u32 {
  input + 200
}

#[napi]
pub fn optional_callback_types(
  #[napi(scope)] scope: &mut Scope,
  callback: Option<Function<String, UnknownReturnValue>>,
) -> Result<()> {
  if let Some(callback) = callback {
    scope.call(&callback, "Hello".to_owned())?;
  }
  Ok(())
}
