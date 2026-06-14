use convert_case::Case;
use proc_macro2::{Ident, Literal};
use syn::{Attribute, Expr, Type};

#[derive(Debug, Clone, Copy)]
pub struct PropertyDescriptor {
  pub writable: bool,
  pub enumerable: bool,
  pub configurable: bool,
}

#[derive(Debug, Clone, Default)]
pub struct TsOverrides {
  pub ts_type: Option<String>,
  pub ts_args_type: Option<String>,
  pub ts_return_type: Option<String>,
  pub ts_generic_types: Option<String>,
  pub skip_typescript: bool,
}

#[derive(Debug, Clone)]
pub struct ClassContext {
  pub name: Ident,
  pub js_name: String,
  pub fn_self: Option<FnSelf>,
  pub is_generator: bool,
  pub is_async_generator: bool,
}

#[derive(Debug, Clone)]
pub struct NapiFn {
  pub name: Ident,
  pub js_name: String,
  pub attrs: Vec<Attribute>,
  pub args: Vec<NapiFnArg>,
  pub ret: Option<syn::Type>,
  pub is_ret_result: bool,
  pub is_async: bool,
  pub kind: FnKind,
  pub vis: syn::Visibility,
  pub class: Option<ClassContext>,
  pub js_mod: Option<String>,
  pub ts: TsOverrides,
  pub comments: Vec<String>,
  pub descriptor: PropertyDescriptor,
  pub catch_unwind: bool,
  pub unsafe_: bool,
  pub register_name: Ident,
  pub no_export: bool,
}

impl NapiFn {
  pub fn parent(&self) -> Option<&Ident> {
    self.class.as_ref().map(|c| &c.name)
  }

  pub fn parent_js_name(&self) -> Option<&str> {
    self.class.as_ref().map(|c| c.js_name.as_str())
  }

  pub fn fn_self(&self) -> Option<&FnSelf> {
    self.class.as_ref().and_then(|c| c.fn_self.as_ref())
  }

  pub fn parent_is_generator(&self) -> bool {
    self.class.as_ref().is_some_and(|c| c.is_generator)
  }

  pub fn parent_is_async_generator(&self) -> bool {
    self.class.as_ref().is_some_and(|c| c.is_async_generator)
  }

  pub fn is_module_exports(&self) -> bool {
    matches!(self.kind, FnKind::ModuleExport)
  }

  pub fn post_init_chain(&self) -> &[Ident] {
    match &self.kind {
      FnKind::Constructor { post_init_chain } => post_init_chain,
      _ => &[],
    }
  }
}

#[derive(Debug, Clone)]
pub struct CallbackArg {
  pub pat: Box<syn::Pat>,
  pub args: Vec<syn::Type>,
  pub ret: Option<syn::Type>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectKind {
  Env,
  This,
  Scope,
  Rest,
}

#[derive(Debug, Clone)]
pub struct NapiFnArg {
  pub kind: NapiFnArgKind,
  pub ts_arg_type: Option<String>,
  pub inject: Option<InjectKind>,
}

impl NapiFnArg {
  /// if type was overridden with `#[napi(ts_arg_type = "...")]` use that instead
  pub fn use_overridden_type_or(&self, default: impl FnOnce() -> String) -> String {
    self.ts_arg_type.as_ref().cloned().unwrap_or_else(default)
  }
}

#[derive(Debug, Clone)]
pub enum NapiFnArgKind {
  PatType(Box<syn::PatType>),
  Callback(Box<CallbackArg>),
}

#[derive(Debug, Clone)]
pub enum FnKind {
  Normal,
  ModuleExport,
  Constructor { post_init_chain: Vec<Ident> },
  Factory,
  Getter,
  Setter,
  PostInit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FnSelf {
  Value,
  Ref,
  MutRef,
}

#[derive(Debug, Clone)]
pub struct NapiStruct {
  pub name: Ident,
  pub js_name: String,
  pub comments: Vec<String>,
  pub js_mod: Option<String>,
  pub use_nullable: bool,
  pub register_name: Ident,
  pub kind: NapiStructKind,
  pub has_lifetime: bool,
}

impl NapiStruct {
  pub fn is_generator(&self) -> bool {
    matches!(&self.kind, NapiStructKind::Class(c) if c.is_generator)
  }

  pub fn is_async_generator(&self) -> bool {
    matches!(&self.kind, NapiStructKind::Class(c) if c.is_async_generator)
  }
}

#[derive(Debug, Clone)]
pub enum NapiStructKind {
  Transparent(NapiTransparent),
  Class(NapiClass),
  Object(NapiObject),
  StructuredEnum(NapiStructuredEnum),
  Array(NapiArray),
}

#[derive(Debug, Clone)]
pub struct NapiTransparent {
  pub ty: Type,
  pub object_from_js: bool,
  pub object_to_js: bool,
}

#[derive(Debug, Clone)]
pub struct NapiClass {
  pub fields: Vec<NapiStructField>,
  pub ctor: bool,
  pub subclass: bool,
  pub parent: Option<NativeParentSpec>,
  pub implement_iterator: bool,
  pub implement_async_iterator: bool,
  pub is_tuple: bool,
  pub use_custom_finalize: bool,
  pub is_generator: bool,
  pub is_async_generator: bool,
}

#[derive(Debug, Clone)]
pub struct NativeParentSpec {
  pub rust_path: Type,
  pub js_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NapiObject {
  pub fields: Vec<NapiStructField>,
  pub object_from_js: bool,
  pub object_to_js: bool,
  pub is_tuple: bool,
}

#[derive(Debug, Clone)]
pub struct NapiArray {
  pub fields: Vec<NapiStructField>,
  pub object_from_js: bool,
  pub object_to_js: bool,
}

#[derive(Debug, Clone)]
pub struct NapiStructuredEnum {
  pub variants: Vec<NapiStructuredEnumVariant>,
  pub object_from_js: bool,
  pub object_to_js: bool,
  pub discriminant: String,
  pub discriminant_case: Option<Case<'static>>,
}

#[derive(Debug, Clone)]
pub struct NapiStructuredEnumVariant {
  pub name: Ident,
  pub fields: Vec<NapiStructField>,
  pub is_tuple: bool,
}

#[derive(Debug, Clone)]
pub struct NapiStructField {
  pub name: syn::Member,
  pub js_name: String,
  pub ty: syn::Type,
  pub getter: bool,
  pub setter: bool,
  pub descriptor: PropertyDescriptor,
  pub comments: Vec<String>,
  pub skip_typescript: bool,
  pub ts_type: Option<String>,
  pub has_lifetime: bool,
}

#[derive(Debug, Clone)]
pub struct NapiImpl {
  pub name: Ident,
  pub js_name: String,
  pub is_class: bool,
  pub has_lifetime: bool,
  pub items: Vec<NapiFn>,
  pub iterator_yield_type: Option<Type>,
  pub iterator_next_type: Option<Type>,
  pub iterator_return_type: Option<Type>,
  pub async_iterator_yield_type: Option<Type>,
  pub async_iterator_next_type: Option<Type>,
  pub async_iterator_return_type: Option<Type>,
  pub js_mod: Option<String>,
  pub comments: Vec<String>,
  pub register_name: Ident,
}

#[derive(Debug, Clone)]
pub struct NapiEnum {
  pub name: Ident,
  pub js_name: String,
  pub variants: Vec<NapiEnumVariant>,
  pub js_mod: Option<String>,
  pub comments: Vec<String>,
  pub skip_typescript: bool,
  pub register_name: Ident,
  pub is_string_enum: bool,
  pub object_from_js: bool,
  pub object_to_js: bool,
}

#[derive(Debug, Clone)]
pub enum NapiEnumValue {
  String(String),
  Number(i32),
}

impl From<&NapiEnumValue> for Literal {
  fn from(val: &NapiEnumValue) -> Self {
    match val {
      NapiEnumValue::String(string) => Literal::string(string),
      NapiEnumValue::Number(number) => Literal::i32_unsuffixed(number.to_owned()),
    }
  }
}

#[derive(Debug, Clone)]
pub struct NapiEnumVariant {
  pub name: Ident,
  pub val: NapiEnumValue,
  pub comments: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NapiConst {
  pub name: Ident,
  pub js_name: String,
  pub type_name: Type,
  pub value: Expr,
  pub js_mod: Option<String>,
  pub comments: Vec<String>,
  pub skip_typescript: bool,
  pub register_name: Ident,
}

#[derive(Debug, Clone)]
pub struct NapiMod {
  pub name: Ident,
  pub js_name: String,
}

#[derive(Debug, Clone)]
pub struct NapiType {
  pub name: Ident,
  pub js_name: String,
  pub value: Type,
  pub register_name: Ident,
  pub skip_typescript: bool,
  pub ts_type: Option<String>,
  pub js_mod: Option<String>,
  pub comments: Vec<String>,
}
