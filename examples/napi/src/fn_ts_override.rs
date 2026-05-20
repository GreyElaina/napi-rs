use napi::bindgen_prelude::{FnArgs, Function, JsObjectValue, Object, Result, Scope};

#[napi(ts_args_type = "a: { foo: number }", ts_return_type = "string[]")]
fn ts_rename(a: Object) -> Result<Object> {
  a.get_property_names()
}

#[napi]
fn override_individual_arg_on_function(
  scope: &mut Scope,
  not_overridden: String,
  #[napi(ts_arg_type = "() => string")] f: Function<(), String>,
  not_overridden2: u32,
) -> String {
  let u = scope.call(&f, ()).unwrap();

  format!("oia: {}-{}-{}", not_overridden, not_overridden2, u)
}

#[napi]
fn override_individual_arg_on_function_with_cb_arg<'env, 'scope>(
  scope: &mut Scope<'env, 'scope>,
  #[napi(ts_arg_type = "(town: string, name?: string | undefined | null) => string")]
  callback: Function<'scope, FnArgs<(String, Option<String>)>, Object<'scope>>,
  not_overridden: u32,
) -> Result<Object<'scope>> {
  scope.call(
    &callback,
    (format!("World({})", not_overridden), None::<String>),
  )
}

#[napi(ts_type = "(operation: 'add' | 'subtract' | 'multiply', a: number, b: number): number")]
fn override_whole_function_type(operation: String, a: i32, b: i32) -> i32 {
  match operation.as_str() {
    "add" => a + b,
    "subtract" => a - b,
    "multiply" => a * b,
    _ => panic!("Invalid operation"),
  }
}
