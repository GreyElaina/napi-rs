use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use crate::{
  bindgen_runtime::{FromJs, Local, Scope},
  JsValue, Result, Value,
};

use super::{Object, Unknown};

#[derive(Clone, Copy)]
pub struct This<'env, T = Object<'env>> {
  pub object: T,
  _phantom: &'env PhantomData<()>,
}

impl<T> From<T> for This<'_, T> {
  fn from(value: T) -> Self {
    Self {
      object: value,
      _phantom: &PhantomData,
    }
  }
}

impl<T> Deref for This<'_, T> {
  type Target = T;

  fn deref(&self) -> &Self::Target {
    &self.object
  }
}

impl<T> DerefMut for This<'_, T> {
  fn deref_mut(&mut self) -> &mut Self::Target {
    &mut self.object
  }
}

impl<'env, T: JsValue<'env>> JsValue<'env> for This<'_, T> {
  fn value(&self) -> Value {
    self.object.value()
  }
}

impl<'env, 'scope, T> FromJs<'env, 'scope> for This<'scope, T>
where
  T: FromJs<'env, 'scope>,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(Self {
      object: T::from_js(scope, value)?,
      _phantom: &PhantomData,
    })
  }
}
