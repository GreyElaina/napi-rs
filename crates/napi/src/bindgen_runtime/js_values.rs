use std::{
  ptr,
  rc::Rc,
  sync::{Arc, Mutex},
};

use crate::{check_status, sys, Env, Error, JsValue, Result, Status, ValueType};

use super::{Local, Scope};

mod array;
mod arraybuffer;
#[cfg(feature = "napi6")]
mod bigint;
mod boolean;
mod buffer;
mod class;
#[cfg(all(feature = "chrono_date", feature = "napi5"))]
mod date;
mod either;
mod external;
mod function;
mod map;
mod nil;
mod number;
mod object;
mod promise;
mod promise_future;
mod scope;
#[cfg(feature = "serde-json")]
mod serde;
mod set;
#[cfg(feature = "web_stream")]
mod stream;
mod string;
mod symbol;
#[cfg(all(feature = "async", feature = "napi4"))]
mod task;
mod this;
pub(crate) mod value_ref;

pub use crate::js_values::Unknown;
#[cfg(feature = "napi5")]
pub use crate::JsDate as Date;
pub use array::*;
pub use arraybuffer::*;
#[cfg(feature = "napi6")]
pub use bigint::*;
pub use buffer::*;
pub use class::*;
pub use either::*;
pub use external::*;
pub use function::*;
pub use nil::*;
pub use object::*;
pub use promise::*;
pub use promise_future::*;
pub use scope::*;
#[cfg(feature = "web_stream")]
pub use stream::*;
pub use string::*;
pub use symbol::*;
#[cfg(all(feature = "async", feature = "napi4"))]
pub use task::*;
pub use this::*;
pub use value_ref::*;

pub trait TypeName {
  fn type_name() -> &'static str;

  fn value_type() -> ValueType;

  fn ts_type() -> String {
    Self::type_name().to_owned()
  }
}

pub trait IntoJs<'scope>: Sized {
  type Output;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>>;

  fn into_unknown(self, scope: &mut Scope<'_, 'scope>) -> Result<Unknown<'scope>>
  where
    Self: 'scope,
  {
    let local = self.into_js(scope)?;
    Ok(unsafe { Unknown::from_raw_unchecked(scope.env().raw(), local.raw()) })
  }
}

#[doc(hidden)]
pub trait JsRefTarget<'scope, Ref> {
  fn create_ref(self, scope: &mut Scope<'_, 'scope>) -> Result<Ref>;
}

impl<'scope, T> IntoJs<'scope> for Local<'scope, T> {
  type Output = T;

  fn into_js(self, _: &mut Scope<'_, 'scope>) -> Result<Self> {
    Ok(self)
  }
}

pub(crate) unsafe fn into_js_raw<T>(env: sys::napi_env, value: T) -> Result<sys::napi_value>
where
  for<'scope> T: IntoJs<'scope>,
{
  let mut env = unsafe { Env::from_raw(env) };
  env.with_scope(|scope| value.into_js(scope).map(|local| local.raw()))
}

impl<'env, 'scope, T: JsValue<'env>> IntoJs<'scope> for T {
  type Output = T;

  fn into_js(self, _: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    Ok(unsafe { Local::from_raw(self.raw()) })
  }
}

pub trait FromJs<'env, 'scope>: Sized {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self>;
}

pub(crate) fn ensure_value_type(
  env: sys::napi_env,
  raw: sys::napi_value,
  expected: ValueType,
) -> Result<()> {
  let mut value_type = -1;
  check_status!(
    unsafe { sys::napi_typeof(env, raw, &mut value_type) },
    "Failed to detect napi value type",
  )?;

  let received = ValueType::from(value_type);
  if received == expected {
    Ok(())
  } else {
    Err(Error::new(
      Status::InvalidArg,
      format!("Expect value to be {expected}, but received {received}"),
    ))
  }
}

pub(crate) fn ensure_is_promise(env: sys::napi_env, raw: sys::napi_value) -> Result<()> {
  let mut is_promise = false;
  check_status!(
    unsafe { sys::napi_is_promise(env, raw, &mut is_promise) },
    "Failed to check if value is promise",
  )?;
  if is_promise {
    Ok(())
  } else {
    Err(Error::new(
      Status::InvalidArg,
      "Expected Promise object".to_owned(),
    ))
  }
}

pub(crate) fn ensure_is_buffer(env: sys::napi_env, raw: sys::napi_value) -> Result<()> {
  let mut is_buffer = false;
  check_status!(
    unsafe { sys::napi_is_buffer(env, raw, &mut is_buffer) },
    "Failed to validate napi buffer",
  )?;
  if is_buffer {
    Ok(())
  } else {
    Err(Error::new(
      Status::InvalidArg,
      "Expected a Buffer value".to_owned(),
    ))
  }
}

pub(crate) fn ensure_is_array(env: sys::napi_env, raw: sys::napi_value) -> Result<()> {
  let mut is_array = false;
  check_status!(
    unsafe { sys::napi_is_array(env, raw, &mut is_array) },
    "Failed to check given napi value is array",
  )?;
  if is_array {
    Ok(())
  } else {
    Err(Error::new(
      Status::InvalidArg,
      "Expected an array".to_owned(),
    ))
  }
}

impl<T: TypeName> TypeName for Option<T> {
  fn type_name() -> &'static str {
    T::type_name()
  }

  fn value_type() -> ValueType {
    T::value_type()
  }

  fn ts_type() -> String {
    format!("{} | null", T::ts_type())
  }
}

impl<'env, 'scope, T> FromJs<'env, 'scope> for Option<T>
where
  T: FromJs<'env, 'scope>,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let mut value_type = 0;
    check_status!(
      unsafe { sys::napi_typeof(scope.env().raw(), value.raw(), &mut value_type) },
      "Failed to detect optional JavaScript value type",
    )?;

    match value_type {
      sys::ValueType::napi_undefined | sys::ValueType::napi_null => Ok(None),
      _ => T::from_js(scope, value).map(Some),
    }
  }
}

impl<'env, 'scope, T> FromJs<'env, 'scope> for Rc<T>
where
  T: FromJs<'env, 'scope>,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(Rc::new(T::from_js(scope, value)?))
  }
}

impl<'env, 'scope, T> FromJs<'env, 'scope> for Arc<T>
where
  T: FromJs<'env, 'scope>,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(Arc::new(T::from_js(scope, value)?))
  }
}

impl<'env, 'scope, T> FromJs<'env, 'scope> for Mutex<T>
where
  T: FromJs<'env, 'scope>,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(Mutex::new(T::from_js(scope, value)?))
  }
}

impl<'env, 'scope> FromJs<'env, 'scope> for Unknown<'scope> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(unsafe { Unknown::from_raw_unchecked(scope.env().raw(), value.raw()) })
  }
}

impl<'scope, T> IntoJs<'scope> for Option<T>
where
  T: IntoJs<'scope> + 'scope,
{
  type Output = Unknown<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    match self {
      Some(value) => {
        let local = value.into_js(scope)?;
        Ok(unsafe { Local::from_raw(local.raw()) })
      }
      None => {
        let local = Null.into_js(scope)?;
        Ok(unsafe { Local::from_raw(local.raw()) })
      }
    }
  }
}

impl<'scope, T> IntoJs<'scope> for Result<T>
where
  T: IntoJs<'scope> + 'scope,
{
  type Output = Unknown<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    match self {
      Ok(value) => {
        let local = value.into_js(scope)?;
        Ok(unsafe { Local::from_raw(local.raw()) })
      }
      Err(e) => {
        let env = scope.env().raw();
        let error_code = format!("{:?}", e.status).into_js(scope)?.raw();
        let reason = e.reason.clone().into_js(scope)?.raw();
        let mut error = ptr::null_mut();
        check_status!(
          unsafe { sys::napi_create_error(env, error_code, reason, &mut error) },
          "Failed to create napi error"
        )?;

        Ok(unsafe { Local::from_raw(error) })
      }
    }
  }
}

impl<T: TypeName> TypeName for Rc<T> {
  fn type_name() -> &'static str {
    T::type_name()
  }

  fn value_type() -> ValueType {
    T::value_type()
  }

  fn ts_type() -> String {
    T::ts_type()
  }
}

impl<'scope, T> IntoJs<'scope> for Rc<T>
where
  T: IntoJs<'scope> + Clone + 'scope,
{
  type Output = T::Output;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    self.as_ref().clone().into_js(scope)
  }
}

impl<'scope, T> IntoJs<'scope> for &Rc<T>
where
  T: IntoJs<'scope> + Clone + 'scope,
{
  type Output = T::Output;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    self.as_ref().clone().into_js(scope)
  }
}

impl<'scope, T> IntoJs<'scope> for &mut Rc<T>
where
  T: IntoJs<'scope> + Clone + 'scope,
{
  type Output = T::Output;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    self.as_ref().clone().into_js(scope)
  }
}

impl<T: TypeName> TypeName for Arc<T> {
  fn type_name() -> &'static str {
    T::type_name()
  }

  fn value_type() -> ValueType {
    T::value_type()
  }

  fn ts_type() -> String {
    T::ts_type()
  }
}

impl<'scope, T> IntoJs<'scope> for Arc<T>
where
  T: IntoJs<'scope> + Clone + 'scope,
{
  type Output = T::Output;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    self.as_ref().clone().into_js(scope)
  }
}

impl<'scope, T> IntoJs<'scope> for &Arc<T>
where
  T: IntoJs<'scope> + Clone + 'scope,
{
  type Output = T::Output;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    self.as_ref().clone().into_js(scope)
  }
}

impl<'scope, T> IntoJs<'scope> for &mut Arc<T>
where
  T: IntoJs<'scope> + Clone + 'scope,
{
  type Output = T::Output;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    self.as_ref().clone().into_js(scope)
  }
}

impl<T: TypeName> TypeName for Mutex<T> {
  fn type_name() -> &'static str {
    T::type_name()
  }

  fn value_type() -> ValueType {
    T::value_type()
  }

  fn ts_type() -> String {
    T::ts_type()
  }
}

impl<'scope, T> IntoJs<'scope> for Mutex<T>
where
  T: IntoJs<'scope> + Clone + 'scope,
{
  type Output = T::Output;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    match self.lock() {
      Ok(inner) => inner.clone().into_js(scope),
      Err(_) => Err(Error::new(
        Status::GenericFailure,
        "Failed to acquire a lock",
      )),
    }
  }
}

impl<'scope, T> IntoJs<'scope> for &Mutex<T>
where
  T: IntoJs<'scope> + Clone + 'scope,
{
  type Output = T::Output;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    match self.lock() {
      Ok(inner) => inner.clone().into_js(scope),
      Err(_) => Err(Error::new(
        Status::GenericFailure,
        "Failed to acquire a lock",
      )),
    }
  }
}

impl<'scope, T> IntoJs<'scope> for &mut Mutex<T>
where
  T: IntoJs<'scope> + Clone + 'scope,
{
  type Output = T::Output;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    self
      .get_mut()
      .map_err(|_| Error::new(Status::GenericFailure, "Failed to acquire a lock"))?
      .clone()
      .into_js(scope)
  }
}
