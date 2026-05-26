use std::{marker::PhantomData, ptr};

use crate::{bindgen_prelude::*, check_status, Value};

#[derive(Clone, Copy)]
pub struct Array<'env> {
  pub(crate) env: sys::napi_env,
  pub(crate) inner: sys::napi_value,
  pub(crate) len: u32,
  _marker: std::marker::PhantomData<&'env ()>,
}

impl<'env> Array<'env> {
  pub(crate) fn new(env: sys::napi_env, len: u32) -> Result<Self> {
    let mut ptr = ptr::null_mut();
    unsafe {
      check_status!(
        sys::napi_create_array_with_length(env, len as usize, &mut ptr),
        "Failed to create napi Array"
      )?;
    }

    Ok(Array {
      env,
      inner: ptr,
      len,
      _marker: std::marker::PhantomData,
    })
  }

  pub fn get_reference<'scope, T: NapiReceiver>(
    &self,
    scope: &mut Scope<'_, 'scope>,
    index: u32,
  ) -> Result<Option<Ref<Class<T>>>> {
    if index >= self.len() {
      return Ok(None);
    }

    let mut ret = ptr::null_mut();
    unsafe {
      check_status!(
        sys::napi_get_element(scope.env().raw(), self.inner, index, &mut ret),
        "Failed to get element with index `{}`",
        index,
      )?;

      Ok(Some(Ref::<Class<T>>::from_object_unchecked(scope, ret)?))
    }
  }

  pub fn set<T>(&mut self, index: u32, val: T) -> Result<()>
  where
    for<'scope> T: IntoJs<'scope>,
  {
    unsafe {
      let napi_val = into_js_raw(self.env, val)?;

      check_status!(
        sys::napi_set_element(self.env, self.inner, index, napi_val),
        "Failed to set element with index `{}`",
        index,
      )?;

      if index >= self.len() {
        self.len = index + 1;
      }

      Ok(())
    }
  }

  pub fn insert<T>(&mut self, val: T) -> Result<()>
  where
    for<'scope> T: IntoJs<'scope>,
  {
    self.set(self.len(), val)?;
    Ok(())
  }

  #[allow(clippy::len_without_is_empty)]
  pub fn len(&self) -> u32 {
    self.len
  }

  pub fn coerce_to_object(self) -> Result<Object<'env>> {
    let mut new_raw_value = ptr::null_mut();
    check_status!(unsafe { sys::napi_coerce_to_object(self.env, self.inner, &mut new_raw_value) })?;
    Ok(Object(
      Value {
        env: self.env,
        value: new_raw_value,
        value_type: ValueType::Object,
      },
      PhantomData,
    ))
  }

  pub(crate) fn into_local<'scope>(self) -> Local<'scope, Array<'scope>> {
    unsafe { Local::from_raw(self.inner) }
  }
}

impl TypeName for Array<'_> {
  fn type_name() -> &'static str {
    "Array"
  }

  fn value_type() -> ValueType {
    ValueType::Object
  }
}

impl<'env> JsValue<'env> for Array<'env> {
  fn value(&self) -> Value {
    Value {
      env: self.env,
      value: self.inner,
      value_type: ValueType::Object,
    }
  }
}

impl<'env> JsObjectValue<'env> for Array<'env> {}

impl<'env, 'scope> FromJs<'env, 'scope> for Array<'scope> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let mut len = 0;

    check_status!(
      unsafe { sys::napi_get_array_length(scope.env().raw(), value.raw(), &mut len) },
      "Failed to get Array length",
    )?;

    Ok(Array {
      inner: value.raw(),
      env: scope.env().raw(),
      len,
      _marker: std::marker::PhantomData,
    })
  }
}

impl<'env> ValidateNapiValue for Array<'env> {
  unsafe fn validate(env: sys::napi_env, napi_val: sys::napi_value) -> Result<sys::napi_value> {
    let mut is_array = false;
    check_status!(
      unsafe { sys::napi_is_array(env, napi_val, &mut is_array) },
      "Failed to check given napi value is array"
    )?;
    if !is_array {
      return Err(Error::new(
        Status::InvalidArg,
        "Expected an array".to_owned(),
      ));
    }
    Ok(ptr::null_mut())
  }
}

impl Array<'_> {
  /// Create `Array` from `Vec<T>`
  pub fn from_vec<T>(env: &Env, value: Vec<T>) -> Result<Self>
  where
    for<'scope> T: IntoJs<'scope>,
  {
    let mut arr = Array::new(env.0, value.len() as u32)?;
    value.into_iter().enumerate().try_for_each(|(index, val)| {
      arr.set(index as u32, val)?;
      Ok::<(), Error>(())
    })?;
    Ok(arr)
  }

  /// Create `Array` from `&Vec<String>`
  pub fn from_ref_vec_string(env: &Env, value: &[String]) -> Result<Self> {
    let mut arr = Array::new(env.0, value.len() as u32)?;
    value.iter().enumerate().try_for_each(|(index, val)| {
      arr.set(index as u32, val.as_str())?;
      Ok::<(), Error>(())
    })?;
    Ok(arr)
  }

  /// Create `Array` from `&Vec<T: Copy + IntoJs>`
  pub fn from_ref_vec<T>(env: &Env, value: &[T]) -> Result<Self>
  where
    T: Copy,
    for<'scope> T: IntoJs<'scope>,
  {
    let mut arr = Array::new(env.0, value.len() as u32)?;
    value.iter().enumerate().try_for_each(|(index, val)| {
      arr.set(index as u32, *val)?;
      Ok::<(), Error>(())
    })?;
    Ok(arr)
  }
}

impl<T> TypeName for Vec<T> {
  fn type_name() -> &'static str {
    "Array<T>"
  }

  fn value_type() -> ValueType {
    ValueType::Object
  }
}

impl<'scope, T, const N: usize> IntoJs<'scope> for [T; N]
where
  T: IntoJs<'scope> + Copy + 'scope,
{
  type Output = Array<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let arr = Array::new(env, self.len() as u32)?;

    for (index, value) in self.into_iter().enumerate() {
      let local = value.into_js(scope)?;
      check_status!(
        unsafe { sys::napi_set_element(env, arr.inner, index as u32, local.raw()) },
        "Failed to set element with index `{}`",
        index,
      )?;
    }

    Ok(arr.into_local())
  }
}

impl<'scope, T, const N: usize> IntoJs<'scope> for &'scope [T; N]
where
  &'scope T: IntoJs<'scope>,
{
  type Output = Array<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let arr = Array::new(env, self.len() as u32)?;

    for (index, value) in self.iter().enumerate() {
      let local = value.into_js(scope)?;
      check_status!(
        unsafe { sys::napi_set_element(env, arr.inner, index as u32, local.raw()) },
        "Failed to set element with index `{}`",
        index,
      )?;
    }

    Ok(arr.into_local())
  }
}

impl<'scope, T> IntoJs<'scope> for Vec<T>
where
  T: IntoJs<'scope> + 'scope,
{
  type Output = Array<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let arr = Array::new(env, self.len() as u32)?;

    for (index, value) in self.into_iter().enumerate() {
      let local = value.into_js(scope)?;
      check_status!(
        unsafe { sys::napi_set_element(env, arr.inner, index as u32, local.raw()) },
        "Failed to set element with index `{}`",
        index,
      )?;
    }

    Ok(arr.into_local())
  }
}

impl<'scope, T> IntoJs<'scope> for &'scope Vec<T>
where
  &'scope T: IntoJs<'scope>,
{
  type Output = Array<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    self.as_slice().into_js(scope)
  }
}

impl<'scope, T> IntoJs<'scope> for &'scope [T]
where
  &'scope T: IntoJs<'scope>,
{
  type Output = Array<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let arr = Array::new(env, self.len() as u32)?;

    for (index, value) in self.iter().enumerate() {
      let local = value.into_js(scope)?;
      check_status!(
        unsafe { sys::napi_set_element(env, arr.inner, index as u32, local.raw()) },
        "Failed to set element with index `{}`",
        index,
      )?;
    }

    Ok(arr.into_local())
  }
}

impl<'scope, T> IntoJs<'scope> for &'scope mut Vec<T>
where
  &'scope T: IntoJs<'scope>,
{
  type Output = Array<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    self.as_slice().into_js(scope)
  }
}

impl<'env, 'scope, T> FromJs<'env, 'scope> for Vec<T>
where
  T: FromJs<'env, 'scope>,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let mut len = 0;
    check_status!(
      unsafe { sys::napi_get_array_length(scope.env().raw(), value.raw(), &mut len) },
      "Failed to get Array length",
    )?;
    let arr = Array {
      inner: value.raw(),
      env: scope.env().raw(),
      len,
      _marker: std::marker::PhantomData,
    };
    let mut vec = Vec::with_capacity(arr.len() as usize);

    for i in 0..arr.len() {
      if let Some(val) = scope.get_optional_element::<T>(&arr, i)? {
        vec.push(val);
      } else {
        return Err(Error::new(
          Status::InvalidArg,
          "Found inconsistent data type in Array<T> when converting to Rust Vec<T>".to_owned(),
        ));
      }
    }

    Ok(vec)
  }
}

impl<T> ValidateNapiValue for Vec<T> {
  unsafe fn validate(env: sys::napi_env, napi_val: sys::napi_value) -> Result<sys::napi_value> {
    let mut is_array = false;
    check_status!(
      unsafe { sys::napi_is_array(env, napi_val, &mut is_array) },
      "Failed to check given napi value is array"
    )?;
    if !is_array {
      return Err(Error::new(
        Status::InvalidArg,
        "Expected an array".to_owned(),
      ));
    }
    Ok(ptr::null_mut())
  }
}

macro_rules! arr_get_js {
  ($arr:expr, $scope:expr, $n:expr, $err:expr) => {
    if let Some(e) = $scope.get_optional_element(&$arr, $n)? {
      e
    } else {
      return $err($n);
    }
  };
}

macro_rules! tuple_from_js {
  ($total:expr, $($n:expr),+,) => {
    fn from_js(scope: &mut Scope<'env, 'scope>, value: Local<'scope, Unknown<'scope>>) -> Result<Self> {
      let arr = Array::from_js(scope, value)?;
      let err = |v| Err(Error::new(
        Status::InvalidArg,
        format!(
          "Found inconsistent data type in Array[{}] when converting to Rust T",
          v
        )
        .to_owned(),
      ));
      if arr.len() < $total {
        return Err(Error::new(
            Status::InvalidArg,
            format!("Array length < {}",$total).to_owned(),
        ));
      }
      Ok(($(arr_get_js!(arr, scope, $n, err),)+))
    }
  }
}

macro_rules! impl_tuple_validate_napi_value {
  ($($ident:ident),+) => {
    impl<$($ident),*> ValidateNapiValue for ($($ident,)*) {}
    impl<$($ident),*> TypeName for ($($ident,)*) {
      fn type_name() -> &'static str {
        concat!("Tuple", "(", $(stringify!($ident), ","),*, ")")
      }
      fn value_type() -> ValueType {
        ValueType::Object
      }
    }
  };
}

macro_rules! impl_from_tuple {
  (
    $($typs:ident),*;
    $($tidents:expr),+;
    $length:expr
  ) => {};
}

macro_rules! impl_from_js_tuple {
  (
    $($typs:ident),*;
    $($tidents:expr),+;
    $length:expr
  ) => {
    impl<'env, 'scope, $($typs),*> FromJs<'env, 'scope> for ($($typs,)*)
      where $($typs: FromJs<'env, 'scope>,)* {
      tuple_from_js!($length, $($tidents,)*);
    }
  };
}

macro_rules! impl_to_tuple {
  (
    $($typs:ident),*;
    $($tidents:expr),+;
    $length:expr
  ) => {
    impl<'scope, $($typs),*> IntoJs<'scope> for ($($typs,)*)
      where $($typs: IntoJs<'scope> + 'scope,)* {
      type Output = Array<'scope>;

      fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
        let env = scope.env().raw();
        let arr = Array::new(env, $length as u32)?;

        #[allow(non_snake_case)]
        let ($($typs,)*) = self;
        let mut i = 0;

        $(
          i += 1;
          let local = $typs.into_js(scope)?;
          check_status!(
            unsafe { sys::napi_set_element(env, arr.inner, i - 1, local.raw()) },
            "Failed to set element with index `{}`",
            i - 1,
          )?;
        )*

        Ok(arr.into_local())
      }
    }
  };
}

macro_rules! impl_tuples {
  (
    ;;$length:expr,
    $shift:expr
  ) => {};
  (
    $typ:ident$(, $($typs:ident),*)?;
    $tident:expr$(, $($tidents:expr),*)?;
    $length:expr,
    $shift:expr
  ) => {
    impl_tuples!(
      $($($typs),*)?;
      $($($tidents),*)?;
      $length - 1,
      $shift + 1
    );
    impl_from_tuple!(
      $typ$(, $($typs),*)?;
      $tident - $shift$(, $($tidents - $shift),*)?;
      $length
    );
    impl_from_js_tuple!(
      $typ$(, $($typs),*)?;
      $tident - $shift$(, $($tidents - $shift),*)?;
      $length
    );
    impl_to_tuple!(
      $typ$(, $($typs),*)?;
      $tident - $shift$(, $($tidents - $shift),*)?;
      $length
    );
    impl_tuple_validate_napi_value!($typ$(, $($typs),*)?);
  };
}

impl_tuples!(
  T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15;
  0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15;
  16, 0
);
