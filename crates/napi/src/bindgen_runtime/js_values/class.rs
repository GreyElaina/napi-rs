use std::alloc;
use std::cell::{Cell, Ref, RefCell, RefMut};
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::mem::{self, ManuallyDrop};
use std::ops::{Deref, DerefMut};
use std::ptr::{self, NonNull};
use std::rc::{Rc, Weak};

use crate::{
  bindgen_runtime::{
    ConstructorReceiver, EnvRecord, FrameObject, FrameScope, IntoJs, Local, Object, Result, Scope,
  },
  catch_unwind_boundary, check_status, run_unwind_boundary, sys, Error, Status,
};

pub const CLASS_STORAGE_ABI_MAGIC: u64 = 0x4e41_5049_4353_5452;
pub const CLASS_STORAGE_ABI_VERSION: u32 = 1;

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ClassAccess {
  class: *const ClassInfo,
  offset: usize,
}

impl ClassAccess {
  #[doc(hidden)]
  pub const unsafe fn new(class: &'static ClassInfo, offset: usize) -> Self {
    Self { class, offset }
  }

  pub fn class(&self) -> *const ClassInfo {
    self.class
  }

  pub fn offset(&self) -> usize {
    self.offset
  }
}

/// Native class exported through napi-rs module registration.
///
/// # Safety
///
/// Implementors must be registered via `#[napi]` and must honor the storage layout
/// contract of [`ClassChain`] for their inheritance chain.
pub unsafe trait NapiClass: NapiReceiver<Access = ClassAccess> + Sized + 'static {
  type Parent: NativeParent;

  const CLASS: &'static ClassDef<Self>;
}

/// Subclass of another [`NapiClass`].
///
/// # Safety
///
/// Same requirements as [`NapiClass`]. Parent and child layouts must match codegen `extends`.
pub unsafe trait NapiSubclass: NapiClass {}

pub trait NativeParent {
  type Initializer;

  fn erased_class_def() -> Option<ErasedClassDef> {
    None
  }
}

impl NativeParent for () {
  type Initializer = ();
}

impl<T: NapiSubclass> NativeParent for T {
  type Initializer = ClassInitializer<T>;

  fn erased_class_def() -> Option<ErasedClassDef> {
    Some(T::CLASS.erase())
  }
}

/// Rust value stored inside a JavaScript wrapper object.
///
/// # Safety
///
/// Validation and reference methods must only be used on objects that already carry
/// this receiver's class storage.
pub unsafe trait NapiReceiver: Sized + 'static {
  type Access: Copy + Eq + 'static;

  type Borrow<'a>: Deref<Target = Self>
  where
    Self: 'a;

  type BorrowMut<'a>: DerefMut<Target = Self>
  where
    Self: 'a;

  fn validate_object<'scope>(
    context: &mut FrameScope<'_, 'scope>,
    object: FrameObject<'scope>,
  ) -> Result<(Self::Access, ClassStorageRef<'scope>)> {
    let object = object.raw_for(context)?;
    unsafe { Self::validate_raw_object(context.scope_mut(), object) }
  }

  /// # Safety
  ///
  /// `object` must be a valid `napi_value` for a wrapper whose storage matches this receiver.
  #[doc(hidden)]
  unsafe fn validate_raw_object<'scope>(
    scope: &mut Scope<'_, 'scope>,
    object: sys::napi_value,
  ) -> Result<(Self::Access, ClassStorageRef<'scope>)>;

  /// # Safety
  ///
  /// `storage` and `access` must come from a successful `validate_raw_object` for this receiver.
  unsafe fn ref_from_validated_object<'a>(
    storage: ClassStorageRef<'a>,
    access: Self::Access,
  ) -> Result<Self::Borrow<'a>>;

  /// # Safety
  ///
  /// `storage` and `access` must come from a successful `validate_raw_object` for this receiver.
  unsafe fn mut_from_validated_object<'a>(
    storage: ClassStorageRef<'a>,
    access: Self::Access,
  ) -> Result<Self::BorrowMut<'a>>;
}

pub struct ClassInfo {
  rust_name: &'static str,
  js_name: &'static str,
  subclassable: bool,
}

impl ClassInfo {
  #[doc(hidden)]
  pub const unsafe fn new(
    rust_name: &'static str,
    js_name: &'static str,
    subclassable: bool,
  ) -> Self {
    Self {
      rust_name,
      js_name,
      subclassable,
    }
  }

  pub fn rust_name(&self) -> &'static str {
    self.rust_name
  }

  pub fn js_name(&self) -> &'static str {
    self.js_name
  }

  pub fn subclassable(&self) -> bool {
    self.subclassable
  }
}

#[derive(Clone, Copy)]
pub struct ClassKey(&'static ClassInfo);

impl ClassKey {
  pub fn new(info: &'static ClassInfo) -> Self {
    Self(info)
  }
}

impl Eq for ClassKey {}

impl PartialEq for ClassKey {
  fn eq(&self, other: &Self) -> bool {
    std::ptr::addr_eq(self.0, other.0)
  }
}

impl Hash for ClassKey {
  fn hash<H: Hasher>(&self, state: &mut H) {
    ptr::from_ref(self.0).hash(state);
  }
}

pub struct ClassDef<T: NapiClass> {
  info: &'static ClassInfo,
  layout: fn() -> &'static ClassLayout,
  marker: PhantomData<fn() -> T>,
}

unsafe impl<T: NapiClass> Sync for ClassDef<T> {}

impl<T: NapiClass> ClassDef<T> {
  pub fn info(&self) -> &'static ClassInfo {
    self.info
  }

  pub fn layout(&self) -> &'static ClassLayout {
    (self.layout)()
  }

  pub fn key(&self) -> ClassKey {
    ClassKey::new(self.info)
  }

  pub fn erase(&'static self) -> ErasedClassDef {
    ErasedClassDef {
      info: self.info,
      layout: self.layout,
    }
  }

  fn ensure_receiver_class(
    &'static self,
    receiver: &ConstructorReceiver<'_, '_, '_, T>,
  ) -> Result<()> {
    if std::ptr::addr_eq(receiver.class(), self.info()) {
      Ok(())
    } else {
      Err(Error::new(
        Status::InvalidArg,
        format!(
          "Constructor receiver class mismatch for {}",
          self.info().rust_name()
        ),
      ))
    }
  }

  #[doc(hidden)]
  pub unsafe fn try_wrap_internal_construction(
    &'static self,
    receiver: ConstructorReceiver<'_, '_, '_, T>,
  ) -> Result<InternalConstructionResult>
  where
    T: ClassChain,
  {
    self.ensure_receiver_class(&receiver)?;

    let storage = PENDING_CLASS_STORAGE.with(|pending| {
      let mut pending = pending.borrow_mut();
      match pending.last() {
        None => Ok(None),
        Some(top) if std::ptr::addr_eq(top.layout(), self.layout()) => Ok(pending.pop()),
        Some(_) => Err(Error::new(
          Status::InvalidArg,
          format!(
            "Internal construction slot mismatch for {}",
            self.info().rust_name()
          ),
        )),
      }
    })?;

    let Some(storage) = storage else {
      return Ok(InternalConstructionResult::Absent);
    };

    unsafe { storage.wrap(receiver.env(), receiver.raw()) }?;
    Ok(InternalConstructionResult::Wrapped(receiver.raw()))
  }

  #[doc(hidden)]
  pub fn wrap_receiver(
    &'static self,
    receiver: ConstructorReceiver<'_, '_, '_, T>,
    init: ClassInitializer<T>,
  ) -> Result<sys::napi_value>
  where
    T: ClassChain,
  {
    self
      .wrap_receiver_with_value(receiver, init)
      .map(|(object, _)| object)
  }

  #[doc(hidden)]
  pub fn wrap_receiver_with_value(
    &'static self,
    receiver: ConstructorReceiver<'_, '_, '_, T>,
    init: ClassInitializer<T>,
  ) -> Result<(sys::napi_value, *mut T)>
  where
    T: ClassChain,
  {
    self.ensure_receiver_class(&receiver)?;

    let record = receiver.record();
    let storage = unsafe { PendingClassStorage::new(record, init) }?;
    let value = storage.segment::<T>(self.info())?;
    unsafe { storage.wrap(receiver.env(), receiver.raw()) }?;
    Ok((receiver.raw(), value.as_ptr()))
  }

  #[doc(hidden)]
  pub unsafe fn new_object_from_initializer(
    &'static self,
    context: &mut FrameScope<'_, '_>,
    init: ClassInitializer<T>,
  ) -> Result<sys::napi_value>
  where
    T: ClassChain,
  {
    unsafe { self.new_object_from_scope(context.scope_mut(), init) }
  }

  #[doc(hidden)]
  pub unsafe fn new_object_from_scope(
    &'static self,
    scope: &mut Scope<'_, '_>,
    init: ClassInitializer<T>,
  ) -> Result<sys::napi_value>
  where
    T: ClassChain,
  {
    unsafe {
      self
        .new_object_with_value_from_scope(scope, init)
        .map(|(object, _)| object)
    }
  }

  #[doc(hidden)]
  pub unsafe fn new_object_with_value_from_scope(
    &'static self,
    scope: &mut Scope<'_, '_>,
    init: ClassInitializer<T>,
  ) -> Result<(sys::napi_value, *mut T)>
  where
    T: ClassChain,
  {
    let record = Rc::downgrade(scope.record());
    let storage = unsafe { PendingClassStorage::new(record, init) }?;
    let value = storage.segment::<T>(self.info())?;
    let guard = PendingClassStorageGuard::push(storage);

    let ctor_ref = scope.record().constructor(self.key())?;

    let Some(ctor_ref) = ctor_ref else {
      return Err(Error::new(
        Status::InvalidArg,
        format!(
          "Failed to get constructor of class `{}`",
          self.info().js_name()
        ),
      ));
    };

    let mut ctor = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_reference_value(scope.env().raw(), ctor_ref, &mut ctor) },
      "Failed to get constructor reference of class `{}`",
      self.info().js_name(),
    )?;

    let mut result = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_new_instance(scope.env().raw(), ctor, 0, ptr::null(), &mut result,) },
      "Failed to construct class `{}`",
      self.info().js_name(),
    )?;

    guard.ensure_consumed(self.info().js_name())?;
    Ok((result, value.as_ptr()))
  }

  #[doc(hidden)]
  pub const unsafe fn new(info: &'static ClassInfo, layout: fn() -> &'static ClassLayout) -> Self {
    Self {
      info,
      layout,
      marker: PhantomData,
    }
  }
}

#[derive(Clone, Copy)]
pub struct ErasedClassDef {
  info: &'static ClassInfo,
  layout: fn() -> &'static ClassLayout,
}

impl ErasedClassDef {
  pub fn info(&self) -> &'static ClassInfo {
    self.info
  }

  pub fn layout(&self) -> &'static ClassLayout {
    (self.layout)()
  }

  pub fn key(&self) -> ClassKey {
    ClassKey::new(self.info)
  }
}

pub enum InternalConstructionResult {
  Wrapped(sys::napi_value),
  Absent,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClassEntry {
  class: *const ClassInfo,
  offset: usize,
}

unsafe impl Sync for ClassEntry {}

impl ClassEntry {
  #[doc(hidden)]
  pub const unsafe fn new(class: &'static ClassInfo, offset: usize) -> Self {
    Self {
      class: class as *const ClassInfo,
      offset,
    }
  }

  pub fn class(&self) -> &'static ClassInfo {
    unsafe { &*self.class }
  }

  pub(crate) fn access(&self) -> ClassAccess {
    ClassAccess {
      class: self.class,
      offset: self.offset,
    }
  }
}

#[repr(C)]
pub struct ClassLayout {
  parent: Option<&'static ClassLayout>,
  entry: ClassEntry,
  size: usize,
  align: usize,
  drop_initialized: unsafe fn(NonNull<u8>),
}

unsafe impl Sync for ClassLayout {}
unsafe impl Send for ClassLayout {}

impl ClassLayout {
  #[doc(hidden)]
  pub const unsafe fn new(
    parent: Option<&'static ClassLayout>,
    entry: ClassEntry,
    size: usize,
    align: usize,
    drop_initialized: unsafe fn(NonNull<u8>),
  ) -> Self {
    Self {
      parent,
      entry,
      size,
      align,
      drop_initialized,
    }
  }

  pub fn class_count(&self) -> usize {
    self.parent.map_or(0, ClassLayout::class_count) + 1
  }

  pub fn for_each_entry(&self, f: &mut impl FnMut(&ClassEntry)) {
    if let Some(parent) = self.parent {
      parent.for_each_entry(f);
    }
    f(&self.entry);
  }

  pub fn size(&self) -> usize {
    self.size
  }

  pub fn align(&self) -> usize {
    self.align
  }

  pub fn find(&self, class: &'static ClassInfo) -> Option<ClassAccess> {
    if std::ptr::addr_eq(self.entry.class, class as *const ClassInfo) {
      Some(self.entry.access())
    } else {
      self.parent.and_then(|parent| parent.find(class))
    }
  }

  /// Drops class values in-place without deallocating `data`.
  ///
  /// # Safety
  ///
  /// `data` must point at initialized storage owned by this layout chain entry.
  pub unsafe fn drop_initialized(&self, data: NonNull<u8>) {
    unsafe { (self.drop_initialized)(data) };
  }
}

#[repr(C)]
pub struct ClassStorageHeader {
  abi_magic: u64,
  abi_version: u32,
  header_size: u32,
  layout: *const ClassLayout,
  data: NonNull<u8>,
  state: NonNull<ClassStorageState>,
}

impl ClassStorageHeader {
  #[doc(hidden)]
  pub unsafe fn new(
    layout: &'static ClassLayout,
    data: NonNull<u8>,
    state: NonNull<ClassStorageState>,
  ) -> Self {
    Self {
      abi_magic: CLASS_STORAGE_ABI_MAGIC,
      abi_version: CLASS_STORAGE_ABI_VERSION,
      header_size: mem::size_of::<Self>() as u32,
      layout: layout as *const ClassLayout,
      data,
      state,
    }
  }

  pub fn validate_abi(&self) -> bool {
    class_storage_abi_matches(self.abi_magic, self.abi_version, self.header_size)
  }

  pub(crate) fn layout(&self) -> &'static ClassLayout {
    unsafe { &*self.layout }
  }

  pub(crate) fn data(&self) -> NonNull<u8> {
    self.data
  }

  pub(crate) fn state(&self) -> NonNull<ClassStorageState> {
    self.state
  }

  pub(crate) fn segment<T>(&self, access: ClassAccess) -> NonNull<T> {
    let ptr = unsafe { self.data.as_ptr().add(access.offset()) };
    NonNull::new(ptr.cast()).expect("class segment pointer must not be null")
  }
}

unsafe fn validate_class_storage_abi_prefix(header: *const ClassStorageHeader) -> bool {
  let abi_magic = unsafe { ptr::addr_of!((*header).abi_magic).read_unaligned() };
  let abi_version = unsafe { ptr::addr_of!((*header).abi_version).read_unaligned() };
  let header_size = unsafe { ptr::addr_of!((*header).header_size).read_unaligned() };
  class_storage_abi_matches(abi_magic, abi_version, header_size)
}

fn class_storage_abi_matches(abi_magic: u64, abi_version: u32, header_size: u32) -> bool {
  abi_magic == CLASS_STORAGE_ABI_MAGIC
    && abi_version == CLASS_STORAGE_ABI_VERSION
    && header_size as usize == mem::size_of::<ClassStorageHeader>()
}

pub struct ClassStorageState {
  record: Weak<EnvRecord>,
  borrow_cell: RefCell<()>,
}

impl ClassStorageState {
  #[doc(hidden)]
  pub fn new(record: Weak<EnvRecord>) -> Self {
    Self {
      record,
      borrow_cell: RefCell::new(()),
    }
  }

  pub fn record(&self) -> &Weak<EnvRecord> {
    &self.record
  }

  pub fn borrow_cell(&self) -> &RefCell<()> {
    &self.borrow_cell
  }
}

#[derive(Clone, Copy)]
pub struct ClassStorageRef<'scope> {
  header: NonNull<ClassStorageHeader>,
  marker: PhantomData<&'scope ClassStorageHeader>,
}

impl<'scope> ClassStorageRef<'scope> {
  #[doc(hidden)]
  pub unsafe fn new(header: NonNull<ClassStorageHeader>) -> Self {
    Self {
      header,
      marker: PhantomData,
    }
  }

  pub(crate) fn header(&self) -> &ClassStorageHeader {
    unsafe { self.header.as_ref() }
  }

  pub(crate) fn header_ptr(&self) -> NonNull<ClassStorageHeader> {
    self.header
  }

  pub(crate) fn scoped_state(&self) -> &'scope ClassStorageState {
    unsafe { self.header().state().as_ref() }
  }

  pub(crate) fn segment<T>(&self, access: ClassAccess) -> NonNull<T> {
    let ptr = unsafe { self.header().data().as_ptr().add(access.offset()) };
    NonNull::new(ptr.cast()).expect("class segment pointer must not be null")
  }

  pub fn access_for(&self, class: &'static ClassInfo) -> Result<ClassAccess> {
    self.header().layout().find(class).ok_or_else(|| {
      Error::new(
        Status::InvalidArg,
        format!("Object is not an instance of {}", class.rust_name()),
      )
    })
  }

  #[doc(hidden)]
  pub unsafe fn validate_raw_object(
    scope: &mut Scope<'_, 'scope>,
    object: sys::napi_value,
    class: &'static ClassInfo,
  ) -> Result<(ClassAccess, Self)> {
    let mut wrapped = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_unwrap(scope.env().raw(), object, &mut wrapped) },
      "Object has no native class storage"
    )?;

    let header = NonNull::new(wrapped.cast::<ClassStorageHeader>()).ok_or_else(|| {
      Error::new(
        Status::InvalidArg,
        "Class storage header is null".to_owned(),
      )
    })?;

    if !unsafe { validate_class_storage_abi_prefix(header.as_ptr()) } {
      return Err(Error::new(
        Status::InvalidArg,
        "Class storage ABI mismatch".to_owned(),
      ));
    }

    let header_ref = unsafe { header.as_ref() };

    let access = header_ref.layout().find(class).ok_or_else(|| {
      Error::new(
        Status::InvalidArg,
        format!("Object is not an instance of {}", class.rust_name()),
      )
    })?;

    let state_ref = unsafe { header_ref.state().as_ref() };
    let record = state_ref.record().upgrade().ok_or_else(|| {
      Error::new(
        Status::InvalidArg,
        "Class storage owner environment is no longer available".to_owned(),
      )
    })?;
    let scope_record = scope.record();
    if !Rc::ptr_eq(&record, scope_record) {
      return Err(Error::new(
        Status::InvalidArg,
        "Class storage owner environment does not match the current environment".to_owned(),
      ));
    }

    let storage = unsafe { Self::new(header) };
    Ok((access, storage))
  }
}

pub struct ClassBorrow<'a, T: NapiClass> {
  storage: ClassStorageRef<'a>,
  value: Ref<'a, T>,
}

impl<'a, T: NapiClass> ClassBorrow<'a, T> {
  #[doc(hidden)]
  pub unsafe fn from_validated_parts(
    storage: ClassStorageRef<'a>,
    access: ClassAccess,
  ) -> Result<Self> {
    let value = storage.segment(access);
    let value = Ref::map(
      storage
        .scoped_state()
        .borrow_cell()
        .try_borrow()
        .map_err(|_| {
          Error::new(
            Status::InvalidArg,
            "Class storage is already mutably borrowed".to_owned(),
          )
        })?,
      |_| unsafe { value.as_ref() },
    );
    Ok(Self { storage, value })
  }

  pub fn as_super(&self) -> Result<&T::Parent>
  where
    T::Parent: NapiClass,
  {
    let access = self.storage.access_for(T::Parent::CLASS.info())?;
    Ok(unsafe { self.storage.segment::<T::Parent>(access).as_ref() })
  }
}

pub struct ClassBorrowMut<'a, T: NapiClass> {
  storage: ClassStorageRef<'a>,
  value: RefMut<'a, T>,
}

impl<'a, T: NapiClass> ClassBorrowMut<'a, T> {
  #[doc(hidden)]
  pub unsafe fn from_validated_parts(
    storage: ClassStorageRef<'a>,
    access: ClassAccess,
  ) -> Result<Self> {
    let mut value = storage.segment(access);
    let value = RefMut::map(
      storage
        .scoped_state()
        .borrow_cell()
        .try_borrow_mut()
        .map_err(|_| {
          Error::new(
            Status::InvalidArg,
            "Class storage is already borrowed".to_owned(),
          )
        })?,
      |_| unsafe { value.as_mut() },
    );
    Ok(Self { storage, value })
  }

  pub fn as_super_mut(&mut self) -> Result<&mut T::Parent>
  where
    T::Parent: NapiClass,
  {
    let access = self.storage.access_for(T::Parent::CLASS.info())?;
    Ok(unsafe { self.storage.segment::<T::Parent>(access).as_mut() })
  }
}

impl<T: NapiClass> Deref for ClassBorrow<'_, T> {
  type Target = T;

  fn deref(&self) -> &Self::Target {
    &self.value
  }
}

impl<T: NapiClass> Deref for ClassBorrowMut<'_, T> {
  type Target = T;

  fn deref(&self) -> &Self::Target {
    &self.value
  }
}

impl<T: NapiClass> DerefMut for ClassBorrowMut<'_, T> {
  fn deref_mut(&mut self) -> &mut Self::Target {
    &mut self.value
  }
}

#[doc(hidden)]
pub unsafe fn drop_segment<T: 'static>(segment: NonNull<T>) {
  run_unwind_boundary("dropping class segment", || unsafe {
    std::ptr::drop_in_place(segment.as_ptr())
  });
}

/// Class with a known native storage layout and destructor chain.
///
/// # Safety
///
/// `write_init`, `drop_segments`, and `drop_initialized` must maintain the ABI described by
/// [`CLASS_STORAGE_ABI_MAGIC`] and must not be called on uninitialized storage.
pub unsafe trait ClassChain: NapiClass {
  type Layout;

  const LAYOUT: &'static ClassLayout;

  fn layout() -> &'static ClassLayout {
    Self::LAYOUT
  }

  /// # Safety
  ///
  /// `dst` must be uninitialized memory sized for [`Self::Layout`].
  unsafe fn write_init(init: ClassInitializer<Self>, dst: NonNull<Self::Layout>);

  /// # Safety
  ///
  /// `data` must point at storage initialized by [`Self::write_init`].
  unsafe fn drop_segments(data: NonNull<Self::Layout>);

  /// Drops class values in-place without deallocating the storage.
  ///
  /// # Safety
  ///
  /// `data` must point at the initialized payload region for this class chain.
  unsafe fn drop_initialized(data: NonNull<u8>);
}

pub struct ClassInitializer<T: NapiClass> {
  value: T,
  parent: <T::Parent as NativeParent>::Initializer,
}

impl<T> From<T> for ClassInitializer<T>
where
  T: NapiClass<Parent = ()>,
{
  fn from(value: T) -> Self {
    Self { value, parent: () }
  }
}

impl<T> ClassInitializer<T>
where
  T: NapiClass,
  T::Parent: NapiClass + NativeParent<Initializer = ClassInitializer<T::Parent>>,
{
  pub fn from_parent(parent: ClassInitializer<T::Parent>, value: T) -> Self {
    Self { value, parent }
  }
}

impl<T: NapiClass> ClassInitializer<T> {
  #[doc(hidden)]
  pub fn into_value_and_parent(self) -> (T, <T::Parent as NativeParent>::Initializer) {
    (self.value, self.parent)
  }
}

impl<'scope, T> IntoJs<'scope> for ClassInitializer<T>
where
  T: NapiClass + ClassChain,
{
  type Output = Object<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let raw = unsafe { T::CLASS.new_object_from_scope(scope, self)? };
    Ok(unsafe { Local::from_raw(raw) })
  }
}

pub trait IntoClassInitializer<T: NapiClass> {
  fn into_class_initializer(self) -> ClassInitializer<T>;
}

impl<T> IntoClassInitializer<T> for T
where
  T: NapiClass<Parent = ()>,
{
  fn into_class_initializer(self) -> ClassInitializer<T> {
    ClassInitializer::from(self)
  }
}

impl<T: NapiClass> IntoClassInitializer<T> for ClassInitializer<T> {
  fn into_class_initializer(self) -> ClassInitializer<T> {
    self
  }
}

fn combined_class_storage_layout(
  data_layout: alloc::Layout,
) -> Option<(alloc::Layout, usize, usize)> {
  let header_layout = alloc::Layout::new::<ClassStorageHeader>();
  let (combined, state_offset) = header_layout
    .extend(alloc::Layout::new::<ClassStorageState>())
    .ok()?;
  let (combined, data_offset) = combined.extend(data_layout).ok()?;
  Some((combined.pad_to_align(), state_offset, data_offset))
}

struct PendingClassStorageAllocation {
  header: NonNull<ClassStorageHeader>,
  combined_layout: alloc::Layout,
}

impl PendingClassStorageAllocation {
  fn new(
    layout: &'static ClassLayout,
    record: Weak<EnvRecord>,
    data_layout: alloc::Layout,
  ) -> Result<Self> {
    let (combined_layout, state_offset, data_offset) = combined_class_storage_layout(data_layout)
      .ok_or_else(|| {
      Error::new(
        Status::GenericFailure,
        "Class storage layout overflow".to_owned(),
      )
    })?;

    let base = NonNull::new(unsafe { alloc::alloc(combined_layout) }).ok_or_else(|| {
      Error::new(
        Status::GenericFailure,
        "Allocate class storage failed".to_owned(),
      )
    })?;

    let state_ptr = unsafe {
      NonNull::new_unchecked(base.as_ptr().add(state_offset).cast::<ClassStorageState>())
    };
    let data_ptr = unsafe { NonNull::new_unchecked(base.as_ptr().add(data_offset)) };

    unsafe { state_ptr.as_ptr().write(ClassStorageState::new(record)) };
    let header_ptr = base.cast::<ClassStorageHeader>();
    unsafe {
      header_ptr
        .as_ptr()
        .write(ClassStorageHeader::new(layout, data_ptr, state_ptr))
    };

    Ok(Self {
      header: header_ptr,
      combined_layout,
    })
  }

  fn data(&self) -> NonNull<u8> {
    unsafe { self.header.as_ref().data() }
  }

  fn into_header(self) -> NonNull<ClassStorageHeader> {
    let allocation = ManuallyDrop::new(self);
    allocation.header
  }
}

impl Drop for PendingClassStorageAllocation {
  fn drop(&mut self) {
    unsafe {
      let state = self.header.as_ref().state();
      ptr::drop_in_place(state.as_ptr());
      alloc::dealloc(self.header.as_ptr().cast(), self.combined_layout);
    }
  }
}

struct PendingClassStorage {
  header: Option<NonNull<ClassStorageHeader>>,
  id: u64,
}

thread_local! {
  static PENDING_CLASS_STORAGE: RefCell<Vec<PendingClassStorage>> =
    const { RefCell::new(Vec::new()) };
  static NEXT_PENDING_CLASS_STORAGE_ID: Cell<u64> = const { Cell::new(1) };
}

impl PendingClassStorage {
  unsafe fn new<T>(record: Weak<EnvRecord>, init: ClassInitializer<T>) -> Result<Self>
  where
    T: ClassChain,
  {
    let layout = T::layout();
    let data_layout =
      alloc::Layout::from_size_align(layout.size(), layout.align()).map_err(|_| {
        Error::new(
          Status::InvalidArg,
          format!(
            "Invalid class storage layout for {}",
            T::CLASS.info().rust_name()
          ),
        )
      })?;
    let allocation = PendingClassStorageAllocation::new(layout, deferred, data_layout)?;

    unsafe { T::write_init(init, allocation.data().cast()) };
    let header = allocation.into_header();

    Ok(Self {
      header: Some(header),
      id: NEXT_PENDING_CLASS_STORAGE_ID.with(|next| {
        let id = next.get();
        next.set(id.wrapping_add(1));
        id
      }),
    })
  }

  fn header(&self) -> NonNull<ClassStorageHeader> {
    self
      .header
      .expect("PendingClassStorage accessed after wrap")
  }

  fn layout(&self) -> &'static ClassLayout {
    unsafe { self.header().as_ref().layout() }
  }

  fn segment<T>(&self, class: &'static ClassInfo) -> Result<NonNull<T>> {
    let header = unsafe { self.header().as_ref() };
    let access = header.layout().find(class).ok_or_else(|| {
      Error::new(
        Status::InvalidArg,
        format!("Class storage does not contain {}", class.rust_name()),
      )
    })?;
    Ok(header.segment(access))
  }

  unsafe fn wrap(mut self, env: sys::napi_env, object: sys::napi_value) -> Result<()> {
    let header = self.header();
    check_status!(
      unsafe {
        sys::napi_wrap(
          env,
          object,
          header.as_ptr().cast(),
          Some(class_storage_finalize),
          ptr::null_mut(),
          ptr::null_mut(),
        )
      },
      "Wrap class storage failed"
    )?;

    self.header = None;
    Ok(())
  }
}

struct PendingClassStorageGuard {
  id: u64,
}

impl PendingClassStorageGuard {
  fn push(storage: PendingClassStorage) -> Self {
    let id = storage.id;
    PENDING_CLASS_STORAGE.with(|pending| pending.borrow_mut().push(storage));
    Self { id }
  }

  fn consumed(&self) -> bool {
    PENDING_CLASS_STORAGE
      .with(|pending| pending.borrow().iter().all(|storage| storage.id != self.id))
  }

  fn ensure_consumed(&self, js_name: &str) -> Result<()> {
    if self.consumed() {
      Ok(())
    } else {
      Err(Error::new(
        Status::InvalidArg,
        format!("Constructor of class `{js_name}` did not consume pending storage"),
      ))
    }
  }
}

impl Drop for PendingClassStorageGuard {
  fn drop(&mut self) {
    PENDING_CLASS_STORAGE.with(|pending| {
      let mut pending = pending.borrow_mut();
      if let Some(index) = pending.iter().rposition(|storage| storage.id == self.id) {
        pending.remove(index);
      }
    });
  }
}

impl Drop for PendingClassStorage {
  fn drop(&mut self) {
    if let Some(header) = self.header.take() {
      unsafe { drop_class_storage(header) };
    }
  }
}

unsafe extern "C" fn class_storage_finalize(
  _env: sys::napi_env,
  finalize_data: *mut std::ffi::c_void,
  _finalize_hint: *mut std::ffi::c_void,
) {
  if let Some(header) = NonNull::new(finalize_data.cast::<ClassStorageHeader>()) {
    unsafe { drop_class_storage(header) };
  }
}

unsafe fn drop_class_storage(header: NonNull<ClassStorageHeader>) {
  let header_ref = unsafe { header.as_ref() };
  let class_layout = header_ref.layout();
  let state = header_ref.state();

  let drop_completed = catch_unwind_boundary("dropping class storage", || {
    if header_ref.validate_abi() {
      unsafe { class_layout.drop_initialized(header_ref.data()) };
    }
  });
  if drop_completed.is_none() {
    return;
  }

  let data_layout = alloc::Layout::from_size_align(class_layout.size(), class_layout.align())
    .expect("class layout was valid at allocation time");
  let (combined_layout, _, _) = combined_class_storage_layout(data_layout)
    .expect("combined layout was valid at allocation time");

  unsafe {
    ptr::drop_in_place(state.as_ptr());
    alloc::dealloc(header.as_ptr().cast(), combined_layout);
  }
}
