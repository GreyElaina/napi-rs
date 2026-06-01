#[cfg(all(feature = "tokio_rt", feature = "napi4"))]
use std::future::Future;

#[cfg(feature = "napi4")]
use crate::bindgen_runtime::IntoJs;
#[cfg(feature = "napi4")]
use crate::bindgen_runtime::Promise;
#[cfg(feature = "napi4")]
use crate::bindgen_runtime::Scope;
#[cfg(all(feature = "tokio_rt", feature = "napi4"))]
use crate::js_values::EnvFinalizeCallback;
#[cfg(feature = "napi4")]
use crate::JsDeferred;
#[cfg(feature = "napi4")]
use crate::Result;

#[cfg(all(feature = "tokio_rt", feature = "napi4"))]
use super::runtime;
use super::Env;

impl<'env> Env<'env> {
  #[cfg(all(feature = "tokio_rt", feature = "napi4", feature = "noop"))]
  fn spawn_future_with_completion<Data, PromiseValue, Fut, Complete>(
    &self,
    _future: Fut,
    _complete: Complete,
    _finalize: Option<EnvFinalizeCallback>,
  ) -> Result<Promise<'env, PromiseValue>>
  where
    Data: 'static + Send,
    PromiseValue: 'static,
    for<'scope> PromiseValue: IntoJs<'scope>,
    Fut: 'static + Send + Future<Output = Result<Data>>,
    Complete: 'static
      + Send
      + for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>, Data) -> Result<PromiseValue>,
  {
    Ok(unsafe { Promise::from_raw(self.0, std::ptr::null_mut()) })
  }

  #[cfg(all(feature = "tokio_rt", feature = "napi4", not(feature = "noop")))]
  fn spawn_future_with_completion<Data, PromiseValue, Fut, Complete>(
    &self,
    future: Fut,
    complete: Complete,
    finalize: Option<EnvFinalizeCallback>,
  ) -> Result<Promise<'env, PromiseValue>>
  where
    Data: 'static + Send,
    PromiseValue: 'static,
    for<'scope> PromiseValue: IntoJs<'scope>,
    Fut: 'static + Send + Future<Output = Result<Data>>,
    Complete: 'static
      + Send
      + for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>, Data) -> Result<PromiseValue>,
  {
    let (mut deferred, promise) = JsDeferred::new(self)?;
    deferred.set_finalize_callback(finalize);
    let deferred_for_panic = deferred.clone();

    let inner = async move {
      match future.await {
        Ok(value) => deferred.resolve(move |scope| complete(scope, value)),
        Err(error) => deferred.reject(error.into()),
      }
    };

    let join_handle = runtime::spawn(inner);

    runtime::spawn(async move {
      if let Err(error) = join_handle.await {
        if let Ok(reason) = error.try_into_panic() {
          if let Some(message) = reason.downcast_ref::<&str>() {
            deferred_for_panic.reject(crate::Error::new(crate::Status::GenericFailure, message));
          } else {
            deferred_for_panic.reject(crate::Error::new(
              crate::Status::GenericFailure,
              "Panic in async function",
            ));
          }
        }
      }
    });

    Ok(unsafe { Promise::from_raw(self.0, promise.0.value) })
  }

  #[cfg(all(feature = "tokio_rt", feature = "napi4"))]
  /// Spawn a future, return a JavaScript Promise which takes the result of the future
  pub fn spawn_future<T, F>(&self, fut: F) -> Result<Promise<'_, T>>
  where
    T: 'static + Send,
    F: 'static + Send + Future<Output = Result<T>>,
    for<'scope> T: IntoJs<'scope>,
  {
    self.spawn_future_with_completion(fut, |_, value| Ok(value), None)
  }

  #[cfg(all(feature = "tokio_rt", feature = "napi4"))]
  /// Spawn a future with a callback
  /// So you can access the `Env` and resolved value after the future completed
  pub fn spawn_future_with_callback<
    T: 'static + Send,
    V: 'static,
    F: 'static + Send + Future<Output = Result<T>>,
    R: 'static + Send + for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>, T) -> Result<V>,
  >(
    &self,
    fut: F,
    callback: R,
  ) -> Result<Promise<'env, V>>
  where
    for<'scope> V: IntoJs<'scope>,
  {
    self.spawn_future_with_completion(fut, callback, None)
  }

  #[cfg(all(feature = "tokio_rt", feature = "napi4"))]
  #[doc(hidden)]
  pub fn spawn_future_with_callback_and_finalize<
    T: 'static + Send,
    V: 'static,
    F: 'static + Send + Future<Output = Result<T>>,
    R: 'static + Send + for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>, T) -> Result<V>,
  >(
    &self,
    fut: F,
    callback: R,
    finalize: Box<dyn for<'callback_env> FnOnce(Env<'callback_env>) + Send>,
  ) -> Result<Promise<'env, V>>
  where
    for<'scope> V: IntoJs<'scope>,
  {
    self.spawn_future_with_completion(fut, callback, Some(finalize))
  }

  /// Creates a deferred promise, which can be resolved or rejected from a background thread.
  #[cfg(feature = "napi4")]
  pub fn create_deferred<Data, Resolver>(
    &self,
  ) -> Result<(JsDeferred<Data, Resolver>, Promise<'env, Data>)>
  where
    Data: 'static,
    for<'scope> Data: IntoJs<'scope>,
    Resolver: for<'callback, 'scope> FnOnce(&mut Scope<'callback, 'scope>) -> Result<Data> + Send,
  {
    let (deferred, promise) = JsDeferred::new(self)?;
    Ok((deferred, unsafe {
      Promise::from_raw(self.0, promise.0.value)
    }))
  }
}
