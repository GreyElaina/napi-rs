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
#[cfg(not(feature = "noop"))]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(not(feature = "noop"))]
use std::sync::Once;
#[cfg(all(not(feature = "noop"), feature = "node_version_detect"))]
use std::sync::OnceLock;

use linkme::distributed_slice;
#[cfg(not(feature = "noop"))]
use rustc_hash::FxBuildHasher;

#[cfg(feature = "noop")]
use crate::bindgen_runtime::ErasedClassDef;
#[cfg(not(feature = "noop"))]
use crate::bindgen_runtime::{ClassInfo, ClassKey, ClassStorageRef, EnvRecord, ErasedClassDef};
#[cfg(all(not(feature = "noop"), feature = "node_version_detect"))]
use crate::NodeVersion;
#[cfg(not(feature = "noop"))]
use crate::{check_status, JsError};
#[cfg(all(not(feature = "noop"), feature = "node_version_detect"))]
use crate::check_status_or_throw;
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

pub struct ClassImplDescriptor {
  pub class: fn() -> ErasedClassDef,
  pub js_mod: Option<&'static str>,
  pub js_name_hint: &'static str,
  pub implement_iterator: bool,
  pub props: fn() -> Vec<Property>,
}

pub struct ModuleExportDescriptor {
  pub js_mod: Option<&'static str>,
  pub js_name: &'static str,
  pub callback: ExportRegisterCallback,
}

pub struct ModuleExportHookDescriptor {
  pub callback: ExportRegisterHookCallback,
}

pub struct ModuleInitDescriptor {
  pub init: fn(),
}

#[distributed_slice]
pub static CLASS_STRUCT_DESCRIPTORS: [ClassStructDescriptor];

#[distributed_slice]
pub static CLASS_IMPL_DESCRIPTORS: [ClassImplDescriptor];

#[distributed_slice]
pub static MODULE_EXPORT_DESCRIPTORS: [ModuleExportDescriptor];

#[distributed_slice]
pub static MODULE_EXPORT_HOOK_DESCRIPTORS: [ModuleExportHookDescriptor];

#[distributed_slice]
pub static MODULE_INIT_DESCRIPTORS: [ModuleInitDescriptor];

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
struct ModuleRegistration {
  env: sys::napi_env,
  exports: sys::napi_value,
  export_objects: HashSet<String>,
  committed_exports: Vec<(sys::napi_value, &'static str)>,
  created_export_objects: Vec<String>,
  metadata_installed: bool,
  committed_constructor_refs: Vec<(ClassKey, sys::napi_ref)>,
}

#[cfg(not(feature = "noop"))]
impl ModuleRegistration {
  fn new(env: sys::napi_env, exports: sys::napi_value) -> Self {
    Self {
      env,
      exports,
      export_objects: HashSet::default(),
      committed_exports: Vec::new(),
      created_export_objects: Vec::new(),
      metadata_installed: false,
      committed_constructor_refs: Vec::new(),
    }
  }

  unsafe fn export_object(
    &mut self,
    js_mod: Option<&'static str>,
  ) -> Result<(sys::napi_value, bool)> {
    let Some(js_mod_str) = js_mod else {
      return Ok((self.exports, false));
    };

    let mod_name = unsafe { CStr::from_bytes_with_nul_unchecked(js_mod_str.as_bytes()) };
    let mut exports_js_mod = ptr::null_mut();
    let created = !self.export_objects.contains(js_mod_str);
    if created {
      check_status!(
        unsafe { sys::napi_create_object(self.env, &mut exports_js_mod) },
        "Create export JavaScript Object [{}] failed",
        js_mod_str,
      )?;
      check_status!(
        unsafe {
          sys::napi_set_named_property(self.env, self.exports, mod_name.as_ptr(), exports_js_mod)
        },
        "Set exports Object [{}] into exports object failed",
        js_mod_str,
      )?;
      self.export_objects.insert(js_mod_str.to_string());
      self
        .created_export_objects
        .push(js_mod_str.trim_end_matches('\0').to_string());
    } else {
      check_status!(
        unsafe {
          sys::napi_get_named_property(
            self.env,
            self.exports,
            mod_name.as_ptr(),
            &mut exports_js_mod,
          )
        },
        "Get mod {} from exports failed",
        js_mod_str,
      )?;
    }

    Ok((exports_js_mod, created))
  }

  unsafe fn set_export(
    &mut self,
    js_mod: Option<&'static str>,
    js_name: &'static str,
    value: sys::napi_value,
  ) -> Result<(sys::napi_value, bool)> {
    let (export_object, created_export_object) = unsafe { self.export_object(js_mod) }?;
    let name = unsafe { CStr::from_bytes_with_nul_unchecked(js_name.as_bytes()) };
    check_status!(
      unsafe { sys::napi_set_named_property(self.env, export_object, name.as_ptr(), value) },
      "Failed to register export `{}`",
      js_name,
    )?;
    self.committed_exports.push((export_object, js_name));
    Ok((export_object, created_export_object))
  }

  unsafe fn register_exports(&mut self) -> Result<()> {
    for descriptor in MODULE_EXPORT_DESCRIPTORS {
      let value = unsafe { (descriptor.callback)(self.env) }?;
      unsafe { self.set_export(descriptor.js_mod, descriptor.js_name, value) }?;
    }

    Ok(())
  }

  unsafe fn rollback(&mut self) {
    for (key, reference) in self.committed_constructor_refs.drain(..).rev() {
      let record = EnvRecord::acquire(self.env);
      if let Err(error) = record.with_data_mut(|data| {
        data.constructors_mut().remove(&key);
      }) {
        eprintln!("napi-rs: failed to rollback constructor record: {error:?}");
      }

      let status = unsafe { sys::napi_delete_reference(self.env, reference) };
      if status != sys::Status::napi_ok {
        eprintln!("napi-rs: failed to rollback constructor reference");
      }
    }

    for (object, name) in self.committed_exports.drain(..).rev() {
      let name = unsafe { CStr::from_bytes_with_nul_unchecked(name.as_bytes()) };
      unsafe { delete_named_property(self.env, object, name) };
    }

    if self.metadata_installed {
      let metadata_name = c"__napiClassMetadata";
      unsafe { delete_named_property(self.env, self.exports, metadata_name) };
      self.metadata_installed = false;
    }

    for name in self.created_export_objects.drain(..).rev() {
      let Ok(name) = CString::new(name.as_str()) else {
        continue;
      };
      unsafe { delete_named_property(self.env, self.exports, &name) };
    }

    self.export_objects.clear();
  }

  unsafe fn commit_classes(&mut self, staged: &[StagedClassRegistration]) -> Result<()> {
    for item in staged {
      unsafe { self.set_export(item.js_mod, item.js_name, item.exported_value) }?;
    }

    let metadata = staged
      .iter()
      .map(|item| item.metadata.clone())
      .collect::<Vec<_>>();
    unsafe { install_class_metadata(self.env, self.exports, &metadata) }?;
    self.metadata_installed = true;

    let record = EnvRecord::acquire(self.env);
    record.with_data_mut(|data| {
      for item in staged {
        data
          .constructors_mut()
          .insert(item.class.key(), item.constructor_ref);
      }
    })?;
    self.committed_constructor_refs = staged
      .iter()
      .map(|item| (item.class.key(), item.constructor_ref))
      .collect();

    Ok(())
  }

  unsafe fn run_export_hook(&mut self) -> Result<()> {
    match MODULE_EXPORT_HOOK_DESCRIPTORS.len() {
      0 => Ok(()),
      1 => {
        let cb = MODULE_EXPORT_HOOK_DESCRIPTORS[0].callback;
        unsafe { cb(self.env, self.exports) }.map(|_| ())
      }
      _ => Err(Error::new(
        Status::InvalidArg,
        "Duplicate module_exports registration".to_owned(),
      )),
    }
  }
}

#[cfg(not(feature = "noop"))]
#[derive(Clone, Copy, Eq, PartialEq)]
enum VisitState {
  Visiting,
  Visited,
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
static MODULE_COUNT: AtomicUsize = AtomicUsize::new(0);
#[cfg(not(feature = "noop"))]
static FIRST_MODULE_REGISTERED: AtomicBool = AtomicBool::new(false);
#[cfg(not(feature = "noop"))]
static MODULE_INIT_ONCE: Once = Once::new();

#[cfg(not(feature = "noop"))]
#[inline]
fn wait_first_thread_registered() {
  while !FIRST_MODULE_REGISTERED.load(Ordering::SeqCst) {
    std::hint::spin_loop();
  }
}

#[cfg(not(feature = "noop"))]
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
  #[cfg(any(target_env = "msvc", feature = "dyn-symbols"))]
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
  MODULE_INIT_ONCE.call_once(|| {
    for descriptor in MODULE_INIT_DESCRIPTORS {
      (descriptor.init)();
    }
  });

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

  let mut registration = ModuleRegistration::new(env, exports);

  if let Err(error) = unsafe { registration.register_exports() } {
    unsafe { registration.rollback() };
    unsafe { JsError::from(error).throw_into(env) };
    return ptr::null_mut();
  }

  {
    let staged_classes = {
      let registry = collect_class_registry_from_descriptors();
      unsafe { stage_all_classes(env, &registry) }
    };

    match staged_classes {
      Ok(staged) => {
        if let Err(error) = unsafe { registration.commit_classes(&staged) } {
          unsafe { registration.rollback() };
          unsafe { rollback_staged_class_refs(env, &staged) };
          unsafe { JsError::from(error).throw_into(env) };
          return ptr::null_mut();
        }
      }
      Err(error) => {
        unsafe { registration.rollback() };
        unsafe { JsError::from(error).throw_into(env) };
        return ptr::null_mut();
      }
    }
  }

  if let Err(error) = unsafe { registration.run_export_hook() } {
    unsafe { registration.rollback() };
    unsafe { JsError::from(error).throw_into(env) };
    return ptr::null_mut();
  }

  FIRST_MODULE_REGISTERED.store(true, Ordering::SeqCst);
  exports
}

