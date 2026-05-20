use std::fmt::{Display, Formatter};

use convert_case::Case;
use quote::ToTokens;
use syn::{Member, Pat, PathArguments, PathSegment, Type};

use super::{r#struct::CLASS_STRUCTS, ty_to_ts_type, ToTypeDef, TypeDef};
use crate::{
  type_semantics::NapiTypeExt, typegen::JSDoc, util::to_case, CallbackArg, FnKind, NapiFn,
};

pub(crate) struct FnArg {
  pub(crate) arg: String,
  pub(crate) ts_type: String,
  pub(crate) is_optional: bool,
}

pub(crate) struct FnArgList {
  this: Option<FnArg>,
  args: Vec<FnArg>,
  last_required: Option<usize>,
  is_setter: bool,
}

fn parent_ts_name(parent: Option<&proc_macro2::Ident>) -> Option<String> {
  let parent = parent?;
  let origin_name = parent.to_string();
  Some(
    CLASS_STRUCTS
      .with_borrow(|classes| classes.get(&origin_name).cloned())
      .unwrap_or_else(|| to_case(origin_name, Case::Pascal)),
  )
}

fn receiver_class_name(inner: &Type, parent: Option<&proc_macro2::Ident>) -> Option<String> {
  let parent = parent?;
  let Type::Path(path) = inner else {
    return None;
  };
  if path.qself.is_some() || path.path.segments.len() != 1 {
    return None;
  }
  let ident = &path.path.segments[0].ident;
  if inner.is_self_type() || ident == parent {
    parent_ts_name(Some(parent))
  } else {
    None
  }
}

fn is_scope_type(ty: &Type) -> bool {
  match ty {
    Type::Path(path) => path
      .path
      .segments
      .last()
      .is_some_and(|segment| segment.ident == "Scope"),
    Type::Reference(reference) => is_scope_type(reference.elem.as_ref()),
    _ => false,
  }
}

fn class_value_return_type(ty: &Type, parent: Option<&proc_macro2::Ident>) -> Option<String> {
  if let Some(input) = ty.as_class_input() {
    return receiver_class_name(input.inner(), parent);
  }
  if let Some(inner) = ty.as_class_initializer_inner() {
    return receiver_class_name(inner, parent);
  }

  let inner = ty.option_inner()?;
  class_value_return_type(inner, parent).map(|ty| format!("{ty} | null"))
}

fn impl_self_arg_type(ty: &Type, parent: Option<&proc_macro2::Ident>) -> Option<(String, bool)> {
  if let Some(input) = ty.as_class_input() {
    if input.inner().is_self_type() {
      return parent_ts_name(parent).map(|name| (name, false));
    }
  }

  if let Type::Reference(reference) = ty {
    if reference.elem.is_self_type() {
      return parent_ts_name(parent).map(|name| (name, false));
    }
  }

  let inner = ty.option_inner()?;
  if let Some((name, _)) = impl_self_arg_type(inner, parent) {
    return Some((format!("{name} | undefined | null"), true));
  }
  None
}

fn has_receiver_frame_input_arg(item: &NapiFn) -> bool {
  item.args.iter().any(|arg| {
    let crate::NapiFnArgKind::PatType(path) = &arg.kind else {
      return false;
    };
    matches!(
      path.pat.as_ref(),
      Pat::Ident(pat) if pat.ident == "this" && path.ty.as_class_input().is_some()
    )
  })
}

impl FnArgList {
  fn with_setter_context(mut self, is_setter: bool) -> Self {
    self.is_setter = is_setter;
    self
  }
}

impl Display for FnArgList {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    if let Some(this) = &self.this {
      write!(f, "this: {}", this.ts_type)?;
    }
    for (i, arg) in self.args.iter().enumerate() {
      if i != 0 || self.this.is_some() {
        write!(f, ", ")?;
      }
      // For setters, never mark parameter as optional (TS1051: A 'set' accessor cannot have an optional parameter)
      let is_optional = !self.is_setter
        && arg.is_optional
        && self
          .last_required
          .is_none_or(|last_required| i > last_required);
      if is_optional {
        write!(f, "{}?: {}", arg.arg, arg.ts_type)?;
      } else {
        write!(f, "{}: {}", arg.arg, arg.ts_type)?;
      }
    }
    Ok(())
  }
}

impl FromIterator<FnArg> for FnArgList {
  fn from_iter<T: IntoIterator<Item = FnArg>>(iter: T) -> Self {
    let mut args = Vec::new();
    let mut this = None;
    for arg in iter.into_iter() {
      if arg.arg != "this" {
        args.push(arg);
      } else {
        this = Some(arg);
      }
    }
    let last_required = args
      .iter()
      .enumerate()
      .rfind(|(_, arg)| !arg.is_optional)
      .map(|(i, _)| i);
    FnArgList {
      this,
      args,
      last_required,
      is_setter: false,
    }
  }
}

impl ToTypeDef for NapiFn {
  fn to_type_def(&self) -> Option<TypeDef> {
    if self.skip_typescript || self.module_exports || self.no_export {
      return None;
    }

    let prefix = self.gen_ts_func_prefix();
    let def = match self.ts_type.as_ref() {
      Some(ts_type) => format!("{prefix} {name}{ts_type}", name = self.js_name),
      None => format!(
        r#"{prefix} {name}{generic}({args}){ret}"#,
        name = &self.js_name,
        generic = &self
          .ts_generic_types
          .as_ref()
          .map(|g| format!("<{g}>"))
          .unwrap_or_default(),
        args = self
          .ts_args_type
          .clone()
          .unwrap_or_else(|| self.gen_ts_func_args()),
        ret = self
          .ts_return_type
          .clone()
          .map(|t| format!(": {t}"))
          .unwrap_or_else(|| self.gen_ts_func_ret()),
      ),
    };

    Some(TypeDef {
      kind: "fn".to_owned(),
      name: self.js_name.clone(),
      original_name: None,
      def,
      extends: None,
      native_parent: None,
      js_mod: self.js_mod.to_owned(),
      js_doc: JSDoc::new(&self.comments),
    })
  }
}

fn gen_callback_type(callback: &CallbackArg) -> String {
  format!(
    "({args}) => {ret}",
    args = &callback
      .args
      .iter()
      .enumerate()
      .map(|(i, arg)| {
        let (ts_type, is_optional) = ty_to_ts_type(arg, false, false, false);
        FnArg {
          arg: format!("arg{i}"),
          ts_type,
          is_optional,
        }
      })
      .collect::<FnArgList>(),
    ret = match &callback.ret {
      Some(ty) => ty_to_ts_type(ty, true, false, false).0,
      None => "void".to_owned(),
    }
  )
}

fn gen_ts_func_arg(pat: &Pat) -> String {
  match pat {
    Pat::Struct(s) => format!(
      "{{ {} }}",
      s.fields
        .iter()
        .map(|field| {
          let member_str = match &field.member {
            Member::Named(ident) => ident.to_string(),
            Member::Unnamed(index) => format!("field{}", index.index),
          };
          let nested_str = gen_ts_func_arg(&field.pat);
          if member_str == nested_str {
            to_case(member_str, Case::Camel)
          } else {
            format!("{}: {}", to_case(member_str, Case::Camel), nested_str)
          }
        })
        .collect::<Vec<_>>()
        .join(", ")
        .as_str()
    ),
    Pat::TupleStruct(ts) => format!(
      "{{ {} }}",
      ts.elems
        .iter()
        .enumerate()
        .map(|(index, elem)| {
          let member_str = format!("field{index}");
          let nested_str = gen_ts_func_arg(elem);
          format!("{member_str}: {nested_str}")
        })
        .collect::<Vec<_>>()
        .join(", "),
    ),
    Pat::Tuple(t) => format!(
      "[{}]",
      t.elems
        .iter()
        .map(gen_ts_func_arg)
        .collect::<Vec<_>>()
        .join(", ")
    ),
    Pat::Wild(_) => "_".to_string(),
    _ => to_case(pat.to_token_stream().to_string(), Case::Camel),
  }
}

impl NapiFn {
  fn gen_ts_func_args(&self) -> String {
    format!(
      "{}",
      self
        .args
        .iter()
        .filter_map(|arg| match &arg.kind {
          crate::NapiFnArgKind::PatType(path) => {
            let ty_string = path.ty.to_token_stream().to_string();
            if ty_string == "Env" || is_scope_type(&path.ty) {
              return None;
            }
            if let syn::Type::Reference(syn::TypeReference { elem, .. }) = &*path.ty {
              if let syn::Type::Path(path) = elem.as_ref() {
                if let Some(PathSegment { ident, .. }) = path.path.segments.last() {
                  if ident == "Env" {
                    return None;
                  }
                }
              }
            }
            if let Some(input) = path.ty.as_class_input() {
              let is_this_arg = matches!(
                path.pat.as_ref(),
                Pat::Ident(pat_ident) if pat_ident.ident == "this"
              );
              if is_this_arg {
                if self.parent.is_some() {
                  return None;
                }
                if self.kind != FnKind::Normal {
                  return None;
                }
                let (ts_type, _) = ty_to_ts_type(input.inner(), false, false, false);
                let ts_type = arg.use_overridden_type_or(|| ts_type);
                return Some(FnArg {
                  arg: "this".to_owned(),
                  ts_type,
                  is_optional: false,
                });
              }
            }

            if let syn::Type::Path(path) = path.ty.as_ref() {
              if let Some(PathSegment { ident, arguments }) = path.path.segments.last() {
                if ident == "This" || ident == "this" {
                  if self.kind != FnKind::Normal {
                    return None;
                  }
                  if let PathArguments::AngleBracketed(syn::AngleBracketedGenericArguments {
                    args: angle_bracketed_args,
                    ..
                  }) = arguments
                  {
                    if let Some(syn::GenericArgument::Type(ty)) = angle_bracketed_args.first() {
                      let (ts_type, _) = ty_to_ts_type(ty, false, false, false);
                      return Some(FnArg {
                        arg: "this".to_owned(),
                        ts_type,
                        is_optional: false,
                      });
                    }
                  } else {
                    return Some(FnArg {
                      arg: "this".to_owned(),
                      ts_type: "this".to_owned(),
                      is_optional: false,
                    });
                  }
                  return None;
                }
              }
            }

            let mut path = path.clone();
            // remove mutability from PatIdent
            if let Pat::Ident(i) = path.pat.as_mut() {
              i.mutability = None;
            }

            let (ts_type, is_optional) = impl_self_arg_type(&path.ty, self.parent.as_ref())
              .unwrap_or_else(|| ty_to_ts_type(&path.ty, false, false, false));
            let ts_type = arg.use_overridden_type_or(|| ts_type);
            let arg = gen_ts_func_arg(&path.pat);
            Some(FnArg {
              arg,
              ts_type,
              is_optional,
            })
          }
          crate::NapiFnArgKind::Callback(cb) => {
            let ts_type = arg.use_overridden_type_or(|| gen_callback_type(cb));
            let arg = to_case(cb.pat.to_token_stream().to_string(), Case::Camel);

            Some(FnArg {
              arg,
              ts_type,
              is_optional: false,
            })
          }
        })
        .collect::<FnArgList>()
        .with_setter_context(matches!(self.kind, FnKind::Setter))
    )
  }

  fn gen_ts_func_prefix(&self) -> &'static str {
    if self.parent.is_some() {
      match self.kind {
        crate::FnKind::Normal => match self.fn_self {
          Some(_) => "",
          None if has_receiver_frame_input_arg(self) => "",
          None => "static",
        },
        crate::FnKind::Factory => "static",
        crate::FnKind::Constructor => "",
        crate::FnKind::Getter => "get",
        crate::FnKind::Setter => "set",
      }
    } else {
      "function"
    }
  }

  fn gen_ts_func_ret(&self) -> String {
    match self.kind {
      FnKind::Constructor | FnKind::Setter => "".to_owned(),
      FnKind::Factory => self
        .parent
        .clone()
        .map(|i| {
          let origin_name = i.to_string();
          let parent = CLASS_STRUCTS
            .with_borrow(|c| c.get(&origin_name).cloned())
            .unwrap_or_else(|| to_case(origin_name, Case::Pascal));

          if self.is_async {
            format!(": Promise<{parent}>")
          } else {
            format!(": {parent}")
          }
        })
        .unwrap_or_else(|| "".to_owned()),
      _ => {
        let ret = if let Some(ret) = &self.ret {
          let (ts_type, _) = class_value_return_type(ret, self.parent.as_ref())
            .map(|ty| (ty, false))
            .unwrap_or_else(|| ty_to_ts_type(ret, true, false, false));
          if ts_type == "undefined" {
            "void".to_owned()
          } else if ts_type == "Self" {
            if self.fn_self.is_some() {
              "this".to_owned()
            } else if let Some(parent) = &self.parent {
              let origin_name = parent.to_string();
              CLASS_STRUCTS
                .with_borrow(|classes| classes.get(&origin_name).cloned())
                .unwrap_or_else(|| to_case(origin_name, Case::Pascal))
            } else {
              ts_type
            }
          } else {
            ts_type
          }
        } else {
          "void".to_owned()
        };
        if self.is_async {
          format!(": Promise<{ret}>")
        } else {
          format!(": {ret}")
        }
      }
    }
  }
}
