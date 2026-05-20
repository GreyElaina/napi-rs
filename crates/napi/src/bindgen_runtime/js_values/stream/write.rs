use std::{marker::PhantomData, ptr};

use crate::{
  bindgen_prelude::{
    FnArgs, FromJs, Function, JsObjectValue, Local, Object, Promise, Scope, TypeName, Unknown,
    ValidateNapiValue,
  },
  bindgen_runtime::with_env,
  check_status, sys, Error, JsValue, Result, Status, Value, ValueType,
};

pub struct WriteableStream<'env> {
  pub(crate) value: sys::napi_value,
  pub(crate) env: sys::napi_env,
  pub(crate) _scope: &'env PhantomData<()>,
}

impl<'env> JsValue<'env> for WriteableStream<'env> {
  fn value(&self) -> Value {
    Value {
      env: self.env,
      value: self.value,
      value_type: ValueType::Object,
    }
  }
}

impl<'env> JsObjectValue<'env> for WriteableStream<'env> {}

impl TypeName for WriteableStream<'_> {
  fn type_name() -> &'static str {
    "WriteableStream"
  }

  fn value_type() -> ValueType {
    ValueType::Object
  }
}

impl ValidateNapiValue for WriteableStream<'_> {
  unsafe fn validate(
    env: napi_sys::napi_env,
    napi_val: napi_sys::napi_value,
  ) -> Result<napi_sys::napi_value> {
    unsafe {
      with_env(env, |mut env_wrapper| {
        env_wrapper.with_scope(|scope| {
          let global = scope.env().get_global()?;
          let constructor: Function<'_, (), ()> =
            scope.get_named_property(&global, "WritableStream")?;
          let mut is_instance = false;
          check_status!(
            sys::napi_instanceof(env, napi_val, constructor.value, &mut is_instance),
            "Check WritableStream instance failed"
          )?;
          if !is_instance {
            return Err(Error::new(
              Status::InvalidArg,
              "Value is not a WritableStream",
            ));
          }
          Ok(ptr::null_mut())
        })
      })
    }
  }
}

impl<'env, 'scope> FromJs<'env, 'scope> for WriteableStream<'scope> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(Self {
      value: value.raw(),
      env: scope.env().raw(),
      _scope: &PhantomData,
    })
  }
}

impl WriteableStream<'_> {
  fn with_stream_object<R>(
    &self,
    f: impl for<'env, 'scope> FnOnce(
      &mut Scope<'env, 'scope>,
      Local<'scope, Unknown<'scope>>,
      Object<'scope>,
    ) -> Result<R>,
  ) -> Result<R> {
    unsafe {
      with_env(self.env, |mut env| {
        env.with_scope(|scope| {
          let stream = Local::from_value(scope, self, "WriteableStream")?;
          let stream_object = Object::from_js(scope, stream)?;
          f(scope, stream, stream_object)
        })
      })
    }
  }

  pub fn ready(&self) -> Result<Promise<'_, ()>> {
    let promise = self.with_stream_object(|scope, _, stream| {
      let promise: Promise<'_, ()> = scope.get_named_property(&stream, "ready")?;
      Ok(promise.value().value)
    })?;
    Ok(unsafe { Promise::from_raw(self.env, promise) })
  }

  /// The `abort()` method of the `WritableStream` interface aborts the stream,
  /// signaling that the producer can no longer successfully write to the stream and it is to be immediately moved to an error state,
  /// with any queued writes discarded.
  pub fn abort(&mut self, reason: String) -> Result<Promise<'_, ()>> {
    let promise = self.with_stream_object(|scope, stream, stream_object| {
      let abort: Function<'_, FnArgs<(String,)>, Promise<'_, ()>> =
        scope.get_named_property(&stream_object, "abort")?;
      let promise = scope.apply(&abort, stream, FnArgs::from((reason,)))?;
      Ok(promise.value().value)
    })?;
    Ok(unsafe { Promise::from_raw(self.env, promise) })
  }

  /// The `close()` method of the `WritableStream` interface closes the associated stream.
  ///
  /// All chunks written before this method is called are sent before the returned promise is fulfilled.
  pub fn close(&mut self) -> Result<Promise<'_, ()>> {
    let promise = self.with_stream_object(|scope, stream, stream_object| {
      let close: Function<'_, (), Promise<'_, ()>> =
        scope.get_named_property(&stream_object, "close")?;
      let promise = scope.apply(&close, stream, ())?;
      Ok(promise.value().value)
    })?;
    Ok(unsafe { Promise::from_raw(self.env, promise) })
  }
}
