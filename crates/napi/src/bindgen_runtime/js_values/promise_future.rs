use std::{
  cell::Cell,
  convert::identity,
  future,
  pin::Pin,
  rc::Rc,
  task::{Context, Poll},
};

use futures::channel::oneshot::{channel, Receiver};

use crate::{Error, Result, Status};

use super::{CallbackContext, FromJs, Local, Promise, Scope, TypeName, Unknown};

/// A Rust Future backed by a JavaScript Promise.
///
/// This `PromiseFuture<T>` can be awaited in Rust.
///
/// example:
///
/// ```no_run
/// #[napi]
/// pub fn await_promise_in_rust(promise: PromiseFuture<u32>) {
///   let value = promise.await.unwrap();
///
///   println!("{value}");
/// }
/// ```
pub struct PromiseFuture<T: 'static + for<'env, 'scope> FromJs<'env, 'scope>> {
  value: Pin<Box<Receiver<Result<T>>>>,
}

impl<T> TypeName for PromiseFuture<T>
where
  T: TypeName + 'static + for<'env, 'scope> FromJs<'env, 'scope>,
{
  fn type_name() -> &'static str {
    "Promise"
  }

  fn value_type() -> crate::ValueType {
    crate::ValueType::Object
  }

  fn ts_type() -> String {
    format!("Promise<{}>", T::ts_type())
  }
}

unsafe impl<T> Send for PromiseFuture<T> where
  T: Send + 'static + for<'env, 'scope> FromJs<'env, 'scope>
{
}

impl<T> PromiseFuture<T>
where
  T: 'static + for<'env, 'scope> FromJs<'env, 'scope>,
{
  pub(crate) fn from_promise(promise: Promise<'_, T>) -> crate::Result<Self> {
    let (tx, rx) = channel();
    let tx_box = Rc::new(Cell::new(Some(tx)));
    let tx_in_catch = tx_box.clone();
    promise
      .then(move |scope, ctx| {
        scope.env().raw();
        if let Some(sender) = tx_box.replace(None) {
          // no need to handle the send error here, the receiver has been dropped
          drop(sender.send(Ok(ctx.value)));
        }
        Ok(())
      })?
      .catch(move |scope, ctx: CallbackContext<Error>| {
        scope.env().raw();
        if let Some(sender) = tx_in_catch.replace(None) {
          // no need to handle the send error here, the receiver has been dropped
          drop(sender.send(Err(ctx.value)));
        }
        Ok(())
      })?;

    Ok(PromiseFuture {
      value: Box::pin(rx),
    })
  }
}

impl<'env, 'scope, T> FromJs<'env, 'scope> for PromiseFuture<T>
where
  T: 'static + for<'value_env, 'value_scope> FromJs<'value_env, 'value_scope>,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> crate::Result<Self> {
    let promise_object = Promise::<T>::from_js(scope, value)?;
    Self::from_promise(promise_object)
  }
}

impl<T> future::Future for PromiseFuture<T>
where
  T: 'static + for<'env, 'scope> FromJs<'env, 'scope>,
{
  type Output = Result<T>;

  fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
    match self.value.as_mut().poll(cx) {
      Poll::Pending => Poll::Pending,
      Poll::Ready(v) => Poll::Ready(
        v.map_err(|e| Error::new(Status::GenericFailure, format!("{e}")))
          .and_then(identity),
      ),
    }
  }
}
