use napi::{
  bindgen_prelude::{
    Buffer, Class, ClassBorrow, ClassBorrowMut, ClassInitializer, Function, JsObjectValue,
    JsValue, Promise, Ref, This, Unknown,
  },
  Env, Property, Result,
};

use crate::r#enum::Kind;

/// `constructor` option for `struct` requires all fields to be public,
/// otherwise tag impl fn as constructor
/// #[napi(constructor)]
#[napi]
pub struct Animal {
  #[napi(readonly)]
  /// Kind of animal
  pub kind: Kind,

  name: String,

  optional_value: Option<i32>,
}

#[napi]
impl Animal {
  /// This is the constructor
  #[napi(constructor)]
  pub fn new(kind: Kind, name: String) -> Self {
    Animal {
      kind,
      name,
      optional_value: None,
    }
  }

  /// This is a factory method
  #[napi(factory)]
  pub fn with_kind(kind: Kind) -> Self {
    Animal {
      kind,
      name: "Default".to_owned(),
      optional_value: None,
    }
  }

  #[napi(getter)]
  pub fn get_name(&self) -> &str {
    self.name.as_str()
  }

  #[napi(setter)]
  pub fn set_name(&mut self, name: String) {
    self.name = name;
  }

  #[napi(getter, js_name = "type")]
  pub fn kind(&self) -> Kind {
    self.kind
  }

  #[napi(setter, js_name = "type")]
  pub fn set_kind(&mut self, kind: Kind) {
    self.kind = kind;
  }

  #[napi(getter)]
  pub fn get_optional_value(&self) -> Option<i32> {
    self.optional_value
  }

  /// This is to test that setter with optional parameter generates valid TypeScript.
  /// TypeScript does not allow optional parameters in setters (TS1051).
  #[napi(setter)]
  pub fn set_optional_value(&mut self, value: Option<i32>) {
    self.optional_value = value;
  }

  /// This is a
  /// multi-line comment
  /// with an emoji 🚀
  #[napi]
  pub fn whoami(&self) -> String {
    match self.kind {
      Kind::Dog => {
        format!("Dog: {}", self.name)
      }
      Kind::Cat => format!("Cat: {}", self.name),
      Kind::Duck => format!("Duck: {}", self.name),
    }
  }

  #[napi]
  /// This is static...
  pub fn get_dog_kind() -> Kind {
    Kind::Dog
  }

  #[napi]
  /// Here are some characters and character sequences
  /// that should be escaped correctly:
  /// \[]{}/\:""{
  /// }
  /// Accept header "*/json" should not break the comment block
  pub fn return_other_class(&self) -> ClassInitializer<Dog> {
    ClassInitializer::from(Dog {
      name: "Doge".to_owned(),
    })
  }

  #[napi]
  pub fn return_other_class_with_custom_constructor(&self) -> ClassInitializer<Bird> {
    ClassInitializer::from(Bird::new("parrot".to_owned()))
  }

  #[napi]
  pub fn override_individual_arg_on_method(
    &self,
    #[napi(env)] mut env: Env,
    normal_ty: String,
    #[napi(ts_arg_type = "{n: string}")] overridden_ty: napi::bindgen_prelude::Object,
  ) -> ClassInitializer<Bird> {
    let obj = overridden_ty.coerce_to_object().unwrap();
    let the_n = env
      .with_scope(|scope| scope.get_optional_named_property::<String, _>(&obj, "n"))
      .unwrap();

    ClassInitializer::from(Bird::new(format!("{}-{}", normal_ty, the_n.unwrap())))
  }
}

#[napi(constructor)]
pub struct Dog {
  pub name: String,
}

#[cfg_attr(not(feature = "cfg_attr_napi"), napi_derive::napi)]
pub struct Bird {
  pub name: String,
}

#[cfg_attr(not(feature = "cfg_attr_napi"), napi_derive::napi)]
impl Bird {
  #[cfg_attr(not(feature = "cfg_attr_napi"), napi_derive::napi(constructor))]
  pub fn new(name: String) -> Self {
    Bird { name }
  }

  #[cfg_attr(not(feature = "cfg_attr_napi"), napi_derive::napi)]
  pub fn get_count(&self) -> u32 {
    1234
  }

  #[cfg_attr(not(feature = "cfg_attr_napi"), napi_derive::napi)]
  pub fn get_name_async<'env>(
    &self,
    #[napi(env)] env: &'env Env<'env>,
  ) -> Result<Promise<'env, String>> {
    let name = self.name.clone();
    env.spawn_future(async move {
      tokio::time::sleep(std::time::Duration::new(1, 0)).await;
      Ok(name)
    })
  }

  #[cfg_attr(not(feature = "cfg_attr_napi"), napi_derive::napi)]
  pub fn accept_slice_method(&self, slice: &[u8]) -> u32 {
    slice.len() as u32
  }
}

/// Smoking test for type generation
#[napi]
#[repr(transparent)]
pub struct Blake2bHasher(u32);

#[napi]
impl Blake2bHasher {
  #[napi(factory)]
  pub fn with_key(key: &Blake2bKey) -> Self {
    Blake2bHasher(key.get_inner())
  }
}

#[napi]
impl Blake2bHasher {
  #[napi]
  pub fn update(&mut self, data: Buffer) {
    self.0 += data.len() as u32;
  }
}

#[napi]
pub struct Blake2bKey(u32);

impl Blake2bKey {
  fn get_inner(&self) -> u32 {
    self.0
  }
}

#[napi]
pub struct Context {
  data: String,
  pub maybe_need: Option<bool>,
}

// Test for return `napi::Result` and `Result`
#[napi]
impl Context {
  #[napi(constructor)]
  pub fn new() -> napi::Result<Self> {
    Ok(Self {
      data: "not empty".into(),
      maybe_need: None,
    })
  }

  #[napi(factory)]
  pub fn with_data(data: String) -> Result<Self> {
    Ok(Self {
      data,
      maybe_need: Some(true),
    })
  }

  #[napi]
  pub fn method(&self) -> String {
    self.data.clone()
  }
}

#[napi(constructor)]
pub struct AnimalWithDefaultConstructor {
  pub name: String,
  pub kind: u32,
}

// Test for skip_typescript
#[napi]
pub struct NinjaTurtle {
  pub name: String,
  #[napi(skip_typescript)]
  pub mask_color: String,
}

#[napi]
impl NinjaTurtle {
  #[napi]
  pub fn is_instance_of(#[napi(env)] mut env: Env, value: Unknown) -> Result<bool> {
    env.with_scope(|scope| scope.is_class_value::<Self, _>(&value))
  }

  /// Create your ninja turtle! 🐢
  #[napi(factory)]
  pub fn new_raph() -> Self {
    Self {
      name: "Raphael".to_owned(),
      mask_color: "Red".to_owned(),
    }
  }

  /// We are not going to expose this character, so we just skip it...
  #[napi(factory, skip_typescript)]
  pub fn new_leo() -> Self {
    Self {
      name: "Leonardo".to_owned(),
      mask_color: "Blue".to_owned(),
    }
  }

  #[napi]
  pub fn get_mask_color(&self) -> &str {
    self.mask_color.as_str()
  }

  #[napi]
  pub fn get_name(&self) -> &str {
    self.name.as_str()
  }

  #[napi]
  pub fn return_this<'scope>(&'scope self, #[napi(this)] this: This<'scope>) -> This<'scope> {
    this
  }
}

#[napi(js_name = "Assets")]
pub struct JsAssets {}

#[napi]
impl JsAssets {
  #[napi(constructor)]
  #[allow(clippy::new_without_default)]
  pub fn new() -> Self {
    JsAssets {}
  }

  #[napi]
  pub fn get(&mut self, _id: u32) -> Option<ClassInitializer<JsAsset>> {
    Some(ClassInitializer::from(JsAsset {}))
  }
}

#[napi(js_name = "Asset")]
pub struct JsAsset {}

#[napi]
impl JsAsset {
  #[napi(constructor)]
  #[allow(clippy::new_without_default)]
  pub fn new() -> Self {
    Self {}
  }

  #[napi(getter)]
  pub fn get_file_path(&self) -> u32 {
    1
  }
}

#[napi]
pub struct Optional {}

#[napi]
impl Optional {
  #[napi]
  pub fn option_end(required: String, optional: Option<String>) -> String {
    match optional {
      None => required,
      Some(optional) => format!("{} {}", required, optional),
    }
  }

  #[napi]
  pub fn option_start(optional: Option<String>, required: String) -> String {
    match optional {
      None => required,
      Some(optional) => format!("{} {}", optional, required),
    }
  }

  #[napi]
  pub fn option_start_end(
    optional1: Option<String>,
    required: String,
    optional2: Option<String>,
  ) -> String {
    match (optional1, optional2) {
      (None, None) => required,
      (None, Some(optional2)) => format!("{} {}", required, optional2),
      (Some(optional1), None) => format!("{} {}", optional1, required),
      (Some(optional1), Some(optional2)) => format!("{} {} {}", optional1, required, optional2),
    }
  }

  #[napi]
  pub fn option_only(optional: Option<String>) -> String {
    match optional {
      None => "".to_string(),
      Some(optional) => optional,
    }
  }
}

#[napi(object)]
pub struct ObjectFieldClassReference {
  pub bird: Ref<Class<Bird>>,
}

#[napi]
pub fn create_object_with_class_field(
  #[napi(env)] mut env: Env,
) -> Result<ObjectFieldClassReference> {
  env.with_scope(|scope| {
    Ok(ObjectFieldClassReference {
      bird: scope.reference(Bird {
        name: "Carolyn".to_owned(),
      })?,
    })
  })
}

#[napi]
pub fn receive_object_with_class_field(
  object: ObjectFieldClassReference,
) -> Result<Ref<Class<Bird>>> {
  Ok(object.bird)
}

#[napi(subclass)]
pub struct RendererNode {
  id: u32,
}

impl RendererNode {
  pub fn new(id: u32) -> Self {
    Self { id }
  }
}

#[napi]
impl RendererNode {
  #[napi]
  pub fn id(&self) -> u32 {
    self.id
  }

  #[napi]
  pub fn node_kind(&self) -> &'static str {
    "renderer"
  }

  #[napi]
  pub fn receiver_id(#[napi(this)] this: ClassBorrow<Self>) -> u32 {
    this.id
  }

  #[napi]
  pub fn owned_receiver_id(
    #[napi(env)] mut env: Env,
    #[napi(this)] this: Ref<Class<Self>>,
  ) -> Result<u32> {
    env.with_scope(|scope| {
      let this = this.as_class_local(scope)?;
      let this = this.borrow()?;
      Ok(this.id)
    })
  }

  #[napi]
  pub fn env_mut_marker(&self, #[napi(env)] env: &mut Env) -> Result<bool> {
    env.with_scope(|scope| {
      scope.env().get_global()?;
      Ok(true)
    })
  }

  #[napi]
  pub fn set_id_from_receiver(#[napi(this)] mut this: ClassBorrowMut<Self>, id: u32) {
    this.id = id;
  }

  #[napi]
  pub fn has_same_id(&self, other: ClassBorrow<Self>) -> bool {
    self.id == other.id
  }

  #[napi]
  pub fn has_same_id_ref(&self, other: &Self) -> bool {
    self.id == other.id
  }
}

#[napi(extends = RendererNode, subclass)]
pub struct ImageNode {
  width: u32,
}

#[napi]
impl ImageNode {
  #[napi(constructor)]
  pub fn new(id: u32, width: u32) -> ClassInitializer<Self> {
    ClassInitializer::from_parent(
      ClassInitializer::from(RendererNode::new(id)),
      Self { width },
    )
  }

  #[napi(getter)]
  pub fn width(&self) -> u32 {
    self.width
  }

  #[napi(setter)]
  pub fn set_width(&mut self, width: u32) {
    self.width = width;
  }

  #[napi]
  pub fn image_kind(&self) -> &'static str {
    "image"
  }

  #[napi]
  pub fn set_super_id(#[napi(this)] mut this: ClassBorrowMut<Self>, id: u32) -> Result<()> {
    this.as_super_mut()?.id = id;
    Ok(())
  }
}

#[napi(extends = ImageNode)]
pub struct PngImageNode {
  height: u32,
}

#[napi]
impl PngImageNode {
  #[napi(constructor)]
  pub fn new(id: u32, width: u32, height: u32) -> ClassInitializer<Self> {
    ClassInitializer::from_parent(ImageNode::new(id, width), Self { height })
  }

  #[napi(getter)]
  pub fn height(&self) -> u32 {
    self.height
  }
}

#[napi(constructor)]
pub struct NotWritableClass {
  #[napi(writable = false)]
  pub name: String,
}

#[napi]
impl NotWritableClass {
  #[napi(writable = false)]
  pub fn set_name(&mut self, name: String) {
    self.name = name;
  }
}

#[napi]
pub struct CustomFinalize {
  width: u32,
  height: u32,
  inner: Vec<u8>,
}

#[napi]
impl CustomFinalize {
  #[napi(constructor)]
  pub fn new(width: u32, height: u32) -> Self {
    let inner = vec![0; (width * height * 4) as usize];
    Self {
      width,
      height,
      inner,
    }
  }
}

#[napi(constructor)]
pub struct Width {
  pub value: i32,
}

#[napi(constructor)]
pub struct SelfReferenceField {
  pub next: Option<Ref<Class<Self>>>,
}

#[napi]
impl SelfReferenceField {
  #[napi]
  pub fn current(#[napi(this)] this: Ref<Class<Self>>) -> Ref<Class<Self>> {
    this
  }

  #[napi]
  pub fn maybe_next(&self, #[napi(env)] mut env: Env) -> Result<Option<Ref<Class<Self>>>> {
    env.with_scope(|scope| match &self.next {
      Some(next) => next.clone(scope).map(Some),
      None => Ok(None),
    })
  }

  #[napi]
  pub fn new_detached() -> ClassInitializer<Self> {
    ClassInitializer::from(Self { next: None })
  }
}

#[napi]
pub fn plus_one(#[napi(env)] mut env: Env, #[napi(this)] this: Ref<Class<Width>>) -> Result<i32> {
  env.with_scope(|scope| {
    let bound = this.as_class_local(scope)?;
    let value = bound.borrow()?.value;
    Ok(value + 1)
  })
}

#[napi]
pub struct GetterSetterWithClosures {}

#[napi]
impl GetterSetterWithClosures {
  #[napi(constructor)]
  pub fn new(#[napi(env)] env: &Env, #[napi(this)] mut this: This) -> Result<Self> {
    let age_symbol = env.create_symbol(Some("age"))?;

    this.define_properties(&[
      Property::new()
        .with_utf8_name("name")?
        .with_setter_closure(move |_env, mut this, value: String| {
          this.set_named_property("_name", format!("I'm {}", value))?;
          Ok(())
        })
        .with_getter_closure(|mut env, this| {
          env.with_scope(|scope| scope.get_named_property::<String, _>(&this, "_name"))
        }),
      Property::new()
        .with_utf8_name("age")?
        .with_getter_closure(|_env, _this| Ok(0.3)),
      Property::new()
        .with_name(env, age_symbol)?
        .with_getter_closure(|_env, _this| Ok(0.3)),
    ])?;

    this.set_property(env.create_string("ageSymbol")?, age_symbol)?;
    Ok(Self {})
  }
}

#[napi]
pub struct CatchOnConstructor {}

#[napi]
impl CatchOnConstructor {
  #[napi(constructor, catch_unwind)]
  pub fn new() -> Self {
    Self {}
  }
}

#[napi]
pub struct CatchOnConstructor2 {}

#[napi]
impl CatchOnConstructor2 {
  #[napi(constructor, catch_unwind)]
  pub fn new() -> Self {
    panic!("CatchOnConstructor2 panic");
  }
}

#[napi(js_name = "MyJsNamedClass")]
pub struct OriginalRustNameForJsNamedStruct {
  value: String,
}

#[napi]
impl OriginalRustNameForJsNamedStruct {
  #[napi(constructor)]
  pub fn new(value: String) -> Self {
    OriginalRustNameForJsNamedStruct { value }
  }

  #[napi]
  pub fn get_value(&self) -> String {
    self.value.clone()
  }

  #[napi]
  pub fn multiply_value(&self, times: u32) -> String {
    self.value.repeat(times as usize)
  }
}

// Test case for js_name struct with methods only (no constructor)
#[napi(js_name = "JSOnlyMethodsClass")]
pub struct RustOnlyMethodsClass {
  pub data: String,
}

#[napi]
impl RustOnlyMethodsClass {
  #[napi]
  pub fn process_data(&self) -> String {
    format!("processed: {}", self.data)
  }

  #[napi]
  pub fn get_length(&self) -> u32 {
    self.data.len() as u32
  }
}

// Test case for issue #2746: instanceof failure for objects returned from getters
#[napi]
pub struct Thing;

#[napi]
pub struct ThingList;

#[napi]
impl ThingList {
  #[napi(constructor)]
  pub fn new() -> Self {
    Self
  }

  #[napi(getter)]
  pub fn thing() -> ClassInitializer<Thing> {
    ClassInitializer::from(Thing)
  }
}

#[napi(
  ts_return_type = r#"typeof DynamicRustClass\n\nclass DynamicRustClass {
  constructor(value: number)
  rustMethod(): number
}"#
)]
pub fn define_class<'env>(#[napi(env)] env: &'env Env) -> Result<Function<'env>> {
  env.define_class(
    "DynamicRustClass",
    rust_class_constructor_c_callback,
    &[Property::new()
      .with_utf8_name("rustMethod")?
      .with_method(rust_class_method_c_callback)],
  )
}

#[napi(no_export)]
fn rust_class_constructor(value: i32, #[napi(this)] mut this: This) -> Result<()> {
  this.set_named_property("dynamicValue", value)?;
  Ok(())
}

#[napi(no_export)]
fn rust_class_method(#[napi(env)] mut env: Env, #[napi(this)] this: This) -> Result<i32> {
  env.with_scope(|scope| scope.get_named_property(&this, "dynamicValue"))
}
