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

// ── JsRefKind trait + marker types ──────────────────────────────────

pub trait JsRefKind {
  type Access: Copy + Eq + 'static;
}

pub struct Class<T: NapiReceiver>(PhantomData<fn() -> T>);

impl<T: NapiReceiver> JsRefKind for Class<T> {
  type Access = T::Access;
}

pub struct Obj;

impl JsRefKind for Obj {
  type Access = ();
}

pub struct Func<A, R>(PhantomData<fn(A) -> R>);

impl<A, R> JsRefKind for Func<A, R> {
  type Access = ();
}

pub struct Sym;

impl JsRefKind for Sym {
  type Access = ();
}

pub struct Unk;

impl JsRefKind for Unk {
  type Access = ();
}

pub struct Ext<T: 'static>(PhantomData<fn() -> T>);

impl<T: 'static> JsRefKind for Ext<T> {
  type Access = ();
}

// ── RefState (Layer 0) ──────────────────────────────────────────────

pub(crate) struct RefState {
  raw: Cell<sys::napi_ref>,
  record: Weak<EnvRecord>,
}

impl RefState {
  pub(crate) fn new(raw: sys::napi_ref, record: Weak<EnvRecord>) -> Self {
    Self {
      raw: Cell::new(raw),
      record,
    }
  }

  pub(crate) fn owner_record(&self) -> Result<Rc<EnvRecord>> {
    self.record.upgrade().ok_or_else(|| {
      Error::new(
        Status::InvalidArg,
        "Ref owner environment is no longer available".to_owned(),
      )
    })
  }

  pub(crate) fn raw_ref(&self) -> Result<sys::napi_ref> {
    let raw = self.raw.get();
    if raw.is_null() {
      Err(Error::new(
        Status::InvalidArg,
        "Ref is already closed".to_owned(),
      ))
    } else {
      Ok(raw)
    }
  }

  pub(crate) fn take_raw(&self) -> Result<sys::napi_ref> {
    let raw = self.raw.replace(ptr::null_mut());
    if raw.is_null() {
      Err(Error::new(
        Status::InvalidArg,
        "Ref is already closed".to_owned(),
      ))
    } else {
      Ok(raw)
    }
  }
}

impl Drop for RefState {
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

// ── Ref<K> / WeakRef<K> ────────────────────────────────────────────

pub struct Ref<K: JsRefKind> {
  pub(crate) state: RefState,
  pub(crate) access: K::Access,
  marker: PhantomData<fn() -> K>,
  not_send: PhantomData<Rc<()>>,
}

pub struct WeakRef<K: JsRefKind> {
  pub(crate) state: RefState,
  pub(crate) access: K::Access,
  marker: PhantomData<fn() -> K>,
  not_send: PhantomData<Rc<()>>,
}

impl<K: JsRefKind> Ref<K> {
  pub(crate) fn new(state: RefState, access: K::Access) -> Self {
    Self {
      state,
      access,
      marker: PhantomData,
      not_send: PhantomData,
    }
  }

  pub(crate) fn close_in(self, scope: &mut Scope<'_, '_>) -> Result<()> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;
    delete_reference(scope.env().raw(), self.state.take_raw()?)
  }

  pub(crate) fn downgrade_in(&self, scope: &mut Scope<'_, '_>) -> Result<WeakRef<K>> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;
    let object = reference_value(scope.env().raw(), self.state.raw_ref()?)?;
    let raw = create_reference(scope.env().raw(), object, 0)?;
    Ok(WeakRef::new(
      RefState::new(raw, Rc::downgrade(&record)),
      self.access,
    ))
  }
}

impl<K: JsRefKind> WeakRef<K> {
  pub(crate) fn new(state: RefState, access: K::Access) -> Self {
    Self {
      state,
      access,
      marker: PhantomData,
      not_send: PhantomData,
    }
  }

  pub(crate) fn close_in(self, scope: &mut Scope<'_, '_>) -> Result<()> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;
    delete_reference(scope.env().raw(), self.state.take_raw()?)
  }

  pub(crate) fn upgrade_raw(
    &self,
    scope: &mut Scope<'_, '_>,
  ) -> Result<Option<(sys::napi_value, Rc<EnvRecord>)>> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;
    let raw = self.state.raw_ref()?;
    let object = reference_value(scope.env().raw(), raw)?;
    if object.is_null() {
      return Ok(None);
    }
    Ok(Some((object, record)))
  }
}

// ── Type aliases ────────────────────────────────────────────────────

pub type Reference<T> = Ref<Class<T>>;
pub type WeakReference<T> = WeakRef<Class<T>>;

// ── ClassLocal ──────────────────────────────────────────────────────

pub struct ClassLocal<'env, 'scope, T: NapiReceiver> {
  object: Local<'scope, Object<'scope>>,
  storage: ClassStorageRef<'scope>,
  access: T::Access,
  marker: PhantomData<fn(&'env (), T)>,
}

impl<'env, 'scope, T: NapiReceiver> ClassLocal<'env, 'scope, T> {
  pub fn as_object(&self) -> Local<'scope, Object<'scope>> {
    self.object
  }
}

// ── Class-specific impls (Layer 2) ──────────────────────────────────

impl<T: NapiReceiver> Ref<Class<T>> {
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
    ensure_same_access(self.access, access)?;

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

  pub(crate) fn from_frame_object<'scope>(
    context: &mut FrameScope<'_, 'scope>,
    object: FrameObject<'scope>,
  ) -> Result<Self> {
    let raw_object = object.raw_for(context)?;
    let (access, _) = T::validate_object(context, object)?;
    let raw = create_reference(context.scope_mut().env().raw(), raw_object, 1)?;
    let record = Rc::downgrade(context.scope_mut().record());
    Ok(Self::new(RefState::new(raw, record), access))
  }

  pub(crate) fn cast_in<U: NapiReceiver>(
    &self,
    scope: &mut Scope<'_, '_>,
  ) -> Result<Ref<Class<U>>> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;
    let object = reference_value(scope.env().raw(), self.state.raw_ref()?)?;
    unsafe { Ref::<Class<U>>::from_object_unchecked(scope, object) }
  }

  #[doc(hidden)]
  pub(crate) unsafe fn from_object_unchecked(
    scope: &mut Scope<'_, '_>,
    object: sys::napi_value,
  ) -> Result<Self> {
    let (access, _) = unsafe { T::validate_raw_object(scope, object) }?;
    let raw = create_reference(scope.env().raw(), object, 1)?;
    let record = Rc::downgrade(scope.record());
    Ok(Self::new(RefState::new(raw, record), access))
  }

  fn from_class_local<'env, 'scope>(
    scope: &mut Scope<'env, 'scope>,
    local: &ClassLocal<'env, 'scope, T>,
    initial_refcount: u32,
  ) -> Result<Self> {
    let raw = create_reference(scope.env().raw(), local.object.raw(), initial_refcount)?;
    let record = Rc::downgrade(scope.record());
    Ok(Self::new(RefState::new(raw, record), local.access))
  }
}

impl<'scope, T: NapiReceiver> IntoJs<'scope> for Ref<Class<T>> {
  type Output = Object<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let record = self.state.owner_record()?;
    ensure_same_record(&record, scope)?;
    let raw = reference_value(scope.env().raw(), self.state.raw_ref()?)?;
    Ok(unsafe { Local::from_raw(raw) })
  }
}

impl<'env, 'scope, T: NapiReceiver> FromJs<'env, 'scope> for Ref<Class<T>> {
  fn from_js(
    scope: &mut Scope<'env, 'scope>,
    value: Local<'scope, Unknown<'scope>>,
  ) -> Result<Self> {
    unsafe { Self::from_object_unchecked(scope, value.raw()) }
  }
}

impl<T: NapiClass> TypeName for Ref<Class<T>> {
  fn type_name() -> &'static str {
    T::CLASS.info().js_name()
  }

  fn value_type() -> ValueType {
    ValueType::Object
  }
}

impl<T: NapiReceiver> WeakRef<Class<T>> {
  pub(crate) fn upgrade_in(
    &self,
    scope: &mut Scope<'_, '_>,
  ) -> Result<Option<Ref<Class<T>>>> {
    let Some((object, record)) = self.upgrade_raw(scope)? else {
      return Ok(None);
    };
    let (access, _) = unsafe { T::validate_raw_object(scope, object) }?;
    ensure_same_access(self.access, access)?;
    let raw = create_reference(scope.env().raw(), object, 1)?;
    Ok(Some(Ref::new(
      RefState::new(raw, Rc::downgrade(&record)),
      self.access,
    )))
  }
}

// ── Scope methods ───────────────────────────────────────────────────

impl<'env, 'scope> Scope<'env, 'scope> {
  pub fn reference<T>(&mut self, value: T) -> Result<Reference<T>>
  where
    T: NapiClass + ClassChain + IntoClassInitializer<T>,
  {
    Ref::<Class<T>>::new_in(self, value)
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
    Ref::<Class<T>>::from_class_local(self, local, 1)
  }

  pub fn create_weak_reference<T: NapiReceiver>(
    &mut self,
    local: &ClassLocal<'env, 'scope, T>,
  ) -> Result<WeakReference<T>> {
    let raw = create_reference(self.env().raw(), local.object.raw(), 0)?;
    let record = Rc::downgrade(self.record());
    Ok(WeakRef::new(RefState::new(raw, record), local.access))
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

// ── Helpers ─────────────────────────────────────────────────────────

pub(crate) fn ensure_same_record(record: &Rc<EnvRecord>, scope: &Scope<'_, '_>) -> Result<()> {
  let current = scope.record();
  if Rc::ptr_eq(record, current) {
    Ok(())
  } else {
    Err(owner_mismatch())
  }
}

pub(crate) fn ensure_record_match(
  expected: &Rc<EnvRecord>,
  actual: &Rc<EnvRecord>,
) -> Result<()> {
  if Rc::ptr_eq(expected, actual) {
    Ok(())
  } else {
    Err(owner_mismatch())
  }
}

pub(crate) fn ensure_same_access<T: Copy + Eq>(expected: T, actual: T) -> Result<()> {
  if expected == actual {
    Ok(())
  } else {
    Err(Error::new(
      Status::InvalidArg,
      "Reference class access does not match the current object".to_owned(),
    ))
  }
}

pub(crate) fn owner_mismatch() -> Error {
  Error::new(
    Status::InvalidArg,
    "Ref owner environment does not match the current environment".to_owned(),
  )
}

pub(crate) fn create_reference(
  env: sys::napi_env,
  object: sys::napi_value,
  initial_refcount: u32,
) -> Result<sys::napi_ref> {
  let mut raw = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_create_reference(env, object, initial_refcount, &mut raw) },
    "Create reference failed",
  )?;
  Ok(raw)
}

pub(crate) fn reference_value(
  env: sys::napi_env,
  raw: sys::napi_ref,
) -> Result<sys::napi_value> {
  let mut object = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_get_reference_value(env, raw, &mut object) },
    "Get reference value failed",
  )?;
  Ok(object)
}

pub(crate) fn delete_reference(env: sys::napi_env, raw: sys::napi_ref) -> Result<()> {
  check_status!(
    unsafe { sys::napi_delete_reference(env, raw) },
    "Delete reference failed",
  )
}
