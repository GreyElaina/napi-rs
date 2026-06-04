use std::collections::HashMap;
use std::str::Chars;

use napi_derive_backend::{BindgenResult, Diagnostic};
use proc_macro2::Ident;
use quote::ToTokens;
use syn::{ExprLit, GenericArgument};

pub(crate) fn get_ty(mut ty: &mut syn::Type) -> &mut syn::Type {
  while let syn::Type::Group(g) = ty {
    ty = &mut g.elem;
  }
  ty
}

pub(crate) fn extract_path_ident(path: &mut syn::Path) -> BindgenResult<(Ident, bool)> {
  let mut has_lifetime = false;
  for segment in path.segments.iter_mut() {
    match &segment.arguments {
      syn::PathArguments::None => {}
      syn::PathArguments::AngleBracketed(generic) => {
        if let Some(GenericArgument::Lifetime(_)) = generic.args.first() {
          has_lifetime = true;
        } else {
          bail_span!(path, "Only 1 lifetime is supported for now");
        }
      }
      _ => bail_span!(path, "paths with type parameters are not supported yet"),
    }
  }

  match path.segments.last() {
    Some(value) => Ok((value.ident.clone(), has_lifetime)),
    None => {
      bail_span!(path, "empty idents are not supported");
    }
  }
}

pub(crate) fn extract_callback_trait_types(
  arguments: &syn::PathArguments,
) -> BindgenResult<(Vec<syn::Type>, Option<syn::Type>)> {
  match arguments {
    syn::PathArguments::None => Ok((vec![], None)),
    syn::PathArguments::AngleBracketed(_) => {
      bail_span!(arguments, "use parentheses for napi callback trait")
    }
    syn::PathArguments::Parenthesized(arguments) => {
      let args = arguments.inputs.iter().cloned().collect::<Vec<_>>();
      let ret = match &arguments.output {
        syn::ReturnType::Type(_, ret_ty) => {
          let ret_ty = &**ret_ty;
          if let Some(ty_of_result) = extract_result_ty(ret_ty)? {
            if ty_of_result.to_token_stream().to_string() == "()" {
              None
            } else {
              Some(ty_of_result)
            }
          } else {
            bail_span!(ret_ty, "The return type of callback can only be `Result`");
          }
        }
        _ => {
          bail_span!(
            arguments,
            "The return type of callback can only be `Result`. Try with `Result<()>`"
          );
        }
      };
      Ok((args, ret))
    }
  }
}

pub(crate) fn extract_result_ty(ty: &syn::Type) -> BindgenResult<Option<syn::Type>> {
  match ty {
    syn::Type::Path(syn::TypePath { qself: None, path }) => {
      let segment = path.segments.last().unwrap();
      if segment.ident != "Result" {
        Ok(None)
      } else {
        match &segment.arguments {
          syn::PathArguments::AngleBracketed(syn::AngleBracketedGenericArguments {
            args, ..
          }) => {
            let ok_arg = args.first().unwrap();
            match ok_arg {
              syn::GenericArgument::Type(ty) => Ok(Some(ty.clone())),
              _ => bail_span!(ok_arg, "unsupported generic type"),
            }
          }
          _ => bail_span!(segment, "unsupported generic type"),
        }
      }
    }
    _ => Ok(None),
  }
}

pub(crate) fn get_expr(mut expr: &syn::Expr) -> &syn::Expr {
  while let syn::Expr::Group(g) = expr {
    expr = &g.expr;
  }
  expr
}

pub(crate) fn extract_doc_comments(attrs: &[syn::Attribute]) -> Vec<String> {
  attrs
    .iter()
    .filter_map(|a| {
      let name_value = a.meta.require_name_value();
      if let Ok(name) = name_value {
        if a.path().is_ident("doc") {
          Some(match &name.value {
            syn::Expr::Lit(ExprLit {
              lit: syn::Lit::Str(str),
              ..
            }) => {
              let quoted = str.token().to_string();
              Some(try_unescape(&quoted).unwrap_or(quoted))
            }
            _ => None,
          })
        } else {
          None
        }
      } else {
        None
      }
    })
    .fold(vec![], |mut acc, a| {
      acc.extend(a);
      acc
    })
}

fn try_unescape(s: &str) -> Option<String> {
  if s.is_empty() {
    return Some(String::new());
  }
  let mut result = String::with_capacity(s.len());
  let mut chars = s.chars();
  for i in 0.. {
    let c = match chars.next() {
      Some(c) => c,
      None => {
        if result.ends_with('"') {
          result.pop();
        }
        return Some(result);
      }
    };
    if i == 0 && c == '"' {
      // ignore leading quote
    } else if c == '\\' {
      let c = chars.next()?;
      match c {
        't' => result.push('\t'),
        'r' => result.push('\r'),
        'n' => result.push('\n'),
        '\\' | '\'' | '"' => result.push(c),
        'u' => {
          if chars.next() != Some('{') {
            return None;
          }
          let (c, next) = unescape_unicode(&mut chars)?;
          result.push(c);
          if next != '}' {
            return None;
          }
        }
        _ => return None,
      }
    } else {
      result.push(c);
    }
  }
  None
}

fn unescape_unicode(chars: &mut Chars) -> Option<(char, char)> {
  let mut value = 0;
  for i in 0..7 {
    let c = chars.next()?;
    let num = match c {
      '0'..='9' => c as u32 - '0' as u32,
      'a'..='f' => c as u32 - 'a' as u32,
      'A'..='F' => c as u32 - 'A' as u32,
      _ => {
        if i == 0 {
          return None;
        }
        let decoded = char::from_u32(value)?;
        return Some((decoded, c));
      }
    };
    if i >= 6 {
      return None;
    }
    value = (value << 4) | num;
  }
  None
}

pub(crate) fn extract_fn_closure_generics(
  generics: &syn::Generics,
) -> BindgenResult<HashMap<String, syn::PathArguments>> {
  let mut errors = vec![];
  let mut map = HashMap::default();
  if generics.params.is_empty() {
    return Ok(map);
  }

  if let Some(where_clause) = &generics.where_clause {
    for prediction in where_clause.predicates.iter() {
      match prediction {
        syn::WherePredicate::Type(syn::PredicateType {
          bounded_ty, bounds, ..
        }) => {
          for bound in bounds {
            match bound {
              syn::TypeParamBound::Trait(t) => {
                for segment in t.path.segments.iter() {
                  match segment.ident.to_string().as_str() {
                    "Fn" | "FnOnce" | "FnMut" => {
                      map.insert(
                        bounded_ty.to_token_stream().to_string(),
                        segment.arguments.clone(),
                      );
                    }
                    _ => {}
                  };
                }
              }
              syn::TypeParamBound::Lifetime(lifetime) => {
                if lifetime.ident != "static" {
                  errors.push(err_span!(
                    bound,
                    "only 'static is supported in lifetime bound for fn arguments"
                  ));
                }
              }
              _ => errors.push(err_span! {
                bound,
                "unsupported bound in napi"
              }),
            }
          }
        }
        _ => errors.push(err_span! {
          prediction,
          "unsupported where clause prediction in napi"
        }),
      };
    }
  }

  for param in generics.params.iter() {
    match param {
      syn::GenericParam::Type(syn::TypeParam { ident, bounds, .. }) => {
        for bound in bounds {
          match bound {
            syn::TypeParamBound::Trait(t) => {
              for segment in t.path.segments.iter() {
                match segment.ident.to_string().as_str() {
                  "Fn" | "FnOnce" | "FnMut" => {
                    map.insert(ident.to_string(), segment.arguments.clone());
                  }
                  _ => {}
                };
              }
            }
            syn::TypeParamBound::Lifetime(lifetime) => {
              if lifetime.ident != "static" {
                errors.push(err_span!(
                  bound,
                  "only 'static is supported in lifetime bound for fn arguments"
                ));
              }
            }
            _ => errors.push(err_span! {
              bound,
              "unsupported bound in napi"
            }),
          }
        }
      }
      syn::GenericParam::Lifetime(_) => {}
      _ => {
        errors.push(err_span!(param, "unsupported napi generic param for fn"));
      }
    }
  }

  Diagnostic::from_vec(errors).and(Ok(map))
}
