use std::os::raw::c_void;

use crate::sys;

/// This function could be used for `BufferSlice::from_external` when no finalization is needed.
pub fn noop_finalize<Hint>(_hint: Hint) {}

#[cfg_attr(target_family = "wasm", allow(unused_variables))]
pub(crate) unsafe extern "C" fn raw_finalize<T>(
  env: sys::napi_env,
  finalize_data: *mut c_void,
  finalize_hint: *mut c_void,
) {
  let tagged_object = finalize_data as *mut T;
  drop(unsafe { Box::from_raw(tagged_object) });
  #[cfg(not(target_family = "wasm"))]
  if !finalize_hint.is_null() {
    let size_hint = unsafe { *Box::from_raw(finalize_hint as *mut i64) };
    if size_hint != 0 {
      let mut adjusted = 0i64;
      let status = unsafe { sys::napi_adjust_external_memory(env, -size_hint, &mut adjusted) };
      debug_assert!(
        status == sys::Status::napi_ok,
        "Calling napi_adjust_external_memory failed"
      );
    }
  };
}

pub(crate) unsafe extern "C" fn raw_finalize_with_custom_callback<Hint, Finalize>(
  _env: sys::napi_env,
  _finalize_data: *mut c_void,
  finalize_hint: *mut c_void,
) where
  Finalize: FnOnce(Hint),
{
  let (hint, callback) = unsafe { *Box::from_raw(finalize_hint as *mut (Hint, Finalize)) };
  crate::run_unwind_boundary("running custom finalizer callback", || callback(hint));
}
