#[cfg(all(feature = "napi4", not(feature = "noop")))]
use std::cell::Cell;
#[cfg(not(feature = "noop"))]
use std::collections::HashMap;
#[cfg(not(feature = "noop"))]
use std::collections::HashSet;
#[cfg(not(feature = "noop"))]
use std::ffi::CStr;
#[cfg(not(feature = "noop"))]
use std::ffi::{c_void, CString};
#[cfg(all(not(feature = "noop"), feature = "node_version_detect"))]
use std::mem::MaybeUninit;
#[cfg(not(feature = "noop"))]
use std::ptr;
#[cfg(all(not(feature = "noop"), feature = "node_version_detect"))]
use std::sync::OnceLock;
#[cfg(not(feature = "noop"))]
use std::sync::{
  atomic::{AtomicBool, AtomicUsize, Ordering},
  LazyLock, RwLock,
};

#[cfg(not(target_family = "wasm"))]
use linkme::distributed_slice;
#[cfg(not(feature = "noop"))]
use rustc_hash::FxBuildHasher;

#[cfg(not(feature = "noop"))]
use crate::bindgen_runtime::{
  ClassInfo, ClassKey, ClassStorageRef, EnvRecord, ErasedClassDef, NapiClass, NativeParent,
};
#[cfg(all(feature = "noop", not(target_family = "wasm")))]
use crate::bindgen_runtime::ErasedClassDef;
#[cfg(all(not(feature = "noop"), feature = "napi4"))]
use crate::Env;
#[cfg(all(not(feature = "noop"), feature = "node_version_detect"))]
use crate::NodeVersion;
#[cfg(not(feature = "noop"))]
use crate::{check_status, check_status_or_throw, JsError};
use crate::{sys, Property, Result};
#[cfg(not(feature = "noop"))]
use crate::{Error, Status};

// #[napi] fn
pub type ExportRegisterCallback = unsafe fn(sys::napi_env) -> Result<sys::napi_value>;
// #[napi(module_exports)] fn
pub type ExportRegisterHookCallback =
  unsafe fn(sys::napi_env, sys::napi_value) -> Result<sys::napi_value>;

#[cfg(all(not(feature = "noop"), feature = "node_version_detect"))]
pub static NODE_VERSION: OnceLock<NodeVersion> = OnceLock::new();

#[cfg(feature = "node_version_detect")]
pub static mut NODE_VERSION_MAJOR: u32 = 0;
#[cfg(feature = "node_version_detect")]
pub static mut NODE_VERSION_MINOR: u32 = 0;
#[cfg(feature = "node_version_detect")]
pub static mut NODE_VERSION_PATCH: u32 = 0;

#[cfg(not(feature = "noop"))]
type ModuleRegisterCallback =
  RwLock<Vec<(Option<&'static str>, (&'static str, ExportRegisterCallback))>>;

#[cfg(not(feature = "noop"))]
type ClassPropertyRegistry =
  HashMap<ClassKey, HashMap<Option<&'static str>, ClassRegistration, FxBuildHasher>, FxBuildHasher>;

#[cfg(not(feature = "noop"))]
struct ClassRegistration {
  class: ErasedClassDef,
  parent: Option<ErasedClassDef>,
  js_name: &'static str,
  props: Vec<Property>,
  hidden_constructor: sys::napi_callback,
  constructible: bool,
  implement_iterator: bool,
}

#[cfg(not(target_family = "wasm"))]
pub struct ClassStructDescriptor {
  pub class: fn() -> ErasedClassDef,
  pub parent: fn() -> Option<ErasedClassDef>,
  pub js_mod: Option<&'static str>,
  pub js_name: &'static str,
  pub hidden_constructor: sys::napi_callback,
  pub constructible: bool,
  pub implement_iterator: bool,
  pub props: fn() -> Vec<Property>,
}

#[cfg(not(target_family = "wasm"))]
pub struct ClassImplDescriptor {
  pub class: fn() -> ErasedClassDef,
  pub js_mod: Option<&'static str>,
  pub js_name_hint: &'static str,
  pub implement_iterator: bool,
  pub props: fn() -> Vec<Property>,
}

#[cfg(not(target_family = "wasm"))]
#[distributed_slice]
pub static CLASS_STRUCT_DESCRIPTORS: [ClassStructDescriptor];

#[cfg(not(target_family = "wasm"))]
#[distributed_slice]
pub static CLASS_IMPL_DESCRIPTORS: [ClassImplDescriptor];

#[cfg(not(feature = "noop"))]
#[derive(Clone)]
struct ClassMetadata {
  js_mod: Option<String>,
  name: String,
  parent: Option<String>,
  constructible: bool,
  iterator: bool,
  constructor: sys::napi_value,
}

#[cfg(not(feature = "noop"))]
struct StagedClassRegistration {
  js_mod: Option<&'static str>,
  js_name: &'static str,
  class: ErasedClassDef,
  constructor_ref: sys::napi_ref,
  exported_value: sys::napi_value,
  metadata: ClassMetadata,
}

#[cfg(not(feature = "noop"))]
#[derive(Clone, Copy, Eq, PartialEq)]
enum VisitState {
  Visiting,
  Visited,
}

// Stores class metadata registered by napi macros.
// Since class properties do not contain any napi_value, ModuleClassProperty is thread-safe.
// This structure is shared between the main JS thread and worker threads.
#[cfg(not(feature = "noop"))]
#[derive(Default)]
struct ModuleClassProperty(RwLock<ClassPropertyRegistry>);

#[cfg(not(feature = "noop"))]
unsafe impl Send for ModuleClassProperty {}
#[cfg(not(feature = "noop"))]
unsafe impl Sync for ModuleClassProperty {}

#[cfg(not(feature = "noop"))]
impl ModuleClassProperty {
  pub(crate) fn borrow_mut<F, R>(&self, f: F) -> R
  where
    F: FnOnce(&mut ClassPropertyRegistry) -> R,
  {
    let mut write_lock = self.0.write().unwrap();
    f(&mut write_lock)
  }

  #[cfg(target_family = "wasm")]
  fn borrow<F, R>(&self, f: F) -> R
  where
    F: FnOnce(&ClassPropertyRegistry) -> R,
  {
    let write_lock = self.0.read().unwrap();
    f(&write_lock)
  }
}

#[cfg(not(feature = "noop"))]
fn find_class_registration<'a>(
  classes: &[(Option<&'static str>, &'a ClassRegistration)],
  js_mod: Option<&'static str>,
  class: ErasedClassDef,
) -> Option<&'a ClassRegistration> {
  classes.iter().find_map(|(registered_mod, registration)| {
    (*registered_mod == js_mod && registration.class.key() == class.key()).then_some(*registration)
  })
}

#[cfg(not(feature = "noop"))]
fn visit_class_registration<'a>(
  classes: &[(Option<&'static str>, &'a ClassRegistration)],
  js_mod: Option<&'static str>,
  registration: &'a ClassRegistration,
  states: &mut HashMap<(Option<&'static str>, ClassKey), VisitState, FxBuildHasher>,
  ordered: &mut Vec<(Option<&'static str>, &'a ClassRegistration)>,
) -> Result<()> {
  let class = registration.class;
  let key = (js_mod, class.key());
  match states.get(&key).copied() {
    Some(VisitState::Visited) => return Ok(()),
    Some(VisitState::Visiting) => {
      return Err(Error::new(
        Status::InvalidArg,
        format!(
          "Native class inheritance cycle detected for `{}`",
          registration.js_name
        ),
      ));
    }
    None => {}
  }

  states.insert(key, VisitState::Visiting);
  if let Some(parent) = registration.parent {
    if !parent.info().subclassable() {
      return Err(Error::new(
        Status::InvalidArg,
        format!(
          "Native class parent `{}` is not subclassable",
          parent.info().rust_name()
        ),
      ));
    }
    let parent_registration =
      find_class_registration(classes, js_mod, parent).ok_or_else(|| {
        Error::new(
          Status::InvalidArg,
          format!(
            "Native class parent `{}` for `{}` is not registered in the same module scope",
            parent.info().rust_name(),
            registration.js_name,
          ),
        )
      })?;
    visit_class_registration(classes, js_mod, parent_registration, states, ordered)?;
  }

  states.insert(key, VisitState::Visited);
  ordered.push((js_mod, registration));
  Ok(())
}

#[cfg(not(feature = "noop"))]
fn ordered_class_registrations(
  registry: &ClassPropertyRegistry,
) -> Result<Vec<(Option<&'static str>, &ClassRegistration)>> {
  let mut classes = Vec::new();
  let mut keys = HashSet::new();
  for js_mods in registry.values() {
    for (js_mod, registration) in js_mods {
      let key = (*js_mod, registration.class.key());
      if !keys.insert(key) {
        return Err(Error::new(
          Status::InvalidArg,
          format!(
            "Duplicate native class registration for `{}`",
            registration.js_name
          ),
        ));
      }
      classes.push((*js_mod, registration));
    }
  }

  let mut states = HashMap::default();
  let mut ordered = Vec::with_capacity(classes.len());
  for (js_mod, registration) in &classes {
    visit_class_registration(&classes, *js_mod, registration, &mut states, &mut ordered)?;
  }
  Ok(ordered)
}

#[cfg(not(feature = "noop"))]
static MODULE_REGISTER_CALLBACK: LazyLock<ModuleRegisterCallback> = LazyLock::new(Default::default);
#[cfg(not(feature = "noop"))]
static MODULE_REGISTER_HOOK_CALLBACK: LazyLock<RwLock<Option<ExportRegisterHookCallback>>> =
  LazyLock::new(Default::default);
#[cfg(not(feature = "noop"))]
// Legacy WASM registration state. Non-WASM class registration is descriptor-driven.
static MODULE_CLASS_PROPERTIES: LazyLock<ModuleClassProperty> = LazyLock::new(Default::default);
#[cfg(not(feature = "noop"))]
static MODULE_COUNT: AtomicUsize = AtomicUsize::new(0);
#[cfg(not(feature = "noop"))]
static FIRST_MODULE_REGISTERED: AtomicBool = AtomicBool::new(false);
#[cfg(all(
  feature = "tokio_rt",
  not(target_family = "wasm"),
  not(feature = "noop")
))]
static ENV_CLEANUP_HOOK_ADDED: RwLock<bool> = RwLock::new(false);
#[cfg(all(feature = "napi4", not(feature = "noop")))]
pub(crate) static CUSTOM_GC_TSFN: std::sync::atomic::AtomicPtr<sys::napi_threadsafe_function__> =
  std::sync::atomic::AtomicPtr::new(ptr::null_mut());
#[cfg(all(feature = "napi4", not(feature = "noop")))]
pub(crate) static CUSTOM_GC_TSFN_DESTROYED: AtomicBool = AtomicBool::new(false);
thread_local! {
  #[cfg(all(feature = "napi4", not(feature = "noop")))]
  // Store thread id of the thread that created the CustomGC ThreadsafeFunction.
  pub(crate) static THREADS_CAN_ACCESS_ENV: Cell<bool> = const { Cell::new(false) };
}

#[cfg(not(feature = "noop"))]
#[inline]
fn wait_first_thread_registered() {
  while !FIRST_MODULE_REGISTERED.load(Ordering::SeqCst) {
    std::hint::spin_loop();
  }
}

#[cfg(not(feature = "noop"))]
#[doc(hidden)]
pub fn register_module_export(
  js_mod: Option<&'static str>,
  name: &'static str,
  cb: ExportRegisterCallback,
) {
  MODULE_REGISTER_CALLBACK
    .write()
    .expect("Register module export failed")
    .push((js_mod, (name, cb)));
}

#[cfg(feature = "noop")]
#[doc(hidden)]
pub fn register_module_export(
  _js_mod: Option<&'static str>,
  _name: &'static str,
  _cb: ExportRegisterCallback,
) {
}

#[cfg(not(feature = "noop"))]
#[doc(hidden)]
pub fn register_module_export_hook(cb: ExportRegisterHookCallback) {
  let mut inner = MODULE_REGISTER_HOOK_CALLBACK
    .write()
    .expect("Write MODULE_REGISTER_HOOK_CALLBACK failed");
  *inner = Some(cb);
}

#[cfg(feature = "noop")]
#[doc(hidden)]
pub fn register_module_export_hook(_cb: ExportRegisterHookCallback) {}

#[cfg(not(feature = "noop"))]
#[doc(hidden)]
pub fn register_napi_class<T>(
  js_mod: Option<&'static str>,
  js_name: &'static str,
  props: Vec<Property>,
  hidden_constructor: sys::napi_callback,
  constructible: bool,
  implement_iterator: bool,
) where
  T: NapiClass,
{
  // Kept for the WASM export-registration path. Non-WASM impl methods are
  // collected through ClassImplDescriptor instead.
  let class = T::CLASS.erase();
  let parent = <T::Parent as NativeParent>::erased_class_def();
  MODULE_CLASS_PROPERTIES.borrow_mut(|inner| {
    let val = inner.entry(class.key()).or_default();
    let val = val.entry(js_mod).or_insert_with(|| ClassRegistration {
      class,
      parent,
      js_name,
      props: Vec::new(),
      hidden_constructor: None,
      constructible,
      implement_iterator,
    });
    val.class = class;
    val.parent = parent;
    val.js_name = js_name;
    val.constructible |= constructible;
    if hidden_constructor.is_some() {
      val.hidden_constructor = hidden_constructor;
    }
    val.implement_iterator |= implement_iterator;
    val.props.extend(props);
  });
}

#[cfg(feature = "noop")]
#[doc(hidden)]
#[allow(unused_variables)]
pub fn register_napi_class<T>(
  js_mod: Option<&'static str>,
  js_name: &'static str,
  props: Vec<Property>,
  hidden_constructor: sys::napi_callback,
  constructible: bool,
  implement_iterator: bool,
) {
}

#[cfg(not(feature = "noop"))]
#[doc(hidden)]
pub fn register_napi_class_impl<T>(
  js_mod: Option<&'static str>,
  js_name: &'static str,
  props: Vec<Property>,
  implement_iterator: bool,
) where
  T: NapiClass,
{
  let class = T::CLASS.erase();
  let parent = <T::Parent as NativeParent>::erased_class_def();
  MODULE_CLASS_PROPERTIES.borrow_mut(|inner| {
    let val = inner.entry(class.key()).or_default();
    let val = val.entry(js_mod).or_insert_with(|| ClassRegistration {
      class,
      parent,
      js_name,
      props: Vec::new(),
      hidden_constructor: None,
      constructible: false,
      implement_iterator,
    });
    val.class = class;
    val.parent = parent;
    val.constructible |= props.iter().any(|prop| prop.is_ctor);
    val.implement_iterator |= implement_iterator;
    val.props.extend(props);
  });
}

#[cfg(feature = "noop")]
#[doc(hidden)]
#[allow(unused_variables)]
pub fn register_napi_class_impl<T>(
  js_mod: Option<&'static str>,
  js_name: &'static str,
  props: Vec<Property>,
  implement_iterator: bool,
) {
}

#[cfg(all(not(feature = "noop"), not(target_family = "wasm")))]
fn collect_class_registry_from_descriptors() -> ClassPropertyRegistry {
  let mut registry = ClassPropertyRegistry::default();

  for descriptor in CLASS_STRUCT_DESCRIPTORS {
    let class = (descriptor.class)();
    let parent = (descriptor.parent)();
    let registration = registry
      .entry(class.key())
      .or_default()
      .entry(descriptor.js_mod)
      .or_insert_with(|| ClassRegistration {
        class,
        parent,
        js_name: descriptor.js_name,
        props: Vec::new(),
        hidden_constructor: None,
        constructible: descriptor.constructible,
        implement_iterator: descriptor.implement_iterator,
      });

    registration.class = class;
    registration.parent = parent;
    registration.js_name = descriptor.js_name;
    registration.hidden_constructor = descriptor.hidden_constructor;
    registration.constructible |= descriptor.constructible;
    registration.implement_iterator |= descriptor.implement_iterator;
    registration.props.extend((descriptor.props)());
  }

  for descriptor in CLASS_IMPL_DESCRIPTORS {
    let class = (descriptor.class)();
    let props = (descriptor.props)();
    let has_constructor = props.iter().any(|prop| prop.is_ctor);
    let registration = registry
      .entry(class.key())
      .or_default()
      .entry(descriptor.js_mod)
      .or_insert_with(|| ClassRegistration {
        class,
        parent: None,
        js_name: descriptor.js_name_hint,
        props: Vec::new(),
        hidden_constructor: None,
        constructible: false,
        implement_iterator: descriptor.implement_iterator,
      });

    registration.class = class;
    registration.constructible |= has_constructor;
    registration.implement_iterator |= descriptor.implement_iterator;
    registration.props.extend(props);
  }

  registry
}

#[cfg(not(feature = "noop"))]
unsafe extern "C" fn class_has_instance(
  env: sys::napi_env,
  cbinfo: sys::napi_callback_info,
) -> sys::napi_value {
  let result = (|| -> Result<sys::napi_value> {
    let mut this = ptr::null_mut();
    let mut args = [ptr::null_mut(); 1];
    let mut argc = args.len();
    let mut data = ptr::null_mut::<c_void>();

    check_status!(
      unsafe {
        sys::napi_get_cb_info(
          env,
          cbinfo,
          &mut argc,
          args.as_mut_ptr(),
          &mut this,
          &mut data,
        )
      },
      "Read Symbol.hasInstance callback failed",
    )?;

    let class = unsafe { (data as *const ClassInfo).as_ref() }.ok_or_else(|| {
      crate::Error::new(
        crate::Status::InvalidArg,
        "Missing class metadata for Symbol.hasInstance".to_owned(),
      )
    })?;
    let value = args.first().copied().unwrap_or(ptr::null_mut());
    let instance = unsafe {
      crate::bindgen_runtime::EnvRecord::enter_scope(env, |scope| {
        Ok(ClassStorageRef::validate_raw_object(scope, value, class).is_ok())
      })
    }?;

    let mut js_bool = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_get_boolean(env, instance, &mut js_bool) },
      "Create Symbol.hasInstance result failed",
    )?;
    Ok(js_bool)
  })();

  result.unwrap_or_else(|error: crate::Error| {
    unsafe { JsError::from(error).throw_into(env) };
    ptr::null_mut()
  })
}

#[cfg(not(feature = "noop"))]
unsafe fn symbol_has_instance(env: sys::napi_env) -> Result<sys::napi_value> {
  let mut global = ptr::null_mut();
  check_status!(
    sys::napi_get_global(env, &mut global),
    "Get global object failed for Symbol.hasInstance",
  )?;

  let mut symbol = ptr::null_mut();
  check_status!(
    sys::napi_get_named_property(env, global, c"Symbol".as_ptr().cast(), &mut symbol),
    "Get Symbol constructor failed",
  )?;

  let mut has_instance = ptr::null_mut();
  check_status!(
    sys::napi_get_named_property(
      env,
      symbol,
      c"hasInstance".as_ptr().cast(),
      &mut has_instance,
    ),
    "Get Symbol.hasInstance failed",
  )?;

  Ok(has_instance)
}

#[cfg(not(feature = "noop"))]
unsafe fn install_has_instance(
  env: sys::napi_env,
  target: sys::napi_value,
  class: ErasedClassDef,
) -> Result<()> {
  let symbol = unsafe { symbol_has_instance(env) }?;
  let descriptor = sys::napi_property_descriptor {
    utf8name: ptr::null(),
    name: symbol,
    method: Some(class_has_instance),
    getter: None,
    setter: None,
    value: ptr::null_mut(),
    attributes: sys::PropertyAttributes::configurable,
    data: class.info() as *const ClassInfo as *mut c_void,
  };

  check_status!(
    unsafe { sys::napi_define_properties(env, target, 1, &descriptor) },
    "Install Symbol.hasInstance failed for class `{}`",
    class.info().js_name(),
  )
}

#[cfg(not(feature = "noop"))]
unsafe fn create_js_string(env: sys::napi_env, value: &str) -> Result<sys::napi_value> {
  let value = CString::new(value)?;
  let mut result = ptr::null_mut();
  check_status!(
    unsafe {
      sys::napi_create_string_utf8(
        env,
        value.as_ptr(),
        value.as_bytes().len() as isize,
        &mut result,
      )
    },
    "Create class metadata string failed",
  )?;
  Ok(result)
}

#[cfg(not(feature = "noop"))]
unsafe fn set_named_value(
  env: sys::napi_env,
  object: sys::napi_value,
  name: &str,
  value: sys::napi_value,
) -> Result<()> {
  let name = CString::new(name)?;
  check_status!(
    unsafe { sys::napi_set_named_property(env, object, name.as_ptr(), value) },
    "Set class metadata property failed",
  )
}

#[cfg(not(feature = "noop"))]
unsafe fn set_named_string(
  env: sys::napi_env,
  object: sys::napi_value,
  name: &str,
  value: &str,
) -> Result<()> {
  let value = unsafe { create_js_string(env, value) }?;
  unsafe { set_named_value(env, object, name, value) }
}

#[cfg(not(feature = "noop"))]
unsafe fn set_named_bool(
  env: sys::napi_env,
  object: sys::napi_value,
  name: &str,
  value: bool,
) -> Result<()> {
  let mut js_bool = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_get_boolean(env, value, &mut js_bool) },
    "Create class metadata bool failed",
  )?;
  unsafe { set_named_value(env, object, name, js_bool) }
}

#[cfg(not(feature = "noop"))]
unsafe fn install_class_metadata(
  env: sys::napi_env,
  exports: sys::napi_value,
  metadata: &[ClassMetadata],
) -> Result<()> {
  let mut array = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_create_array_with_length(env, metadata.len(), &mut array) },
    "Create class metadata array failed",
  )?;

  for (index, item) in metadata.iter().enumerate() {
    let mut object = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_object(env, &mut object) },
      "Create class metadata entry failed",
    )?;

    unsafe {
      if let Some(js_mod) = &item.js_mod {
        set_named_string(env, object, "module", js_mod)?;
      }
      set_named_string(env, object, "name", &item.name)?;
      if let Some(parent) = &item.parent {
        set_named_string(env, object, "parent", parent)?;
      }
      set_named_bool(env, object, "constructible", item.constructible)?;
      set_named_bool(env, object, "iterator", item.iterator)?;
      set_named_value(env, object, "constructor", item.constructor)?;
    }

    check_status!(
      unsafe { sys::napi_set_element(env, array, index as u32, object) },
      "Set class metadata entry failed",
    )?;
  }

  let name = CString::new("__napiClassMetadata")?;
  let descriptor = sys::napi_property_descriptor {
    utf8name: name.as_ptr(),
    name: ptr::null_mut(),
    method: None,
    getter: None,
    setter: None,
    value: array,
    attributes: sys::PropertyAttributes::configurable,
    data: ptr::null_mut(),
  };

  check_status!(
    unsafe { sys::napi_define_properties(env, exports, 1, &descriptor) },
    "Install class metadata failed",
  )
}

#[cfg(not(feature = "noop"))]
unsafe fn install_class_prototype_props(
  env: sys::napi_env,
  class_ptr: sys::napi_value,
  props: &[&Property],
  js_name: &str,
) -> Result<()> {
  if props.is_empty() {
    return Ok(());
  }

  let mut prototype = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_get_named_property(env, class_ptr, c"prototype".as_ptr(), &mut prototype) },
    "Get prototype failed for class `{}`",
    js_name,
  )?;

  let raw_props: Vec<_> = props.iter().map(|prop| prop.raw()).collect();
  check_status!(
    unsafe { sys::napi_define_properties(env, prototype, raw_props.len(), raw_props.as_ptr()) },
    "Define prototype properties failed for class `{}`",
    js_name,
  )
}

#[cfg(not(feature = "noop"))]
unsafe fn stage_class_registration(
  env: sys::napi_env,
  js_mod: Option<&'static str>,
  registration: &ClassRegistration,
) -> Result<Option<StagedClassRegistration>> {
  let class = registration.class;

  let js_name = registration.js_name;
  let hidden_constructor = registration.hidden_constructor.ok_or_else(|| {
    Error::new(
      Status::InvalidArg,
      format!("Native class `{js_name}` requires a hidden constructor"),
    )
  })?;
  let (ctor, own_props): (Vec<&Property>, Vec<&Property>) =
    registration.props.iter().partition(|prop| prop.is_ctor);
  let (static_props, prototype_props): (Vec<&Property>, Vec<&Property>) =
    own_props.iter().copied().partition(|prop| prop.is_static());

  let public_ctor = ctor
    .first()
    .map(|c| c.raw().method.unwrap())
    .unwrap_or(hidden_constructor);
  let class_ctor = if registration.constructible {
    public_ctor
  } else {
    hidden_constructor
  };
  let class_props: Vec<&Property> = if registration.constructible {
    static_props.clone()
  } else {
    Vec::new()
  };
  let raw_props: Vec<_> = class_props.iter().map(|prop| prop.raw()).collect();

  let js_class_name = unsafe { CStr::from_bytes_with_nul_unchecked(js_name.as_bytes()) };
  let mut class_ptr = ptr::null_mut();
  check_status!(
    unsafe {
      sys::napi_define_class(
        env,
        js_class_name.as_ptr(),
        js_name.len() as isize - 1,
        Some(class_ctor),
        ptr::null_mut(),
        raw_props.len(),
        raw_props.as_ptr(),
        &mut class_ptr,
      )
    },
    "Failed to register class `{}`",
    js_name,
  )?;

  unsafe { install_class_prototype_props(env, class_ptr, &prototype_props, js_name) }?;

  unsafe { install_has_instance(env, class_ptr, class) }?;

  let exported_value = if registration.constructible {
    class_ptr
  } else {
    let mut public_value = ptr::null_mut();
    check_status!(
      unsafe { sys::napi_create_object(env, &mut public_value) },
      "Create non-constructible class value failed for class `{}`",
      js_name,
    )?;
    let raw_static_props: Vec<_> = static_props.iter().map(|prop| prop.raw()).collect();
    if !raw_static_props.is_empty() {
      check_status!(
        unsafe {
          sys::napi_define_properties(
            env,
            public_value,
            raw_static_props.len(),
            raw_static_props.as_ptr(),
          )
        },
        "Define static properties failed for class `{}`",
        js_name,
      )?;
    }
    unsafe { install_has_instance(env, public_value, class) }?;
    public_value
  };

  let mut constructor_ref = ptr::null_mut();
  check_status!(
    unsafe { sys::napi_create_reference(env, class_ptr, 1, &mut constructor_ref) },
    "Create constructor reference failed for class `{}`",
    js_name,
  )?;

  Ok(Some(StagedClassRegistration {
    js_mod,
    js_name,
    class,
    constructor_ref,
    exported_value,
    metadata: ClassMetadata {
      js_mod: js_mod.map(|value| value.trim_end_matches('\0').to_string()),
      name: class.info().js_name().trim_end_matches('\0').to_string(),
      parent: registration
        .parent
        .map(|parent| parent.info().js_name().trim_end_matches('\0').to_string()),
      constructible: registration.constructible,
      iterator: registration.implement_iterator,
      constructor: class_ptr,
    },
  }))
}

#[cfg(not(feature = "noop"))]
unsafe fn stage_all_classes(
  env: sys::napi_env,
  registry: &ClassPropertyRegistry,
) -> Result<Vec<StagedClassRegistration>> {
  let ordered_classes = ordered_class_registrations(registry)?;
  let mut staged = Vec::with_capacity(ordered_classes.len());

  for (js_mod, class_registration) in &ordered_classes {
    match unsafe { stage_class_registration(env, *js_mod, class_registration) } {
      Ok(Some(item)) => staged.push(item),
      Ok(None) => {}
      Err(error) => {
        unsafe { rollback_staged_class_refs(env, &staged) };
        return Err(error);
      }
    }
  }

  Ok(staged)
}

#[cfg(not(feature = "noop"))]
unsafe fn rollback_staged_class_refs(env: sys::napi_env, staged: &[StagedClassRegistration]) {
  for item in staged {
    let status = unsafe { sys::napi_delete_reference(env, item.constructor_ref) };
    if status != sys::Status::napi_ok {
      eprintln!(
        "napi-rs: failed to rollback constructor reference for `{}`",
        item.js_name
      );
    }
  }
}

#[cfg(not(feature = "noop"))]
unsafe fn delete_named_property(env: sys::napi_env, object: sys::napi_value, name: &CStr) {
  let mut key = ptr::null_mut();
  let status = unsafe { sys::napi_create_string_utf8(env, name.as_ptr(), -1, &mut key) };
  if status != sys::Status::napi_ok {
    eprintln!("napi-rs: failed to create key while rolling back class registration");
    return;
  }

  let mut deleted = false;
  let status = unsafe { sys::napi_delete_property(env, object, key, &mut deleted) };
  if status != sys::Status::napi_ok {
    eprintln!("napi-rs: failed to rollback class registration property");
  }
}

#[cfg(not(feature = "noop"))]
unsafe fn rollback_staged_class_exports(
  env: sys::napi_env,
  exports: sys::napi_value,
  committed: &[(sys::napi_value, &'static str)],
  created_export_objects: &[String],
) {
  for (object, name) in committed.iter().rev() {
    let name = unsafe { CStr::from_bytes_with_nul_unchecked(name.as_bytes()) };
    unsafe { delete_named_property(env, *object, name) };
  }

  let metadata_name = c"__napiClassMetadata";
  unsafe { delete_named_property(env, exports, metadata_name) };

  for name in created_export_objects.iter().rev() {
    let Ok(name) = CString::new(name.as_str()) else {
      continue;
    };
    unsafe { delete_named_property(env, exports, &name) };
  }
}

#[cfg(not(feature = "noop"))]
unsafe fn module_exports_object(
  env: sys::napi_env,
  exports: sys::napi_value,
  exports_objects: &mut HashSet<String>,
  js_mod: Option<&'static str>,
) -> Result<(sys::napi_value, bool)> {
  let Some(js_mod_str) = js_mod else {
    return Ok((exports, false));
  };

  let mod_name = unsafe { CStr::from_bytes_with_nul_unchecked(js_mod_str.as_bytes()) };
  let mut exports_js_mod = ptr::null_mut();
  let created = !exports_objects.contains(js_mod_str);
  if created {
    check_status!(
      unsafe { sys::napi_create_object(env, &mut exports_js_mod) },
      "Create export JavaScript Object [{}] failed",
      js_mod_str,
    )?;
    check_status!(
      unsafe { sys::napi_set_named_property(env, exports, mod_name.as_ptr(), exports_js_mod) },
      "Set exports Object [{}] into exports object failed",
      js_mod_str,
    )?;
    exports_objects.insert(js_mod_str.to_string());
  } else {
    check_status!(
      unsafe { sys::napi_get_named_property(env, exports, mod_name.as_ptr(), &mut exports_js_mod) },
      "Get mod {} from exports failed",
      js_mod_str,
    )?;
  }

  Ok((exports_js_mod, created))
}

#[cfg(not(feature = "noop"))]
unsafe fn commit_staged_classes(
  env: sys::napi_env,
  exports: sys::napi_value,
  exports_objects: &mut HashSet<String>,
  staged: &[StagedClassRegistration],
) -> Result<()> {
  let mut committed = Vec::with_capacity(staged.len());
  let mut created_export_objects = Vec::new();

  for item in staged {
    let (export_object, created_export_object) =
      unsafe { module_exports_object(env, exports, exports_objects, item.js_mod) }?;
    if created_export_object {
      if let Some(js_mod) = item.js_mod {
        created_export_objects.push(js_mod.trim_end_matches('\0').to_string());
      }
    }

    let js_name = unsafe { CStr::from_bytes_with_nul_unchecked(item.js_name.as_bytes()) };
    if let Err(error) = check_status!(
      unsafe {
        sys::napi_set_named_property(env, export_object, js_name.as_ptr(), item.exported_value)
      },
      "Failed to register class `{}`",
      item.js_name,
    ) {
      unsafe { rollback_staged_class_exports(env, exports, &committed, &created_export_objects) };
      return Err(error);
    }

    committed.push((export_object, item.js_name));
  }

  let metadata = staged
    .iter()
    .map(|item| item.metadata.clone())
    .collect::<Vec<_>>();
  if let Err(error) = unsafe { install_class_metadata(env, exports, &metadata) } {
    unsafe { rollback_staged_class_exports(env, exports, &committed, &created_export_objects) };
    return Err(error);
  }

  let record = EnvRecord::acquire(env);
  if let Err(error) = record.with_data_mut(|data| {
    for item in staged {
      data
        .constructors_mut()
        .insert(item.class.key(), item.constructor_ref);
    }
  }) {
    unsafe { rollback_staged_class_exports(env, exports, &committed, &created_export_objects) };
    return Err(error);
  }

  Ok(())
}

#[cfg(all(target_family = "wasm", not(feature = "noop")))]
#[no_mangle]
unsafe extern "C" fn napi_register_wasm_v1(
  env: sys::napi_env,
  exports: sys::napi_value,
) -> sys::napi_value {
  unsafe { napi_register_module_v1(env, exports) }
}

#[cfg(not(feature = "noop"))]
#[no_mangle]
/// Register the n-api module exports.
///
/// # Safety
/// This method is meant to be called by Node.js while importing the n-api module.
/// Only call this method if the current module is **not** imported by a node-like runtime.
///
/// Arguments `env` and `exports` must **not** be null.
pub unsafe extern "C" fn napi_register_module_v1(
  env: sys::napi_env,
  exports: sys::napi_value,
) -> sys::napi_value {
  #[cfg(any(
    target_env = "msvc",
    all(not(target_family = "wasm"), feature = "dyn-symbols")
  ))]
  unsafe {
    sys::setup();
  }

  match unsafe {
    crate::bindgen_runtime::EnvRecord::enter_scope(env, |scope| {
      Ok(napi_register_module_v1_inner(scope.env().raw(), exports))
    })
  } {
    Ok(value) => value,
    Err(error) => {
      unsafe { JsError::from(error).throw_into(env) };
      ptr::null_mut()
    }
  }
}

unsafe fn napi_register_module_v1_inner(
  env: sys::napi_env,
  exports: sys::napi_value,
) -> sys::napi_value {
  #[cfg(feature = "node_version_detect")]
  {
    NODE_VERSION.get_or_init(|| {
      let mut node_version = MaybeUninit::uninit();
      check_status_or_throw!(
        env,
        unsafe { sys::napi_get_node_version(env, node_version.as_mut_ptr()) },
        "Failed to get node version"
      );
      let node_version = *node_version.assume_init();
      unsafe {
        NODE_VERSION_MAJOR = node_version.major;
        NODE_VERSION_MINOR = node_version.minor;
        NODE_VERSION_PATCH = node_version.patch;
      }
      NodeVersion {
        major: node_version.major,
        minor: node_version.minor,
        patch: node_version.patch,
        release: unsafe { CStr::from_ptr(node_version.release).to_str().unwrap() },
      }
    });
  }

  if MODULE_COUNT.fetch_add(1, Ordering::SeqCst) != 0 {
    wait_first_thread_registered();
  }

  let mut exports_objects: HashSet<String> = HashSet::default();

  {
    let mut register_callback = MODULE_REGISTER_CALLBACK
      .write()
      .expect("Write MODULE_REGISTER_CALLBACK in napi_register_module_v1 failed");
    register_callback
      .iter_mut()
      .fold(
        HashMap::<Option<&'static str>, Vec<(&'static str, ExportRegisterCallback)>>::new(),
        |mut acc, (js_mod, item)| {
          if let Some(k) = acc.get_mut(js_mod) {
            k.push(*item);
          } else {
            acc.insert(*js_mod, vec![*item]);
          }
          acc
        },
      )
      .iter()
      .for_each(|(js_mod, items)| {
        let mut exports_js_mod = ptr::null_mut();
        if let Some(js_mod_str) = js_mod {
          let mod_name_c_str =
            unsafe { CStr::from_bytes_with_nul_unchecked(js_mod_str.as_bytes()) };
          if exports_objects.contains(*js_mod_str) {
            check_status_or_throw!(
              env,
              unsafe {
                sys::napi_get_named_property(
                  env,
                  exports,
                  mod_name_c_str.as_ptr(),
                  &mut exports_js_mod,
                )
              },
              "Get mod {} from exports failed",
              js_mod_str,
            );
          } else {
            check_status_or_throw!(
              env,
              unsafe { sys::napi_create_object(env, &mut exports_js_mod) },
              "Create export JavaScript Object [{}] failed",
              js_mod_str
            );
            check_status_or_throw!(
              env,
              unsafe {
                sys::napi_set_named_property(env, exports, mod_name_c_str.as_ptr(), exports_js_mod)
              },
              "Set exports Object [{}] into exports object failed",
              js_mod_str
            );
            exports_objects.insert(js_mod_str.to_string());
          }
        }
        for (name, callback) in items {
          unsafe {
            let js_name = CStr::from_bytes_with_nul_unchecked(name.as_bytes());
            if let Err(e) = callback(env).and_then(|v| {
              let exported_object = if exports_js_mod.is_null() {
                exports
              } else {
                exports_js_mod
              };
              check_status!(
                sys::napi_set_named_property(env, exported_object, js_name.as_ptr(), v),
                "Failed to register export `{}`",
                name,
              )
            }) {
              JsError::from(e).throw_into(env)
            }
          }
        }
      });
  }

  {
    #[cfg(not(target_family = "wasm"))]
    let staged_classes = {
      let registry = collect_class_registry_from_descriptors();
      unsafe { stage_all_classes(env, &registry) }
    };

    #[cfg(target_family = "wasm")]
    let staged_classes =
      MODULE_CLASS_PROPERTIES.borrow(|inner| unsafe { stage_all_classes(env, inner) });

    match staged_classes {
      Ok(staged) => {
        if let Err(error) =
          unsafe { commit_staged_classes(env, exports, &mut exports_objects, &staged) }
        {
          unsafe { rollback_staged_class_refs(env, &staged) };
          unsafe { JsError::from(error).throw_into(env) };
          return ptr::null_mut();
        }
      }
      Err(error) => {
        unsafe { JsError::from(error).throw_into(env) };
        return ptr::null_mut();
      }
    }
  }

  let module_register_hook_callback = MODULE_REGISTER_HOOK_CALLBACK
    .read()
    .expect("Read MODULE_REGISTER_HOOK_CALLBACK failed");
  if let Some(cb) = module_register_hook_callback.as_ref() {
    if let Err(e) = cb(env, exports) {
      JsError::from(e).throw_into(env);
    }
  }

  #[cfg(feature = "napi4")]
  {
    create_custom_gc(env);
    #[cfg(feature = "tokio_rt")]
    {
      crate::env::start_async_runtime();
      #[cfg(not(target_family = "wasm"))]
      {
        let mut env_cleanup_hook_added = ENV_CLEANUP_HOOK_ADDED.write().unwrap();
        if !*env_cleanup_hook_added {
          check_status_or_throw!(
            env,
            unsafe { sys::napi_add_env_cleanup_hook(env, Some(thread_cleanup), ptr::null_mut()) },
            "Failed to add env cleanup hook"
          );
          *env_cleanup_hook_added = true;
          drop(env_cleanup_hook_added);
        }
      }
    }
  }

  #[cfg(all(feature = "tokio_rt", feature = "napi4", target_family = "wasm"))]
  check_status_or_throw!(
    env,
    unsafe {
      sys::napi_wrap(
        env,
        exports,
        std::ptr::null_mut(),
        Some(thread_cleanup),
        std::ptr::null_mut(),
        std::ptr::null_mut(),
      )
    },
    "Failed to add remove thread id cleanup hook"
  );

  FIRST_MODULE_REGISTERED.store(true, Ordering::SeqCst);
  exports
}

#[cfg(all(feature = "napi4", not(feature = "noop")))]
fn create_custom_gc(env: sys::napi_env) {
  if !FIRST_MODULE_REGISTERED.load(Ordering::SeqCst) {
    let mut custom_gc_fn = ptr::null_mut();
    check_status_or_throw!(
      env,
      unsafe {
        sys::napi_create_function(
          env,
          c"custom_gc".as_ptr(),
          9,
          Some(empty),
          ptr::null_mut(),
          &mut custom_gc_fn,
        )
      },
      "Create Custom GC Function in napi_register_module_v1 failed"
    );
    let mut async_resource_name = ptr::null_mut();
    check_status_or_throw!(
      env,
      unsafe {
        sys::napi_create_string_utf8(env, c"CustomGC".as_ptr(), 8, &mut async_resource_name)
      },
      "Create async resource string in napi_register_module_v1"
    );
    let mut custom_gc_tsfn = ptr::null_mut();
    check_status_or_throw!(
      env,
      unsafe {
        sys::napi_create_threadsafe_function(
          env,
          custom_gc_fn,
          ptr::null_mut(),
          async_resource_name,
          0,
          1,
          ptr::null_mut(),
          Some(custom_gc_finalize),
          ptr::null_mut(),
          Some(custom_gc),
          &mut custom_gc_tsfn,
        )
      },
      "Create Custom GC ThreadsafeFunction in napi_register_module_v1 failed"
    );
    check_status_or_throw!(
      env,
      unsafe { sys::napi_unref_threadsafe_function(env, custom_gc_tsfn) },
      "Unref Custom GC ThreadsafeFunction in napi_register_module_v1 failed"
    );
    CUSTOM_GC_TSFN.store(custom_gc_tsfn, Ordering::Relaxed);
  }

  THREADS_CAN_ACCESS_ENV.with(|cell| cell.set(true));
}

#[cfg(all(
  not(feature = "noop"),
  all(feature = "tokio_rt", feature = "napi4"),
  not(target_family = "wasm")
))]
unsafe extern "C" fn thread_cleanup(_data: *mut std::ffi::c_void) {
  if MODULE_COUNT.fetch_sub(1, Ordering::Relaxed) == 1 {
    crate::env::shutdown_async_runtime();
  }
}

#[cfg(all(
  not(feature = "noop"),
  all(feature = "tokio_rt", feature = "napi4"),
  target_family = "wasm"
))]
unsafe extern "C" fn thread_cleanup(
  _env: sys::napi_env,
  _id: *mut std::ffi::c_void,
  _data: *mut std::ffi::c_void,
) {
  if MODULE_COUNT.fetch_sub(1, Ordering::Relaxed) == 1 {
    crate::env::shutdown_async_runtime();
  }
}

#[cfg(all(feature = "napi4", not(feature = "noop")))]
#[allow(unused)]
unsafe extern "C" fn empty(env: sys::napi_env, info: sys::napi_callback_info) -> sys::napi_value {
  ptr::null_mut()
}

#[cfg(all(feature = "napi4", not(feature = "noop")))]
#[allow(unused_variables)]
unsafe extern "C" fn custom_gc_finalize(
  env: sys::napi_env,
  finalize_data: *mut std::ffi::c_void,
  finalize_hint: *mut std::ffi::c_void,
) {
  CUSTOM_GC_TSFN_DESTROYED.store(true, Ordering::SeqCst);
}

#[cfg(all(feature = "napi4", not(feature = "noop")))]
// recycle the ArrayBuffer/Buffer Reference if the ArrayBuffer/Buffer is not dropped on the main thread
unsafe extern "C" fn custom_gc(
  env: sys::napi_env,
  js_callback: sys::napi_value,
  _context: *mut std::ffi::c_void,
  data: *mut std::ffi::c_void,
) {
  if env.is_null() || js_callback.is_null() || data.is_null() {
    return;
  }

  let result = unsafe {
    crate::bindgen_runtime::EnvRecord::enter_scope(env, |scope| custom_gc_impl(scope.env(), data))
  };
  if let Err(error) = result {
    unsafe { JsError::from(error).throw_into(env) };
  }
}

#[cfg(all(feature = "napi4", not(feature = "noop")))]
fn custom_gc_impl(env_wrapper: &Env<'_>, data: *mut std::ffi::c_void) -> Result<()> {
  if THREADS_CAN_ACCESS_ENV.with(|cell| !cell.get()) {
    return Ok(());
  }
  let env = env_wrapper.raw();
  let mut ref_count = 0;
  check_status!(
    unsafe { sys::napi_reference_unref(env, data.cast(), &mut ref_count) },
    "Failed to unref Buffer reference in Custom GC"
  )?;
  debug_assert!(
    ref_count == 0,
    "Buffer reference count in Custom GC is not 0"
  );
  check_status!(
    unsafe { sys::napi_delete_reference(env, data.cast()) },
    "Failed to delete Buffer reference in Custom GC"
  )?;
  Ok(())
}
