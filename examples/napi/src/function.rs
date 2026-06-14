use napi::{
  bindgen_prelude::{Class, EnvRecord, FnArgs, Function, FunctionRef, Ref, Scope},
  Env, Result,
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
  ctx: Ref<Class<Animal>>,
  callback: Function<(), ()>,
) -> Result<()> {
  scope.apply(&callback, ctx, ())
}

#[napi]
pub fn apply1(
  #[napi(scope)] scope: &mut Scope,
  ctx: Ref<Class<Animal>>,
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

#[napi]
pub fn call_function_with_arg_and_ctx(
  #[napi(scope)] scope: &mut Scope,
  ctx: Ref<Class<Animal>>,
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
  callback: Option<Function<String, ()>>,
) -> Result<()> {
  if let Some(callback) = callback {
    scope.call(&callback, "Hello".to_owned())?;
  }
  Ok(())
}

#[napi]
pub fn verify_env_record_current() -> bool {
  EnvRecord::current().is_ok()
}

#[napi]
pub struct FnRefHolder {
  sum_cb: FunctionRef<FnArgs<(u32, u32)>, u32>,
  fmt_cb: FunctionRef<String, String>,
}

#[napi]
impl FnRefHolder {
  #[napi(constructor)]
  pub fn new(
    sum_cb: FunctionRef<FnArgs<(u32, u32)>, u32>,
    fmt_cb: FunctionRef<String, String>,
  ) -> Self {
    Self { sum_cb, fmt_cb }
  }

  #[napi]
  pub fn call_sum(&self, a: u32, b: u32) -> Result<u32> {
    let cb = &self.sum_cb;
    cb.with_scope(|scope| {
      let func = scope.borrow_function(cb)?;
      scope.call(&func, FnArgs::from((a, b)))
    })
  }

  #[napi]
  pub fn call_fmt(&self, input: String) -> Result<String> {
    let cb = &self.fmt_cb;
    cb.with_scope(|scope| {
      let func = scope.borrow_function(cb)?;
      scope.call(&func, input)
    })
  }
}
