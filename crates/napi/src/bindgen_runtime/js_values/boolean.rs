use std::marker::PhantomData;

use crate::{bindgen_prelude::*, check_status, sys, ValueType};

#[derive(Clone, Copy)]
pub struct Boolean<'scope>(PhantomData<&'scope ()>);

impl TypeName for bool {
  fn type_name() -> &'static str {
    "bool"
  }

  fn ts_type() -> String {
    "boolean".to_owned()
  }

  fn value_type() -> ValueType {
    ValueType::Boolean
  }
}

impl<'scope> IntoJs<'scope> for bool {
  type Output = Boolean<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let mut ptr = std::ptr::null_mut();

    check_status!(
      unsafe { sys::napi_get_boolean(env, self, &mut ptr) },
      "Failed to convert rust type `bool` into napi value",
    )?;

    Ok(unsafe { Local::from_raw(ptr) })
  }
}

impl<'scope> IntoJs<'scope> for &bool {
  type Output = Boolean<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    self.to_owned().into_js(scope)
  }
}

impl<'scope> IntoJs<'scope> for &mut bool {
  type Output = Boolean<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    self.to_owned().into_js(scope)
  }
}

impl<'env, 'scope> FromJs<'env, 'scope> for bool {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let mut ret = false;
    check_status!(
      unsafe { sys::napi_get_value_bool(scope.env().raw(), value.raw(), &mut ret) },
      "Failed to convert JavaScript value into rust type `bool`",
    )?;
    Ok(ret)
  }
}
