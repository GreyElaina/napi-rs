use super::typedef;
use crate::parser::{
  attrs::{
    find_napi_attr_with_namespace, parse_napi_attr, ConstAttrs, EnumAttrs, FnAttrs, ImplAttrs,
    ModAttrs, StructAttrs,
  },
  convert_const, convert_enum, convert_fn, convert_impl, convert_struct, convert_type,
};
use napi_derive_backend::{BindgenResult, Napi, TryToTokens};
use proc_macro2::TokenStream;
use quote::ToTokens;
use std::sync::atomic::{AtomicBool, Ordering};
use syn::Item;

static BUILT_FLAG: AtomicBool = AtomicBool::new(false);

pub fn expand(attr: TokenStream, input: TokenStream) -> BindgenResult<TokenStream> {
  if let Ok(built) =
    BUILT_FLAG.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
  {
    if !built {
      typedef::prepare_type_def_file();
    }
  }

  let mut item = syn::parse2::<Item>(input)?;

  // borrow checker said no
  if let Item::Mod(js_mod) = item {
    return expand_mod(attr, js_mod);
  }

  match &mut item {
    Item::Fn(f) => {
      let opts: FnAttrs = parse_napi_attr(attr)?;
      validate_fn_attrs(f, &opts)?;
      let napi = convert_fn(f, &opts)?;
      emit(f, napi)
    }
    Item::Struct(s) => {
      let napi = convert_struct(s, &parse_napi_attr(attr)?)?;
      emit(s, napi)
    }
    Item::Impl(i) => {
      let napi = convert_impl(i, &parse_napi_attr(attr)?)?;
      emit(i, napi)
    }
    Item::Enum(e) => {
      let napi = convert_enum(e, &parse_napi_attr(attr)?)?;
      emit(e, napi)
    }
    Item::Const(c) => {
      let napi = convert_const(c, &parse_napi_attr(attr)?)?;
      emit(c, napi)
    }
    Item::Type(t) => {
      let napi = convert_type(t, &parse_napi_attr(attr)?)?;
      emit(t, napi)
    }
    _ => bail_span!(
      item,
      "#[napi] can only be applied to a function, struct, enum, const, type or impl."
    ),
  }
}

fn emit(item: &impl ToTokens, napi: Napi) -> BindgenResult<TokenStream> {
  let mut tokens = TokenStream::new();
  item.to_tokens(&mut tokens);
  napi.try_to_tokens(&mut tokens)?;
  typedef::output_type_def(&napi);
  Ok(tokens)
}

fn validate_fn_attrs(f: &syn::ItemFn, opts: &FnAttrs) -> BindgenResult<()> {
  if opts.ts_type.is_some() && (opts.ts_args_type.is_some() || opts.ts_return_type.is_some()) {
    bail_span!(
      f,
      "#[napi] with ts_type cannot be combined with ts_args_type, ts_return_type in function"
    );
  }
  if opts.return_if_invalid.is_present() && opts.strict.is_present() {
    bail_span!(
      f,
      "#[napi(return_if_invalid)] can't be used with #[napi(strict)]"
    );
  }
  Ok(())
}

fn expand_mod(attr: TokenStream, mut js_mod: syn::ItemMod) -> BindgenResult<TokenStream> {
  let mod_opts: ModAttrs = parse_napi_attr(attr)?;
  let js_name = mod_opts
    .js_name
    .map(|fs| fs.value)
    .unwrap_or_else(|| js_mod.ident.to_string());

  let mut tokens = TokenStream::new();

  if let Some((_, mut items)) = js_mod.content.clone() {
    for item in items.iter_mut() {
      let mut empty_attrs = vec![];
      let item_attrs = match item {
        Item::Fn(ref mut f) => &mut f.attrs,
        Item::Struct(ref mut s) => &mut s.attrs,
        Item::Enum(ref mut e) => &mut e.attrs,
        Item::Const(ref mut c) => &mut c.attrs,
        Item::Impl(ref mut i) => &mut i.attrs,
        Item::Mod(m) => {
          let has_napi = m.attrs.iter().any(|a| a.path().is_ident("napi"));
          if has_napi {
            bail_span!(m, "napi module cannot be nested under another napi module");
          }
          &mut empty_attrs
        }
        _ => &mut empty_attrs,
      };

      let has_napi = item_attrs.iter().any(|a| a.path().is_ident("napi"));
      if !has_napi {
        item.to_tokens(&mut tokens);
        continue;
      }

      match item {
        Item::Fn(f) => {
          if let Some(opts) =
            find_napi_attr_with_namespace::<FnAttrs>(&mut f.attrs, &js_name)?
          {
            validate_fn_attrs(f, &opts)?;
            let napi = convert_fn(f, &opts)?;
            emit_into(&mut tokens, f, napi)?;
          } else {
            f.to_tokens(&mut tokens);
          }
        }
        Item::Struct(s) => {
          if let Some(opts) =
            find_napi_attr_with_namespace::<StructAttrs>(&mut s.attrs, &js_name)?
          {
            let napi = convert_struct(s, &opts)?;
            emit_into(&mut tokens, s, napi)?;
          } else {
            s.to_tokens(&mut tokens);
          }
        }
        Item::Enum(e) => {
          if let Some(opts) =
            find_napi_attr_with_namespace::<EnumAttrs>(&mut e.attrs, &js_name)?
          {
            let napi = convert_enum(e, &opts)?;
            emit_into(&mut tokens, e, napi)?;
          } else {
            e.to_tokens(&mut tokens);
          }
        }
        Item::Const(c) => {
          if let Some(opts) =
            find_napi_attr_with_namespace::<ConstAttrs>(&mut c.attrs, &js_name)?
          {
            let napi = convert_const(c, &opts)?;
            emit_into(&mut tokens, c, napi)?;
          } else {
            c.to_tokens(&mut tokens);
          }
        }
        Item::Impl(i) => {
          if let Some(opts) =
            find_napi_attr_with_namespace::<ImplAttrs>(&mut i.attrs, &js_name)?
          {
            let napi = convert_impl(i, &opts)?;
            emit_into(&mut tokens, i, napi)?;
          } else {
            i.to_tokens(&mut tokens);
          }
        }
        other => other.to_tokens(&mut tokens),
      }
    }
    js_mod.content = None;
  };

  let js_mod_attrs: Vec<syn::Attribute> = js_mod
    .attrs
    .clone()
    .into_iter()
    .filter(|attr| attr.path().is_ident("napi"))
    .collect();
  let mod_name = js_mod.ident;
  let visible = js_mod.vis;
  let mod_tokens = quote! { #(#js_mod_attrs)* #visible mod #mod_name { #tokens } };
  Ok(mod_tokens)
}

fn emit_into(
  tokens: &mut TokenStream,
  item: &impl ToTokens,
  napi: Napi,
) -> BindgenResult<()> {
  item.to_tokens(tokens);
  napi.try_to_tokens(tokens)?;
  typedef::output_type_def(&napi);
  Ok(())
}
