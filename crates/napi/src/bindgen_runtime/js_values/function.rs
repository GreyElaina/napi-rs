use std::{marker::PhantomData, ptr, rc::Rc};

use super::{
  value_ref::{create_reference, ensure_same_record, reference_value, RefState},
  Either, FromJs, Func, IntoJs, JsRefTarget, Local, Ref, Scope, TypeName, Unknown,
  ValidateNapiValue,
};

use crate::{
  bindgen_runtime::JsObjectValue, check_pending_exception, check_status, sys, Env, JsValue, Result,
  ValueType,
};

pub trait IntoJsArgs<'scope> {
  fn into_js_args(self, scope: &mut Scope<'_, 'scope>) -> Result<JsArgs<'scope>>;
}

pub struct JsArgs<'scope> {
  values: Vec<sys::napi_value>,
  scope: PhantomData<&'scope ()>,
}

impl JsArgs<'_> {
  pub fn empty() -> Self {
    Self {
      values: Vec::new(),
      scope: PhantomData,
    }
  }
}

impl<'scope> JsArgs<'scope> {
  pub fn single<T>(scope: &mut Scope<'_, 'scope>, value: T) -> Result<Self>
  where
    T: IntoJs<'scope> + 'scope,
  {
    let mut args = Self::empty();
    args.push(scope, value)?;
    Ok(args)
  }

  pub fn push<T>(&mut self, scope: &mut Scope<'_, 'scope>, value: T) -> Result<()>
  where
    T: IntoJs<'scope> + 'scope,
  {
    self.values.push(value.into_js(scope)?.raw());
    Ok(())
  }

  fn from_values(values: Vec<sys::napi_value>) -> Self {
    Self {
      values,
      scope: PhantomData,
    }
  }

  pub(crate) fn as_slice(&self) -> &[sys::napi_value] {
    &self.values
  }

  pub(crate) fn as_mut_ptr(&mut self) -> *mut sys::napi_value {
    self.values.as_mut_ptr()
  }

  pub(crate) fn len(&self) -> usize {
    self.values.len()
  }

  pub(crate) fn insert_front(&mut self, value: sys::napi_value) {
    self.values.insert(0, value);
  }
}

impl<'scope, T> IntoJsArgs<'scope> for T
where
  T: IntoJs<'scope> + 'scope,
{
  fn into_js_args(self, scope: &mut Scope<'_, 'scope>) -> Result<JsArgs<'scope>> {
    if std::mem::size_of::<T>() == 0 {
      Ok(JsArgs::empty())
    } else {
      JsArgs::single(scope, self)
    }
  }
}

pub trait FromJsArgs<'env, 'scope> {
  fn from_js_args(scope: &mut Scope<'env, 'scope>, args: JsArgSlice<'scope>) -> Result<Self>
  where
    Self: Sized;
}

#[derive(Clone, Copy)]
pub struct JsArgSlice<'scope> {
  values: &'scope [sys::napi_value],
}

impl<'scope> JsArgSlice<'scope> {
  pub(crate) fn new(values: &'scope [sys::napi_value]) -> Self {
    Self { values }
  }

  pub fn len(self) -> usize {
    self.values.len()
  }

  pub fn is_empty(self) -> bool {
    self.values.is_empty()
  }

  pub fn get<'env, T>(self, scope: &mut Scope<'env, 'scope>, index: usize) -> Result<Option<T>>
  where
    T: FromJs<'env, 'scope>,
  {
    self
      .values
      .get(index)
      .copied()
      .map(|raw| {
        let value = unsafe { Local::from_raw(raw) };
        T::from_js(scope, value)
      })
      .transpose()
  }

  pub fn get_required<'env, T>(self, scope: &mut Scope<'env, 'scope>, index: usize) -> Result<T>
  where
    T: FromJs<'env, 'scope>,
  {
    self.get(scope, index)?.ok_or_else(|| {
      crate::Error::new(
        crate::Status::GenericFailure,
        "Arguments index out of range".to_owned(),
      )
    })
  }

  pub fn collect<'env, T>(self, scope: &mut Scope<'env, 'scope>) -> Result<Vec<T>>
  where
    T: FromJs<'env, 'scope>,
  {
    let mut values = Vec::with_capacity(self.values.len());
    for raw in self.values {
      let value = unsafe { Local::from_raw(*raw) };
      values.push(T::from_js(scope, value)?);
    }
    Ok(values)
  }
}

#[repr(C)]
pub struct FnArgs<T> {
  pub data: T,
}

impl<T> From<T> for FnArgs<T> {
  fn from(value: T) -> Self {
    FnArgs { data: value }
  }
}

macro_rules! impl_tuple_conversion {
  (@unit $ident:ident) => { () };
  ($($ident:ident: $index:tt),*) => {
    impl<'scope, $($ident),*> IntoJsArgs<'scope> for FnArgs<($($ident,)*)>
    where
      $($ident: IntoJs<'scope> + 'scope),*
    {
      fn into_js_args(self, scope: &mut Scope<'_, 'scope>) -> Result<JsArgs<'scope>> {
        #[allow(non_snake_case)]
        let ($($ident,)*) = self.data;
        Ok(JsArgs::from_values(vec![$($ident.into_js(scope)?.raw()),*]))
      }
    }

    impl<'env, 'scope, $($ident),*> FromJsArgs<'env, 'scope> for ($($ident,)*)
    where
      $($ident: FromJs<'env, 'scope>),*
    {
      fn from_js_args(
        scope: &mut Scope<'env, 'scope>,
        args: JsArgSlice<'scope>,
      ) -> $crate::Result<Self> {
        const EXPECTED_LEN: usize = <[()]>::len(&[$(impl_tuple_conversion!(@unit $ident)),*]);
        if args.len() != EXPECTED_LEN {
          return Err(crate::Error::new(
          crate::Status::InvalidArg,
          "Invalid number of arguments",
          ));
        }
        Ok(($(
          args.get_required::<$ident>(scope, $index)?
        ,)*))
      }
    }

    impl<'env, 'scope, $($ident),*> FromJsArgs<'env, 'scope> for FnArgs<($($ident,)*)>
    where
      $($ident: FromJs<'env, 'scope>),*
    {
      fn from_js_args(
        scope: &mut Scope<'env, 'scope>,
        args: JsArgSlice<'scope>,
      ) -> $crate::Result<Self> {
        Ok(FnArgs {
          data: <($($ident,)*)>::from_js_args(scope, args)?,
        })
      }
    }
  };
}

impl_tuple_conversion!(A: 0);
impl_tuple_conversion!(A: 0, B: 1);
impl_tuple_conversion!(A: 0, B: 1, C: 2);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11, M: 12);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11, M: 12, N: 13);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11, M: 12, N: 13, O: 14);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11, M: 12, N: 13, O: 14, P: 15);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11, M: 12, N: 13, O: 14, P: 15, Q: 16);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11, M: 12, N: 13, O: 14, P: 15, Q: 16, R: 17);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11, M: 12, N: 13, O: 14, P: 15, Q: 16, R: 17, S: 18);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11, M: 12, N: 13, O: 14, P: 15, Q: 16, R: 17, S: 18, T: 19);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11, M: 12, N: 13, O: 14, P: 15, Q: 16, R: 17, S: 18, T: 19, U: 20);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11, M: 12, N: 13, O: 14, P: 15, Q: 16, R: 17, S: 18, T: 19, U: 20, V: 21);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11, M: 12, N: 13, O: 14, P: 15, Q: 16, R: 17, S: 18, T: 19, U: 20, V: 21, W: 22);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11, M: 12, N: 13, O: 14, P: 15, Q: 16, R: 17, S: 18, T: 19, U: 20, V: 21, W: 22, X: 23);
impl_tuple_conversion!(A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11, M: 12, N: 13, O: 14, P: 15, Q: 16, R: 17, S: 18, T: 19, U: 20, V: 21, W: 22, X: 23, Y: 24);
impl_tuple_conversion!(
  A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7, I: 8, J: 9, K: 10, L: 11, M: 12, N: 13, O: 14, P: 15, Q: 16, R: 17, S: 18, T: 19, U: 20, V: 21, W: 22, X: 23, Y: 24, Z: 25
);

#[derive(Clone, Copy)]
/// A JavaScript function.
/// It can only live in the scope of a function call.
/// If you want to use it outside the scope of a function call, you can turn it into a reference.
/// By calling the `create_ref` method.
pub struct Function<'scope, Args = Unknown<'scope>, Return = Unknown<'scope>> {
  pub(crate) env: sys::napi_env,
  pub(crate) value: sys::napi_value,
  pub(crate) _args: std::marker::PhantomData<Args>,
  pub(crate) _return: std::marker::PhantomData<Return>,
  pub(crate) _scope: std::marker::PhantomData<&'scope ()>,
}

impl<Args, Return> TypeName for Function<'_, Args, Return> {
  fn type_name() -> &'static str {
    "Function"
  }

  fn value_type() -> crate::ValueType {
    ValueType::Function
  }
}

impl<'env, Args, Return> JsValue<'env> for Function<'env, Args, Return> {
  fn value(&self) -> crate::Value {
    crate::Value {
      value: self.value,
      env: self.env,
      value_type: ValueType::Function,
    }
  }
}

impl<'env, Args, Return> JsObjectValue<'env> for Function<'env, Args, Return> {}

impl<Args, Return> Function<'_, Args, Return> {
  pub(crate) unsafe fn from_raw(env: sys::napi_env, value: sys::napi_value) -> Self {
    Function {
      env,
      value,
      _args: std::marker::PhantomData,
      _return: std::marker::PhantomData,
      _scope: std::marker::PhantomData,
    }
  }
}

impl<'env, 'scope, Args, Return> FromJs<'env, 'scope> for Function<'scope, Args, Return> {
  fn from_js(
    scope: &mut super::Scope<'env, 'scope>,
    value: super::Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(Function {
      env: scope.env().raw(),
      value: value.raw(),
      _args: std::marker::PhantomData,
      _return: std::marker::PhantomData,
      _scope: std::marker::PhantomData,
    })
  }
}

impl<Args, Return> ValidateNapiValue for Function<'_, Args, Return> {}

impl<Args, Return> Function<'_, Args, Return> {
  /// Get the name of the JavaScript function.
  pub fn name(&self) -> Result<String> {
    let mut name = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_get_named_property(self.env, self.value, c"name".as_ptr().cast(), &mut name)
      },
      "Get function name failed"
    )?;
    let mut env = unsafe { Env::from_raw(self.env) };
    env.with_scope(|scope| String::from_js(scope, unsafe { Local::from_raw(name) }))
  }

  /// Create a new instance of the JavaScript Class.
  pub(crate) fn new_instance<'env, 'scope, CallArgs>(
    &self,
    scope: &mut Scope<'env, 'scope>,
    args: CallArgs,
  ) -> Result<Unknown<'scope>>
  where
    CallArgs: IntoJsArgs<'scope>,
  {
    let mut raw_instance = ptr::null_mut();
    let mut args = args.into_js_args(scope)?;
    check_status!(
      unsafe {
        sys::napi_new_instance(
          self.env,
          self.value,
          args.len(),
          args.as_mut_ptr().cast(),
          &mut raw_instance,
        )
      },
      "Create new instance failed"
    )?;
    let value = unsafe { Local::from_raw(raw_instance) };
    Unknown::from_js(scope, value)
  }

}

pub type FunctionRef<Args, Return> = Ref<Func<Args, Return>>;

impl<'scope, Args, Return> JsRefTarget<'scope, Ref<Func<Args, Return>>>
  for &Function<'_, Args, Return>
{
  fn create_ref(self, scope: &mut Scope<'_, 'scope>) -> Result<Ref<Func<Args, Return>>> {
    scope.ensure_value_env(self.env, "Function")?;
    let raw = create_reference(scope.env().raw(), self.value, 1)?;
    Ok(Ref::new(
      RefState::new(raw, Rc::downgrade(scope.record())),
      (),
    ))
  }
}

impl<Args, Return> Function<'_, Args, Return> {
  /// Call the JavaScript function.
  /// `this` in the JavaScript function will be `undefined`.
  /// If you want to specify `this`, you can use the `apply` method.
  pub(crate) fn call<'env, 'scope, CallArgs>(
    &self,
    scope: &mut Scope<'env, 'scope>,
    args: CallArgs,
  ) -> Result<Return>
  where
    CallArgs: IntoJsArgs<'scope>,
    Return: FromJs<'env, 'scope>,
  {
    let mut raw_this = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_undefined(self.env, &mut raw_this) },
      "Get undefined value failed"
    )?;
    let args = args.into_js_args(scope)?;
    let raw_return = unsafe { call_function_raw(self.env, raw_this, self.value, args.as_slice()) }?;
    let value = unsafe { Local::from_raw(raw_return) };
    Return::from_js(scope, value)
  }

  /// Call the JavaScript function.
  /// `this` in the JavaScript function will be the provided `this`.
  pub(crate) fn apply<'env, 'scope, Context, CallArgs>(
    &self,
    scope: &mut Scope<'env, 'scope>,
    this: Context,
    args: CallArgs,
  ) -> Result<Return>
  where
    CallArgs: IntoJsArgs<'scope>,
    Context: IntoJs<'scope> + 'scope,
    Return: FromJs<'env, 'scope>,
  {
    let raw_this = this.into_js(scope)?.raw();
    let args = args.into_js_args(scope)?;
    let raw_return = unsafe { call_function_raw(self.env, raw_this, self.value, args.as_slice()) }?;
    let value = unsafe { Local::from_raw(raw_return) };
    Return::from_js(scope, value)
  }

  /// Call `Function.bind`
  pub(crate) fn bind<'scope, T>(
    &self,
    scope: &mut Scope<'_, 'scope>,
    this: T,
  ) -> Result<Function<'scope, Args, Return>>
  where
    T: IntoJs<'scope> + 'scope,
  {
    let raw_this = this.into_js(scope)?.raw();
    let mut bind_function = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_get_named_property(self.env, self.value, c"bind".as_ptr(), &mut bind_function)
      },
      "Get bind function failed"
    )?;
    let bound_function =
      unsafe { call_function_raw(self.env, self.value, bind_function, &[raw_this]) }?;
    Ok(Function {
      env: self.env,
      value: bound_function,
      _args: std::marker::PhantomData,
      _return: std::marker::PhantomData,
      _scope: std::marker::PhantomData,
    })
  }
}

impl<'env, 'scope> Scope<'env, 'scope> {
  pub fn call<Args, Return, CallArgs>(
    &mut self,
    function: &Function<'_, Args, Return>,
    args: CallArgs,
  ) -> Result<Return>
  where
    CallArgs: IntoJsArgs<'scope>,
    Return: FromJs<'env, 'scope>,
  {
    function.call(self, args)
  }

  pub fn apply<Args, Return, Context, CallArgs>(
    &mut self,
    function: &Function<'_, Args, Return>,
    this: Context,
    args: CallArgs,
  ) -> Result<Return>
  where
    CallArgs: IntoJsArgs<'scope>,
    Context: IntoJs<'scope> + 'scope,
    Return: FromJs<'env, 'scope>,
  {
    function.apply(self, this, args)
  }

  pub fn bind_function<Args, Return, T>(
    &mut self,
    function: &Function<'_, Args, Return>,
    this: T,
  ) -> Result<Function<'scope, Args, Return>>
  where
    T: IntoJs<'scope> + 'scope,
  {
    function.bind(self, this)
  }

  pub fn new_instance<Args, Return, CallArgs>(
    &mut self,
    constructor: &Function<'_, Args, Return>,
    args: CallArgs,
  ) -> Result<Unknown<'scope>>
  where
    CallArgs: IntoJsArgs<'scope>,
  {
    constructor.new_instance(self, args)
  }

  pub fn borrow_function<Args, Return>(
    &mut self,
    function: &Ref<Func<Args, Return>>,
  ) -> Result<Function<'scope, Args, Return>> {
    function.borrow(self)
  }
}

unsafe fn call_function_raw(
  env: sys::napi_env,
  receiver: sys::napi_value,
  function: sys::napi_value,
  args: &[sys::napi_value],
) -> Result<sys::napi_value> {
  let mut raw_return = ptr::null_mut();
  check_pending_exception!(
    env,
    unsafe {
      sys::napi_call_function(
        env,
        receiver,
        function,
        args.len(),
        args.as_ptr(),
        &mut raw_return,
      )
    },
    "Call Function failed"
  )?;
  Ok(raw_return)
}

impl<Args, Return> Ref<Func<Args, Return>> {
  pub(crate) fn borrow<'env, 'scope>(
    &self,
    scope: &mut Scope<'env, 'scope>,
  ) -> Result<Function<'scope, Args, Return>> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;
    let value = reference_value(scope.env().raw(), self.state.raw_ref()?)?;
    Ok(Function {
      env: scope.env().raw(),
      value,
      _args: std::marker::PhantomData,
      _return: std::marker::PhantomData,
      _scope: std::marker::PhantomData,
    })
  }
}

impl<Args, Return> TypeName for Ref<Func<Args, Return>> {
  fn type_name() -> &'static str {
    "Function"
  }

  fn value_type() -> crate::ValueType {
    ValueType::Function
  }
}

impl<'env, 'scope, Args, Return> FromJs<'env, 'scope> for Ref<Func<Args, Return>> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let function = Function::from_js(scope, value)?;
    scope.create_ref(&function)
  }
}

impl<Args, Return> ValidateNapiValue for Ref<Func<Args, Return>> {}

pub struct FunctionCallContext<'env, 'scope, 'context> {
  pub(crate) args: JsArgSlice<'scope>,
  pub(crate) this: sys::napi_value,
  pub(crate) scope: &'context mut Scope<'env, 'scope>,
}

impl<'env, 'scope, 'context> FunctionCallContext<'env, 'scope, 'context> {
  /// Get the number of arguments from the JavaScript function call.
  pub fn length(&self) -> usize {
    self.args.len()
  }

  pub fn env(&self) -> Env<'_> {
    *self.scope.env()
  }

  pub fn get<ArgType: FromJs<'env, 'scope>>(&mut self, index: usize) -> Result<ArgType> {
    self.args.get_required(self.scope, index)
  }

  pub fn try_get<ArgType: TypeName + FromJs<'env, 'scope>>(
    &mut self,
    index: usize,
  ) -> Result<Either<ArgType, ()>> {
    match self.args.get(self.scope, index)? {
      Some(value) => Ok(Either::A(value)),
      None => Ok(Either::B(())),
    }
  }

  /// Get the first argument from the JavaScript function call.
  pub fn first_arg<T: FromJs<'env, 'scope>>(&mut self) -> Result<T> {
    if self.args.is_empty() {
      return Err(crate::Error::new(
        crate::Status::InvalidArg,
        "There is no arguments",
      ));
    }
    self.args.get_required(self.scope, 0)
  }

  /// Get the arguments from the JavaScript function call.
  /// The arguments will be converted to a tuple.
  /// If the number of arguments is not equal to the number of tuple elements, an error will be returned.
  /// example:
  /// ```rust
  /// let (num, string) = ctx.args::<(u32, String)>()?;
  /// ````
  pub fn args<Args: FromJsArgs<'env, 'scope>>(&mut self) -> Result<Args> {
    Args::from_js_args(self.scope, self.args)
  }

  /// Get the arguments Vec from the JavaScript function call.
  pub fn arguments<T: FromJs<'env, 'scope>>(&mut self) -> Result<Vec<T>> {
    self.args.collect(self.scope)
  }

  /// Get the `this` from the JavaScript function call.
  pub fn this<This: FromJs<'env, 'scope>>(&mut self) -> Result<This> {
    let value = unsafe { Local::from_raw(self.this) };
    This::from_js(self.scope, value)
  }
}
