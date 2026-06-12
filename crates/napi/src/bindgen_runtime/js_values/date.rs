use std::{ptr, str::FromStr};

use chrono::{DateTime, Local as ChronoLocal, LocalResult, NaiveDateTime, TimeZone};

use crate::{bindgen_prelude::*, check_status, sys, ValueType};

impl<Tz: TimeZone> TypeName for DateTime<Tz> {
  fn type_name() -> &'static str {
    "DateTime"
  }

  fn value_type() -> ValueType {
    ValueType::Object
  }
}

fn ensure_is_date(env: sys::napi_env, raw: sys::napi_value) -> Result<()> {
  let mut is_date = false;
  check_status!(unsafe { sys::napi_is_date(env, raw, &mut is_date) })?;
  if is_date {
    Ok(())
  } else {
    Err(Error::new(
      Status::InvalidArg,
      "Expected a Date object".to_owned(),
    ))
  }
}

impl<'scope> IntoJs<'scope> for NaiveDateTime {
  type Output = Date<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let mut ptr = std::ptr::null_mut();
    let millis_since_epoch_utc = self.and_utc().timestamp_millis() as f64;

    check_status!(
      unsafe { sys::napi_create_date(env, millis_since_epoch_utc, &mut ptr) },
      "Failed to convert rust type `NaiveDateTime` into napi value",
    )?;

    Ok(unsafe { Local::from_raw(ptr) })
  }
}

impl<'scope, Tz: TimeZone> IntoJs<'scope> for DateTime<Tz> {
  type Output = Date<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let mut ptr = std::ptr::null_mut();
    let millis_since_epoch_utc = self.timestamp_millis() as f64;

    check_status!(
      unsafe { sys::napi_create_date(env, millis_since_epoch_utc, &mut ptr) },
      "Failed to convert rust type `DateTime` into napi value",
    )?;

    Ok(unsafe { Local::from_raw(ptr) })
  }
}

impl<'env, 'scope, Tz> FromJs<'env, 'scope> for DateTime<Tz>
where
  Tz: TimeZone,
  DateTime<Tz>: From<DateTime<ChronoLocal>>,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    ensure_is_date(scope.env().raw(), value.raw())?;
    let mut milliseconds_since_epoch_utc = 0.0;

    check_status!(
      unsafe {
        sys::napi_get_date_value(
          scope.env().raw(),
          value.raw(),
          &mut milliseconds_since_epoch_utc,
        )
      },
      "Failed to convert napi value into rust type `DateTime`",
    )?;

    match ChronoLocal.timestamp_millis_opt(milliseconds_since_epoch_utc as i64) {
      LocalResult::Single(dt) => Ok(dt.into()),
      _ => Err(Error::new(
        Status::DateExpected,
        "Found invalid date".to_owned(),
      )),
    }
  }
}

impl<'env, 'scope> FromJs<'env, 'scope> for crate::JsDate<'scope> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    ensure_is_date(scope.env().raw(), value.raw())?;
    Ok(crate::JsDate(
      crate::Value {
        env: scope.env().raw(),
        value: value.raw(),
        value_type: ValueType::Object,
      },
      std::marker::PhantomData,
    ))
  }
}

impl<'env, 'scope> FromJs<'env, 'scope> for NaiveDateTime {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let env = scope.env().raw();
    let mut to_iso_string = ptr::null_mut();
    check_status!(
      unsafe {
        napi_sys::napi_create_string_utf8(
          env,
          c"toISOString".as_ptr().cast(),
          11,
          &mut to_iso_string,
        )
      },
      "create toISOString JavaScript string failed"
    )?;
    let mut to_iso_string_method = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_property(env, value.raw(), to_iso_string, &mut to_iso_string_method) },
      "get toISOString method failed"
    )?;
    let mut iso_string_value = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_call_function(
          env,
          value.raw(),
          to_iso_string_method,
          0,
          ptr::null(),
          &mut iso_string_value,
        )
      },
      "Call toISOString on Date Object failed"
    )?;

    let string = unsafe { Local::from_raw(iso_string_value) };
    let iso_string = String::from_js(scope, string)?;
    let parse_input = iso_string.strip_suffix('Z').unwrap_or(&iso_string);

    NaiveDateTime::from_str(parse_input).map_err(|err| {
      Error::new(
        Status::InvalidArg,
        format!("Failed to convert napi value into rust type `NaiveDateTime` {err} {iso_string}"),
      )
    })
  }
}
