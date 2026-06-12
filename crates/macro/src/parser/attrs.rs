use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::OnceLock;

use darling::ast::NestedMeta;
use darling::util::Flag;
use darling::FromMeta;
use napi_derive_backend::{BindgenResult, Diagnostic};
use proc_macro2::{Ident, Span, TokenStream};
use quote::ToTokens;
use syn::spanned::Spanned;

// ---------------------------------------------------------------------------
// Helper types
// ---------------------------------------------------------------------------

/// A string value that accepts either a string literal or a bare identifier:
/// `#[napi(js_name = "foo")]` or `#[napi(js_name = foo)]`.
#[derive(Debug, Clone)]
pub struct FlexibleString {
  pub value: String,
  pub span: Span,
}

impl FromMeta for FlexibleString {
  fn from_expr(expr: &syn::Expr) -> darling::Result<Self> {
    match expr {
      syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(s),
        ..
      }) => Ok(FlexibleString {
        value: s.value(),
        span: s.span(),
      }),
      syn::Expr::Path(ep) => {
        if let Some(ident) = ep.path.get_ident() {
          Ok(FlexibleString {
            value: ident.to_string(),
            span: ident.span(),
          })
        } else {
          Err(darling::Error::custom("expected string literal or identifier").with_span(expr))
        }
      }
      _ => Err(darling::Error::custom("expected string literal or identifier").with_span(expr)),
    }
  }
}

/// An attribute that can appear as a bare flag or with a string/ident value:
/// `#[napi(string_enum)]` or `#[napi(string_enum = "camelCase")]`.
#[derive(Debug, Clone)]
pub struct OptionalFlexibleString(pub Option<FlexibleString>);

impl FromMeta for OptionalFlexibleString {
  fn from_word() -> darling::Result<Self> {
    Ok(OptionalFlexibleString(None))
  }

  fn from_expr(expr: &syn::Expr) -> darling::Result<Self> {
    FlexibleString::from_expr(expr).map(|s| OptionalFlexibleString(Some(s)))
  }
}

/// An attribute that can appear as a bare flag or with an ident value:
/// `#[napi(getter)]` or `#[napi(getter = my_prop)]`.
#[derive(Debug, Clone)]
pub struct OptionalIdent(pub Option<Ident>);

impl FromMeta for OptionalIdent {
  fn from_word() -> darling::Result<Self> {
    Ok(OptionalIdent(None))
  }

  fn from_expr(expr: &syn::Expr) -> darling::Result<Self> {
    Ident::from_expr(expr).map(|i| OptionalIdent(Some(i)))
  }
}

/// A boolean attribute with a compile-time default when present as bare flag.
/// `from_none` also returns the default, so the field is always populated.
#[derive(Debug, Clone, Copy)]
pub struct BoolWithDefault<const DEFAULT: bool>(pub bool);

impl<const DEFAULT: bool> FromMeta for BoolWithDefault<DEFAULT> {
  fn from_none() -> Option<Self> {
    Some(BoolWithDefault(DEFAULT))
  }

  fn from_word() -> darling::Result<Self> {
    Ok(BoolWithDefault(DEFAULT))
  }

  fn from_expr(expr: &syn::Expr) -> darling::Result<Self> {
    bool::from_expr(expr).map(BoolWithDefault)
  }
}

pub type TrueByDefault = BoolWithDefault<true>;
pub type FalseByDefault = BoolWithDefault<false>;

// ---------------------------------------------------------------------------
// Error conversion
// ---------------------------------------------------------------------------

pub fn darling_to_diagnostic(err: darling::Error) -> Diagnostic {
  Diagnostic::from(syn::Error::from(err))
}

// ---------------------------------------------------------------------------
// Attribute parsing helpers
// ---------------------------------------------------------------------------

/// Parse the proc-macro attribute token stream (the content inside `#[napi(...)]`)
/// into a darling-derived type.
pub fn parse_napi_attr<T: FromMeta>(attr: TokenStream) -> BindgenResult<T> {
  let items = NestedMeta::parse_meta_list(attr).map_err(Diagnostic::from)?;
  T::from_list(&items).map_err(darling_to_diagnostic)
}

/// Find a `#[napi(...)]` attribute in a list, parse it into `T`, and remove it.
/// Also handles `#[cfg_attr(condition, napi(...))]`.
pub fn find_napi_attr<T: FromMeta>(attrs: &mut Vec<syn::Attribute>) -> BindgenResult<Option<T>> {
  for (index, attr) in attrs.iter().enumerate() {
    if attr.path().is_ident("napi") {
      let parsed = parse_attr_meta::<T>(attr)?;
      attrs.remove(index);
      return Ok(Some(parsed));
    }

    if is_cfg_attr_napi(attr) {
      let parsed = parse_cfg_attr_napi::<T>(attr)?;
      attrs.remove(index);
      return Ok(Some(parsed));
    }
  }
  Ok(None)
}

fn parse_attr_meta<T: FromMeta>(attr: &syn::Attribute) -> BindgenResult<T> {
  match &attr.meta {
    syn::Meta::Path(_) => T::from_list(&[]).map_err(darling_to_diagnostic),
    syn::Meta::List(list) => {
      let items = NestedMeta::parse_meta_list(list.tokens.clone()).map_err(Diagnostic::from)?;
      T::from_list(&items).map_err(darling_to_diagnostic)
    }
    syn::Meta::NameValue(_) => {
      bail_span!(attr, "invalid #[napi] attribute; expected #[napi] or #[napi(...)]")
    }
  }
}

fn is_cfg_attr_napi(attr: &syn::Attribute) -> bool {
  if !attr
    .meta
    .path()
    .segments
    .first()
    .is_some_and(|s| s.ident == "cfg_attr")
  {
    return false;
  }
  let Ok(list) = attr.meta.require_list() else {
    return false;
  };
  let Ok(args) = list
    .parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
  else {
    return false;
  };
  args
    .iter()
    .last()
    .is_some_and(|m| m.path().segments.last().is_some_and(|s| s.ident == "napi"))
}

fn parse_cfg_attr_napi<T: FromMeta>(attr: &syn::Attribute) -> BindgenResult<T> {
  let list = attr.meta.require_list().map_err(Diagnostic::from)?;
  let args = list
    .parse_args_with(syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
    .map_err(Diagnostic::from)?;

  let napi_meta = args
    .into_iter()
    .last()
    .ok_or_else(|| Diagnostic::span_error(attr.span(), "invalid cfg_attr"))?;

  match &napi_meta {
    syn::Meta::Path(_) => T::from_list(&[]).map_err(darling_to_diagnostic),
    syn::Meta::List(list) => {
      let items = NestedMeta::parse_meta_list(list.tokens.clone()).map_err(Diagnostic::from)?;
      T::from_list(&items).map_err(darling_to_diagnostic)
    }
    _ => bail_span!(attr, "invalid napi attribute inside cfg_attr"),
  }
}

/// Find `#[napi]` in `attrs`, inject `namespace`, parse as `T`, and remove.
pub fn find_napi_attr_with_namespace<T: FromMeta>(
  attrs: &mut Vec<syn::Attribute>,
  namespace: &str,
) -> BindgenResult<Option<T>> {
  let napi_index = attrs
    .iter()
    .position(|a| a.path().is_ident("napi"));

  let Some(index) = napi_index else {
    return Ok(None);
  };

  let attr = &attrs[index];
  let mut items = match &attr.meta {
    syn::Meta::Path(_) => vec![],
    syn::Meta::List(list) => {
      NestedMeta::parse_meta_list(list.tokens.clone()).map_err(Diagnostic::from)?
    }
    syn::Meta::NameValue(nv) => {
      let tokens = nv.value.to_token_stream();
      NestedMeta::parse_meta_list(tokens).map_err(Diagnostic::from)?
    }
  };

  let ns_meta: syn::Meta = syn::parse_quote!(namespace = #namespace);
  items.push(NestedMeta::Meta(ns_meta));

  let parsed = T::from_list(&items).map_err(darling_to_diagnostic)?;
  attrs.remove(index);
  Ok(Some(parsed))
}

// ---------------------------------------------------------------------------
// Per-item attribute structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, FromMeta)]
pub struct FnAttrs {
  pub js_name: Option<FlexibleString>,
  pub namespace: Option<FlexibleString>,
  pub skip_typescript: Flag,

  pub constructor: Flag,
  pub factory: Flag,
  pub getter: Option<OptionalIdent>,
  pub setter: Option<OptionalIdent>,
  pub post_init: Flag,

  pub catch_unwind: Flag,
  pub module_exports: Flag,

  pub ts_type: Option<FlexibleString>,
  pub ts_args_type: Option<FlexibleString>,
  pub ts_return_type: Option<FlexibleString>,
  pub ts_generic_types: Option<FlexibleString>,

  pub writable: TrueByDefault,
  pub enumerable: TrueByDefault,
  pub configurable: TrueByDefault,

  pub no_export: Flag,
}

#[derive(Debug, Clone, FromMeta)]
pub struct StructAttrs {
  pub js_name: Option<FlexibleString>,
  pub namespace: Option<FlexibleString>,

  pub constructor: Flag,
  pub object: Flag,
  pub subclass: Flag,
  pub extends: Option<syn::Path>,
  pub custom_finalize: Flag,
  pub iterator: Flag,
  pub async_iterator: Flag,
  pub transparent: Flag,
  pub array: Flag,

  pub object_from_js: TrueByDefault,
  pub object_to_js: TrueByDefault,
  pub use_nullable: FalseByDefault,
}

#[derive(Debug, Clone, FromMeta)]
pub struct ImplAttrs {
  pub namespace: Option<FlexibleString>,
}

#[derive(Debug, Clone, FromMeta)]
pub struct EnumAttrs {
  pub js_name: Option<FlexibleString>,
  pub namespace: Option<FlexibleString>,
  pub skip_typescript: Flag,

  pub string_enum: Option<OptionalFlexibleString>,
  pub object: Flag,
  pub discriminant: Option<FlexibleString>,
  pub discriminant_case: Option<FlexibleString>,
  pub object_from_js: TrueByDefault,
  pub object_to_js: TrueByDefault,
  pub use_nullable: FalseByDefault,

  pub subclass: Flag,
  pub extends: Option<syn::Path>,
}

#[derive(Debug, Clone, FromMeta)]
pub struct ConstAttrs {
  pub js_name: Option<FlexibleString>,
  pub namespace: Option<FlexibleString>,
  pub skip_typescript: Flag,
}

#[derive(Debug, Clone, FromMeta)]
pub struct TypeAttrs {
  pub js_name: Option<FlexibleString>,
  pub namespace: Option<FlexibleString>,
  pub skip_typescript: Flag,
  pub ts_type: Option<FlexibleString>,
}

#[derive(Debug, Clone, FromMeta)]
pub struct FieldAttrs {
  pub js_name: Option<FlexibleString>,
  pub skip: Flag,
  pub readonly: Flag,
  pub ts_type: Option<FlexibleString>,
  pub skip_typescript: Flag,
  pub writable: TrueByDefault,
  pub enumerable: TrueByDefault,
  pub configurable: TrueByDefault,
}

/// Attributes on enum variant: `#[napi(value = "...")]`.
#[derive(Debug, Clone, FromMeta)]
pub struct EnumVariantAttrs {
  pub value: Option<FlexibleString>,
}

#[derive(Debug, Clone, FromMeta)]
pub struct ModAttrs {
  pub js_name: Option<FlexibleString>,
}

/// Attributes on function parameters: `#[napi(env)]`, `#[napi(ts_arg_type = "...")]`, etc.
#[derive(Debug, Clone, FromMeta)]
pub struct ArgAttrs {
  pub ts_arg_type: Option<FlexibleString>,
  pub env: Flag,
  pub this: Flag,
  pub scope: Flag,
  pub rest: Flag,
}

// ---------------------------------------------------------------------------
// Struct registry (encapsulated global state)
// ---------------------------------------------------------------------------

static REGISTRY: OnceLock<StructRegistry> = OnceLock::new();

pub struct StructRegistry {
  structs: Mutex<HashMap<String, StructRecord>>,
}

pub struct StructRecord {
  pub js_name: String,
  pub ctor_defined: bool,
  pub post_init_method: Option<String>,
  pub parent: Option<String>,
}

impl StructRegistry {
  fn global() -> &'static StructRegistry {
    REGISTRY.get_or_init(|| StructRegistry {
      structs: Mutex::new(HashMap::new()),
    })
  }

  pub fn record(ident: &Ident, js_name: String, parent: Option<String>) {
    let reg = Self::global();
    let mut map = reg.structs.lock().unwrap();
    map.insert(
      ident.to_string(),
      StructRecord {
        js_name,
        ctor_defined: false,
        post_init_method: None,
        parent,
      },
    );
  }

  pub fn check_for_impl(ident: &Ident, has_ctor: bool) -> BindgenResult<String> {
    let reg = Self::global();
    let mut map = reg.structs.lock().unwrap();
    let struct_name = ident.to_string();
    if let Some(record) = map.get_mut(&struct_name) {
      if has_ctor && !cfg!(debug_assertions) {
        if record.ctor_defined {
          bail_span!(
            ident,
            "Constructor has already been defined for struct `{}`",
            &struct_name
          );
        } else {
          record.ctor_defined = true;
        }
      }
      Ok(record.js_name.clone())
    } else {
      bail_span!(
        ident,
        "Did not find struct `{}` parsed before expand #[napi] for impl",
        &struct_name,
      )
    }
  }

  pub fn lookup_js_name(ident: &Ident) -> Option<String> {
    let reg = Self::global();
    let map = reg.structs.lock().ok()?;
    map.get(&ident.to_string()).map(|r| r.js_name.clone())
  }

  pub fn record_post_init(struct_name: &str, method_name: String) {
    let reg = Self::global();
    let mut map = reg.structs.lock().unwrap();
    if let Some(record) = map.get_mut(struct_name) {
      record.post_init_method = Some(method_name);
    }
  }

  pub fn collect_post_init_chain(struct_name: &str) -> Vec<String> {
    let reg = Self::global();
    let map = reg.structs.lock().unwrap();
    let mut chain = Vec::new();
    let mut current = Some(struct_name.to_string());
    while let Some(name) = current {
      let Some(record) = map.get(&name) else { break };
      if record.post_init_method.is_some() {
        chain.push(name.clone());
      }
      current = record.parent.clone();
    }
    chain.reverse();
    chain
  }
}
