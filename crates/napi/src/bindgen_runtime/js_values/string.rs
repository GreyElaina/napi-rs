use std::ffi::c_char;
use std::fmt::Display;
use std::ops::Deref;
use std::ptr;

use crate::{bindgen_prelude::*, check_status, check_status_and_type, sys};

impl TypeName for String {
  fn type_name() -> &'static str {
    "String"
  }

  fn value_type() -> ValueType {
    ValueType::String
  }
}


impl<'scope> IntoJs<'scope> for &String {
  type Output = crate::JsString<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let mut ptr = ptr::null_mut();

    check_status!(
      unsafe {
        sys::napi_create_string_utf8(env, self.as_ptr().cast(), self.len() as isize, &mut ptr)
      },
      "Failed to convert rust `String` into napi `string`"
    )?;

    Ok(unsafe { Local::from_raw(ptr) })
  }
}

impl<'scope> IntoJs<'scope> for &mut String {
  type Output = crate::JsString<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    self.as_str().into_js(scope)
  }
}

impl<'scope> IntoJs<'scope> for String {
  type Output = crate::JsString<'scope>;

  #[inline]
  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    #[allow(clippy::needless_borrows_for_generic_args)]
    (&self).into_js(scope)
  }
}

impl<'env, 'scope> FromJs<'env, 'scope> for String {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let env = scope.env().raw();
    let raw = value.raw();
    let mut len = 0;

    check_status_and_type!(
      unsafe { sys::napi_get_value_string_utf8(env, raw, ptr::null_mut(), 0, &mut len) },
      env,
      raw,
      "Failed to convert JavaScript value `{}` into rust type `String`"
    )?;

    len += 1;
    let mut ret: Vec<u8> = vec![0; len];
    let mut written_char_count = 0;

    check_status!(
      unsafe {
        sys::napi_get_value_string_utf8(
          env,
          raw,
          ret.as_mut_ptr().cast(),
          len,
          &mut written_char_count,
        )
      },
      "Failed to convert JavaScript value into rust type `String`"
    )?;

    ret.truncate(written_char_count);
    Ok(unsafe { String::from_utf8_unchecked(ret) })
  }
}

impl<'scope> IntoJs<'scope> for &str {
  type Output = crate::JsString<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let mut ptr = ptr::null_mut();

    check_status!(
      unsafe {
        sys::napi_create_string_utf8(env, self.as_ptr().cast(), self.len() as isize, &mut ptr)
      },
      "Failed to convert rust `&str` into napi `string`"
    )?;

    Ok(unsafe { Local::from_raw(ptr) })
  }
}

#[derive(Debug)]
pub struct Utf16String(Vec<u16>);


impl From<String> for Utf16String {
  fn from(s: String) -> Self {
    Utf16String(s.encode_utf16().collect())
  }
}

impl Display for Utf16String {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", String::from_utf16_lossy(self))
  }
}

impl Deref for Utf16String {
  type Target = [u16];

  fn deref(&self) -> &Self::Target {
    self.0.as_ref()
  }
}

impl TypeName for Utf16String {
  fn type_name() -> &'static str {
    "String(utf16)"
  }

  fn value_type() -> ValueType {
    ValueType::String
  }
}

impl<'env, 'scope> FromJs<'env, 'scope> for Utf16String {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let env = scope.env().raw();
    let mut str_len = 0;

    check_status!(unsafe {
      sys::napi_get_value_string_utf16(env, value.raw(), std::ptr::null_mut(), 0, &mut str_len)
    })?;

    str_len += 1;
    let mut ret = Vec::with_capacity(str_len);

    check_status!(unsafe {
      sys::napi_get_value_string_utf16(env, value.raw(), ret.as_mut_ptr(), str_len, &mut str_len)
    })?;

    unsafe { ret.set_len(str_len) };
    Ok(Utf16String(ret))
  }
}

impl<'scope> IntoJs<'scope> for Utf16String {
  type Output = crate::JsString<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let mut ptr = ptr::null_mut();

    check_status!(
      unsafe {
        sys::napi_create_string_utf16(env, self.0.as_ptr().cast(), self.len() as isize, &mut ptr)
      },
      "Failed to convert napi `string` into rust type `String`"
    )?;

    Ok(unsafe { Local::from_raw(ptr) })
  }
}

#[derive(Debug)]
pub struct Latin1String(Vec<u8>);


impl From<String> for Latin1String {
  fn from(s: String) -> Self {
    Latin1String(s.into_bytes())
  }
}

#[cfg(feature = "latin1")]
impl Display for Latin1String {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let mut dst_slice = vec![0; self.0.len() * 2];
    let written =
      encoding_rs::mem::convert_latin1_to_utf8(self.0.as_slice(), dst_slice.as_mut_slice());
    dst_slice.truncate(written);
    write!(f, "{}", unsafe { String::from_utf8_unchecked(dst_slice) })
  }
}

impl Deref for Latin1String {
  type Target = [u8];

  fn deref(&self) -> &Self::Target {
    self.0.as_slice()
  }
}

impl TypeName for Latin1String {
  fn type_name() -> &'static str {
    "String(latin1)"
  }

  fn value_type() -> ValueType {
    ValueType::String
  }
}

impl<'env, 'scope> FromJs<'env, 'scope> for Latin1String {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let env = scope.env().raw();
    let mut str_len = 0;

    check_status!(unsafe {
      sys::napi_get_value_string_latin1(env, value.raw(), std::ptr::null_mut(), 0, &mut str_len)
    })?;

    str_len += 1;
    let mut ret: Vec<u8> = Vec::with_capacity(str_len);

    check_status!(unsafe {
      sys::napi_get_value_string_latin1(
        env,
        value.raw(),
        ret.as_mut_ptr().cast(),
        str_len,
        &mut str_len,
      )
    })?;

    unsafe { ret.set_len(str_len) };
    Ok(Latin1String(ret))
  }
}

impl<'scope> IntoJs<'scope> for Latin1String {
  type Output = crate::JsString<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let mut ptr = ptr::null_mut();

    check_status!(
      unsafe {
        sys::napi_create_string_latin1(env, self.0.as_ptr().cast(), self.len() as isize, &mut ptr)
      },
      "Failed to convert rust type `String` into napi `latin1 string`"
    )?;

    Ok(unsafe { Local::from_raw(ptr) })
  }
}

pub const NAPI_AUTO_LENGTH: isize = -1;

#[derive(Debug)]
/// A wrapper around the raw c_char pointer to a C string.
///
/// This is useful when you want to return a C string to JavaScript directly via NAPI-RS function without converting it to Rust string or performing any memory allocation.
///
/// The `RawCString` doesn't implement `FromJs`, so you can't convert a JavaScript String to it.
pub struct RawCString {
  length: isize,
  inner: *const c_char,
}

impl RawCString {
  /// Create a new `RawCString` from a raw pointer and length.
  ///
  /// If the inner string is null-terminated, you can pass `` as the length.
  pub fn new(inner: *const c_char, length: isize) -> Self {
    Self { inner, length }
  }
}

impl<'scope> IntoJs<'scope> for RawCString {
  type Output = crate::JsString<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let mut ptr = ptr::null_mut();

    check_status!(
      unsafe { napi_sys::napi_create_string_utf8(env, self.inner, self.length, &mut ptr) },
      "Failed to convert rust `&str` into napi `string`"
    )?;

    Ok(unsafe { Local::from_raw(ptr) })
  }
}
