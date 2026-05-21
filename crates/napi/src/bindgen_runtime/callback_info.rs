use std::marker::PhantomData;
use std::ptr;
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use crate::iterator::ScopedGenerator;
use crate::{bindgen_prelude::*, check_status};

pub struct ConstructorReceiver<'env, 'scope, 'frame, T: NapiClass> {
  raw: sys::napi_value,
  env: sys::napi_env,
  record: Weak<EnvRecord>,
  class: &'static ClassInfo,
  marker: PhantomData<(&'env (), &'scope (), &'frame CallbackValues, T)>,
}

impl<'env, 'scope, T: NapiClass> ConstructorReceiver<'env, 'scope, 'scope, T> {
  pub(crate) fn new(frame: &CallbackFrame<'env, 'scope>) -> Result<Self> {
    let scope = &frame.context.scope;
    Ok(Self {
      raw: unsafe { frame.values().as_ref().this() },
      env: scope.env().raw(),
      record: Rc::downgrade(scope.record()),
      class: T::CLASS.info(),
      marker: PhantomData,
    })
  }

  pub(crate) fn raw(&self) -> sys::napi_value {
    self.raw
  }

  pub(crate) fn env(&self) -> sys::napi_env {
    self.env
  }

  pub(crate) fn record(&self) -> Weak<EnvRecord> {
    self.record.clone()
  }

  pub(crate) fn class(&self) -> &'static ClassInfo {
    self.class
  }
}

pub(crate) struct CallbackValues {
  this: sys::napi_value,
  args: Vec<sys::napi_value>,
  data: *mut std::ffi::c_void,
}

impl CallbackValues {
  pub(crate) fn new(
    this: sys::napi_value,
    args: Vec<sys::napi_value>,
    data: *mut std::ffi::c_void,
  ) -> Self {
    Self { this, args, data }
  }

  pub(crate) fn this(&self) -> sys::napi_value {
    self.this
  }

  pub(crate) fn arg(&self, index: usize) -> Option<sys::napi_value> {
    self.args.get(index).copied()
  }

  pub(crate) fn data(&self) -> *mut std::ffi::c_void {
    self.data
  }
}

pub struct AsyncArgRefs<const N: usize> {
  refs: [sys::napi_ref; N],
  len: usize,
}

impl<const N: usize> AsyncArgRefs<N> {
  pub fn new() -> Self {
    Self {
      refs: [ptr::null_mut(); N],
      len: 0,
    }
  }

  fn retain(&mut self, env: &Env<'_>, raw: sys::napi_value) -> Result<()> {
    if raw.is_null() {
      return Err(Error::new(
        Status::InvalidArg,
        "Referenced callback value is null".to_owned(),
      ));
    }
    if self.len == N {
      return Err(Error::new(
        Status::InvalidArg,
        "Callback reference storage is full".to_owned(),
      ));
    }

    let mut raw_ref = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_reference(env.raw(), raw, 1, &mut raw_ref) },
      "failed to create napi ref"
    )?;
    self.refs[self.len] = raw_ref;
    self.len += 1;
    Ok(())
  }

  pub fn finalize(self, env: Env<'_>) {
    for raw_ref in self.refs.into_iter().take(self.len) {
      assert_eq!(
        unsafe { sys::napi_reference_unref(env.raw(), raw_ref, &mut 0) },
        sys::Status::napi_ok,
        "failed to unref napi ref"
      );
      assert_eq!(
        unsafe { sys::napi_delete_reference(env.raw(), raw_ref) },
        sys::Status::napi_ok,
        "failed to delete napi ref"
      );
    }
  }
}

impl<const N: usize> Default for AsyncArgRefs<N> {
  fn default() -> Self {
    Self::new()
  }
}

unsafe impl<const N: usize> Send for AsyncArgRefs<N> {}
unsafe impl<const N: usize> Sync for AsyncArgRefs<N> {}

#[derive(Clone, Copy)]
pub struct FrameObject<'scope> {
  raw: sys::napi_value,
  frame: NonNull<CallbackValues>,
  marker: PhantomData<&'scope CallbackValues>,
}

impl<'scope> FrameObject<'scope> {
  pub(crate) fn new(raw: sys::napi_value, frame: NonNull<CallbackValues>) -> Self {
    Self {
      raw,
      frame,
      marker: PhantomData,
    }
  }

  pub(crate) fn raw_for(&self, context: &FrameScope<'_, 'scope>) -> Result<sys::napi_value> {
    if self.frame == context.frame {
      Ok(self.raw)
    } else {
      Err(Error::new(
        Status::InvalidArg,
        "Frame object does not belong to this callback frame".to_owned(),
      ))
    }
  }
}

pub struct FrameScope<'env, 'scope> {
  scope: &'scope mut Scope<'env, 'scope>,
  frame: NonNull<CallbackValues>,
}

impl<'env, 'scope> FrameScope<'env, 'scope> {
  pub(crate) fn new(
    scope: &'scope mut Scope<'env, 'scope>,
    frame: NonNull<CallbackValues>,
  ) -> Self {
    Self { scope, frame }
  }

  pub fn scope_mut<'frame>(&'frame mut self) -> &'frame mut Scope<'env, 'scope> {
    self.scope
  }
}

pub struct CallbackDecoder<'env, const N: usize> {
  env: Env<'env>,
  values: CallbackValues,
}

impl<'env, const N: usize> CallbackDecoder<'env, N> {
  #[allow(clippy::not_unsafe_ptr_arg_deref)]
  pub(crate) fn new(
    env: Env<'env>,
    callback_info: sys::napi_callback_info,
    required_argc: Option<usize>,
  ) -> Result<Self> {
    let mut this = ptr::null_mut();
    let mut args = [ptr::null_mut(); N];
    let mut argc = N;
    let mut data = ptr::null_mut();

    unsafe {
      check_status!(
        sys::napi_get_cb_info(
          env.raw(),
          callback_info,
          &mut argc,
          args.as_mut_ptr(),
          &mut this,
          &mut data,
        ),
        "Failed to initialize napi callback frame."
      )?;
    };

    if let Some(required_argc) = required_argc {
      if required_argc > argc {
        return Err(Error::new(
          Status::InvalidArg,
          format!(
            "{} arguments required by received {}.",
            required_argc, &argc
          ),
        ));
      }
    }

    Ok(Self {
      env,
      values: CallbackValues::new(this, args.into_iter().take(argc).collect(), data),
    })
  }

  pub(crate) fn dynamic(
    env: Env<'env>,
    callback_info: sys::napi_callback_info,
    inline_argc: usize,
  ) -> Result<Self> {
    let mut this = ptr::null_mut();
    let mut args = vec![ptr::null_mut(); inline_argc];
    let mut argc = inline_argc;
    let mut data = ptr::null_mut();

    check_status!(
      unsafe {
        sys::napi_get_cb_info(
          env.raw(),
          callback_info,
          &mut argc,
          args.as_mut_ptr(),
          &mut this,
          &mut data,
        )
      },
      "Failed to initialize dynamic napi callback frame."
    )?;

    if argc > inline_argc {
      args = vec![ptr::null_mut(); argc];
      check_status!(
        unsafe {
          sys::napi_get_cb_info(
            env.raw(),
            callback_info,
            &mut argc,
            args.as_mut_ptr(),
            &mut this,
            &mut data,
          )
        },
        "Failed to initialize dynamic napi callback frame."
      )?;
    }
    args.truncate(argc);

    Ok(Self {
      env,
      values: CallbackValues::new(this, args, data),
    })
  }

  pub fn with_frame<R>(
    &mut self,
    f: impl for<'scope> FnOnce(CallbackFrame<'env, 'scope>) -> Result<R>,
  ) -> Result<R> {
    let values = NonNull::from(&mut self.values);
    self.env.with_scope(|scope| {
      let context = FrameScope::new(scope, values);
      f(CallbackFrame { context })
    })
  }

  pub fn with_frame_in_scope<'scope, R>(
    &mut self,
    scope: &'scope mut Scope<'env, 'scope>,
    f: impl FnOnce(CallbackFrame<'env, 'scope>) -> Result<R>,
  ) -> Result<R> {
    let values = NonNull::from(&mut self.values);
    let context = FrameScope::new(scope, values);
    f(CallbackFrame { context })
  }
}

pub struct CallbackFrame<'env, 'scope> {
  context: FrameScope<'env, 'scope>,
}

impl<'env, 'scope> CallbackFrame<'env, 'scope> {
  pub fn context_mut(&mut self) -> &mut FrameScope<'env, 'scope> {
    &mut self.context
  }

  pub fn scope_mut<'frame>(&'frame mut self) -> &'frame mut Scope<'env, 'scope> {
    self.context.scope_mut()
  }

  pub(crate) fn into_scope(self) -> &'scope mut Scope<'env, 'scope> {
    self.context.scope
  }

  pub fn constructor_receiver<T: NapiClass>(
    &self,
  ) -> Result<ConstructorReceiver<'env, 'scope, 'scope, T>> {
    ConstructorReceiver::new(self)
  }

  #[doc(hidden)]
  pub fn env(&self) -> Env<'env> {
    *self.context.scope.env()
  }

  fn values(&self) -> NonNull<CallbackValues> {
    self.context.frame
  }

  pub(crate) fn raw_this(&self) -> sys::napi_value {
    unsafe { self.values().as_ref().this() }
  }

  pub(crate) fn raw_env(&self) -> sys::napi_env {
    self.context.scope.env().raw()
  }

  pub(crate) fn raw_data(&self) -> *mut std::ffi::c_void {
    unsafe { self.values().as_ref().data() }
  }

  fn undefined(&self) -> Result<sys::napi_value> {
    let mut value = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_undefined(self.context.scope.env().raw(), &mut value) },
      "Failed to create undefined callback argument"
    )?;
    Ok(value)
  }

  pub(crate) fn raw_arg(&self, index: usize) -> Result<sys::napi_value> {
    unsafe { self.values().as_ref().arg(index) }
      .map(Ok)
      .unwrap_or_else(|| self.undefined())
  }

  #[cfg(feature = "napi5")]
  pub(crate) fn raw_args(&self) -> &'scope [sys::napi_value] {
    unsafe { &self.values().as_ref().args }
  }

  pub(crate) fn retain_value<const N: usize>(
    &self,
    refs: &mut AsyncArgRefs<N>,
    raw: sys::napi_value,
  ) -> Result<()> {
    refs.retain(self.context.scope.env(), raw)
  }

  pub(crate) fn validate_value<T: ValidateNapiValue>(
    &self,
    raw: sys::napi_value,
  ) -> Result<sys::napi_value> {
    unsafe { T::validate(self.context.scope.env().raw(), raw) }
  }

  pub(crate) fn assert_value_type(&self, raw: sys::napi_value, expected: ValueType) -> Result<()> {
    let mut value_type = 0;
    check_status!(
      unsafe { sys::napi_typeof(self.context.scope.env().raw(), raw, &mut value_type) },
      "Failed to detect napi value type",
    )?;
    let received = ValueType::from(value_type);
    if received == expected {
      Ok(())
    } else {
      Err(Error::new(
        Status::InvalidArg,
        format!("Expect value to be {expected}, but received {received}"),
      ))
    }
  }

  pub fn rest_args(&self, from: usize) -> JsArgSlice<'scope> {
    let args = unsafe { &self.values().as_ref().args };
    let slice = if from < args.len() {
      &args[from..]
    } else {
      &[]
    };
    JsArgSlice::new(slice)
  }

  pub fn arg<T: FromJs<'env, 'scope>>(&mut self, index: usize) -> Result<T> {
    let raw = self.raw_arg(index)?;
    let scope = self.context.scope_mut();
    let value = unsafe { Local::from_raw(raw) };
    T::from_js(scope, value)
  }

  pub fn optional_arg<T: FromJs<'env, 'scope>>(&mut self, index: usize) -> Result<Option<T>> {
    let Some(raw) = (unsafe { self.values().as_ref().arg(index) }) else {
      return Ok(None);
    };
    let scope = self.context.scope_mut();
    let value = unsafe { Local::from_raw(raw) };
    T::from_js(scope, value).map(Some)
  }

  pub fn this<T: FromJs<'env, 'scope>>(&mut self) -> Result<T> {
    let raw = self.raw_this();
    let scope = self.context.scope_mut();
    let value = unsafe { Local::from_raw(raw) };
    T::from_js(scope, value)
  }

  pub fn return_value<T>(&mut self, value: T) -> Result<sys::napi_value>
  where
    T: IntoJs<'scope> + 'scope,
  {
    value
      .into_js(self.context.scope_mut())
      .map(|local| local.raw())
  }

  pub fn construct_generator<
    const IsEmptyStructHint: bool,
    T: for<'a> ScopedGenerator<'a> + NapiClass + ClassChain + 'static,
  >(
    &mut self,
    js_name: &str,
    init: impl IntoClassInitializer<T>,
  ) -> Result<sys::napi_value> {
    construct_generator_on_this(self.constructor_receiver::<T>()?, js_name, init)
  }

  pub fn generator_factory<T: NapiClass + ClassChain + for<'a> ScopedGenerator<'a> + 'static>(
    &mut self,
    js_name: &str,
    init: impl IntoClassInitializer<T>,
  ) -> Result<sys::napi_value> {
    generator_factory_on_this::<T>(self.context.scope_mut().env().raw(), js_name, init)
  }

  #[cfg(feature = "tokio_rt")]
  pub fn construct_async_generator<
    const IsEmptyStructHint: bool,
    T: crate::bindgen_runtime::AsyncGenerator + NapiClass + ClassChain + 'static,
  >(
    &mut self,
    js_name: &str,
    init: impl IntoClassInitializer<T>,
  ) -> Result<sys::napi_value> {
    construct_async_generator_on_this(self.constructor_receiver::<T>()?, js_name, init)
  }

  #[cfg(feature = "tokio_rt")]
  pub fn async_generator_factory<
    T: crate::bindgen_runtime::AsyncGenerator + NapiClass + ClassChain + 'static,
  >(
    &mut self,
    js_name: &str,
    init: impl IntoClassInitializer<T>,
  ) -> Result<sys::napi_value> {
    async_generator_factory_on_this::<T>(self.context.scope_mut().env().raw(), js_name, init)
  }

  pub fn this_object(&mut self) -> FrameObject<'scope> {
    let raw = unsafe { self.values().as_ref().this() };
    FrameObject::new(raw, self.values())
  }

  pub fn arg_object(&mut self, index: usize) -> Result<FrameObject<'scope>> {
    let raw = unsafe { self.values().as_ref().arg(index) }.ok_or_else(|| {
      Error::new(
        Status::InvalidArg,
        format!("Argument {} is not available in this callback frame", index),
      )
    })?;
    Ok(FrameObject::new(raw, self.values()))
  }

  fn arg_optional_object(&mut self, index: usize) -> Result<Option<FrameObject<'scope>>> {
    let Some(raw) = (unsafe { self.values().as_ref().arg(index) }) else {
      return Ok(None);
    };
    if raw.is_null() {
      return Ok(None);
    }

    let mut value_type = -1;
    check_status!(
      unsafe { sys::napi_typeof(self.context.scope_mut().env().raw(), raw, &mut value_type) },
      "Failed to get optional class argument type"
    )?;
    if matches!(
      value_type,
      sys::ValueType::napi_null | sys::ValueType::napi_undefined
    ) {
      return Ok(None);
    }

    Ok(Some(FrameObject::new(raw, self.values())))
  }

  fn class_from_object<T: NapiClass>(
    &mut self,
    object: FrameObject<'scope>,
  ) -> Result<T::Ref<'scope>> {
    let raw = object.raw_for(&self.context)?;
    let (access, storage) = T::validate_object(&mut self.context, object)?;
    unsafe { T::ref_from_validated_object(raw, storage, access) }
  }

  fn class_mut_from_object<T: NapiClass>(
    &mut self,
    object: FrameObject<'scope>,
  ) -> Result<T::Mut<'scope>> {
    let raw = object.raw_for(&self.context)?;
    let (access, storage) = T::validate_object(&mut self.context, object)?;
    unsafe { T::mut_from_validated_object(raw, storage, access) }
  }

  fn reference_from_object<T: NapiReceiver>(
    &mut self,
    object: FrameObject<'scope>,
  ) -> Result<Reference<T>> {
    Reference::from_frame_object(&mut self.context, object)
  }

  pub fn this_class<T: NapiClass>(&mut self) -> Result<T::Ref<'scope>> {
    let object = self.this_object();
    self.class_from_object::<T>(object)
  }

  pub fn this_class_mut<T: NapiClass>(&mut self) -> Result<T::Mut<'scope>> {
    let object = self.this_object();
    self.class_mut_from_object::<T>(object)
  }

  pub fn arg_class<T: NapiClass>(&mut self, index: usize) -> Result<T::Ref<'scope>> {
    let object = self.arg_object(index)?;
    self.class_from_object::<T>(object)
  }

  pub fn arg_class_mut<T: NapiClass>(&mut self, index: usize) -> Result<T::Mut<'scope>> {
    let object = self.arg_object(index)?;
    self.class_mut_from_object::<T>(object)
  }

  pub fn arg_opt_class<T: NapiClass>(&mut self, index: usize) -> Result<Option<T::Ref<'scope>>> {
    let Some(object) = self.arg_optional_object(index)? else {
      return Ok(None);
    };
    self.class_from_object::<T>(object).map(Some)
  }

  pub fn arg_opt_class_mut<T: NapiClass>(
    &mut self,
    index: usize,
  ) -> Result<Option<T::Mut<'scope>>> {
    let Some(object) = self.arg_optional_object(index)? else {
      return Ok(None);
    };
    self.class_mut_from_object::<T>(object).map(Some)
  }

  pub fn this_reference<T: NapiReceiver>(&mut self) -> Result<Reference<T>> {
    let object = self.this_object();
    self.reference_from_object(object)
  }

  pub fn arg_reference<T: NapiReceiver>(&mut self, index: usize) -> Result<Reference<T>> {
    let object = self.arg_object(index)?;
    self.reference_from_object(object)
  }

  pub fn arg_opt_reference<T: NapiReceiver>(
    &mut self,
    index: usize,
  ) -> Result<Option<Reference<T>>> {
    let Some(object) = self.arg_optional_object(index)? else {
      return Ok(None);
    };
    self.reference_from_object(object).map(Some)
  }
}

fn construct_generator_on_this<T>(
  receiver: ConstructorReceiver<'_, '_, '_, T>,
  js_name: &str,
  init: impl IntoClassInitializer<T>,
) -> Result<sys::napi_value>
where
  T: for<'a> ScopedGenerator<'a> + NapiClass + ClassChain + 'static,
{
  ensure_class_name(js_name)?;
  let init = init.into_class_initializer();
  let env = receiver.env();
  let instance = T::CLASS.wrap_receiver(receiver, init)?;
  unsafe { crate::__private::create_iterator::<T>(env, instance) };
  Ok(instance)
}

fn generator_factory_on_this<T>(
  env: sys::napi_env,
  js_name: &str,
  init: impl IntoClassInitializer<T>,
) -> Result<sys::napi_value>
where
  T: NapiClass + ClassChain + for<'a> ScopedGenerator<'a> + 'static,
{
  ensure_class_name(js_name)?;
  let init = init.into_class_initializer();
  let instance =
    unsafe { EnvRecord::enter_scope(env, |scope| T::CLASS.new_object_from_scope(scope, init)) }?;
  unsafe { crate::__private::create_iterator::<T>(env, instance) };
  Ok(instance)
}

#[cfg(feature = "tokio_rt")]
fn construct_async_generator_on_this<T>(
  receiver: ConstructorReceiver<'_, '_, '_, T>,
  js_name: &str,
  init: impl IntoClassInitializer<T>,
) -> Result<sys::napi_value>
where
  T: crate::bindgen_runtime::AsyncGenerator + NapiClass + ClassChain + 'static,
{
  ensure_class_name(js_name)?;
  let init = init.into_class_initializer();
  let env = receiver.env();
  let instance = T::CLASS.wrap_receiver(receiver, init)?;
  unsafe { crate::__private::create_async_iterator::<T>(env, instance) };
  Ok(instance)
}

#[cfg(feature = "tokio_rt")]
fn async_generator_factory_on_this<T>(
  env: sys::napi_env,
  js_name: &str,
  init: impl IntoClassInitializer<T>,
) -> Result<sys::napi_value>
where
  T: crate::bindgen_runtime::AsyncGenerator + NapiClass + ClassChain + 'static,
{
  ensure_class_name(js_name)?;
  let init = init.into_class_initializer();
  let instance =
    unsafe { EnvRecord::enter_scope(env, |scope| T::CLASS.new_object_from_scope(scope, init)) }?;
  unsafe { crate::__private::create_async_iterator::<T>(env, instance) };
  Ok(instance)
}

fn ensure_class_name(js_name: &str) -> Result<()> {
  if js_name.trim_end_matches('\0').is_empty() {
    Err(Error::new(
      Status::InvalidArg,
      "Class name is required for class construction",
    ))
  } else {
    Ok(())
  }
}

/// Runtime-owned binding entry for generated ordinary Node callbacks.
///
/// # Safety
///
/// The caller must be the generated ABI callback invoked by Node-API, and `raw_env`
/// and `callback_info` must be the matching pair provided by that invocation.
#[doc(hidden)]
pub unsafe fn __napi_binding_entry<const N: usize>(
  raw_env: sys::napi_env,
  callback_info: sys::napi_callback_info,
  invoke: impl for<'env, 'scope> FnOnce(CallbackFrame<'env, 'scope>) -> Result<sys::napi_value>,
) -> sys::napi_value {
  unsafe {
    EnvRecord::enter_scope(raw_env, |scope| {
      let env = *scope.env();
      let mut decoder = CallbackDecoder::<N>::new(env, callback_info, None)?;
      decoder.with_frame_in_scope(scope, invoke)
    })
  }
  .unwrap_or_else(|error| {
    unsafe { JsError::from(error).throw_into(raw_env) };
    ptr::null_mut::<sys::napi_value__>()
  })
}

#[doc(hidden)]
pub unsafe fn __napi_binding_entry_variadic(
  raw_env: sys::napi_env,
  callback_info: sys::napi_callback_info,
  hint: usize,
  invoke: impl for<'env, 'scope> FnOnce(CallbackFrame<'env, 'scope>) -> Result<sys::napi_value>,
) -> sys::napi_value {
  unsafe {
    EnvRecord::enter_scope(raw_env, |scope| {
      let env = *scope.env();
      let mut decoder = CallbackDecoder::<0>::dynamic(env, callback_info, hint)?;
      decoder.with_frame_in_scope(scope, invoke)
    })
  }
  .unwrap_or_else(|error| {
    unsafe { JsError::from(error).throw_into(raw_env) };
    ptr::null_mut::<sys::napi_value__>()
  })
}
