use std::ffi::CStr;
use std::future::Future;
use std::ptr;

use crate::{
  bindgen_runtime::{
    with_env, CallbackDecoder, FromJs, IntoJs, Local, NapiClass, Object, Scope, Unknown,
  },
  check_status, check_status_or_throw, sys, Env, Error, JsError, Status,
};

/// Hidden property name for storing the instance reference in async generators.
/// This prevents premature garbage collection of the instance while the async generator is in use.
/// See: https://github.com/napi-rs/napi-rs/issues/3119
const INSTANCE_REF_KEY: &CStr = c"[[InstanceRef]]";

struct AsyncIteratorResult<T> {
  value: Option<T>,
  done: bool,
}

impl<'scope, T> IntoJs<'scope> for AsyncIteratorResult<T>
where
  for<'js_scope> T: IntoJs<'js_scope>,
{
  type Output = Object<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> crate::Result<Local<'scope, Self::Output>> {
    let mut obj = Object::new(scope.env())?;
    if let Some(value) = self.value {
      obj.set("value", value)?;
    } else {
      obj.set("value", ())?;
    }
    obj.set("done", self.done)?;
    obj.into_js(scope)
  }
}

/// Implement a Iterator for the JavaScript Class.
/// This feature is an experimental feature and is not yet stable.
pub trait AsyncGenerator {
  type Yield: for<'scope> IntoJs<'scope> + Send + 'static;
  type Next: for<'env, 'scope> FromJs<'env, 'scope>;
  type Return: for<'env, 'scope> FromJs<'env, 'scope>;

  /// Handle the `AsyncGenerator.next()`
  /// <https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/AsyncGenerator/next>
  fn next(
    &mut self,
    value: Option<Self::Next>,
  ) -> impl Future<Output = crate::Result<Option<Self::Yield>>> + Send + 'static + use<Self>;

  #[allow(unused_variables)]
  /// Implement complete to handle the `AsyncGenerator.return()`
  /// <https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/AsyncGenerator/return>
  fn complete(
    &mut self,
    value: Option<Self::Return>,
  ) -> impl Future<Output = crate::Result<Option<Self::Yield>>> + Send + 'static + use<Self> {
    async move { Ok(None) }
  }

  #[allow(unused_variables)]
  /// Implement catch to handle the `AsyncGenerator.throw()`
  /// <https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/AsyncGenerator/throw>
  fn catch(
    &mut self,
    env: Env,
    value: Unknown,
  ) -> impl Future<Output = crate::Result<Option<Self::Yield>>> + Send + 'static + use<Self> {
    let err = value.into();
    async move { Err(err) }
  }
}

pub unsafe fn create_async_iterator<T: AsyncGenerator + NapiClass>(
  env: sys::napi_env,
  instance: sys::napi_value,
) {
  let mut global = ptr::null_mut();
  check_status_or_throw!(
    env,
    unsafe { sys::napi_get_global(env, &mut global) },
    "Get global object failed",
  );
  let mut symbol_object = ptr::null_mut();
  check_status_or_throw!(
    env,
    unsafe {
      sys::napi_get_named_property(env, global, c"Symbol".as_ptr().cast(), &mut symbol_object)
    },
    "Get global object failed",
  );
  let mut iterator_symbol = ptr::null_mut();
  check_status_or_throw!(
    env,
    unsafe {
      sys::napi_get_named_property(
        env,
        symbol_object,
        c"asyncIterator".as_ptr().cast(),
        &mut iterator_symbol,
      )
    },
    "Get Symbol.asyncIterator failed",
  );
  let mut generator_function = ptr::null_mut();
  check_status_or_throw!(
    env,
    unsafe {
      sys::napi_create_function(
        env,
        c"AsyncIterator".as_ptr().cast(),
        8,
        Some(symbol_async_generator::<T>),
        ptr::null_mut(),
        &mut generator_function,
      )
    },
    "Create asyncIterator function failed",
  );
  check_status_or_throw!(
    env,
    unsafe { sys::napi_set_property(env, instance, iterator_symbol, generator_function) },
    "Failed to set Symbol.asyncIterator on class instance",
  );
}

#[doc(hidden)]
pub unsafe extern "C" fn symbol_async_generator<T: AsyncGenerator + NapiClass>(
  env: sys::napi_env,
  info: sys::napi_callback_info,
) -> sys::napi_value {
  match unsafe {
    with_env(env, |env_wrapper| {
      symbol_async_generator_impl::<T>(env_wrapper, info)
    })
  } {
    Ok(value) => value,
    Err(e) => {
      unsafe { JsError::from(e).throw_into(env) };
      ptr::null_mut()
    }
  }
}

fn symbol_async_generator_impl<T: AsyncGenerator + NapiClass>(
  env_wrapper: Env<'_>,
  info: sys::napi_callback_info,
) -> crate::Result<sys::napi_value> {
  let mut decoder = CallbackDecoder::<0>::new(env_wrapper, info, None)?;
  decoder.with_frame(|frame| {
    let env = frame.env();
    let this = frame.raw_this();
    let mut generator_object = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_object(env.raw(), &mut generator_object) },
      "Create Generator object failed"
    )?;
    let mut next_function = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_create_function(
          env.raw(),
          c"next".as_ptr().cast(),
          4,
          Some(generator_next::<T>),
          ptr::null_mut(),
          &mut next_function,
        )
      },
      "Create next function failed"
    )?;
    let mut return_function = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_create_function(
          env.raw(),
          c"return".as_ptr().cast(),
          6,
          Some(generator_return::<T>),
          ptr::null_mut(),
          &mut return_function,
        )
      },
      "Create next function failed"
    )?;
    let mut throw_function = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_create_function(
          env.raw(),
          c"throw".as_ptr().cast(),
          5,
          Some(generator_throw::<T>),
          ptr::null_mut(),
          &mut throw_function,
        )
      },
      "Create next function failed"
    )?;

    check_status!(
      unsafe {
        sys::napi_set_named_property(
          env.raw(),
          generator_object,
          c"next".as_ptr().cast(),
          next_function,
        )
      },
      "Set next function on Generator object failed"
    )?;

    check_status!(
      unsafe {
        sys::napi_set_named_property(
          env.raw(),
          generator_object,
          c"return".as_ptr().cast(),
          return_function,
        )
      },
      "Set return function on Generator object failed"
    )?;

    check_status!(
      unsafe {
        sys::napi_set_named_property(
          env.raw(),
          generator_object,
          c"throw".as_ptr().cast(),
          throw_function,
        )
      },
      "Set throw function on Generator object failed"
    )?;

    // The generator object needs to keep the instance alive while iteration is in progress.
    // Without this reference, the instance can be garbage collected while the generator
    // is still being used by the iterator methods.
    let mut instance_ref = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_reference(env.raw(), this, 1, &mut instance_ref) },
      "Failed to create reference to instance in async generator"
    )?;

    // Store the reference as an external value so it can be cleaned up later
    let mut ref_holder = ptr::null_mut();
    unsafe extern "C" fn cleanup_instance_ref(
      env: sys::napi_env,
      data: *mut std::ffi::c_void,
      _hint: *mut std::ffi::c_void,
    ) {
      let instance_ref = data as sys::napi_ref;
      if !instance_ref.is_null() {
        if crate::bindgen_runtime::defer_ref_for_env(env, instance_ref) {
          return;
        }

        if cfg!(debug_assertions) {
          eprintln!("napi-rs: async generator instance reference leaked during env teardown");
        }
      }
    }

    check_status!(
      unsafe {
        sys::napi_create_external(
          env.raw(),
          instance_ref.cast(),
          Some(cleanup_instance_ref),
          ptr::null_mut(),
          &mut ref_holder,
        )
      },
      "Failed to create external for instance reference"
    )?;

    // Store as a hidden property on the generator object
    // Use napi_define_properties with default attributes (non-enumerable, non-writable, non-configurable)
    // to make this property truly hidden from user code
    let properties = [sys::napi_property_descriptor {
      utf8name: INSTANCE_REF_KEY.as_ptr().cast(),
      name: ptr::null_mut(),
      method: None,
      getter: None,
      setter: None,
      value: ref_holder,
      attributes: sys::PropertyAttributes::default,
      data: ptr::null_mut(),
    }];

    check_status!(
      unsafe { sys::napi_define_properties(env.raw(), generator_object, 1, properties.as_ptr()) },
      "Failed to define instance reference property on generator object"
    )?;

    Ok(generator_object)
  })
}

fn async_generator_instance(
  env: sys::napi_env,
  generator_object: sys::napi_value,
) -> crate::Result<sys::napi_value> {
  let mut ref_holder = ptr::null_mut();
  check_status!(
    unsafe {
      sys::napi_get_named_property(
        env,
        generator_object,
        INSTANCE_REF_KEY.as_ptr().cast(),
        &mut ref_holder,
      )
    },
    "Get async generator instance reference holder failed"
  )?;

  let mut data = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_get_value_external(env, ref_holder, &mut data) },
    "Get async generator instance reference failed"
  )?;

  let mut instance = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_get_reference_value(env, data.cast(), &mut instance) },
    "Get async generator instance failed"
  )?;

  if instance.is_null() {
    return Err(Error::new(
      Status::InvalidArg,
      "Async generator instance is no longer available".to_owned(),
    ));
  }

  Ok(instance)
}

unsafe extern "C" fn generator_next<T: AsyncGenerator + NapiClass>(
  env: sys::napi_env,
  info: sys::napi_callback_info,
) -> sys::napi_value {
  match unsafe { with_env(env, |env_wrapper| generator_next_fn::<T>(env_wrapper, info)) } {
    Ok(value) => value,
    Err(e) => unsafe {
      let js_error: JsError = e.into();
      js_error.throw_into(env);
      ptr::null_mut()
    },
  }
}

fn generator_next_fn<T: AsyncGenerator + NapiClass>(
  env_wrapper: Env<'_>,
  info: sys::napi_callback_info,
) -> crate::Result<sys::napi_value> {
  let mut decoder = CallbackDecoder::<1>::new(env_wrapper, info, None)?;
  decoder.with_frame(|mut frame| {
    let env = frame.env();
    let instance = async_generator_instance(env.raw(), frame.raw_this())?;
    let input = frame.optional_arg::<T::Next>(0)?;
    let item = {
      let scope = frame.scope_mut();
      let (access, storage) = unsafe { T::validate_raw_object(scope, instance)? };
      let mut generator = unsafe { T::mut_from_validated_object(instance, storage, access)? };
      <T as AsyncGenerator>::next(&mut *generator, input)
    };

    let promise = env.spawn_future_with_callback(item, |_, value| {
      Ok(AsyncIteratorResult {
        done: value.is_none(),
        value,
      })
    })?;
    Ok(promise.inner)
  })
}

unsafe extern "C" fn generator_return<T: AsyncGenerator + NapiClass>(
  env: sys::napi_env,
  info: sys::napi_callback_info,
) -> sys::napi_value {
  match unsafe {
    with_env(env, |env_wrapper| {
      generator_return_impl::<T>(env_wrapper, info)
    })
  } {
    Ok(value) => value,
    Err(e) => {
      unsafe { JsError::from(e).throw_into(env) };
      ptr::null_mut()
    }
  }
}

fn generator_return_impl<T: AsyncGenerator + NapiClass>(
  env_wrapper: Env<'_>,
  info: sys::napi_callback_info,
) -> crate::Result<sys::napi_value> {
  let mut decoder = CallbackDecoder::<1>::new(env_wrapper, info, None)?;
  decoder.with_frame(|mut frame| {
    let env = frame.env();
    let instance = async_generator_instance(env.raw(), frame.raw_this())?;
    let input = frame.optional_arg::<T::Return>(0)?;
    let item = {
      let scope = frame.scope_mut();
      let (access, storage) = unsafe { T::validate_raw_object(scope, instance)? };
      let mut generator = unsafe { T::mut_from_validated_object(instance, storage, access)? };
      <T as AsyncGenerator>::complete(&mut *generator, input)
    };

    let promise = env.spawn_future_with_callback(item, |_, value| {
      // Per async iterator protocol, return() must ALWAYS set done: true.
      Ok(AsyncIteratorResult { value, done: true })
    })?;
    Ok(promise.inner)
  })
}

unsafe extern "C" fn generator_throw<T: AsyncGenerator + NapiClass>(
  env: sys::napi_env,
  info: sys::napi_callback_info,
) -> sys::napi_value {
  match unsafe {
    with_env(env, |env_wrapper| {
      generator_throw_impl::<T>(env_wrapper, info)
    })
  } {
    Ok(value) => value,
    Err(e) => {
      unsafe { JsError::from(e).throw_into(env) };
      ptr::null_mut()
    }
  }
}

fn generator_throw_impl<T: AsyncGenerator + NapiClass>(
  env_wrapper: Env<'_>,
  info: sys::napi_callback_info,
) -> crate::Result<sys::napi_value> {
  let mut decoder = CallbackDecoder::<1>::new(env_wrapper, info, None)?;
  decoder.with_frame(|mut frame| {
    let env = frame.env();
    let instance = async_generator_instance(env.raw(), frame.raw_this())?;
    let thrown = frame.arg::<Unknown>(0)?;
    let caught = {
      let scope = frame.scope_mut();
      let (access, storage) = unsafe { T::validate_raw_object(scope, instance)? };
      let mut generator = unsafe { T::mut_from_validated_object(instance, storage, access)? };
      <T as AsyncGenerator>::catch(&mut *generator, env, thrown)
    };
    let promise = env.spawn_future_with_callback(caught, |_, value| {
      Ok(AsyncIteratorResult { value, done: false })
    })?;
    Ok(promise.inner)
  })
}
