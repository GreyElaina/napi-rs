use std::collections::{BTreeSet, HashSet};
use std::hash::{BuildHasher, Hash};
use std::ptr;

#[cfg(feature = "object_indexmap")]
use indexmap::IndexSet;

use crate::bindgen_prelude::*;

impl<V: TypeName, S> TypeName for HashSet<V, S> {
  fn type_name() -> &'static str {
    "HashSet"
  }

  fn value_type() -> ValueType {
    ValueType::Object
  }

  fn ts_type() -> String {
    format!("Array<{}>", V::ts_type())
  }
}

impl<'scope, V, S> IntoJs<'scope> for HashSet<V, S>
where
  V: IntoJs<'scope> + 'scope,
{
  type Output = Object<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    set_from_iter(scope, self)
  }
}

impl<'env, 'scope, V, S> FromJs<'env, 'scope> for HashSet<V, S>
where
  V: FromJs<'env, 'scope> + PartialEq + Eq + Hash,
  S: Default + BuildHasher,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    set_to_hash_collection(scope, value.raw())
  }
}

impl<V: TypeName> TypeName for BTreeSet<V> {
  fn type_name() -> &'static str {
    "BTreeSet"
  }

  fn value_type() -> ValueType {
    ValueType::Object
  }

  fn ts_type() -> String {
    format!("Array<{}>", V::ts_type())
  }
}

impl<'scope, V> IntoJs<'scope> for BTreeSet<V>
where
  V: IntoJs<'scope> + 'scope,
{
  type Output = Object<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    set_from_iter(scope, self)
  }
}

impl<'env, 'scope, V> FromJs<'env, 'scope> for BTreeSet<V>
where
  V: FromJs<'env, 'scope> + Ord,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    set_to_ordered_collection(scope, value.raw())
  }
}

#[cfg(feature = "object_indexmap")]
impl<V: TypeName, S> TypeName for IndexSet<V, S> {
  fn type_name() -> &'static str {
    "IndexSet"
  }

  fn value_type() -> ValueType {
    ValueType::Object
  }

  fn ts_type() -> String {
    format!("Array<{}>", V::ts_type())
  }
}
#[cfg(feature = "object_indexmap")]
#[cfg(feature = "object_indexmap")]
impl<'scope, V, S> IntoJs<'scope> for IndexSet<V, S>
where
  V: IntoJs<'scope> + 'scope,
{
  type Output = Object<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    set_from_iter(scope, self)
  }
}
#[cfg(feature = "object_indexmap")]
#[cfg(feature = "object_indexmap")]
impl<'env, 'scope, V, S> FromJs<'env, 'scope> for IndexSet<V, S>
where
  V: FromJs<'env, 'scope> + PartialEq + Eq + Hash,
  S: Default + BuildHasher,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    set_to_hash_collection(scope, value.raw())
  }
}

fn set_to_hash_collection<'env, 'scope, V, C>(
  scope: &mut Scope<'env, 'scope>,
  raw_set: sys::napi_value,
) -> Result<C>
where
  V: FromJs<'env, 'scope> + PartialEq + Eq + Hash,
  C: Default + Extend<V>,
{
  let mut collection = C::default();
  extend_from_set_iterator::<V, C>(scope, raw_set, &mut collection)?;
  Ok(collection)
}

fn set_to_ordered_collection<'env, 'scope, V, C>(
  scope: &mut Scope<'env, 'scope>,
  raw_set: sys::napi_value,
) -> Result<C>
where
  V: FromJs<'env, 'scope> + Ord,
  C: Default + Extend<V>,
{
  let mut collection = C::default();
  extend_from_set_iterator::<V, C>(scope, raw_set, &mut collection)?;
  Ok(collection)
}

fn extend_from_set_iterator<'env, 'scope, V, C>(
  scope: &mut Scope<'env, 'scope>,
  raw_set: sys::napi_value,
  collection: &mut C,
) -> Result<()>
where
  V: FromJs<'env, 'scope>,
  C: Extend<V>,
{
  let env = scope.env().raw();
  let mut values_fn = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_get_named_property(env, raw_set, c"values".as_ptr(), &mut values_fn) },
    "Get Set values method failed"
  )?;
  let mut iterator = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_call_function(env, raw_set, values_fn, 0, ptr::null(), &mut iterator) },
    "Call Set values method failed"
  )?;
  let mut next_fn = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_get_named_property(env, iterator, c"next".as_ptr(), &mut next_fn) },
    "Get Set iterator next method failed"
  )?;

  loop {
    let mut iteration = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_call_function(env, iterator, next_fn, 0, ptr::null(), &mut iteration) },
      "Call Set iterator next method failed"
    )?;
    let iteration = unsafe { Object::from_raw(env, iteration) };
    let done = scope
      .get_optional_named_property::<bool, _>(&iteration, "done")?
      .ok_or_else(|| {
        Error::new(
          Status::InvalidArg,
          "Set iterator result is missing `done`".to_owned(),
        )
      })?;
    if done {
      return Ok(());
    }
    let value = scope
      .get_optional_named_property::<V, _>(&iteration, "value")?
      .ok_or_else(|| {
        Error::new(
          Status::InvalidArg,
          "Set iterator result is missing `value`".to_owned(),
        )
      })?;
    collection.extend([value]);
  }
}

fn set_from_iter<'scope, V>(
  scope: &mut Scope<'_, 'scope>,
  values: impl IntoIterator<Item = V>,
) -> Result<Local<'scope, Object<'scope>>>
where
  V: IntoJs<'scope> + 'scope,
{
  let env = scope.env().raw();
  let obj = scope.env().get_global()?;
  let set_class: Function<'scope, (), ()> = scope.get_named_property(&obj, "Set")?;
  let values = values.into_iter().collect::<Vec<_>>().into_js(scope)?;
  let mut args = [values.raw()];
  let mut raw_set = std::ptr::null_mut();
  check_status!(
    unsafe {
      sys::napi_new_instance(
        env,
        set_class.value,
        args.len(),
        args.as_mut_ptr(),
        &mut raw_set,
      )
    },
    "Create Set instance failed"
  )?;
  Ok(unsafe { Local::from_raw(raw_set) })
}
