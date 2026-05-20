use std::convert::TryInto;
use std::ffi::CString;
use std::ptr;

use crate::bindgen_runtime::{into_js_raw, IntoJs, Object};
use crate::{check_status, sys, Error, ExtendedErrorInfo, JsError, Result};

use super::Env;

impl Env<'_> {
  /// This API retrieves a napi_extended_error_info structure with information about the last error that occurred.
  ///
  /// The content of the napi_extended_error_info returned is only valid up until an n-api function is called on the same env.
  ///
  /// Do not rely on the content or format of any of the extended information as it is not subject to SemVer and may change at any time. It is intended only for logging purposes.
  ///
  /// This API can be called even if there is a pending JavaScript exception.
  pub fn get_last_error_info(&self) -> Result<ExtendedErrorInfo> {
    let mut raw_extended_error = ptr::null();
    check_status!(unsafe { sys::napi_get_last_error_info(self.0, &mut raw_extended_error) })?;
    unsafe { ptr::read(raw_extended_error) }.try_into()
  }

  /// Throw any JavaScript value
  pub fn throw<T>(&self, value: T) -> Result<()>
  where
    for<'scope> T: IntoJs<'scope>,
  {
    check_status!(unsafe { sys::napi_throw(self.0, into_js_raw(self.0, value)?,) })
  }

  /// This API throws a JavaScript Error with the text provided.
  pub fn throw_error(&self, msg: &str, code: Option<&str>) -> Result<()> {
    let code = code.and_then(|s| CString::new(s).ok());
    let msg = CString::new(msg)?;
    check_status!(unsafe {
      sys::napi_throw_error(
        self.0,
        code.as_ref().map(|s| s.as_ptr()).unwrap_or(ptr::null_mut()),
        msg.as_ptr(),
      )
    })
  }

  /// This API throws a JavaScript RangeError with the text provided.
  pub fn throw_range_error(&self, msg: &str, code: Option<&str>) -> Result<()> {
    let code = code.and_then(|s| CString::new(s).ok());
    let msg = CString::new(msg)?;
    check_status!(unsafe {
      sys::napi_throw_range_error(
        self.0,
        code.as_ref().map(|s| s.as_ptr()).unwrap_or(ptr::null_mut()),
        msg.as_ptr(),
      )
    })
  }

  /// This API throws a JavaScript TypeError with the text provided.
  pub fn throw_type_error(&self, msg: &str, code: Option<&str>) -> Result<()> {
    let code = code.and_then(|s| CString::new(s).ok());
    let msg = CString::new(msg)?;
    check_status!(unsafe {
      sys::napi_throw_type_error(
        self.0,
        code.as_ref().map(|s| s.as_ptr()).unwrap_or(ptr::null_mut()),
        msg.as_ptr(),
      )
    })
  }

  /// This API throws a JavaScript SyntaxError with the text provided.
  #[cfg(feature = "napi9")]
  pub fn throw_syntax_error<S: AsRef<str>, C: AsRef<str>>(&self, msg: S, code: Option<C>) {
    use crate::check_status_or_throw;

    let code = code.as_ref().map(|c| c.as_ref()).unwrap_or("");
    let c_code = CString::new(code).expect("code must be a valid utf-8 string");
    let code_ptr = c_code.as_ptr();
    let msg: CString = CString::new(msg.as_ref()).expect("msg must be a valid utf-8 string");
    let msg_ptr = msg.as_ptr();
    check_status_or_throw!(
      self.0,
      unsafe { sys::node_api_throw_syntax_error(self.0, code_ptr, msg_ptr,) },
      "Throw syntax error failed"
    );
  }

  #[allow(clippy::expect_fun_call)]
  /// In the event of an unrecoverable error in a native module
  ///
  /// A fatal error can be thrown to immediately terminate the process.
  pub fn fatal_error(self, location: &str, message: &str) {
    let location_len = location.len();
    let message_len = message.len();

    unsafe {
      sys::napi_fatal_error(
        location.as_ptr().cast(),
        location_len as isize,
        message.as_ptr().cast(),
        message_len as isize,
      )
    }
  }

  #[cfg(feature = "napi3")]
  /// Trigger an 'uncaughtException' in JavaScript.
  ///
  /// Useful if an async callback throws an exception with no way to recover.
  pub fn fatal_exception(&self, err: Error) {
    unsafe {
      let js_error = JsError::from(err).into_value(self.0);
      debug_assert!(sys::napi_fatal_exception(self.0, js_error) == sys::Status::napi_ok);
    };
  }

  /// Create a JavaScript error object from `Error`
  pub fn create_error(&self, e: Error) -> Result<Object<'_>> {
    let reason = &e.reason;
    let reason_string = self.create_string(reason.as_str())?;
    let status = self.create_string(e.status.as_ref())?;
    let mut result = ptr::null_mut();
    check_status!(unsafe {
      sys::napi_create_error(self.0, status.0.value, reason_string.0.value, &mut result)
    })?;
    Ok(unsafe { Object::from_raw(self.0, result) })
  }
}
