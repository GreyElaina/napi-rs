#![deny(clippy::all)]
#![allow(non_upper_case_globals)]

//! High level Node.js [N-API](https://nodejs.org/api/n-api.html) binding
//!
//! **napi-rs** provides minimal overhead to write N-API modules in `Rust`.
//!
//! ## Feature flags
//!
//! ### napi1 ~ napi10
//!
//! Because `Node.js` N-API has versions. So there are feature flags to choose what version of `N-API` you want to build for.
//! For example, if you want build a library which can be used by `node@10.17.0`, you should choose the `napi5` or lower.
//!
//! The details of N-API versions and support matrix: [Node-API version matrix](https://nodejs.org/api/n-api.html#node-api-version-matrix)
//!
//! ### tokio_rt
//! With `tokio_rt` feature, `napi-rs` provides a ***tokio runtime*** in an additional thread.
//! And you can easily run tokio `future` in it and return `promise`.
//!
//! ```
//! use futures::prelude::*;
//! use napi::bindgen_prelude::*;
//! use tokio;
//!
//! #[napi]
//! pub fn tokio_readfile(js_filepath: String) -> Result<Buffer> {
//!     ctx.env().spawn_future_with_callback(
//!         tokio::fs::read(js_filepath)
//!           .map(|v| v.map_err(|e| Error::new(Status::Unknown, format!("failed to read file, {}", e)))),
//!         |_, data| data.into(),
//!     )
//! }
//! ```
//!
//! ### latin1
//!
//! Decode latin1 string from JavaScript using [encoding_rs](https://docs.rs/encoding_rs).
//!
//! With this feature, you can use `JsString.as_latin1_string` function
//!
//! ### serde-json
//!
//! Enable Serialize/Deserialize data cross `JavaScript Object` and `Rust struct`.
//!
//! ```
//! #[derive(Serialize, Debug, Deserialize)]
//! struct AnObject {
//!     a: u32,
//!     b: Vec<f64>,
//!     c: String,
//! }
//!
//! #[napi]
//! fn deserialize_from_js(arg0: Unknown) -> Result<()> {
//!     let de_serialized: AnObject = ctx.env().from_js_value(arg0)?;
//!     ...
//! }
//!
//! #[napi]
//! fn serialize(env: Env) -> Result<JsUnknown> {
//!     let value = AnyObject { a: 1, b: vec![0.1, 2.22], c: "hello" };
//!     env.to_js_value(&value)
//! }
//! ```
//!

#[cfg(all(target_family = "wasm", not(feature = "noop"), feature = "napi3"))]
#[link(wasm_import_module = "napi")]
extern "C" {
  fn napi_add_env_cleanup_hook(
    env: sys::napi_env,
    fun: Option<unsafe extern "C" fn(arg: *mut core::ffi::c_void)>,
    arg: *mut core::ffi::c_void,
  ) -> sys::napi_status;
}

mod bindgen_runtime;
mod blocking_work;
mod env;
mod error;
mod js_values;
mod status;
#[cfg(feature = "napi4")]
pub mod threadsafe_function;
mod value_type;

mod version;

pub use napi_sys as sys;

pub use bindgen_runtime::iterator;
pub use env::*;
pub use error::*;
pub use js_values::*;
pub use status::Status;
pub use value_type::*;
pub use version::NodeVersion;
#[cfg(feature = "serde-json")]
#[macro_use]
extern crate serde;

#[doc(hidden)]
#[macro_export(local_inner_macros)]
macro_rules! type_of {
  ($env:expr, $value:expr) => {{
    let mut value_type = 0;
    #[allow(unused_unsafe)]
    check_status!(unsafe { $crate::sys::napi_typeof($env, $value, &mut value_type) })
      .and_then(|_| Ok($crate::ValueType::from(value_type)))
  }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! assert_type_of {
  ($env: expr, $value:expr, $value_ty: expr) => {
    $crate::type_of!($env, $value).and_then(|received_type| {
      if received_type == $value_ty {
        Ok(())
      } else {
        Err($crate::Error::new(
          $crate::Status::InvalidArg,
          format!(
            "Expect value to be {}, but received {}",
            $value_ty, received_type
          ),
        ))
      }
    })
  };
}

#[macro_export]
macro_rules! napi_ts {
  ($($(#[doc = $doc:literal])* pub type $name:ident = $ts:literal;)*) => {
    $(
      $(#[doc = $doc])*
      #[::napi_derive::napi(ts_type = $ts)]
      pub type $name = ();
    )*
  };
}

pub mod bindgen_prelude {
  #[cfg(feature = "tokio_rt")]
  pub use crate::env::{
    block_on, create_custom_tokio_runtime, shutdown_async_runtime, spawn, spawn_blocking,
    start_async_runtime, within_runtime_if_available,
  };
  pub use crate::{
    assert_type_of, bindgen_runtime::*, check_pending_exception, check_status,
    check_status_or_throw, error, error::*, sys, type_of, JsError, JsValue, Property,
    PropertyAttributes, Result, Status, ValueType,
  };
  #[cfg(feature = "tracing")]
  pub use ::tracing;

  /// If the feature `tokio_rt` has been enabled this will enter the runtime context and
  /// then call the provided closure. Otherwise it will just call the provided closure.
  #[cfg(not(all(feature = "tokio_rt", feature = "napi4")))]
  pub fn within_runtime_if_available<F: FnOnce() -> T, T>(f: F) -> T {
    f()
  }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
  if let Some(message) = payload.downcast_ref::<String>() {
    message.clone()
  } else if let Some(message) = payload.downcast_ref::<&str>() {
    message.to_string()
  } else {
    "panic from Rust code".to_owned()
  }
}

pub(crate) fn catch_unwind_boundary<R>(context: &'static str, f: impl FnOnce() -> R) -> Option<R> {
  match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
    Ok(value) => Some(value),
    Err(payload) => {
      let message = panic_message(payload.as_ref());
      eprintln!("napi-rs: panic while {context}: {message}");
      None
    }
  }
}

pub(crate) fn catch_unwind_result<R>(context: &'static str, f: impl FnOnce() -> R) -> Result<R> {
  match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
    Ok(value) => Ok(value),
    Err(payload) => {
      let message = panic_message(payload.as_ref());
      eprintln!("napi-rs: panic while {context}: {message}");
      Err(Error::new(Status::GenericFailure, message))
    }
  }
}

pub(crate) fn run_unwind_boundary(context: &'static str, f: impl FnOnce()) {
  catch_unwind_boundary(context, f);
}

#[doc(hidden)]
pub mod __private {
  pub use crate::bindgen_runtime::{
    iterator::create_iterator, register_napi_class, register_napi_class_impl,
  };

  #[cfg(feature = "tokio_rt")]
  pub use crate::bindgen_runtime::async_iterator::create_async_iterator;

  pub use crate::bindgen_runtime::AsyncArgRefs;

  use crate::{bindgen_runtime::CallbackFrame, sys, Result};

  #[doc(hidden)]
  pub unsafe fn validate_raw_value_type(
    env: sys::napi_env,
    raw: sys::napi_value,
    expected: crate::ValueType,
  ) -> Result<sys::napi_value> {
    unsafe { crate::bindgen_runtime::validate_raw_value_type(env, raw, expected) }
  }

  #[doc(hidden)]
  pub fn callback_frame_this(frame: &CallbackFrame<'_, '_>) -> sys::napi_value {
    frame.raw_this()
  }

  #[doc(hidden)]
  pub fn callback_frame_env(frame: &CallbackFrame<'_, '_>) -> sys::napi_env {
    frame.raw_env()
  }

  #[doc(hidden)]
  pub fn callback_frame_arg(
    frame: &CallbackFrame<'_, '_>,
    index: usize,
  ) -> Result<sys::napi_value> {
    frame.raw_arg(index)
  }

  #[doc(hidden)]
  pub fn callback_frame_retain_value<const N: usize>(
    frame: &CallbackFrame<'_, '_>,
    refs: &mut AsyncArgRefs<N>,
    raw: sys::napi_value,
  ) -> Result<()> {
    frame.retain_value(refs, raw)
  }

  #[doc(hidden)]
  pub fn callback_frame_validate_value<T: crate::bindgen_runtime::ValidateNapiValue>(
    frame: &CallbackFrame<'_, '_>,
    raw: sys::napi_value,
  ) -> Result<sys::napi_value> {
    frame.validate_value::<T>(raw)
  }

  #[doc(hidden)]
  pub fn callback_frame_assert_value_type(
    frame: &CallbackFrame<'_, '_>,
    raw: sys::napi_value,
    expected: crate::ValueType,
  ) -> Result<()> {
    frame.assert_value_type(raw, expected)
  }

  /// Runtime-owned binding entry for generated ordinary Node callbacks.
  ///
  /// # Safety
  ///
  /// The caller must be the generated ABI callback invoked by Node-API, and `raw_env`
  /// and `callback_info` must be the matching pair provided by that invocation.
  #[doc(hidden)]
  pub unsafe fn __napi_binding_entry<const N: usize>(
    raw_env: sys::napi_env,
    callback_info: sys::napi_callback_info,
    invoke: impl for<'env, 'scope> FnOnce(CallbackFrame<'env, 'scope>) -> Result<sys::napi_value>,
  ) -> sys::napi_value {
    unsafe {
      crate::bindgen_runtime::callback_info::__napi_binding_entry::<N>(
        raw_env,
        callback_info,
        invoke,
      )
    }
  }

  #[doc(hidden)]
  pub unsafe fn __napi_binding_entry_variadic(
    raw_env: sys::napi_env,
    callback_info: sys::napi_callback_info,
    hint: usize,
    invoke: impl for<'env, 'scope> FnOnce(CallbackFrame<'env, 'scope>) -> Result<sys::napi_value>,
  ) -> sys::napi_value {
    unsafe {
      crate::bindgen_runtime::callback_info::__napi_binding_entry_variadic(
        raw_env,
        callback_info,
        hint,
        invoke,
      )
    }
  }

  pub unsafe fn log_js_value<V: AsRef<[sys::napi_value]>>(
    // `info`, `log`, `warning` or `error`
    method: &str,
    env: sys::napi_env,
    values: V,
  ) {
    use std::ffi::CString;
    use std::ptr;

    let mut g = ptr::null_mut();
    unsafe { sys::napi_get_global(env, &mut g) };
    let mut console = ptr::null_mut();
    let console_c_string = CString::new("console").unwrap();
    let method_c_string = CString::new(method).unwrap();
    unsafe { sys::napi_get_named_property(env, g, console_c_string.as_ptr(), &mut console) };
    let mut method_js_fn = ptr::null_mut();
    unsafe {
      sys::napi_get_named_property(env, console, method_c_string.as_ptr(), &mut method_js_fn)
    };
    unsafe {
      sys::napi_call_function(
        env,
        console,
        method_js_fn,
        values.as_ref().len(),
        values.as_ref().as_ptr(),
        ptr::null_mut(),
      )
    };
  }
}

pub extern crate ctor;

#[cfg(feature = "tokio_rt")]
pub extern crate tokio;

#[cfg(feature = "error_anyhow")]
pub extern crate anyhow;

#[cfg(feature = "web_stream")]
pub extern crate futures_core;
#[cfg(feature = "web_stream")]
pub extern crate tokio_stream;
