use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasher, Hash};

#[cfg(feature = "object_indexmap")]
use indexmap::IndexMap;

use crate::bindgen_prelude::*;

impl<K, V, S> TypeName for HashMap<K, V, S> {
  fn type_name() -> &'static str {
    "HashMap"
  }

  fn value_type() -> ValueType {
    ValueType::Object
  }
}

impl<K, V, S> ValidateNapiValue for HashMap<K, V, S> {}

#[cfg(not(feature = "noop"))]
impl<'scope, K, V, S> IntoJs<'scope> for HashMap<K, V, S>
where
  K: AsRef<str>,
  V: IntoJs<'scope> + 'scope,
{
  type Output = Object<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let raw_env = scope.env().raw();
    let obj = Object::new(scope.env())?;
    #[cfg(all(
      feature = "napi10",
      feature = "node_version_detect",
      feature = "dyn-symbols",
      not(feature = "noop"),
    ))]
    let node_version = NODE_VERSION.get().unwrap();
    for (k, v) in self.into_iter() {
      let value = v.into_js(scope)?;
      #[cfg(all(
        feature = "napi10",
        feature = "node_version_detect",
        feature = "dyn-symbols",
        not(feature = "noop"),
      ))]
      {
        if node_version.major >= 20 && node_version.minor >= 18 {
          fast_set_property(raw_env, obj.0.value, k, value.raw())?;
        } else {
          set_property_raw(raw_env, obj.0.value, k.as_ref(), value.raw())?;
        }
      }
      #[cfg(not(all(
        feature = "napi10",
        feature = "node_version_detect",
        feature = "dyn-symbols"
      )))]
      set_property_raw(raw_env, obj.0.value, k.as_ref(), value.raw())?;
    }

    Ok(obj.into_local())
  }
}

#[cfg(feature = "noop")]
impl<'scope, K, V, S> IntoJs<'scope> for HashMap<K, V, S> {
  type Output = Object<'scope>;

  fn into_js(self, _: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    unimplemented!("HashMap is not supported in noop mode");
  }
}

impl<'env, 'scope, K, V, S> FromJs<'env, 'scope> for HashMap<K, V, S>
where
  K: From<String> + Eq + Hash,
  V: FromJs<'env, 'scope>,
  S: Default + BuildHasher,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let obj = unsafe { Object::from_raw(scope.env().raw(), value.raw()) };
    let keys = scope.keys(&obj)?;
    let mut map = HashMap::with_capacity_and_hasher(keys.len(), S::default());
    for key in keys.into_iter() {
      if let Some(val) = scope.get_optional_named_property::<V, _>(&obj, &key)? {
        map.insert(K::from(key), val);
      }
    }

    Ok(map)
  }
}

impl<K, V> TypeName for BTreeMap<K, V> {
  fn type_name() -> &'static str {
    "BTreeMap"
  }

  fn value_type() -> ValueType {
    ValueType::Object
  }
}

impl<K, V> ValidateNapiValue for BTreeMap<K, V> {}

#[cfg(not(feature = "noop"))]
impl<'scope, K, V> IntoJs<'scope> for BTreeMap<K, V>
where
  K: AsRef<str>,
  V: IntoJs<'scope> + 'scope,
{
  type Output = Object<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let raw_env = scope.env().raw();
    let obj = Object::new(scope.env())?;
    #[cfg(all(
      feature = "napi10",
      feature = "node_version_detect",
      feature = "dyn-symbols",
      not(feature = "noop"),
    ))]
    let node_version = NODE_VERSION.get().unwrap();
    for (k, v) in self.into_iter() {
      let value = v.into_js(scope)?;
      #[cfg(all(
        feature = "napi10",
        feature = "node_version_detect",
        feature = "dyn-symbols",
        not(feature = "noop"),
      ))]
      {
        if node_version.major >= 20 && node_version.minor >= 18 {
          fast_set_property(raw_env, obj.0.value, k, value.raw())?;
        } else {
          set_property_raw(raw_env, obj.0.value, k.as_ref(), value.raw())?;
        }
      }
      #[cfg(not(all(
        feature = "napi10",
        feature = "node_version_detect",
        feature = "dyn-symbols"
      )))]
      set_property_raw(raw_env, obj.0.value, k.as_ref(), value.raw())?;
    }

    Ok(obj.into_local())
  }
}

#[cfg(feature = "noop")]
impl<'scope, K, V> IntoJs<'scope> for BTreeMap<K, V> {
  type Output = Object<'scope>;

  fn into_js(self, _: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    unimplemented!("BTreeMap is not supported in noop mode");
  }
}

impl<'env, 'scope, K, V> FromJs<'env, 'scope> for BTreeMap<K, V>
where
  K: From<String> + Ord,
  V: FromJs<'env, 'scope>,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let obj = unsafe { Object::from_raw(scope.env().raw(), value.raw()) };
    let keys = scope.keys(&obj)?;
    let mut map = BTreeMap::new();
    for key in keys.into_iter() {
      if let Some(val) = scope.get_optional_named_property::<V, _>(&obj, &key)? {
        map.insert(K::from(key), val);
      }
    }

    Ok(map)
  }
}

#[cfg(feature = "object_indexmap")]
impl<K, V, S> TypeName for IndexMap<K, V, S> {
  fn type_name() -> &'static str {
    "IndexMap"
  }

  fn value_type() -> ValueType {
    ValueType::Object
  }
}

#[cfg(feature = "object_indexmap")]
impl<K, V, S> ValidateNapiValue for IndexMap<K, V, S> {}

#[cfg(all(feature = "object_indexmap", not(feature = "noop")))]
impl<'scope, K, V, S> IntoJs<'scope> for IndexMap<K, V, S>
where
  K: AsRef<str>,
  V: IntoJs<'scope> + 'scope,
  S: Default + BuildHasher,
{
  type Output = Object<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let raw_env = scope.env().raw();
    let obj = Object::new(scope.env())?;
    #[cfg(all(
      feature = "napi10",
      feature = "node_version_detect",
      feature = "dyn-symbols",
      not(feature = "noop"),
    ))]
    let node_version = NODE_VERSION.get().unwrap();
    for (k, v) in self.into_iter() {
      let value = v.into_js(scope)?;
      #[cfg(all(
        feature = "napi10",
        feature = "node_version_detect",
        feature = "dyn-symbols",
        not(feature = "noop"),
      ))]
      {
        if node_version.major >= 20 && node_version.minor >= 18 {
          fast_set_property(raw_env, obj.0.value, k, value.raw())?;
        } else {
          set_property_raw(raw_env, obj.0.value, k.as_ref(), value.raw())?;
        }
      }
      #[cfg(not(all(
        feature = "experimental",
        feature = "node_version_detect",
        feature = "dyn-symbols"
      )))]
      set_property_raw(raw_env, obj.0.value, k.as_ref(), value.raw())?;
    }

    Ok(obj.into_local())
  }
}

#[cfg(all(feature = "object_indexmap", feature = "noop"))]
impl<'scope, K, V, S> IntoJs<'scope> for IndexMap<K, V, S> {
  type Output = Object<'scope>;

  fn into_js(self, _: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    unimplemented!("IndexMap is not supported in noop mode");
  }
}

#[cfg(feature = "object_indexmap")]
impl<'env, 'scope, K, V, S> FromJs<'env, 'scope> for IndexMap<K, V, S>
where
  K: From<String> + Hash + Eq,
  V: FromJs<'env, 'scope>,
  S: Default + BuildHasher,
{
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let obj = unsafe { Object::from_raw(scope.env().raw(), value.raw()) };
    let mut map = IndexMap::default();
    for key in scope.keys(&obj)?.into_iter() {
      if let Some(val) = scope.get_optional_named_property::<V, _>(&obj, &key)? {
        map.insert(K::from(key), val);
      }
    }

    Ok(map)
  }
}

fn set_property_raw(
  raw_env: sys::napi_env,
  obj: sys::napi_value,
  key: &str,
  value: sys::napi_value,
) -> Result<()> {
  let key = std::ffi::CString::new(key)?;
  check_status!(
    unsafe { sys::napi_set_named_property(raw_env, obj, key.as_ptr(), value) },
    "Failed to set property"
  )
}

#[cfg(all(
  feature = "napi10",
  feature = "node_version_detect",
  feature = "dyn-symbols",
  not(feature = "noop"),
))]
fn fast_set_property<K: AsRef<str>>(
  raw_env: sys::napi_env,
  obj: sys::napi_value,
  k: K,
  value: sys::napi_value,
) -> Result<()> {
  let mut property_key = std::ptr::null_mut();
  check_status!(
    unsafe {
      sys::node_api_create_property_key_utf8(
        raw_env,
        k.as_ref().as_ptr().cast(),
        k.as_ref().len() as isize,
        &mut property_key,
      )
    },
    "Create property key failed"
  )?;
  check_status!(
    unsafe { sys::napi_set_property(raw_env, obj, property_key, value) },
    "Failed to set property"
  )?;
  Ok(())
}
