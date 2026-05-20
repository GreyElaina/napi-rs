use std::{
  any::Any,
  cell::{Cell, RefCell},
  collections::HashMap,
  ffi::{c_void, CStr, CString},
  marker::PhantomData,
  ptr,
  rc::Rc,
};

use crate::{
  catch_unwind_boundary, catch_unwind_result, check_status, run_unwind_boundary, sys, Error,
  JsGlobal, JsValue, Result, Status, ValueType,
};

use super::{
  create_object_with_properties, Array, ClassKey, FromJs, Function, IntoJs, JsRefTarget, Object,
  Unknown, ValidateNapiValue,
};

pub use crate::Env;

#[doc(hidden)]
pub type ClassConstructorStore = HashMap<ClassKey, sys::napi_ref>;

#[doc(hidden)]
pub struct UserInstanceData {
  value: Option<Box<dyn Any>>,
}

#[doc(hidden)]
pub struct EnvRecord {
  data: RefCell<EnvData>,
  deferred_refs: DeferredRefs,
}

#[doc(hidden)]
pub struct EnvData {
  constructors: ClassConstructorStore,
  user_instance_data: UserInstanceData,
}

#[doc(hidden)]
pub struct DeferredRefs {
  refs: Cell<Vec<sys::napi_ref>>,
}

pub struct Scope<'env, 'scope> {
  env: &'scope mut Env<'env>,
  record: Option<&'scope Rc<EnvRecord>>,
  marker: PhantomData<&'scope mut ()>,
}

#[derive(Clone, Copy)]
pub struct Local<'scope, T> {
  raw: sys::napi_value,
  marker: PhantomData<&'scope T>,
}

thread_local! {
  static ENV_RECORDS: RefCell<HashMap<usize, Rc<EnvRecord>>> = RefCell::new(HashMap::new());
}

#[doc(hidden)]
pub fn env_record(raw: sys::napi_env) -> Rc<EnvRecord> {
  ENV_RECORDS.with(|records| {
    let mut records = records.borrow_mut();
    records
      .entry(raw as usize)
      .or_insert_with(|| {
        let record = Rc::new(EnvRecord::new());
        install_env_record_holder(raw, raw as usize)
          .expect("Install napi-rs EnvRecord holder failed");
        record
      })
      .clone()
  })
}

#[doc(hidden)]
pub fn defer_ref_for_env(raw: sys::napi_env, reference: sys::napi_ref) -> bool {
  ENV_RECORDS.with(|records| {
    let records = records.borrow();
    let Some(record) = records.get(&(raw as usize)) else {
      return false;
    };
    record.deferred_refs().push(reference);
    true
  })
}

fn install_env_record_holder(raw: sys::napi_env, key: usize) -> Result<()> {
  let mut global = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_get_global(raw, &mut global) },
    "Get global object for EnvRecord holder failed"
  )?;

  let mut holder = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_create_object(raw, &mut holder) },
    "Create EnvRecord holder failed"
  )?;

  let key_data = Box::into_raw(Box::new(key));
  let wrap_status = unsafe {
    sys::napi_wrap(
      raw,
      holder,
      key_data.cast(),
      Some(remove_env_record),
      ptr::null_mut(),
      ptr::null_mut(),
    )
  };
  if wrap_status != sys::Status::napi_ok {
    unsafe { drop(Box::from_raw(key_data)) };
    check_status!(wrap_status, "Wrap EnvRecord holder failed")?;
  }

  let property_name = CString::new(format!("__napi_rs_env_record_{key:x}"))?;
  let descriptor = sys::napi_property_descriptor {
    utf8name: property_name.as_ptr(),
    name: ptr::null_mut(),
    method: None,
    getter: None,
    setter: None,
    value: holder,
    attributes: sys::PropertyAttributes::default,
    data: ptr::null_mut(),
  };
  check_status!(
    unsafe { sys::napi_define_properties(raw, global, 1, &descriptor) },
    "Install EnvRecord holder failed"
  )
}

unsafe extern "C" fn remove_env_record(env: sys::napi_env, data: *mut c_void, _: *mut c_void) {
  let key = unsafe { Box::from_raw(data.cast::<usize>()) };
  run_unwind_boundary("tearing down env record", || {
    let record = ENV_RECORDS.with(|records| {
      let mut records = records.borrow_mut();
      records.remove(&*key)
    });
    if let Some(record) = record {
      let mut env = unsafe { Env::from_raw(env) };
      record.teardown(&mut env);
    }
  });
}

impl EnvRecord {
  #[doc(hidden)]
  pub fn new() -> Self {
    Self {
      data: RefCell::new(EnvData {
        constructors: HashMap::new(),
        user_instance_data: UserInstanceData { value: None },
      }),
      deferred_refs: DeferredRefs {
        refs: Cell::new(Vec::new()),
      },
    }
  }

  #[doc(hidden)]
  pub fn deferred_refs(&self) -> &DeferredRefs {
    &self.deferred_refs
  }

  #[doc(hidden)]
  pub fn constructor(&self, key: ClassKey) -> Result<Option<sys::napi_ref>> {
    self.with_data(|data| data.constructors().get(&key).copied())
  }

  #[doc(hidden)]
  pub fn set_constructor(&self, key: ClassKey, value: sys::napi_ref) -> Result<()> {
    self.with_data_mut(|data| {
      data.constructors_mut().insert(key, value);
    })
  }

  #[doc(hidden)]
  pub fn drain_deferred_refs(&self, env: &mut Env<'_>) -> Result<()> {
    delete_refs(env, self.deferred_refs.take())
  }

  fn take_constructor_refs(&self) -> Result<Vec<sys::napi_ref>> {
    self.with_data_mut(|data| {
      data
        .constructors_mut()
        .drain()
        .map(|(_, raw)| raw)
        .collect()
    })
  }

  fn drain_constructor_refs(&self, env: &mut Env<'_>) -> Result<()> {
    self
      .take_constructor_refs()
      .and_then(|refs| delete_refs(env, refs))
  }

  fn teardown(&self, env: &mut Env<'_>) {
    let mut first_error = None;

    if let Err(error) = self.drain_deferred_refs(env) {
      first_error.get_or_insert(error);
    }

    if let Err(error) = self.drain_constructor_refs(env) {
      first_error.get_or_insert(error);
    }

    if let Some(error) = first_error {
      eprintln!("napi-rs: failed to tear down env record: {error:?}");
    }
  }

  #[doc(hidden)]
  pub fn with_data<R>(&self, f: impl FnOnce(&EnvData) -> R) -> Result<R> {
    let data = self.data.try_borrow().map_err(|_| {
      Error::new(
        Status::InvalidArg,
        "Env data is already mutably borrowed".to_owned(),
      )
    })?;
    Ok(f(&data))
  }

  #[doc(hidden)]
  pub fn with_data_mut<R>(&self, f: impl FnOnce(&mut EnvData) -> R) -> Result<R> {
    let mut data = self.data.try_borrow_mut().map_err(|_| {
      Error::new(
        Status::InvalidArg,
        "Env data is already borrowed".to_owned(),
      )
    })?;
    Ok(f(&mut data))
  }
}

impl EnvData {
  #[doc(hidden)]
  pub fn constructors(&self) -> &ClassConstructorStore {
    &self.constructors
  }

  #[doc(hidden)]
  pub fn constructors_mut(&mut self) -> &mut ClassConstructorStore {
    &mut self.constructors
  }

  #[doc(hidden)]
  pub fn user_instance_data(&self) -> &UserInstanceData {
    &self.user_instance_data
  }

  #[doc(hidden)]
  pub fn user_instance_data_mut(&mut self) -> &mut UserInstanceData {
    &mut self.user_instance_data
  }
}

impl UserInstanceData {
  #[doc(hidden)]
  pub fn get<T: 'static>(&self) -> Result<Option<&T>> {
    let Some(value) = self.value.as_ref() else {
      return Ok(None);
    };
    value.downcast_ref().map(Some).ok_or_else(|| {
      Error::new(
        Status::InvalidArg,
        "User instance data type mismatch".to_owned(),
      )
    })
  }

  #[doc(hidden)]
  pub fn get_mut<T: 'static>(&mut self) -> Result<Option<&mut T>> {
    let Some(value) = self.value.as_mut() else {
      return Ok(None);
    };
    value.downcast_mut().map(Some).ok_or_else(|| {
      Error::new(
        Status::InvalidArg,
        "User instance data type mismatch".to_owned(),
      )
    })
  }

  #[doc(hidden)]
  pub fn set<T: 'static>(&mut self, value: T) -> Result<()> {
    let old = self.value.replace(Box::new(value));
    if let Some(old) = old {
      drop_user_instance_data(old)?;
    }
    Ok(())
  }
}

impl Drop for UserInstanceData {
  fn drop(&mut self) {
    if let Some(value) = self.value.take() {
      let result = drop_user_instance_data(value);
      drop(result);
    }
  }
}

fn drop_user_instance_data(value: Box<dyn Any>) -> Result<()> {
  match catch_unwind_boundary("dropping user instance data", || drop(value)) {
    Some(()) => Ok(()),
    None => Err(Error::new(
      Status::GenericFailure,
      "User instance data destructor panicked".to_owned(),
    )),
  }
}

impl DeferredRefs {
  #[doc(hidden)]
  pub fn push(&self, raw: sys::napi_ref) {
    let mut refs = self.refs.take();
    refs.push(raw);
    self.refs.set(refs);
  }

  #[doc(hidden)]
  pub fn take(&self) -> Vec<sys::napi_ref> {
    self.refs.take()
  }
}

impl<'env, 'scope> Scope<'env, 'scope> {
  pub fn env(&self) -> &Env<'env> {
    self.env
  }

  pub fn env_mut(&mut self) -> &mut Env<'env> {
    self.env
  }

  pub fn create_function<Args, Return>(
    &mut self,
    name: &str,
    callback: crate::Callback,
  ) -> Result<Function<'scope, Args, Return>> {
    let mut raw_result = ptr::null_mut();
    check_status!(unsafe {
      sys::napi_create_function(
        self.env.raw(),
        name.as_ptr().cast(),
        name.len() as isize,
        Some(callback),
        ptr::null_mut(),
        &mut raw_result,
      )
    })?;

    let value = unsafe { Local::from_raw(raw_result) };
    Function::from_js(self, value)
  }

  pub fn create_array(&mut self, len: u32) -> Result<Array<'scope>> {
    Array::new(self.env.raw(), len)
  }

  pub fn create_object_with_properties(
    &mut self,
    properties: &[sys::napi_property_descriptor],
  ) -> Result<Object<'scope>> {
    let raw = unsafe { create_object_with_properties(self.env.raw(), properties)? };
    Ok(unsafe { Object::from_raw(self.env.raw(), raw) })
  }

  pub fn create_ref<T, Ref>(&mut self, value: T) -> Result<Ref>
  where
    T: JsRefTarget<'scope, Ref>,
  {
    value.create_ref(self)
  }

  pub fn get_named_property<'value, T, V>(&mut self, object: &V, name: &str) -> Result<T>
  where
    T: FromJs<'env, 'scope>,
    V: crate::JsValue<'value>,
  {
    let key = CString::new(name)?;
    let mut raw_value = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_get_named_property(
          self.env.raw(),
          object.value().value,
          key.as_ptr(),
          &mut raw_value,
        )
      },
      "get_named_property error"
    )?;
    let value = unsafe { Local::from_raw(raw_value) };
    T::from_js(self, value)
  }

  pub fn assert_value_type<T>(
    &mut self,
    value: Local<'scope, T>,
    expected: ValueType,
  ) -> Result<()> {
    let mut raw_type = 0;
    check_status!(unsafe { sys::napi_typeof(self.env.raw(), value.raw(), &mut raw_type) })?;
    let received = ValueType::from(raw_type);
    if received == expected {
      Ok(())
    } else {
      Err(Error::new(
        Status::InvalidArg,
        format!("Expect value to be {expected}, but received {received}"),
      ))
    }
  }

  pub fn validate_value<T: ValidateNapiValue>(
    &self,
    raw: sys::napi_value,
  ) -> Result<sys::napi_value> {
    unsafe { T::validate(self.env.raw(), raw) }
  }

  pub fn create_error_value<C, R>(
    &mut self,
    code: C,
    reason: R,
  ) -> Result<Local<'scope, Object<'scope>>>
  where
    C: IntoJs<'scope> + 'scope,
    R: IntoJs<'scope> + 'scope,
  {
    let code = code.into_js(self)?.raw();
    let reason = reason.into_js(self)?.raw();
    let mut error = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_error(self.env.raw(), code, reason, &mut error) },
      "Failed to create napi error"
    )?;
    Ok(unsafe { Local::from_raw(error) })
  }

  pub fn get_optional_named_property<'value, T, V>(
    &mut self,
    object: &V,
    name: &str,
  ) -> Result<Option<T>>
  where
    T: FromJs<'env, 'scope>,
    V: crate::JsValue<'value>,
  {
    let key = CString::new(name)?;
    let mut raw_value = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_get_named_property(
          self.env.raw(),
          object.value().value,
          key.as_ptr(),
          &mut raw_value,
        )
      },
      "get_named_property error"
    )?;

    let mut value_type = 0;
    check_status!(unsafe { sys::napi_typeof(self.env.raw(), raw_value, &mut value_type) })?;
    if ValueType::from(value_type) == ValueType::Undefined {
      return Ok(None);
    }

    let value = unsafe { Local::from_raw(raw_value) };
    T::from_js(self, value).map(Some)
  }

  pub fn get_property<'value, 'key, T, V, K>(&mut self, object: &V, key: K) -> Result<T>
  where
    T: FromJs<'env, 'scope>,
    V: crate::JsValue<'value>,
    K: crate::JsValue<'key>,
  {
    let mut raw_value = ptr::null_mut();
    check_status!(unsafe {
      sys::napi_get_property(
        self.env.raw(),
        object.value().value,
        key.raw(),
        &mut raw_value,
      )
    })?;
    let value = unsafe { Local::from_raw(raw_value) };
    T::from_js(self, value)
  }

  pub fn get_element<'value, T, V>(&mut self, object: &V, index: u32) -> Result<T>
  where
    T: FromJs<'env, 'scope>,
    V: crate::JsValue<'value>,
  {
    let mut raw_value = ptr::null_mut();
    check_status!(unsafe {
      sys::napi_get_element(self.env.raw(), object.value().value, index, &mut raw_value)
    })?;
    let value = unsafe { Local::from_raw(raw_value) };
    T::from_js(self, value)
  }

  pub fn get_optional_element<T>(&mut self, array: &Array<'_>, index: u32) -> Result<Option<T>>
  where
    T: FromJs<'env, 'scope>,
  {
    if index >= array.len() {
      return Ok(None);
    }

    let mut raw_value = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_element(self.env.raw(), array.value().value, index, &mut raw_value) },
      "Failed to get element with index `{}`",
      index,
    )?;
    let value = unsafe { Local::from_raw(raw_value) };
    T::from_js(self, value).map(Some)
  }

  pub fn get_prototype<'value, T, V>(&mut self, object: &V) -> Result<T>
  where
    T: FromJs<'env, 'scope>,
    V: crate::JsValue<'value>,
  {
    let mut raw_value = ptr::null_mut();
    check_status!(unsafe {
      sys::napi_get_prototype(self.env.raw(), object.value().value, &mut raw_value)
    })?;
    let value = unsafe { Local::from_raw(raw_value) };
    T::from_js(self, value)
  }

  pub fn get_c_named_property<'value, T, V>(&mut self, object: &V, name: &CStr) -> Result<T>
  where
    T: FromJs<'env, 'scope>,
    V: crate::JsValue<'value>,
  {
    let mut raw_value = ptr::null_mut();
    check_status!(
      unsafe {
        sys::napi_get_named_property(
          self.env.raw(),
          object.value().value,
          name.as_ptr(),
          &mut raw_value,
        )
      },
      "get_named_property error"
    )?;
    let value = unsafe { Local::from_raw(raw_value) };
    T::from_js(self, value)
  }

  pub fn keys<'value>(&mut self, object: &Object<'value>) -> Result<Vec<String>> {
    let mut names = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_property_names(self.env.raw(), object.value().value, &mut names) },
      "Failed to get property names of given object"
    )?;

    let names = unsafe { Local::from_raw(names) };
    let array = Array::from_js(self, names)?;
    let mut keys = Vec::with_capacity(array.len() as usize);
    for i in 0..array.len() {
      keys.push(
        self
          .get_optional_element::<String>(&array, i)?
          .ok_or_else(|| {
            Error::new(
              Status::InvalidArg,
              format!("Found inconsistent property name at index {i}"),
            )
          })?,
      );
    }
    Ok(keys)
  }

  #[cfg(all(feature = "tokio_rt", feature = "napi4"))]
  pub fn spawn_future<T, F>(&mut self, future: F) -> Result<super::Promise<'scope, T>>
  where
    T: 'static + Send,
    F: 'static + Send + std::future::Future<Output = Result<T>>,
    for<'local> T: super::IntoJs<'local>,
  {
    use crate::JsValue;

    let promise = self.env.spawn_future(future)?;
    Ok(unsafe { super::Promise::from_raw(self.env.raw(), promise.raw()) })
  }

  #[doc(hidden)]
  pub fn record(&self) -> Option<&'scope Rc<EnvRecord>> {
    self.record
  }

  #[doc(hidden)]
  pub(crate) fn required_record(&self) -> Result<&'scope Rc<EnvRecord>> {
    self.record.ok_or_else(|| {
      Error::new(
        Status::InvalidArg,
        "Scope is not attached to an environment record".to_owned(),
      )
    })
  }

  pub(crate) fn ensure_value_env(&self, value_env: sys::napi_env, value_name: &str) -> Result<()> {
    if self.env.raw() == value_env {
      Ok(())
    } else {
      Err(Error::new(
        Status::InvalidArg,
        format!("{value_name} belongs to a different environment"),
      ))
    }
  }

  pub fn run_script<T>(&mut self, script: impl AsRef<str>) -> Result<T>
  where
    T: FromJs<'env, 'scope>,
  {
    let raw_value = unsafe { run_script_raw(self.env.raw(), script.as_ref()) }?;
    let value = unsafe { Local::from_raw(raw_value) };
    T::from_js(self, value)
  }
}

unsafe fn run_script_raw(env: sys::napi_env, script: &str) -> Result<sys::napi_value> {
  let mut raw_script = ptr::null_mut();
  check_status!(
    unsafe {
      sys::napi_create_string_utf8(
        env,
        script.as_ptr().cast(),
        script.len() as isize,
        &mut raw_script,
      )
    },
    "Create script string failed"
  )?;

  let mut raw_value = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_run_script(env, raw_script, &mut raw_value) },
    "Run script failed"
  )?;
  Ok(raw_value)
}

impl<'scope, T> Local<'scope, T> {
  #[doc(hidden)]
  pub unsafe fn from_raw(raw: sys::napi_value) -> Self {
    Self {
      raw,
      marker: PhantomData,
    }
  }

  #[doc(hidden)]
  pub fn raw(&self) -> sys::napi_value {
    self.raw
  }
}

impl<'scope> Local<'scope, Unknown<'scope>> {
  pub(crate) fn from_value<'env, 'value, V>(
    scope: &Scope<'env, 'scope>,
    value: &V,
    value_name: &str,
  ) -> Result<Self>
  where
    V: JsValue<'value>,
  {
    let value = value.value();
    scope.ensure_value_env(value.env, value_name)?;
    Ok(unsafe { Self::from_raw(value.value) })
  }
}

#[doc(hidden)]
pub unsafe fn with_env<R>(
  raw: sys::napi_env,
  f: impl for<'env> FnOnce(Env<'env>) -> Result<R>,
) -> Result<R> {
  let record = env_record(raw);
  let mut entry_env = unsafe { Env::from_raw(raw) };
  record.drain_deferred_refs(&mut entry_env)?;
  let result = catch_unwind_result("running N-API callback", || {
    let callback_env = unsafe { Env::from_raw(raw) };
    f(callback_env)
  })
  .and_then(|result| result);
  let mut exit_env = unsafe { Env::from_raw(raw) };
  match (result, record.drain_deferred_refs(&mut exit_env)) {
    (Ok(value), Ok(())) => Ok(value),
    (Err(error), _) | (Ok(_), Err(error)) => Err(error),
  }
}

#[doc(hidden)]
pub fn delete_refs(env: &mut Env<'_>, refs: Vec<sys::napi_ref>) -> Result<()> {
  let mut error = None;

  for raw in refs {
    if let Err(err) = check_status!(
      unsafe { sys::napi_delete_reference(env.raw(), raw) },
      "Delete deferred reference failed"
    ) {
      error.get_or_insert(err);
    }
  }

  if let Some(error) = error {
    Err(error)
  } else {
    Ok(())
  }
}

impl<'env> Env<'env> {
  #[doc(hidden)]
  pub(crate) fn record(&self) -> Rc<EnvRecord> {
    env_record(self.raw())
  }

  pub fn with_scope<R>(
    &mut self,
    f: impl for<'scope> FnOnce(&'scope mut Scope<'env, 'scope>) -> Result<R>,
  ) -> Result<R> {
    let record = self.record();
    let mut scope = Scope {
      env: self,
      record: Some(&record),
      marker: PhantomData,
    };
    f(&mut scope)
  }

  pub fn create_array(&self, len: u32) -> Result<Array<'_>> {
    Array::new(self.0, len)
  }

  pub fn get_global(&self) -> Result<JsGlobal<'env>> {
    let mut global = std::ptr::null_mut();
    crate::check_status!(
      unsafe { sys::napi_get_global(self.0, &mut global) },
      "Get global object from Env failed"
    )?;
    Ok(JsGlobal(
      crate::Value {
        value: global,
        env: self.0,
        value_type: crate::ValueType::Object,
      },
      std::marker::PhantomData,
    ))
  }
}
