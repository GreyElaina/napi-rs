use std::ffi::CStr;
use std::ptr;

use crate::{bindgen_runtime::Unknown, check_status, check_status_or_throw, sys, Env, JsValue};

use super::{into_js_raw, with_env, CallbackDecoder, CallbackFrame, FromJs, IntoJs, NapiClass};

const GENERATOR_STATE_KEY: &CStr = c"[[GeneratorState]]";

/// Implement a Iterator for the JavaScript Class.
/// This feature is an experimental feature and is not yet stable.
pub trait Generator {
  type Yield: for<'scope> IntoJs<'scope>;
  type Next: for<'env, 'scope> FromJs<'env, 'scope>;
  type Return: for<'env, 'scope> FromJs<'env, 'scope>;

  /// Handle the `Generator.next()`
  /// <https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Generator/next>
  fn next(&mut self, value: Option<Self::Next>) -> Option<Self::Yield>;

  #[allow(unused_variables)]
  /// Implement complete to handle the `Generator.return()`
  /// <https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Generator/return>
  fn complete(&mut self, value: Option<Self::Return>) -> Option<Self::Yield> {
    None
  }

  #[allow(unused_variables)]
  /// Implement catch to handle the `Generator.throw()`
  /// <https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Generator/throw>
  fn catch<'env>(
    &'env mut self,
    env: Env,
    value: Unknown<'env>,
  ) -> Result<Option<Self::Yield>, Unknown<'env>> {
    Err(value)
  }
}

impl<'env, T: Generator + 'env> ScopedGenerator<'env> for T {
  type Yield = T::Yield;
  type Next = T::Next;
  type Return = T::Return;

  fn next(&mut self, _: &'env Env, value: Option<Self::Next>) -> Option<Self::Yield> {
    T::next(self, value)
  }

  fn complete(&mut self, value: Option<Self::Return>) -> Option<Self::Yield> {
    T::complete(self, value)
  }

  fn catch(
    &'env mut self,
    env: &'env Env,
    value: Unknown<'env>,
  ) -> Result<Option<Self::Yield>, Unknown<'env>> {
    T::catch(self, *env, value)
  }
}

pub trait ScopedGenerator<'env> {
  type Yield: for<'scope> IntoJs<'scope> + 'env;
  type Next: for<'value_env, 'value_scope> FromJs<'value_env, 'value_scope>;
  type Return: for<'value_env, 'value_scope> FromJs<'value_env, 'value_scope>;

  /// Handle the `Generator.next()`
  /// <https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Generator/next>
  fn next(&mut self, env: &'env Env, value: Option<Self::Next>) -> Option<Self::Yield>;

  #[allow(unused_variables)]
  /// Implement complete to handle the `Generator.return()`
  /// <https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Generator/return>
  fn complete(&mut self, value: Option<Self::Return>) -> Option<Self::Yield> {
    None
  }

  #[allow(unused_variables)]
  /// Implement catch to handle the `Generator.throw()`
  /// <https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Generator/throw>
  fn catch(
    &'env mut self,
    env: &'env Env,
    value: Unknown<'env>,
  ) -> Result<Option<Self::Yield>, Unknown<'env>> {
    Err(value)
  }
}

#[doc(hidden)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub unsafe fn create_iterator<T: for<'a> ScopedGenerator<'a> + NapiClass + 'static>(
  env: sys::napi_env,
  instance: sys::napi_value,
) {
  let mut global = ptr::null_mut();
  check_status_or_throw!(
    env,
    sys::napi_get_global(env, &mut global),
    "Get global object failed",
  );

  let mut symbol_object = ptr::null_mut();
  check_status_or_throw!(
    env,
    sys::napi_get_named_property(env, global, c"Symbol".as_ptr().cast(), &mut symbol_object),
    "Get global object failed",
  );

  let mut iterator_symbol = ptr::null_mut();
  check_status_or_throw!(
    env,
    sys::napi_get_named_property(
      env,
      symbol_object,
      c"iterator".as_ptr().cast(),
      &mut iterator_symbol,
    ),
    "Get Symbol.iterator failed",
  );

  let mut next_function = ptr::null_mut();
  check_status_or_throw!(
    env,
    sys::napi_create_function(
      env,
      c"next".as_ptr().cast(),
      4,
      Some(generator_next::<T>),
      ptr::null_mut(),
      &mut next_function,
    ),
    "Create next function failed"
  );

  let mut return_function = ptr::null_mut();
  check_status_or_throw!(
    env,
    sys::napi_create_function(
      env,
      c"return".as_ptr().cast(),
      6,
      Some(generator_return::<T>),
      ptr::null_mut(),
      &mut return_function,
    ),
    "Create return function failed"
  );

  let mut throw_function = ptr::null_mut();
  check_status_or_throw!(
    env,
    sys::napi_create_function(
      env,
      c"throw".as_ptr().cast(),
      5,
      Some(generator_throw::<T>),
      ptr::null_mut(),
      &mut throw_function,
    ),
    "Create throw function failed"
  );

  check_status_or_throw!(
    env,
    sys::napi_set_named_property(env, instance, c"next".as_ptr().cast(), next_function,),
    "Set next function on Generator object failed"
  );

  check_status_or_throw!(
    env,
    sys::napi_set_named_property(env, instance, c"return".as_ptr().cast(), return_function),
    "Set return function on Generator object failed"
  );

  check_status_or_throw!(
    env,
    sys::napi_set_named_property(env, instance, c"throw".as_ptr().cast(), throw_function),
    "Set throw function on Generator object failed"
  );

  let mut generator_state = ptr::null_mut();
  check_status_or_throw!(
    env,
    sys::napi_get_boolean(env, false, &mut generator_state),
    "Create generator state failed"
  );

  let properties = [sys::napi_property_descriptor {
    utf8name: GENERATOR_STATE_KEY.as_ptr().cast(),
    name: ptr::null_mut(),
    method: None,
    getter: None,
    setter: None,
    value: generator_state,
    attributes: sys::PropertyAttributes::writable,
    data: ptr::null_mut(),
  }];

  check_status_or_throw!(
    env,
    sys::napi_define_properties(env, instance, 1, properties.as_ptr()),
    "Define properties on Generator object failed"
  );

  let mut generator_function = ptr::null_mut();
  check_status_or_throw!(
    env,
    sys::napi_create_function(
      env,
      c"Iterator".as_ptr().cast(),
      8,
      Some(symbol_generator::<T>),
      ptr::null_mut(),
      &mut generator_function,
    ),
    "Create iterator function failed",
  );

  check_status_or_throw!(
    env,
    sys::napi_set_property(env, instance, iterator_symbol, generator_function),
    "Failed to set Symbol.iterator on class instance",
  );
}

#[doc(hidden)]
pub unsafe extern "C" fn symbol_generator<T: for<'a> ScopedGenerator<'a> + NapiClass + 'static>(
  env: sys::napi_env,
  info: sys::napi_callback_info,
) -> sys::napi_value {
  match unsafe { with_env(env, |env_wrapper| symbol_generator_impl(env_wrapper, info)) } {
    Ok(value) => value,
    Err(e) => {
      unsafe { crate::JsError::from(e).throw_into(env) };
      ptr::null_mut()
    }
  }
}

fn symbol_generator_impl(
  env_wrapper: Env<'_>,
  info: sys::napi_callback_info,
) -> crate::Result<sys::napi_value> {
  let mut decoder = CallbackDecoder::<0>::new(env_wrapper, info, None)?;
  decoder.with_frame(|frame| Ok(frame.raw_this()))
}

unsafe extern "C" fn generator_next<T: for<'a> ScopedGenerator<'a> + NapiClass + 'static>(
  env: sys::napi_env,
  info: sys::napi_callback_info,
) -> sys::napi_value {
  match unsafe {
    with_env(env, |env_wrapper| {
      generator_next_impl::<T>(env_wrapper, info)
    })
  } {
    Ok(value) => value,
    Err(e) => {
      unsafe { crate::JsError::from(e).throw_into(env) };
      ptr::null_mut()
    }
  }
}

fn generator_next_impl<T: for<'a> ScopedGenerator<'a> + NapiClass + 'static>(
  env_wrapper: Env<'_>,
  info: sys::napi_callback_info,
) -> crate::Result<sys::napi_value> {
  let mut decoder = CallbackDecoder::<1>::new(env_wrapper, info, None)?;
  decoder.with_frame(|mut frame| {
    let mut result = GeneratorResult::new(&frame)?;
    let mut completed = result.is_done()?;
    if !completed {
      completed = {
        let input = frame.optional_arg::<T::Next>(0)?;
        let scope = frame.scope_mut();
        let (access, storage) = unsafe { T::validate_raw_object(scope, result.this())? };
        let mut generator =
          unsafe { T::mut_from_validated_object(result.this(), storage, access)? };
        let generator_env = *scope.env();
        let next = <T as ScopedGenerator<'_>>::next(&mut *generator, &generator_env, input);
        if let Some(value) = next {
          result.set_value(value);
          false
        } else {
          true
        }
      };
    }
    result.set_done(completed)?;

    Ok(result.raw())
  })
}

unsafe extern "C" fn generator_return<T: for<'a> ScopedGenerator<'a> + NapiClass + 'static>(
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
      unsafe { crate::JsError::from(e).throw_into(env) };
      ptr::null_mut()
    }
  }
}

fn generator_return_impl<T: for<'a> ScopedGenerator<'a> + NapiClass + 'static>(
  env_wrapper: Env<'_>,
  info: sys::napi_callback_info,
) -> crate::Result<sys::napi_value> {
  let mut decoder = CallbackDecoder::<1>::new(env_wrapper, info, None)?;
  decoder.with_frame(|mut frame| {
    let mut result = GeneratorResult::new(&frame)?;

    let input = frame.optional_arg::<T::Return>(0)?;
    {
      let scope = frame.scope_mut();
      let (access, storage) = unsafe { T::validate_raw_object(scope, result.this())? };
      let mut generator = unsafe { T::mut_from_validated_object(result.this(), storage, access)? };
      generator.complete(input);
    }
    if let Some(value) = frame.optional_arg::<Unknown>(0)? {
      result.set_raw_value(value.value().value)?;
    }
    result.set_done(true)?;

    Ok(result.raw())
  })
}

unsafe extern "C" fn generator_throw<T: for<'a> ScopedGenerator<'a> + NapiClass + 'static>(
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
      unsafe { crate::JsError::from(e).throw_into(env) };
      ptr::null_mut()
    }
  }
}

fn generator_throw_impl<T: for<'a> ScopedGenerator<'a> + NapiClass + 'static>(
  env_wrapper: Env<'_>,
  info: sys::napi_callback_info,
) -> crate::Result<sys::napi_value> {
  let mut decoder = CallbackDecoder::<1>::new(env_wrapper, info, None)?;
  decoder.with_frame(|mut frame| {
    let mut result = GeneratorResult::new(&frame)?;
    let thrown = frame.arg::<Unknown>(0)?;

    let mut thrown_value = ptr::null_mut();
    let catch_result = {
      let scope = frame.scope_mut();
      let (access, storage) = unsafe { T::validate_raw_object(scope, result.this())? };
      let mut generator = unsafe { T::mut_from_validated_object(result.this(), storage, access)? };
      let generator_env = *scope.env();
      let handled = match <T as ScopedGenerator<'_>>::catch(&mut *generator, &generator_env, thrown)
      {
        Err(error) => {
          thrown_value = error.0.value;
          Ok(None)
        }
        Ok(Some(value)) => {
          result.set_value(value);
          Ok(Some(false))
        }
        Ok(None) => Ok(Some(true)),
      };
      handled
    };

    match catch_result {
      Ok(Some(done)) => {
        result.set_done(done)?;
      }
      Ok(None) => {
        result.set_done(true)?;
        if !thrown_value.is_null() {
          let throw_status = unsafe { sys::napi_throw(result.env(), thrown_value) };
          debug_assert!(
            throw_status == sys::Status::napi_ok,
            "Failed to throw error {}",
            crate::Status::from(throw_status)
          );
        }
        return Ok(ptr::null_mut());
      }
      Err(error) => return Err(error),
    }

    Ok(result.raw())
  })
}

struct GeneratorResult {
  env: sys::napi_env,
  this: sys::napi_value,
  raw: sys::napi_value,
}

impl GeneratorResult {
  fn new(frame: &CallbackFrame<'_, '_>) -> crate::Result<Self> {
    let env = frame.raw_env();
    let mut raw = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_object(env, &mut raw) },
      "Failed to create iterator result object",
    )?;
    Ok(Self {
      env,
      this: frame.raw_this(),
      raw,
    })
  }

  fn env(&self) -> sys::napi_env {
    self.env
  }

  fn this(&self) -> sys::napi_value {
    self.this
  }

  fn raw(&self) -> sys::napi_value {
    self.raw
  }

  fn is_done(&self) -> crate::Result<bool> {
    let mut value = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_get_named_property(
          self.env,
          self.this,
          GENERATOR_STATE_KEY.as_ptr().cast(),
          &mut value,
        )
      },
      "Get generator state failed"
    )?;

    let mut done = false;
    check_status!(
      unsafe { sys::napi_get_value_bool(self.env, value, &mut done) },
      "Read generator state failed"
    )?;
    Ok(done)
  }

  fn set_done(&mut self, done: bool) -> crate::Result<()> {
    let mut value = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_boolean(self.env, done, &mut value) },
      "Create generator state failed"
    )?;
    self.set_state(value)?;
    self.set_result_done(value)
  }

  fn set_state(&mut self, value: sys::napi_value) -> crate::Result<()> {
    check_status!(
      unsafe {
        sys::napi_set_named_property(
          self.env,
          self.this,
          GENERATOR_STATE_KEY.as_ptr().cast(),
          value,
        )
      },
      "Set generator state failed"
    )
  }

  fn set_result_done(&mut self, value: sys::napi_value) -> crate::Result<()> {
    check_status!(
      unsafe { sys::napi_set_named_property(self.env, self.raw, c"done".as_ptr().cast(), value) },
      "Set iterator result done failed"
    )
  }

  fn set_raw_value(&mut self, value: sys::napi_value) -> crate::Result<()> {
    check_status!(
      unsafe { sys::napi_set_named_property(self.env, self.raw, c"value".as_ptr().cast(), value) },
      "Failed to set iterator result value",
    )
  }

  fn set_value<V>(&mut self, value: V)
  where
    for<'scope> V: IntoJs<'scope>,
  {
    set_generator_value(self.env, self.raw, value);
  }
}

fn set_generator_value<V>(env: sys::napi_env, result: sys::napi_value, value: V)
where
  for<'scope> V: IntoJs<'scope>,
{
  match unsafe { into_js_raw(env, value) } {
    Ok(val) => {
      check_status_or_throw!(
        env,
        unsafe { sys::napi_set_named_property(env, result, c"value".as_ptr().cast(), val,) },
        "Failed to set iterator result value",
      );
    }
    Err(e) => {
      unsafe {
        sys::napi_throw_error(
          env,
          format!("{}", e.status).as_ptr().cast(),
          e.reason.as_ptr().cast(),
        )
      };
    }
  }
}
