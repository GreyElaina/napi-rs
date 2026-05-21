use std::ffi::c_void;

pub use callback_info::{
  AsyncArgRefs, CallbackDecoder, CallbackFrame, ConstructorReceiver, FrameObject, FrameScope,
};
pub use env::*;
pub use iterator::Generator;
pub use js_values::*;
pub use module_register::*;

use super::sys;
use crate::Result;

#[cfg(feature = "tokio_rt")]
pub mod async_iterator;
#[cfg(feature = "tokio_rt")]
pub use async_iterator::AsyncGenerator;
pub(crate) mod callback_info;
mod env;
mod error;
pub mod iterator;
pub(crate) mod js_values;
mod module_register;

pub trait ObjectFinalize: Sized {
  fn finalize(self) {}
}

/// # Safety
///
/// called when node buffer is ready for gc
#[doc(hidden)]
pub unsafe extern "C" fn drop_buffer(
  _env: sys::napi_env,
  #[allow(unused)] finalize_data: *mut c_void,
  finalize_hint: *mut c_void,
) {
  #[cfg(all(debug_assertions, not(windows), not(target_family = "wasm")))]
  {
    js_values::BUFFER_DATA.with(|buffer_data| {
      let mut buffer = buffer_data.lock().expect("Unlock Buffer data failed");
      buffer.remove(&(finalize_data as *mut u8));
    });
  }
  unsafe {
    drop(Box::from_raw(finalize_hint as *mut Buffer));
  }
}

/// # Safety
///
/// called when node buffer slice is ready for gc
#[doc(hidden)]
pub unsafe extern "C" fn drop_buffer_slice(
  _env: sys::napi_env,
  finalize_data: *mut c_void,
  finalize_hint: *mut c_void,
) {
  let (len, cap): (usize, usize) = *unsafe { Box::from_raw(finalize_hint.cast()) };
  #[cfg(all(debug_assertions, not(windows), not(target_family = "wasm")))]
  {
    js_values::BUFFER_DATA.with(|buffer_data| {
      let mut buffer = buffer_data.lock().expect("Unlock Buffer data failed");
      buffer.remove(&(finalize_data as *mut u8));
    });
  }
  if finalize_data.is_null() {
    return;
  }
  unsafe {
    drop(Vec::from_raw_parts(finalize_data, len, cap));
  }
}

/// Create an object with properties
///
/// When the `experimental` feature is enabled, uses `napi_create_object_with_properties`
/// which creates the object with all properties in a single optimized call.
/// Otherwise falls back to `napi_create_object` + `napi_define_properties`.
#[doc(hidden)]
#[cfg(not(feature = "noop"))]
#[inline]
pub unsafe fn create_object_with_properties(
  env: sys::napi_env,
  properties: &[sys::napi_property_descriptor],
) -> Result<sys::napi_value> {
  use crate::check_status;

  let mut obj_ptr = std::ptr::null_mut();

  #[cfg(all(
    feature = "experimental",
    feature = "node_version_detect",
    not(target_family = "wasm")
  ))]
  {
    let node_version = NODE_VERSION.get().unwrap();
    if !properties.is_empty()
      && ((node_version.major == 25 && node_version.minor >= 2) || node_version.major > 25)
    {
      // Convert property names from C strings to napi_value
      let mut names: Vec<sys::napi_value> = Vec::with_capacity(properties.len());
      let mut values: Vec<sys::napi_value> = Vec::with_capacity(properties.len());

      for prop in properties {
        let mut name_value = std::ptr::null_mut();
        // utf8name is a null-terminated C string, use -1 to auto-detect length
        check_status!(
          sys::napi_create_string_utf8(env, prop.utf8name, -1, &mut name_value),
          "Failed to create property name string",
        )?;
        names.push(name_value);
        values.push(prop.value);
      }

      let mut result_obj = std::ptr::null_mut();
      check_status!(
        sys::node_api_create_object_with_properties(
          env,
          std::ptr::null_mut(), // prototype_or_null
          names.as_ptr(),
          values.as_ptr(),
          properties.len(),
          &mut result_obj,
        ),
        "Failed to create object with properties",
      )?;
      return Ok(result_obj);
    }
  }

  // Fallback: create object then define properties
  check_status!(
    sys::napi_create_object(env, &mut obj_ptr),
    "Failed to create object",
  )?;

  if !properties.is_empty() {
    check_status!(
      sys::napi_define_properties(env, obj_ptr, properties.len(), properties.as_ptr()),
      "Failed to define properties",
    )?;
  }

  Ok(obj_ptr)
}

#[doc(hidden)]
#[cfg(feature = "noop")]
pub unsafe fn create_object_with_properties(
  _env: sys::napi_env,
  _properties: &[sys::napi_property_descriptor],
) -> Result<sys::napi_value> {
  Ok(std::ptr::null_mut())
}
