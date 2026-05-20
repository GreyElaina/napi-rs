use crate::{bindgen_prelude::*, check_status, sys, type_of, Error, Result};

macro_rules! impl_number_conversions {
  ( $( ($name:literal, $t:ty as $st:ty, $get:ident, $create:ident) ,)* ) => {
    $(
      impl $crate::bindgen_prelude::TypeName for $t {
        fn type_name() -> &'static str {
          $name
        }

        fn value_type() -> crate::ValueType {
          crate::ValueType::Number
        }
      }

      impl $crate::bindgen_prelude::ValidateNapiValue for $t { }

      impl<'scope> IntoJs<'scope> for $t {
        type Output = crate::JsNumber<'scope>;

        fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
          let env = scope.env().raw();
          let mut ptr = std::ptr::null_mut();
          let val: $st = self.into();

          check_status!(
            unsafe { sys::$create(env, val, &mut ptr) },
            "Failed to convert rust type `{}` into napi value",
            $name,
          )?;

          Ok(unsafe { Local::from_raw(ptr) })
        }
      }

      impl<'scope> IntoJs<'scope> for &$t {
        type Output = crate::JsNumber<'scope>;

        fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
          self.to_owned().into_js(scope)
        }
      }

      impl<'scope> IntoJs<'scope> for &mut $t {
        type Output = crate::JsNumber<'scope>;

        fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
          self.to_owned().into_js(scope)
        }
      }

      impl<'env, 'scope> FromJs<'env, 'scope> for $t {
        fn from_js(
          scope: &mut Scope<'env, 'scope>,
          value: Local<'scope, Unknown<'scope>>,
        ) -> Result<Self> {
          let env = scope.env().raw();
          let raw = value.raw();
          let mut ret = 0 as $st;

          check_status!(
            unsafe { sys::$get(env, raw, &mut ret) },
            "Failed to convert JavaScript value {:?} into rust type `{}`",
            type_of!(env, raw)?,
            $name,
          )?;

          ret.try_into().map_err(|_| Error::from_reason(concat!("Failed to convert ", stringify!($st), " to ", stringify!($t))))
        }
      }
    )*
  };
}

impl_number_conversions!(
  ("u8", u8 as u32, napi_get_value_uint32, napi_create_uint32),
  ("i8", i8 as i32, napi_get_value_int32, napi_create_int32),
  ("u16", u16 as u32, napi_get_value_uint32, napi_create_uint32),
  ("i16", i16 as i32, napi_get_value_int32, napi_create_int32),
  ("u32", u32 as u32, napi_get_value_uint32, napi_create_uint32),
  ("i32", i32 as i32, napi_get_value_int32, napi_create_int32),
  ("i64", i64 as i64, napi_get_value_int64, napi_create_int64),
  ("f64", f64 as f64, napi_get_value_double, napi_create_double),
);

impl<'env, 'scope> FromJs<'env, 'scope> for crate::JsNumber<'scope> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(crate::JsNumber(
      crate::Value {
        env: scope.env().raw(),
        value: value.raw(),
        value_type: crate::ValueType::Number,
      },
      std::marker::PhantomData,
    ))
  }
}

impl<'scope> IntoJs<'scope> for f32 {
  type Output = crate::JsNumber<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let mut ptr = std::ptr::null_mut();

    check_status!(
      unsafe { sys::napi_create_double(env, self.into(), &mut ptr) },
      "Failed to convert rust type `f32` into napi value",
    )?;

    Ok(unsafe { Local::from_raw(ptr) })
  }
}
