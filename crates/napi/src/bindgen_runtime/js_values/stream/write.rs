use std::marker::PhantomData;

use crate::{
  bindgen_prelude::{
    FnArgs, FromJs, Function, JsObjectValue, Local, Object, Promise, Scope, TypeName, Unknown,
  },
  bindgen_runtime::EnvRecord,
  sys, JsValue, Result, Value, ValueType,
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

  fn ts_type() -> String {
    "WritableStream".to_owned()
  }

  fn value_type() -> ValueType {
    ValueType::Object
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
      EnvRecord::enter_scope(self.env, |scope| {
        let stream = Local::from_value(scope, self, "WriteableStream")?;
        let stream_object = Object::from_js(scope, stream)?;
        f(scope, stream, stream_object)
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
