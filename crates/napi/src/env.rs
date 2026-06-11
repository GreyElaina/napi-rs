#![allow(deprecated)]

use std::convert::TryInto;
use std::marker::PhantomData;
use std::os::raw::c_char;
#[cfg(feature = "napi5")]
use std::os::raw::c_void;
use std::ptr;

#[cfg(feature = "serde-json")]
use serde::de::DeserializeOwned;
#[cfg(feature = "serde-json")]
use serde::Serialize;

#[cfg(feature = "napi5")]
use crate::bindgen_runtime::IntoJs;
#[cfg(feature = "serde-json")]
use crate::bindgen_runtime::Local;
#[cfg(feature = "napi5")]
use crate::bindgen_runtime::{CallbackDecoder, FromJs};
use crate::bindgen_runtime::{Function, Object, Unknown};
#[cfg(feature = "napi5")]
use crate::bindgen_runtime::{FunctionCallContext, JsArgSlice};
#[cfg(feature = "serde-json")]
use crate::js_values::{De, Ser};
#[cfg(feature = "napi5")]
use crate::JsError;
use crate::{check_status, js_values::*, sys, Error, NodeVersion, Result, Status, ValueType};

pub type Callback = unsafe extern "C" fn(sys::napi_env, sys::napi_callback_info) -> sys::napi_value;

pub(crate) static EMPTY_VEC: Vec<u8> = vec![];

mod cleanup;
mod error;
mod finalizer;
mod promise;
#[cfg(feature = "async")]
mod runtime;
#[cfg(feature = "napi3")]
pub use cleanup::CleanupEnvHook;
pub use finalizer::noop_finalize;
pub(crate) use finalizer::{raw_finalize, raw_finalize_with_custom_callback};
#[cfg(feature = "async")]
pub(crate) use runtime::{AsyncChannel, AsyncDriver, AsyncKeepAlive};

#[derive(Clone, Copy)]
/// `Env` is used to represent a context that the underlying N-API implementation can use to persist VM-specific state.
///
/// Specifically, the same `Env` that was passed in when the initial native function was called must be passed to any subsequent nested N-API calls.
///
/// Caching the `Env` for the purpose of general reuse, and passing the `Env` between instances of the same addon running on different Worker threads is not allowed.
///
/// The `Env` becomes invalid when an instance of a native addon is unloaded.
///
/// Notification of this event is delivered through the callbacks given to `Env::add_env_cleanup_hook` and `Env::set_instance_data`.
pub struct Env<'env>(pub(crate) sys::napi_env, PhantomData<&'env mut ()>);

impl<'env> Env<'env> {
  #[allow(clippy::missing_safety_doc)]
  pub unsafe fn from_raw(env: sys::napi_env) -> Self {
    Self(env, PhantomData)
  }

  /// Create a new JavaScript number from a Rust `i32`
  pub fn create_int32(&self, int: i32) -> Result<JsNumber<'_>> {
    let mut raw_value = ptr::null_mut();
    check_status!(unsafe {
      sys::napi_create_int32(self.0, int, (&mut raw_value) as *mut sys::napi_value)
    })?;
    Ok(JsNumber::from_raw(self.0, raw_value))
  }

  /// Create a new JavaScript number from a Rust `i64`
  pub fn create_int64(&self, int: i64) -> Result<JsNumber<'_>> {
    let mut raw_value = ptr::null_mut();
    check_status!(unsafe {
      sys::napi_create_int64(self.0, int, (&mut raw_value) as *mut sys::napi_value)
    })?;
    Ok(JsNumber::from_raw(self.0, raw_value))
  }

  /// Create a new JavaScript number from a Rust `u32`
  pub fn create_uint32(&self, number: u32) -> Result<JsNumber<'_>> {
    let mut raw_value = ptr::null_mut();
    check_status!(unsafe { sys::napi_create_uint32(self.0, number, &mut raw_value) })?;
    Ok(JsNumber::from_raw(self.0, raw_value))
  }

  /// Create a new JavaScript number from a Rust `f64`
  pub fn create_double(&self, double: f64) -> Result<JsNumber<'_>> {
    let mut raw_value = ptr::null_mut();
    check_status!(unsafe {
      sys::napi_create_double(self.0, double, (&mut raw_value) as *mut sys::napi_value)
    })?;
    Ok(JsNumber::from_raw(self.0, raw_value))
  }

  /// This API creates a new JavaScript string from a Rust type that can be converted to a `&str`
  pub fn create_string<S: AsRef<str>>(&self, s: S) -> Result<JsString<'_>> {
    let s = s.as_ref();
    unsafe { self.create_string_from_c_char(s.as_ptr().cast(), s.len() as isize) }
  }

  /// This API creates a new JavaScript string from a Rust `String`
  pub fn create_string_from_std(&self, s: String) -> Result<JsString<'_>> {
    unsafe { self.create_string_from_c_char(s.as_ptr().cast(), s.len() as isize) }
  }

  /// This API is used for C ffi scenario.
  /// Convert raw *const c_char into JsString
  ///
  /// # Safety
  ///
  /// The caller must guarantee that the `data_ptr` is a valid pointer to either:
  /// - a valid utf-8 string with length of `len`
  /// - a valid utf-8 string terminated by a null character when [crate::bindgen_runtime::NAPI_AUTO_LENGTH] is passed to `len`
  pub unsafe fn create_string_from_c_char(
    &self,
    data_ptr: *const c_char,
    len: isize,
  ) -> Result<JsString<'_>> {
    let mut raw_value = ptr::null_mut();
    check_status!(unsafe { sys::napi_create_string_utf8(self.0, data_ptr, len, &mut raw_value) })?;
    Ok(JsString::from_raw(self.0, raw_value))
  }

  /// This API creates a new JavaScript string from a Rust type that can be converted to a `&[u16]`
  pub fn create_string_utf16<C: AsRef<[u16]>>(&self, chars: C) -> Result<JsString<'_>> {
    let mut raw_value = ptr::null_mut();
    let chars = chars.as_ref();
    check_status!(unsafe {
      sys::napi_create_string_utf16(self.0, chars.as_ptr(), chars.len() as isize, &mut raw_value)
    })?;
    Ok(JsString::from_raw(self.0, raw_value))
  }

  /// This API creates a new JavaScript string from a Rust type that can be converted to a `&[u8]`
  pub fn create_string_latin1<C: AsRef<[u8]>>(&self, chars: C) -> Result<JsString<'_>> {
    let mut raw_value = ptr::null_mut();
    let chars = chars.as_ref();
    check_status!(unsafe {
      sys::napi_create_string_latin1(
        self.0,
        chars.as_ptr().cast(),
        chars.len() as isize,
        &mut raw_value,
      )
    })?;
    Ok(JsString::from_raw(self.0, raw_value))
  }

  /// This API creates a new JavaScript symbol from a optional description
  pub fn create_symbol(&self, description: Option<&str>) -> Result<JsSymbol<'_>> {
    let mut result = ptr::null_mut();
    check_status!(unsafe {
      sys::napi_create_symbol(
        self.0,
        description
          .and_then(|desc| self.create_string(desc).ok())
          .map(|string| string.0.value)
          .unwrap_or(ptr::null_mut()),
        &mut result,
      )
    })?;
    Ok(JsSymbol(
      Value {
        env: self.0,
        value: result,
        value_type: ValueType::Symbol,
      },
      std::marker::PhantomData,
    ))
  }

  /// This API allows an add-on author to create a function object in native code.
  ///
  /// This is the primary mechanism to allow calling into the add-on's native code from JavaScript.
  ///
  /// The newly created function is not automatically visible from script after this call.
  ///
  /// Instead, a property must be explicitly set on any object that is visible to JavaScript, in order for the function to be accessible from script.
  pub fn create_function<Args, Return>(
    &self,
    name: &str,
    callback: Callback,
  ) -> Result<Function<'_, Args, Return>> {
    let mut raw_result = ptr::null_mut();
    let len = name.len();
    check_status!(unsafe {
      sys::napi_create_function(
        self.0,
        name.as_ptr().cast(),
        len as isize,
        Some(callback),
        ptr::null_mut(),
        &mut raw_result,
      )
    })?;

    Ok(unsafe { Function::<Args, Return>::from_raw(self.0, raw_result) })
  }

  #[cfg(feature = "napi5")]
  pub fn create_function_from_closure<Args, Return, F>(
    &self,
    name: &str,
    callback: F,
  ) -> Result<Function<'_, Args, Return>>
  where
    for<'scope> Return: IntoJs<'scope>,
    F: 'static + Fn(FunctionCallContext) -> Result<Return>,
  {
    let closure_data_ptr = Box::into_raw(Box::new(callback));

    let mut raw_result = ptr::null_mut();
    let len = name.len();
    check_status!(unsafe {
      sys::napi_create_function(
        self.0,
        name.as_ptr().cast(),
        len as isize,
        Some(trampoline::<Return, F>),
        closure_data_ptr.cast(), // We let it borrow the data here
        &mut raw_result,
      )
    })?;

    // Note: based on N-API docs, at this point, we have created an effective
    // `&'static dyn Fn…` in Rust parlance, in that thanks to `Box::into_raw()`
    // we are sure the context won't be freed, and thus the callback may use
    // it to call the actual method thanks to the trampoline…
    // But we thus have a data leak: there is nothing yet responsible for
    // running the `drop(Box::from_raw(…))` cleanup code.
    //
    // To solve that, according to the docs, we need to attach a finalizer:
    check_status!(unsafe {
      sys::napi_add_finalizer(
        self.0,
        raw_result,
        closure_data_ptr.cast(),
        Some(finalize_box_trampoline::<F>),
        ptr::null_mut(),
        ptr::null_mut(),
      )
    })?;

    Ok(unsafe { Function::from_raw(self.0, raw_result) })
  }

  /// Create JavaScript class
  pub fn define_class<Args>(
    &self,
    name: &str,
    constructor_cb: Callback,
    properties: &[Property],
  ) -> Result<Function<'_, Args, Unknown<'_>>> {
    let mut raw_result = ptr::null_mut();
    let raw_properties = properties
      .iter()
      .map(|prop| prop.raw())
      .collect::<Vec<sys::napi_property_descriptor>>();
    check_status!(unsafe {
      sys::napi_define_class(
        self.0,
        name.as_ptr().cast(),
        name.len() as isize,
        Some(constructor_cb),
        ptr::null_mut(),
        raw_properties.len(),
        raw_properties.as_ptr(),
        &mut raw_result,
      )
    })?;

    Ok(unsafe { Function::from_raw(self.0, raw_result) })
  }

  pub fn run_in_scope<T, F>(&self, executor: F) -> Result<T>
  where
    F: FnOnce() -> Result<T>,
  {
    let mut handle_scope = ptr::null_mut();
    check_status!(unsafe { sys::napi_open_handle_scope(self.0, &mut handle_scope) })?;

    let result = executor();

    check_status!(unsafe { sys::napi_close_handle_scope(self.0, handle_scope) })?;
    result
  }

  /// `process.versions.napi`
  pub fn get_napi_version(&self) -> Result<u32> {
    unsafe {
      crate::bindgen_runtime::EnvRecord::enter_scope(self.0, |scope| {
        let global = scope.env().get_global()?;
        let process: Object = scope.get_named_property(&global, "process")?;
        let versions: Object = scope.get_named_property(&process, "versions")?;
        let napi_version: String = scope.get_named_property(&versions, "napi")?;
        napi_version
          .parse()
          .map_err(|e| Error::new(Status::InvalidArg, format!("{e}")))
      })
    }
  }

  #[cfg(feature = "napi2")]
  pub fn get_uv_event_loop(&self) -> Result<*mut sys::uv_loop_s> {
    let mut uv_loop: *mut sys::uv_loop_s = ptr::null_mut();
    check_status!(unsafe { sys::napi_get_uv_event_loop(self.0, &mut uv_loop) })?;
    Ok(uv_loop)
  }

  /// This API does not observe leap seconds; they are ignored, as ECMAScript aligns with POSIX time specification.
  ///
  /// This API allocates a JavaScript Date object.
  ///
  /// JavaScript Date objects are described in [Section 20.3](https://tc39.github.io/ecma262/#sec-date-objects) of the ECMAScript Language Specification.
  #[cfg(feature = "napi5")]
  pub fn create_date(&self, time: f64) -> Result<JsDate<'_>> {
    let mut js_value = ptr::null_mut();
    check_status!(unsafe { sys::napi_create_date(self.0, time, &mut js_value) })?;
    Ok(JsDate::from_raw(self.0, js_value))
  }

  #[cfg(feature = "napi6")]
  /// Associate Rust data with the current agent.
  pub fn set_instance_data<T>(&mut self, native: T) -> Result<()>
  where
    T: 'static,
  {
    self
      .record()
      .with_data_mut(|data| data.user_instance_data_mut().set(native))?
  }

  /// Read Rust data associated with the current agent.
  #[cfg(feature = "napi6")]
  pub fn with_instance_data<T, R>(&self, f: impl FnOnce(&T) -> R) -> Result<Option<R>>
  where
    T: 'static,
  {
    self.record().with_data(|data| {
      data
        .user_instance_data()
        .get::<T>()
        .map(|value| value.map(f))
    })?
  }

  /// Mutate Rust data associated with the current agent.
  #[cfg(feature = "napi6")]
  pub fn with_instance_data_mut<T, R>(&mut self, f: impl FnOnce(&mut T) -> R) -> Result<Option<R>>
  where
    T: 'static,
  {
    self.record().with_data_mut(|data| {
      data
        .user_instance_data_mut()
        .get_mut::<T>()
        .map(|value| value.map(f))
    })?
  }

  #[cfg(feature = "napi9")]
  pub fn symbol_for(&self, description: &str) -> Result<JsSymbol<'_>> {
    let mut result = ptr::null_mut();
    check_status!(unsafe {
      sys::node_api_symbol_for(
        self.0,
        description.as_ptr().cast(),
        description.len() as isize,
        &mut result,
      )
    })?;

    Ok(JsSymbol(
      Value {
        env: self.0,
        value: result,
        value_type: ValueType::Symbol,
      },
      std::marker::PhantomData,
    ))
  }

  #[cfg(feature = "napi9")]
  /// This API retrieves the file path of the currently running JS module as a URL. For a file on
  /// the local file system it will start with `file://`.
  ///
  /// # Errors
  ///
  /// The retrieved string may be empty if the add-on loading process fails to establish the
  /// add-on's file name.
  pub fn get_module_file_name(&self) -> Result<String> {
    let mut char_ptr = ptr::null();
    check_status!(
      unsafe { sys::node_api_get_module_file_name(self.0, &mut char_ptr) },
      "call node_api_get_module_file_name failed"
    )?;
    // SAFETY: This is safe because `char_ptr` is guaranteed to not be `null`, and point to
    // null-terminated string data.
    let module_filename = unsafe { std::ffi::CStr::from_ptr(char_ptr) };

    Ok(module_filename.to_string_lossy().into_owned())
  }

  /// ### Serialize `Rust Struct` into `JavaScript Value`
  ///
  /// ```
  /// #[derive(Serialize, Debug, Deserialize)]
  /// struct AnObject {
  ///     a: u32,
  ///     b: Vec<f64>,
  ///     c: String,
  /// }
  ///
  /// #[napi]
  /// fn serialize(env: Env) -> Result<Unknown> {
  ///     let value = AnyObject { a: 1, b: vec![0.1, 2.22], c: "hello" };
  ///     env.to_js_value(&value)
  /// }
  /// ```
  #[cfg(feature = "serde-json")]
  #[allow(clippy::wrong_self_convention)]
  pub fn to_js_value<'js, T>(&self, node: &T) -> Result<Unknown<'js>>
  where
    T: Serialize,
  {
    let s = Ser(self);
    node
      .serialize(s)
      .map(|v| Unknown(v, std::marker::PhantomData))
  }

  /// ### Deserialize data from `JsValue`
  /// ```
  /// #[derive(Serialize, Debug, Deserialize)]
  /// struct AnObject {
  ///     a: u32,
  ///     b: Vec<f64>,
  ///     c: String,
  /// }
  ///
  /// #[napi]
  /// fn deserialize_from_js(env: Env, arg0: Object) -> Result<()> {
  ///     let de_serialized: AnObject = env.from_js_value(arg0)?;
  ///     ...
  /// }
  ///
  #[cfg(feature = "serde-json")]
  pub fn from_js_value<'v, T, V>(&self, value: V) -> Result<T>
  where
    T: DeserializeOwned,
    V: JsValue<'v>,
  {
    let mut env = *self;
    env.with_scope(|scope| {
      let value = Local::from_value(scope, &value, "from_js_value input")?;
      let mut de = De::new(scope, value);
      T::deserialize(&mut de)
    })
  }

  /// This API represents the invocation of the Strict Equality algorithm as defined in [Section 7.2.14](https://tc39.es/ecma262/#sec-strict-equality-comparison) of the ECMAScript Language Specification.
  pub fn strict_equals<'js, A: JsValue<'js>, B: JsValue<'js>>(&self, a: A, b: B) -> Result<bool> {
    let mut result = false;
    check_status!(unsafe { sys::napi_strict_equals(self.0, a.raw(), b.raw(), &mut result) })?;
    Ok(result)
  }

  pub fn get_node_version(&self) -> Result<NodeVersion> {
    let mut result = ptr::null();
    check_status!(unsafe { sys::napi_get_node_version(self.0, &mut result) })?;
    let version = unsafe { *result };
    version.try_into()
  }

  /// get raw env ptr
  #[doc(hidden)]
  pub(crate) fn raw(&self) -> sys::napi_env {
    self.0
  }

  #[allow(clippy::missing_safety_doc)]
  pub unsafe fn raw_unchecked(&self) -> sys::napi_env {
    self.0
  }
}

#[cfg(feature = "napi5")]
pub(crate) unsafe extern "C" fn trampoline<Return, F: Fn(FunctionCallContext) -> Result<Return>>(
  raw_env: sys::napi_env,
  cb_info: sys::napi_callback_info,
) -> sys::napi_value
where
  for<'scope> Return: IntoJs<'scope>,
{
  unsafe {
    crate::bindgen_runtime::EnvRecord::enter_scope(raw_env, |scope| {
      let mut decoder = CallbackDecoder::<0>::dynamic(*scope.env(), cb_info, 4)?;
      decoder.with_frame(|frame| {
        let closure_data_ptr = frame.raw_data();
        let closure: &F = Box::leak(Box::from_raw(closure_data_ptr.cast()));
        let args = JsArgSlice::new(frame.raw_args());
        let this = frame.raw_this();
        let scope = frame.into_scope();
        let ret = {
          let context = FunctionCallContext {
            args,
            this,
            scope: &mut *scope,
          };
          closure(context)?
        };
        ret.into_js(scope).map(|local| local.raw())
      })
    })
  }
  .unwrap_or_else(|e| {
    unsafe { JsError::from(e).throw_into(raw_env) };
    ptr::null_mut()
  })
}

#[cfg(feature = "napi5")]
pub(crate) unsafe extern "C" fn trampoline_setter<
  V,
  F: Fn(Env, crate::bindgen_runtime::This, V) -> Result<()>,
>(
  raw_env: sys::napi_env,
  cb_info: sys::napi_callback_info,
) -> sys::napi_value
where
  V: for<'env, 'scope> FromJs<'env, 'scope>,
{
  use crate::bindgen_runtime::This;
  unsafe {
    crate::bindgen_runtime::EnvRecord::enter_scope(raw_env, |scope| {
      let mut decoder = CallbackDecoder::<1>::new(*scope.env(), cb_info, Some(1))?;
      decoder.with_frame(|mut frame| {
        let closure_data_ptr = property_closures(frame.raw_data())?.setter_closure;
        let closure: &F = Box::leak(Box::from_raw(closure_data_ptr.cast()));
        let value = frame.arg::<V>(0)?;
        let this = frame.this::<This>()?;
        let scope = frame.scope_mut();
        closure(*scope.env(), this, value).map(|()| std::ptr::null_mut())
      })
    })
  }
  .unwrap_or_else(|e| {
    unsafe { JsError::from(e).throw_into(raw_env) };
    ptr::null_mut()
  })
}

#[cfg(feature = "napi5")]
pub(crate) unsafe extern "C" fn trampoline_getter<
  R,
  F: Fn(Env, crate::bindgen_runtime::This) -> Result<R>,
>(
  raw_env: sys::napi_env,
  cb_info: sys::napi_callback_info,
) -> sys::napi_value
where
  for<'scope> R: IntoJs<'scope>,
{
  unsafe {
    crate::bindgen_runtime::EnvRecord::enter_scope(raw_env, |scope| {
      let mut decoder = CallbackDecoder::<0>::new(*scope.env(), cb_info, None)?;
      decoder.with_frame(|mut frame| {
        let closure_data_ptr = property_closures(frame.raw_data())?.getter_closure;
        let closure: &F = Box::leak(Box::from_raw(closure_data_ptr.cast()));
        let this = frame.this::<crate::bindgen_runtime::This>()?;
        let scope = frame.scope_mut();
        let ret = closure(*scope.env(), this)?;
        frame.return_value(ret)
      })
    })
  }
  .unwrap_or_else(|e| {
    unsafe { JsError::from(e).throw_into(raw_env) };
    ptr::null_mut()
  })
}

#[cfg(feature = "napi5")]
fn property_closures(data: *mut c_void) -> Result<&'static PropertyClosures> {
  unsafe { data.cast::<PropertyClosures>().as_ref() }.ok_or_else(|| {
    Error::new(
      Status::InvalidArg,
      "Property closure data is null".to_owned(),
    )
  })
}

#[cfg(feature = "napi5")]
pub(crate) unsafe extern "C" fn finalize_box_trampoline<F>(
  _raw_env: sys::napi_env,
  closure_data_ptr: *mut c_void,
  _finalize_hint: *mut c_void,
) {
  drop(unsafe { Box::<F>::from_raw(closure_data_ptr.cast()) })
}
