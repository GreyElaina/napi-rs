use std::any::{type_name, TypeId};
#[cfg(feature = "napi6")]
use std::convert::TryFrom;
use std::ffi::{c_void, CStr, CString};
use std::marker::PhantomData;
use std::ptr;
use std::rc::{Rc, Weak};

use crate::{bindgen_prelude::*, check_status, raw_finalize, sys, Callback, TaggedObject, Value};
#[cfg(feature = "napi5")]
use crate::{Env, PropertyClosures};

pub trait JsObjectValue<'env>: JsValue<'env> {
  /// Set the property value to the `Object`
  fn set_property<'k, 'v, K, V>(&mut self, key: K, value: V) -> Result<()>
  where
    K: JsValue<'k>,
    V: JsValue<'v>,
  {
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_set_property(env, self.value().value, key.raw(), value.raw())
    })
  }

  /// Set the property value to the `Object`
  fn set_named_property<T>(&mut self, name: &str, value: T) -> Result<()>
  where
    for<'scope> T: IntoJs<'scope>,
  {
    let key = CString::new(name)?;
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_set_named_property(env, self.raw(), key.as_ptr(), into_js_raw(env, value)?)
    })
  }

  /// Set the property value to the `Object`, the property name is a `CStr`
  /// This is useful when the property name comes from a `C` library
  fn set_c_named_property<T>(&mut self, name: &CStr, value: T) -> Result<()>
  where
    for<'scope> T: IntoJs<'scope>,
  {
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_set_named_property(env, self.raw(), name.as_ptr(), into_js_raw(env, value)?)
    })
  }

  /// Create a named method on the `Object`
  fn create_named_method<K>(&mut self, name: K, function: Callback) -> Result<()>
  where
    K: AsRef<str>,
  {
    let mut js_function = ptr::null_mut();
    let len = name.as_ref().len();
    let name = CString::new(name.as_ref())?;
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_create_function(
        env,
        name.as_ptr(),
        len as isize,
        Some(function),
        ptr::null_mut(),
        &mut js_function,
      )
    })?;
    check_status!(
      unsafe { sys::napi_set_named_property(env, self.value().value, name.as_ptr(), js_function) },
      "create_named_method error"
    )
  }

  /// Create a named method on the `Object`, the name is a `CStr`
  /// This is useful when the method name comes from a `C` library
  fn create_c_named_method(&mut self, name: &CStr, function: Callback) -> Result<()> {
    let mut js_function = ptr::null_mut();
    let len = name.count_bytes();
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_create_function(
        env,
        name.as_ptr(),
        len as isize,
        Some(function),
        ptr::null_mut(),
        &mut js_function,
      )
    })?;
    check_status!(
      unsafe { sys::napi_set_named_property(env, self.value().value, name.as_ptr(), js_function) },
      "create_named_method error"
    )
  }

  /// Check if the `Object` has the named property
  fn has_named_property<N: AsRef<str>>(&self, name: N) -> Result<bool> {
    let mut result = false;
    let key = CString::new(name.as_ref())?;
    let env = self.value().env;
    check_status!(
      unsafe { sys::napi_has_named_property(env, self.value().value, key.as_ptr(), &mut result) },
      "has_named_property error"
    )?;
    Ok(result)
  }

  /// Check if the `Object` has the named property
  ///
  /// This is useful when the property name comes from a `C` library
  fn has_c_named_property(&self, name: &CStr) -> Result<bool> {
    let mut result = false;
    let env = self.value().env;
    check_status!(
      unsafe { sys::napi_has_named_property(env, self.value().value, name.as_ptr(), &mut result) },
      "has_c_named_property error"
    )?;
    Ok(result)
  }

  /// Delete the property from the `Object`, the property name can be a `JsValue`
  fn delete_property<'s, S>(&mut self, name: S) -> Result<bool>
  where
    S: JsValue<'s>,
  {
    let mut result = false;
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_delete_property(env, self.value().value, name.raw(), &mut result)
    })?;
    Ok(result)
  }

  /// Delete the property from the `Object`
  fn delete_named_property<K: AsRef<str>>(&mut self, name: K) -> Result<bool> {
    let name = name.as_ref();
    let mut result = false;
    let mut js_key = ptr::null_mut();
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_create_string_utf8(env, name.as_ptr().cast(), name.len() as isize, &mut js_key)
    })?;
    check_status!(unsafe {
      sys::napi_delete_property(env, self.value().value, js_key, &mut result)
    })?;
    Ok(result)
  }

  /// Delete the property from the `Object`
  ///
  /// This is useful when the property name comes from a `C` library
  fn delete_c_named_property(&mut self, name: &CStr) -> Result<bool> {
    let mut result = false;
    let mut js_key = ptr::null_mut();
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_create_string_utf8(env, name.as_ptr(), name.count_bytes() as isize, &mut js_key)
    })?;
    check_status!(unsafe {
      sys::napi_delete_property(env, self.value().value, js_key, &mut result)
    })?;
    Ok(result)
  }

  /// Check if the `Object` has the own property
  fn has_own_property(&self, key: &str) -> Result<bool> {
    let mut result = false;
    let mut js_key = ptr::null_mut();
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_create_string_utf8(env, key.as_ptr().cast(), key.len() as isize, &mut js_key)
    })?;
    check_status!(unsafe {
      sys::napi_has_own_property(env, self.value().value, js_key, &mut result)
    })?;
    Ok(result)
  }

  /// Check if the `Object` has the own property
  ///
  /// This is useful when the property name comes from a `C` library
  fn has_c_own_property(&self, key: &CStr) -> Result<bool> {
    let mut result = false;
    let mut js_key = ptr::null_mut();
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_create_string_utf8(env, key.as_ptr(), key.count_bytes() as isize, &mut js_key)
    })?;
    check_status!(unsafe {
      sys::napi_has_own_property(env, self.value().value, js_key, &mut result)
    })?;
    Ok(result)
  }

  /// The same as `has_own_property`, but accepts a `JsValue` as the property name.
  fn has_own_property_js<'k, K>(&self, key: K) -> Result<bool>
  where
    K: JsValue<'k>,
  {
    let mut result = false;
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_has_own_property(env, self.value().value, key.raw(), &mut result)
    })?;
    Ok(result)
  }

  /// This API checks if the Object passed in has the named property.
  fn has_property(&self, name: &str) -> Result<bool> {
    let mut js_key = ptr::null_mut();
    let mut result = false;
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_create_string_utf8(env, name.as_ptr().cast(), name.len() as isize, &mut js_key)
    })?;
    check_status!(unsafe { sys::napi_has_property(env, self.value().value, js_key, &mut result) })?;
    Ok(result)
  }

  /// This API is the same as `has_property`, but accepts a `JsValue` as the property name.
  /// So you can pass the `JsNumber` or `JsSymbol` as the property name.
  fn has_property_js<'k, K>(&self, name: K) -> Result<bool>
  where
    K: JsValue<'k>,
  {
    let mut result = false;
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_has_property(env, self.value().value, name.raw(), &mut result)
    })?;
    Ok(result)
  }

  /// This API returns the names of the enumerable properties of object as an array of strings.
  /// The properties of object whose key is a symbol will not be included.
  fn get_property_names(&self) -> Result<Object<'env>> {
    let mut raw_value = ptr::null_mut();
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_get_property_names(env, self.value().value, &mut raw_value)
    })?;
    Ok(unsafe { Object::from_raw(env, raw_value) })
  }

  #[cfg(feature = "napi6")]
  /// <https://nodejs.org/api/n-api.html#n_api_napi_get_all_property_names>
  /// This API returns an array containing the names of the available properties of this object.
  fn get_all_property_names(
    &self,
    mode: KeyCollectionMode,
    filter: KeyFilter,
    conversion: KeyConversion,
  ) -> Result<Object<'env>> {
    let mut properties_value = ptr::null_mut();
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_get_all_property_names(
        env,
        self.value().value,
        mode.into(),
        filter.into(),
        conversion.into(),
        &mut properties_value,
      )
    })?;
    Ok(unsafe { Object::from_raw(env, properties_value) })
  }

  /// This returns the equivalent of `Object.getPrototypeOf` (which is not the same as the function's prototype property).
  fn get_prototype(&self) -> Result<Unknown<'env>> {
    let mut result = ptr::null_mut();
    let env = self.value().env;
    check_status!(unsafe { sys::napi_get_prototype(env, self.value().value, &mut result) })?;
    Ok(unsafe { Unknown::from_raw_unchecked(env, result) })
  }

  /// Set the element at the given index
  fn set_element<'t, T>(&mut self, index: u32, value: T) -> Result<()>
  where
    T: JsValue<'t>,
  {
    let env = self.value().env;
    check_status!(unsafe { sys::napi_set_element(env, self.value().value, index, value.raw()) })
  }

  /// Check if the `Array` has the element at the given index
  fn has_element(&self, index: u32) -> Result<bool> {
    let mut result = false;
    let env = self.value().env;
    check_status!(unsafe { sys::napi_has_element(env, self.value().value, index, &mut result) })?;
    Ok(result)
  }

  /// Delete the element at the given index
  fn delete_element(&mut self, index: u32) -> Result<bool> {
    let mut result = false;
    let env = self.value().env;
    check_status!(unsafe {
      sys::napi_delete_element(env, self.value().value, index, &mut result)
    })?;
    Ok(result)
  }

  /// This method allows the efficient definition of multiple properties on a given object.
  fn define_properties(&mut self, properties: &[Property]) -> Result<()> {
    let properties_iter = properties.iter().map(|property| property.raw());
    let env = self.value().env;
    #[cfg(feature = "napi5")]
    {
      if !properties.is_empty() {
        let mut closures = properties_iter
          .clone()
          .map(|p| p.data)
          .filter(|data| !data.is_null())
          .collect::<Vec<*mut std::ffi::c_void>>();
        if !closures.is_empty() {
          let finalize_hint = Box::into_raw(Box::new((closures.len(), closures.capacity())));
          check_status!(
            unsafe {
              sys::napi_add_finalizer(
                env,
                self.value().value,
                closures.as_mut_ptr().cast(),
                Some(finalize_closures),
                finalize_hint.cast(),
                ptr::null_mut(),
              )
            },
            "Failed to add finalizer"
          )?;
          std::mem::forget(closures);
        }
      }
    }
    check_status!(unsafe {
      sys::napi_define_properties(
        env,
        self.value().value,
        properties.len(),
        properties_iter
          .collect::<Vec<sys::napi_property_descriptor>>()
          .as_ptr(),
      )
    })
  }

  /// Perform `is_array` check before get the length
  ///
  /// if `Object` is not array, `ArrayExpected` error returned
  fn get_array_length(&self) -> Result<u32> {
    if !(self.is_array()?) {
      return Err(Error::new(
        Status::ArrayExpected,
        "Object is not array".to_owned(),
      ));
    }
    self.get_array_length_unchecked()
  }

  /// use this API if you can ensure this `Object` is `Array`
  fn get_array_length_unchecked(&self) -> Result<u32> {
    let mut length: u32 = 0;
    let env = self.value().env;
    check_status!(unsafe { sys::napi_get_array_length(env, self.value().value, &mut length) })?;
    Ok(length)
  }

  /// Wrap the native value `T` to this `Object`
  /// the `T` will be dropped when this `Object` is finalized
  fn wrap<T: 'static>(&mut self, native_object: T, size_hint: Option<usize>) -> Result<()> {
    let env = self.value().env;
    let value = unsafe { self.raw() };
    check_status!(unsafe {
      sys::napi_wrap(
        env,
        value,
        Box::into_raw(Box::new(TaggedObject::new(native_object))).cast(),
        Some(raw_finalize::<TaggedObject<T>>),
        Box::into_raw(Box::new(size_hint.unwrap_or(0) as i64)).cast(),
        ptr::null_mut(),
      )
    })
  }

  /// Get the wrapped native value from the `Object`
  ///
  /// Return the `InvalidArg` error if the `Object` is not wrapped the `T`
  #[allow(clippy::mut_from_ref)]
  fn unwrap<T: 'static>(&self) -> Result<&mut T> {
    let env = self.value().env;
    let value = unsafe { self.raw() };
    unsafe {
      let mut unknown_tagged_object: *mut c_void = ptr::null_mut();
      check_status!(
        sys::napi_unwrap(env, value, &mut unknown_tagged_object),
        "Failed to unwrap value of the Object"
      )?;

      let type_id = unknown_tagged_object as *const TypeId;
      if *type_id == TypeId::of::<T>() {
        let tagged_object = unknown_tagged_object as *mut TaggedObject<T>;
        (*tagged_object).object.as_mut().ok_or_else(|| {
          Error::new(
            Status::InvalidArg,
            "Invalid argument, nothing attach to js_object".to_owned(),
          )
        })
      } else {
        Err(Error::new(
          Status::InvalidArg,
          format!(
            "Invalid argument, {} on unwrap is not the type of wrapped object",
            type_name::<T>()
          ),
        ))
      }
    }
  }

  /// Remove the wrapped native value from the `Object`
  ///
  /// Return the `InvalidArg` error if the `Object` is not wrapped the `T`
  fn remove_wrapped<T: 'static>(&mut self) -> Result<()> {
    let env = self.value().env;
    let value = unsafe { self.raw() };
    unsafe {
      let mut unknown_tagged_object = ptr::null_mut();
      check_status!(sys::napi_remove_wrap(
        env,
        value,
        &mut unknown_tagged_object,
      ))?;
      let type_id = unknown_tagged_object as *const TypeId;
      if *type_id == TypeId::of::<T>() {
        drop(Box::from_raw(unknown_tagged_object as *mut TaggedObject<T>));
        Ok(())
      } else {
        Err(Error::new(
          Status::InvalidArg,
          format!(
            "Invalid argument, {} on unwrap is not the type of wrapped object",
            type_name::<T>()
          ),
        ))
      }
    }
  }

  #[cfg(feature = "napi5")]
  /// Adds a `finalize_cb` callback which will be called when the JavaScript object in js_object has been garbage-collected.
  ///
  /// This API can be called multiple times on a single JavaScript object.
  fn add_finalizer<T, Hint, F>(
    &mut self,
    native: T,
    finalize_hint: Hint,
    finalize_cb: F,
  ) -> Result<()>
  where
    T: 'static,
    Hint: 'static,
    F: FnOnce(FinalizeContext<T, Hint>) + 'static,
  {
    let mut maybe_ref = ptr::null_mut();
    let env = self.value().env;
    let value = unsafe { self.raw() };
    let wrap_context = Box::leak(Box::new((native, finalize_cb, ptr::null_mut())));
    check_status!(unsafe {
      sys::napi_add_finalizer(
        env,
        value,
        (wrap_context as *mut (T, F, sys::napi_ref)).cast(),
        Some(finalize_callback::<T, Hint, F>),
        Box::into_raw(Box::new(finalize_hint)).cast(),
        &mut maybe_ref, // Note: this does not point to the boxed one…
      )
    })?;
    wrap_context.2 = maybe_ref;
    Ok(())
  }

  #[cfg(feature = "napi8")]
  /// This method freezes a given object.
  /// This prevents new properties from being added to it, existing properties from being removed, prevents changing the enumerability, configurability, or writability of existing properties, and prevents the values of existing properties from being changed.
  /// It also prevents the object's prototype from being changed. This is described in [Section 19.1.2.6](https://tc39.es/ecma262/#sec-object.freeze) of the ECMA-262 specification.
  fn freeze(&mut self) -> Result<()> {
    let env = self.value().env;
    check_status!(unsafe { sys::napi_object_freeze(env, self.value().value) })
  }

  #[cfg(feature = "napi8")]
  /// This method seals a given object. This prevents new properties from being added to it, as well as marking all existing properties as non-configurable.
  /// This is described in [Section 19.1.2.20](https://tc39.es/ecma262/#sec-object.seal) of the ECMA-262 specification.
  fn seal(&mut self) -> Result<()> {
    let env = self.value().env;
    check_status!(unsafe { sys::napi_object_seal(env, self.value().value) })
  }
}

#[derive(Clone, Copy)]
pub struct Object<'env>(pub(crate) Value, pub(crate) PhantomData<&'env ()>);

impl<'env> JsValue<'env> for Object<'env> {
  fn value(&self) -> Value {
    self.0
  }
}

impl<'env> JsObjectValue<'env> for Object<'env> {}

impl TypeName for Object<'_> {
  fn type_name() -> &'static str {
    "Object"
  }

  fn value_type() -> ValueType {
    ValueType::Object
  }
}

impl ValidateNapiValue for Object<'_> {}

impl<'env, 'scope> FromJs<'env, 'scope> for Object<'scope> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    Ok(unsafe { Object::from_raw(scope.env().raw(), value.raw()) })
  }
}

impl<'scope> IntoJs<'scope> for &Object<'_> {
  type Output = Object<'scope>;

  fn into_js(self, _: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    Ok((*self).into_local())
  }
}

impl Object<'_> {
  /// create a new `Object` from raw values
  #[doc(hidden)]
  pub unsafe fn from_raw(env: sys::napi_env, value: sys::napi_value) -> Self {
    Self(
      Value {
        env,
        value,
        value_type: ValueType::Object,
      },
      PhantomData,
    )
  }

  pub(crate) fn into_local<'scope>(self) -> Local<'scope, Object<'scope>> {
    unsafe { Local::from_raw(self.0.value) }
  }

  /// create a new `Object` from a `Env`
  pub fn new(env: &Env) -> Result<Self> {
    let mut ptr = ptr::null_mut();
    unsafe {
      check_status!(
        sys::napi_create_object(env.0, &mut ptr),
        "Failed to create napi Object"
      )?;
    }

    Ok(Self(
      crate::Value {
        env: env.0,
        value: ptr,
        value_type: ValueType::Object,
      },
      PhantomData,
    ))
  }

  /// Set the property value to the `Object`
  pub fn set<K, V>(&mut self, field: K, val: V) -> Result<()>
  where
    K: AsRef<str>,
    for<'scope> V: IntoJs<'scope>,
  {
    unsafe { self.set_inner(field.as_ref(), into_js_raw(self.0.env, val)?) }
  }

  pub(crate) unsafe fn set_inner(&mut self, field: &str, napi_val: sys::napi_value) -> Result<()> {
    let mut property_key = std::ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_create_string_utf8(
          self.0.env,
          field.as_ptr().cast(),
          field.len() as isize,
          &mut property_key,
        )
      },
      "Failed to create property key with `{field}`"
    )?;

    check_status!(
      unsafe { sys::napi_set_property(self.0.env, self.0.value, property_key, napi_val) },
      "Failed to set property with field `{field}`"
    )?;
    Ok(())
  }
}

impl<'scope, const LEAK_CHECK: bool> JsRefTarget<'scope, ObjectRef<LEAK_CHECK>> for &Object<'_> {
  fn create_ref(self, scope: &mut Scope<'_, 'scope>) -> Result<ObjectRef<LEAK_CHECK>> {
    scope.ensure_value_env(self.0.env, "Object")?;
    let mut ref_ = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_reference(scope.env().raw(), self.0.value, 1, &mut ref_) },
      "Failed to create reference"
    )?;
    Ok(ObjectRef {
      inner: ref_,
      record: Rc::downgrade(scope.record()),
      not_send: PhantomData,
    })
  }
}

/// A reference to a JavaScript object.
///
/// You must call the `unref` method to release the reference, or the object under the hood will be leaked forever.
///
/// Set the `LEAK_CHECK` to `false` to disable the leak check during the `Drop`
pub struct ObjectRef<const LEAK_CHECK: bool = true> {
  pub(crate) inner: sys::napi_ref,
  record: Weak<EnvRecord>,
  not_send: PhantomData<Rc<()>>,
}

impl<const LEAK_CHECK: bool> Drop for ObjectRef<LEAK_CHECK> {
  fn drop(&mut self) {
    if LEAK_CHECK && !self.inner.is_null() {
      eprintln!("ObjectRef is not unref, it considered as a memory leak");
    }
  }
}

impl<const LEAK_CHECK: bool> ObjectRef<LEAK_CHECK> {
  /// Get the object from the reference
  pub fn get_value<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
    ensure_object_ref_owner(&self.record, env)?;
    let mut result = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_reference_value(env.0, self.inner, &mut result) },
      "Failed to get reference value"
    )?;
    Ok(unsafe { Object::from_raw(env.0, result) })
  }

  /// Unref the reference
  pub fn unref(mut self, env: &Env) -> Result<()> {
    ensure_object_ref_owner(&self.record, env)?;
    check_status!(
      unsafe { sys::napi_delete_reference(env.0, self.inner) },
      "delete Ref failed"
    )?;
    self.inner = ptr::null_mut();
    Ok(())
  }
}

impl<'env, 'scope, const LEAK_CHECK: bool> FromJs<'env, 'scope> for ObjectRef<LEAK_CHECK> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    let value = Object::from_js(scope, value)?;
    scope.create_ref(&value)
  }
}

impl<'scope, const LEAK_CHECK: bool> IntoJs<'scope> for &ObjectRef<LEAK_CHECK> {
  type Output = Object<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    ensure_object_ref_owner_record(&self.record, scope.record())?;
    let mut result = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_reference_value(env, self.inner, &mut result) },
      "Failed to get reference value"
    )?;
    Ok(unsafe { Local::from_raw(result) })
  }
}

impl<'scope, const LEAK_CHECK: bool> IntoJs<'scope> for ObjectRef<LEAK_CHECK> {
  type Output = Object<'scope>;

  fn into_js(mut self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    ensure_object_ref_owner_record(&self.record, scope.record())?;
    let mut result = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_reference_value(env, self.inner, &mut result) },
      "Failed to get reference value"
    )?;
    check_status!(
      unsafe { sys::napi_delete_reference(env, self.inner) },
      "delete Ref failed"
    )?;
    self.inner = ptr::null_mut();
    drop(self);
    Ok(unsafe { Local::from_raw(result) })
  }
}

fn ensure_object_ref_owner(record: &Weak<EnvRecord>, env: &Env) -> Result<()> {
  let current = env.record();
  ensure_object_ref_owner_record(record, &current)
}

fn ensure_object_ref_owner_record(record: &Weak<EnvRecord>, current: &Rc<EnvRecord>) -> Result<()> {
  let owner = record.upgrade().ok_or_else(|| {
    Error::new(
      Status::InvalidArg,
      "ObjectRef owner environment is no longer available",
    )
  })?;
  if Rc::ptr_eq(&owner, current) {
    Ok(())
  } else {
    Err(Error::new(
      Status::InvalidArg,
      "ObjectRef owner environment does not match the current environment",
    ))
  }
}

#[cfg(feature = "napi5")]
pub struct FinalizeContext<T: 'static, Hint: 'static> {
  pub value: T,
  pub hint: Hint,
}

#[cfg(feature = "napi6")]
pub enum KeyCollectionMode {
  IncludePrototypes,
  OwnOnly,
}

#[cfg(feature = "napi6")]
impl TryFrom<sys::napi_key_collection_mode> for KeyCollectionMode {
  type Error = Error;

  fn try_from(value: sys::napi_key_collection_mode) -> Result<Self> {
    match value {
      sys::KeyCollectionMode::include_prototypes => Ok(Self::IncludePrototypes),
      sys::KeyCollectionMode::own_only => Ok(Self::OwnOnly),
      _ => Err(Error::new(
        crate::Status::InvalidArg,
        format!("Invalid key collection mode: {value}"),
      )),
    }
  }
}

#[cfg(feature = "napi6")]
impl From<KeyCollectionMode> for sys::napi_key_collection_mode {
  fn from(value: KeyCollectionMode) -> Self {
    match value {
      KeyCollectionMode::IncludePrototypes => sys::KeyCollectionMode::include_prototypes,
      KeyCollectionMode::OwnOnly => sys::KeyCollectionMode::own_only,
    }
  }
}

#[cfg(feature = "napi6")]
pub enum KeyFilter {
  AllProperties,
  Writable,
  Enumerable,
  Configurable,
  SkipStrings,
  SkipSymbols,
}

#[cfg(feature = "napi6")]
impl TryFrom<sys::napi_key_filter> for KeyFilter {
  type Error = Error;

  fn try_from(value: sys::napi_key_filter) -> Result<Self> {
    match value {
      sys::KeyFilter::all_properties => Ok(Self::AllProperties),
      sys::KeyFilter::writable => Ok(Self::Writable),
      sys::KeyFilter::enumerable => Ok(Self::Enumerable),
      sys::KeyFilter::configurable => Ok(Self::Configurable),
      sys::KeyFilter::skip_strings => Ok(Self::SkipStrings),
      sys::KeyFilter::skip_symbols => Ok(Self::SkipSymbols),
      _ => Err(Error::new(
        crate::Status::InvalidArg,
        format!("Invalid key filter [{value}]"),
      )),
    }
  }
}

#[cfg(feature = "napi6")]
impl From<KeyFilter> for sys::napi_key_filter {
  fn from(value: KeyFilter) -> Self {
    match value {
      KeyFilter::AllProperties => sys::KeyFilter::all_properties,
      KeyFilter::Writable => sys::KeyFilter::writable,
      KeyFilter::Enumerable => sys::KeyFilter::enumerable,
      KeyFilter::Configurable => sys::KeyFilter::configurable,
      KeyFilter::SkipStrings => sys::KeyFilter::skip_strings,
      KeyFilter::SkipSymbols => sys::KeyFilter::skip_symbols,
    }
  }
}

#[cfg(feature = "napi6")]
pub enum KeyConversion {
  KeepNumbers,
  NumbersToStrings,
}

#[cfg(feature = "napi6")]
impl TryFrom<sys::napi_key_conversion> for KeyConversion {
  type Error = Error;

  fn try_from(value: sys::napi_key_conversion) -> Result<Self> {
    match value {
      sys::KeyConversion::keep_numbers => Ok(Self::KeepNumbers),
      sys::KeyConversion::numbers_to_strings => Ok(Self::NumbersToStrings),
      _ => Err(Error::new(
        crate::Status::InvalidArg,
        format!("Invalid key conversion [{value}]"),
      )),
    }
  }
}

#[cfg(feature = "napi6")]
impl From<KeyConversion> for sys::napi_key_conversion {
  fn from(value: KeyConversion) -> Self {
    match value {
      KeyConversion::KeepNumbers => sys::KeyConversion::keep_numbers,
      KeyConversion::NumbersToStrings => sys::KeyConversion::numbers_to_strings,
    }
  }
}

/// # Safety
///
/// Called by Node when wrapping an object. `finalize_data` and `finalize_hint` must be the
/// pointers previously passed to `napi_wrap`.
#[cfg(feature = "napi5")]
unsafe extern "C" fn finalize_callback<T, Hint, F>(
  raw_env: sys::napi_env,
  finalize_data: *mut c_void,
  finalize_hint: *mut c_void,
) where
  T: 'static,
  Hint: 'static,
  F: FnOnce(FinalizeContext<T, Hint>),
{
  let (value, callback, raw_ref) =
    unsafe { *Box::from_raw(finalize_data as *mut (T, F, sys::napi_ref)) };
  let hint = unsafe { *Box::from_raw(finalize_hint as *mut Hint) };
  crate::run_unwind_boundary("running object finalizer callback", || {
    callback(FinalizeContext { value, hint })
  });
  if !raw_ref.is_null() && !crate::bindgen_runtime::EnvRecord::defer_ref(raw_env, raw_ref) {
    eprintln!("napi-rs: leaked object finalize reference after env teardown");
  }
}

#[cfg(feature = "napi5")]
pub(crate) unsafe extern "C" fn finalize_closures(
  _env: sys::napi_env,
  data: *mut c_void,
  len: *mut c_void,
) {
  let (length, capacity): (usize, usize) = *unsafe { Box::from_raw(len.cast()) };
  let closures: Vec<*mut PropertyClosures> =
    unsafe { Vec::from_raw_parts(data.cast(), length, capacity) };
  for closure_ptr in closures.into_iter() {
    if !closure_ptr.is_null() {
      let closures = unsafe { Box::from_raw(closure_ptr) };
      // Free the actual closure functions using the stored drop functions
      if !closures.getter_closure.is_null() {
        if let Some(drop_fn) = closures.getter_drop_fn {
          unsafe { drop_fn(closures.getter_closure) };
        }
      }
      if !closures.setter_closure.is_null() {
        if let Some(drop_fn) = closures.setter_drop_fn {
          unsafe { drop_fn(closures.setter_closure) };
        }
      }
    }
  }
}
