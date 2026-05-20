use std::cell::Cell;
use std::marker::PhantomData;
use std::ptr;
use std::rc::{Rc, Weak};

use crate::{
  bindgen_runtime::{
    ClassChain, ClassStorageRef, EnvRecord, FrameObject, FrameScope, FromJs, IntoClassInitializer,
    IntoJs, Local, NapiClass, NapiReceiver, Object, Result, Scope, TypeName, Unknown,
  },
  check_status, sys, Error, Status, ValueType,
};

pub struct Reference<T: NapiReceiver> {
  state: ClassReferenceState<T::Access>,
  marker: PhantomData<fn() -> T>,
  not_send: PhantomData<Rc<()>>,
}

pub struct WeakReference<T: NapiReceiver> {
  state: ClassReferenceState<T::Access>,
  marker: PhantomData<fn() -> T>,
  not_send: PhantomData<Rc<()>>,
}

enum ClassReferenceKind {
  Strong,
  Weak,
}

struct ClassReferenceState<A> {
  raw: Cell<sys::napi_ref>,
  record: Weak<EnvRecord>,
  access: A,
  kind: ClassReferenceKind,
}

pub struct ClassLocal<'env, 'scope, T: NapiReceiver> {
  object: Local<'scope, Object<'scope>>,
  storage: ClassStorageRef<'scope>,
  access: T::Access,
  marker: PhantomData<fn(&'env (), T)>,
}

impl ClassReferenceKind {
  fn owner_unavailable_error(&self) -> Error {
    match self {
      Self::Strong => Error::new(
        Status::InvalidArg,
        "Reference owner environment is no longer available".to_owned(),
      ),
      Self::Weak => Error::new(
        Status::InvalidArg,
        "WeakReference owner environment is no longer available".to_owned(),
      ),
    }
  }

  fn already_closed_error(&self) -> Error {
    match self {
      Self::Strong => Error::new(Status::InvalidArg, "Reference is already closed".to_owned()),
      Self::Weak => Error::new(
        Status::InvalidArg,
        "WeakReference is already closed".to_owned(),
      ),
    }
  }
}

impl<A: Copy> ClassReferenceState<A> {
  fn new(raw: sys::napi_ref, record: Weak<EnvRecord>, access: A, kind: ClassReferenceKind) -> Self {
    Self {
      raw: Cell::new(raw),
      record,
      access,
      kind,
    }
  }

  fn access(&self) -> A {
    self.access
  }

  fn owner_record(&self) -> Result<Rc<EnvRecord>> {
    self
      .record
      .upgrade()
      .ok_or_else(|| self.kind.owner_unavailable_error())
  }

  fn raw_ref(&self) -> Result<sys::napi_ref> {
    let raw = self.raw.get();
    if raw.is_null() {
      Err(self.kind.already_closed_error())
    } else {
      Ok(raw)
    }
  }

  fn take_raw(&self) -> Result<sys::napi_ref> {
    let raw = self.raw.replace(ptr::null_mut());
    if raw.is_null() {
      Err(self.kind.already_closed_error())
    } else {
      Ok(raw)
    }
  }
}

impl<A> Drop for ClassReferenceState<A> {
  fn drop(&mut self) {
    let raw = self.raw.replace(ptr::null_mut());
    if raw.is_null() {
      return;
    }

    if let Some(record) = self.record.upgrade() {
      record.deferred_refs().push(raw);
    }
  }
}

impl<T: NapiReceiver> Reference<T> {
  pub(crate) fn new_in(scope: &mut Scope<'_, '_>, value: T) -> Result<Self>
  where
    T: NapiClass + ClassChain + IntoClassInitializer<T>,
  {
    let init = IntoClassInitializer::<T>::into_class_initializer(value);
    let object = unsafe { T::CLASS.new_object_from_scope(scope, init)? };
    unsafe { Self::from_object_unchecked(scope, object) }
  }

  pub(crate) fn bind<'env, 'scope>(
    &self,
    scope: &mut Scope<'env, 'scope>,
  ) -> Result<ClassLocal<'env, 'scope, T>> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;
    let object = reference_value(scope.env().raw(), self.state.raw_ref()?)?;
    let (access, storage) = unsafe { T::validate_raw_object(scope, object) }?;
    ensure_same_access(self.state.access(), access)?;

    Ok(ClassLocal {
      object: unsafe { Local::from_raw(object) },
      storage,
      access,
      marker: PhantomData,
    })
  }

  pub(crate) fn try_clone_in(&self, scope: &mut Scope<'_, '_>) -> Result<Self> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;
    let object = reference_value(scope.env().raw(), self.state.raw_ref()?)?;
    unsafe { Self::from_object_unchecked(scope, object) }
  }

  pub(crate) fn downgrade_in(&self, scope: &mut Scope<'_, '_>) -> Result<WeakReference<T>> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;
    let object = reference_value(scope.env().raw(), self.state.raw_ref()?)?;
    let raw = create_reference(scope.env().raw(), object, 0)?;

    Ok(WeakReference {
      state: ClassReferenceState::new(
        raw,
        Rc::downgrade(&record),
        self.state.access(),
        ClassReferenceKind::Weak,
      ),
      marker: PhantomData,
      not_send: PhantomData,
    })
  }

  pub(crate) fn from_frame_object<'scope>(
    context: &mut FrameScope<'_, 'scope>,
    object: FrameObject<'scope>,
  ) -> Result<Self> {
    let raw_object = object.raw_for(context)?;
    let (access, _) = T::validate_object(context, object)?;
    let raw = create_reference(context.scope_mut().env().raw(), raw_object, 1)?;
    let record = Rc::downgrade(context.scope_mut().record());

    Ok(Self {
      state: ClassReferenceState::new(raw, record, access, ClassReferenceKind::Strong),
      marker: PhantomData,
      not_send: PhantomData,
    })
  }

  pub(crate) fn cast_in<U: NapiReceiver>(&self, scope: &mut Scope<'_, '_>) -> Result<Reference<U>> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;
    let object = reference_value(scope.env().raw(), self.state.raw_ref()?)?;
    unsafe { Reference::<U>::from_object_unchecked(scope, object) }
  }

  pub(crate) fn close_in(self, scope: &mut Scope<'_, '_>) -> Result<()> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;
    delete_reference(scope.env().raw(), self.state.take_raw()?)
  }

  #[doc(hidden)]
  pub(crate) unsafe fn from_object_unchecked(
    scope: &mut Scope<'_, '_>,
    object: sys::napi_value,
  ) -> Result<Self> {
    let (access, _) = unsafe { T::validate_raw_object(scope, object) }?;
    let raw = create_reference(scope.env().raw(), object, 1)?;
    let record = Rc::downgrade(scope.record());

    Ok(Self {
      state: ClassReferenceState::new(raw, record, access, ClassReferenceKind::Strong),
      marker: PhantomData,
      not_send: PhantomData,
    })
  }
}

impl<'scope, T: NapiReceiver> IntoJs<'scope> for Reference<T> {
  type Output = Object<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let env = scope.env().raw();
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;

    let raw = reference_value(env, self.state.raw_ref()?)?;
    Ok(unsafe { Local::from_raw(raw) })
  }
}

impl<'env, 'scope, T: NapiReceiver> FromJs<'env, 'scope> for Reference<T> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    unsafe { Self::from_object_unchecked(scope, value.raw()) }
  }
}

impl<T: NapiClass> TypeName for Reference<T> {
  fn type_name() -> &'static str {
    T::CLASS.info().js_name()
  }

  fn value_type() -> ValueType {
    ValueType::Object
  }
}

impl<T: NapiReceiver> WeakReference<T> {
  pub(crate) fn upgrade_in(&self, scope: &mut Scope<'_, '_>) -> Result<Option<Reference<T>>> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;

    let raw = self.state.raw_ref()?;
    let object = reference_value(scope.env().raw(), raw)?;
    if object.is_null() {
      return Ok(None);
    }
    let (access, _) = unsafe { T::validate_raw_object(scope, object) }?;
    ensure_same_access(self.state.access(), access)?;

    Ok(Some(Reference {
      state: ClassReferenceState::new(
        create_reference(scope.env().raw(), object, 1)?,
        Rc::downgrade(&record),
        self.state.access(),
        ClassReferenceKind::Strong,
      ),
      marker: PhantomData,
      not_send: PhantomData,
    }))
  }

  pub(crate) fn close_in(self, scope: &mut Scope<'_, '_>) -> Result<()> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;
    delete_reference(scope.env().raw(), self.state.take_raw()?)
  }
}

impl<'env, 'scope, T: NapiReceiver> ClassLocal<'env, 'scope, T> {
  pub fn as_object(&self) -> Local<'scope, Object<'scope>> {
    self.object
  }
}

impl<'env, 'scope> Scope<'env, 'scope> {
  pub fn reference<T>(&mut self, value: T) -> Result<Reference<T>>
  where
    T: NapiClass + ClassChain + IntoClassInitializer<T>,
  {
    Reference::new_in(self, value)
  }

  pub fn bind_reference<T: NapiReceiver>(
    &mut self,
    reference: &Reference<T>,
  ) -> Result<ClassLocal<'env, 'scope, T>> {
    reference.bind(self)
  }

  pub fn clone_reference<T: NapiReceiver>(
    &mut self,
    reference: &Reference<T>,
  ) -> Result<Reference<T>> {
    reference.try_clone_in(self)
  }

  pub fn downgrade_reference<T: NapiReceiver>(
    &mut self,
    reference: &Reference<T>,
  ) -> Result<WeakReference<T>> {
    reference.downgrade_in(self)
  }

  pub fn cast_reference<T: NapiReceiver, U: NapiReceiver>(
    &mut self,
    reference: &Reference<T>,
  ) -> Result<Reference<U>> {
    reference.cast_in(self)
  }

  pub fn close_reference<T: NapiReceiver>(&mut self, reference: Reference<T>) -> Result<()> {
    reference.close_in(self)
  }

  pub fn upgrade_reference<T: NapiReceiver>(
    &mut self,
    reference: &WeakReference<T>,
  ) -> Result<Option<Reference<T>>> {
    reference.upgrade_in(self)
  }

  pub fn close_weak_reference<T: NapiReceiver>(
    &mut self,
    reference: WeakReference<T>,
  ) -> Result<()> {
    reference.close_in(self)
  }

  pub fn create_reference<T: NapiReceiver>(
    &mut self,
    local: &ClassLocal<'env, 'scope, T>,
  ) -> Result<Reference<T>> {
    Reference::from_class_local(self, local, 1)
  }

  pub fn create_weak_reference<T: NapiReceiver>(
    &mut self,
    local: &ClassLocal<'env, 'scope, T>,
  ) -> Result<WeakReference<T>> {
    let raw = create_reference(self.env().raw(), local.object.raw(), 0)?;
    let record = Rc::downgrade(self.record());

    Ok(WeakReference {
      state: ClassReferenceState::new(raw, record, local.access, ClassReferenceKind::Weak),
      marker: PhantomData,
      not_send: PhantomData,
    })
  }

  pub fn borrow_class<'borrow, T: NapiReceiver>(
    &'borrow mut self,
    local: &'borrow ClassLocal<'env, 'scope, T>,
  ) -> Result<T::Ref<'borrow>> {
    let storage = local.storage;
    unsafe { T::ref_from_validated_object(local.object.raw(), storage, local.access) }
  }

  pub fn borrow_class_mut<'borrow, T: NapiReceiver>(
    &'borrow mut self,
    local: &'borrow ClassLocal<'env, 'scope, T>,
  ) -> Result<T::Mut<'borrow>> {
    let storage = local.storage;
    unsafe { T::mut_from_validated_object(local.object.raw(), storage, local.access) }
  }

  pub fn same_class_object<T: NapiReceiver, U: NapiReceiver>(
    &mut self,
    left: &ClassLocal<'env, 'scope, T>,
    right: &ClassLocal<'env, 'scope, U>,
  ) -> Result<bool> {
    let mut result = false;
    check_status!(
      unsafe {
        sys::napi_strict_equals(
          self.env().raw(),
          left.object.raw(),
          right.object.raw(),
          &mut result,
        )
      },
      "Compare class object identity failed",
    )?;
    Ok(result)
  }

  pub fn is_class_object<T: NapiReceiver>(
    &mut self,
    value: Local<'scope, Unknown<'scope>>,
  ) -> bool {
    unsafe { T::validate_raw_object(self, value.raw()).is_ok() }
  }

  pub fn is_class_value<'value, T, V>(&mut self, value: &V) -> Result<bool>
  where
    T: NapiReceiver,
    V: crate::JsValue<'value>,
  {
    let value = value.value();
    self.ensure_value_env(value.env, "class candidate")?;
    Ok(unsafe { T::validate_raw_object(self, value.value).is_ok() })
  }
}

impl<T: NapiReceiver> Reference<T> {
  fn from_class_local<'env, 'scope>(
    scope: &mut Scope<'env, 'scope>,
    local: &ClassLocal<'env, 'scope, T>,
    initial_refcount: u32,
  ) -> Result<Self> {
    let raw = create_reference(scope.env().raw(), local.object.raw(), initial_refcount)?;
    let record = Rc::downgrade(scope.record());

    Ok(Self {
      state: ClassReferenceState::new(raw, record, local.access, ClassReferenceKind::Strong),
      marker: PhantomData,
      not_send: PhantomData,
    })
  }
}

fn ensure_same_record(record: &Rc<EnvRecord>, scope: &Scope<'_, '_>) -> Result<()> {
  let current = scope.record();
  if Rc::ptr_eq(record, current) {
    Ok(())
  } else {
    Err(owner_mismatch())
  }
}

fn ensure_same_access<T: Copy + Eq>(expected: T, actual: T) -> Result<()> {
  if expected == actual {
    Ok(())
  } else {
    Err(Error::new(
      Status::InvalidArg,
      "Reference class access does not match the current object".to_owned(),
    ))
  }
}

fn owner_mismatch() -> Error {
  Error::new(
    Status::InvalidArg,
    "Reference owner environment does not match the current environment".to_owned(),
  )
}

fn create_reference(
  env: sys::napi_env,
  object: sys::napi_value,
  initial_refcount: u32,
) -> Result<sys::napi_ref> {
  let mut raw = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_create_reference(env, object, initial_refcount, &mut raw) },
    "Create class object reference failed",
  )?;
  Ok(raw)
}

fn reference_value(env: sys::napi_env, raw: sys::napi_ref) -> Result<sys::napi_value> {
  let mut object = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_get_reference_value(env, raw, &mut object) },
    "Get class object reference value failed",
  )?;
  Ok(object)
}

fn delete_reference(env: sys::napi_env, raw: sys::napi_ref) -> Result<()> {
  check_status!(
    unsafe { sys::napi_delete_reference(env, raw) },
    "Delete class object reference failed",
  )
}
