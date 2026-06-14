use proc_macro2::{Ident, Literal, Span, TokenStream};
use quote::ToTokens;
use syn::{spanned::Spanned, Type, TypePath, TypeReference};

use crate::{
  codegen::{get_intermediate_ident, js_mod_to_token_stream},
  types::{
    classify::{ClassInput, ClassInputKind, ResolvedTag, SpecialKind},
    inspect::{
      extract_vec_element_type, is_abort_signal, is_external, is_js_arg_slice, resolve_class_type,
      NapiTypeExt,
    },
    resolve::resolve_return_type,
  },
  BindgenResult, CallbackArg, Diagnostic, FnKind, FnSelf, NapiFn, NapiFnArgKind, TryToTokens,
};

fn callback_this_expr() -> TokenStream {
  quote! { napi::__private::callback_frame_this(&frame) }
}

fn callback_arg_expr(index: usize) -> TokenStream {
  quote! { napi::__private::callback_frame_arg(&frame, #index)? }
}

fn into_js_raw(raw_env: TokenStream, value: TokenStream) -> TokenStream {
  quote! {
    {
      let mut return_env = unsafe { napi::bindgen_prelude::Env::from_raw(#raw_env) };
      return_env.with_scope(|scope| {
        napi::bindgen_prelude::IntoJs::into_js(#value, scope).map(|local| local.raw())
      })
    }
  }
}

fn into_js_frame(value: TokenStream) -> TokenStream {
  quote! {
    {
      let mut return_env = unsafe { napi::bindgen_prelude::Env::from_raw(env) };
      return_env.with_scope(|scope| {
        napi::bindgen_prelude::IntoJs::into_js(#value, scope).map(|local| local.raw())
      })
    }
  }
}

fn into_js_reuse_scope(value: TokenStream) -> TokenStream {
  quote! {
    napi::bindgen_prelude::IntoJs::into_js(#value, frame.scope_mut()).map(|local| local.raw())
  }
}
fn arg_needs_class_context(arg: &crate::NapiFnArg) -> bool {
  match &arg.kind {
    crate::NapiFnArgKind::PatType(path) => path.ty.needs_class_context(),
    crate::NapiFnArgKind::Callback(_) => false,
  }
}

fn class_receiver_expr(input: &ClassInput<'_>, class: &TokenStream) -> TokenStream {
  match input.kind() {
    ClassInputKind::Ref => quote! { frame.this_reference::<#class>()? },
    ClassInputKind::ClassRef => quote! { frame.this_class_ref::<#class>()? },
    ClassInputKind::Borrow => quote! { frame.this_class::<#class>()? },
    ClassInputKind::BorrowMut => quote! { frame.this_class_mut::<#class>()? },
  }
}

#[cfg(feature = "tracing")]
fn gen_tracing_debug(js_name: &str, parent_js_name: Option<&String>) -> TokenStream {
  let full_name = if let Some(parent) = parent_js_name {
    format!("{}::{}", parent, js_name)
  } else {
    js_name.to_string()
  };
  quote! {
    napi::bindgen_prelude::tracing::debug!(target: "napi", "{}", #full_name);
  }
}

#[cfg(not(feature = "tracing"))]
fn gen_tracing_debug(_js_name: &str, _parent_js_name: Option<&String>) -> TokenStream {
  quote! {}
}

impl TryToTokens for NapiFn {
  fn try_to_tokens(&self, tokens: &mut TokenStream) -> BindgenResult<()> {
    let name_str = self.name.to_string();
    let intermediate_ident = get_intermediate_ident(&name_str);
    let args_len = self.args.len();
    let has_rest = self
      .args
      .iter()
      .any(|arg| arg.inject == Some(crate::InjectKind::Rest));
    let needs_class_context = self.needs_class_context();

    if self.is_async && self.parent().is_some() && self.fn_self().is_some() {
      return Err(Diagnostic::span_error(
        self.name.span(),
        "async napi class methods cannot borrow self across an await point; use an owned class reference or make the method synchronous",
      ));
    }

    let ArgConversions {
      arg_conversions,
      args: arg_names,
      refs,
      mut_ref_spans,
      unsafe_,
    } = self.gen_arg_conversions()?;
    let attrs = &self.attrs;
    let arg_ref_count = refs.len();
    let receiver = self.gen_fn_receiver()?;
    let receiver_ret_name = Ident::new("_ret", Span::call_site());
    let ret = self.gen_fn_return(&receiver_ret_name, quote! { env })?;
    let register = self.gen_fn_register();
    let tracing_debug = gen_tracing_debug(&self.js_name, self.class.as_ref().map(|c| &c.js_name));

    if self.is_module_exports() {
      (quote! {
        #(#attrs)*
        #[doc(hidden)]
        #[allow(non_snake_case)]
        #[allow(clippy::all)]
        unsafe extern "C" fn #intermediate_ident(
          env: napi::bindgen_prelude::sys::napi_env,
          _napi_module_exports_: napi::bindgen_prelude::sys::napi_value,
        ) -> napi::Result<napi::bindgen_prelude::sys::napi_value> {
          #tracing_debug
          unsafe {
            napi::bindgen_prelude::EnvRecord::enter_scope(env, |scope| {
                let env_wrapper = *scope.env();
                #(#arg_conversions)*
                let #receiver_ret_name = {
                  #receiver(#(#arg_names),*)
                };
                #ret
            })
          }
        }

        #register
      })
      .to_tokens(tokens);

      return Ok(());
    }

    // The JS engine can't properly track mutability in an async context, so refuse to compile
    // code that tries to use async and mutability together without `unsafe` mark.
    if self.is_async && !mut_ref_spans.is_empty() && !unsafe_ {
      return Diagnostic::from_vec(
        mut_ref_spans
          .into_iter()
          .map(|s| Diagnostic::span_error(s, "mutable reference is unsafe with async"))
          .collect(),
      );
    }
    if self.fn_self() == Some(&FnSelf::MutRef) && self.is_async && !self.unsafe_ {
      return Err(Diagnostic::span_error(
        self.name.span(),
        "&mut self in async napi methods should be marked as unsafe",
      ));
    }
    let build_ref_container = if self.is_async {
      quote! {
          let mut napi_args_ref = napi::__private::AsyncArgRefs::<#arg_ref_count>::new();
          #(#refs)*
      }
    } else {
      quote! {}
    };
    let native_call = if !self.is_async {
      quote! {
        let #receiver_ret_name = {
          #receiver(#(#arg_names),*)
        };
        #ret
      }
    } else {
      let call = if self.is_ret_result {
        quote! { #receiver(#(#arg_names),*).await }
      } else {
        quote! { Ok::<_, napi::Error>(#receiver(#(#arg_names),*).await) }
      };
      let async_completion = if matches!(self.kind, FnKind::Factory) {
        let parent = self.parent().ok_or_else(|| {
          Diagnostic::span_error(
            self.name.span(),
            "class factory return codegen requires a parent class",
          )
        })?;
        quote! {
          Ok(
            napi::bindgen_prelude::IntoClassInitializer::<#parent>::into_class_initializer(
              #receiver_ret_name,
            ),
          )
        }
      } else {
        quote! { Ok(#receiver_ret_name) }
      };
      let abort_signal_arg = self
        .args
        .iter()
        .enumerate()
        .find_map(|(i, arg)| match &arg.kind {
          NapiFnArgKind::PatType(p) if arg.inject.is_none() => {
            is_abort_signal(&p.ty).then(|| Ident::new(&format!("arg{i}"), Span::call_site()))
          }
          _ => None,
        });
      if let Some(signal_ident) = abort_signal_arg {
        quote! {
          unsafe {
            let async_env = napi::bindgen_prelude::Env::from_raw(env);
            let __napi_abort_cancel_cell = #signal_ident.cancel_cell().clone();
            let (promise, cancel_handle) = async_env.spawn_promise_cancellable(
              async move { #call },
              move |scope, result| {
                napi_args_ref.finalize(*scope.env());
                result.and_then(|#receiver_ret_name| #async_completion)
              },
            )?;
            __napi_abort_cancel_cell.set(Some(cancel_handle));
            Ok(napi::bindgen_prelude::JsValue::raw(&promise))
          }
        }
      } else {
        quote! {
          unsafe {
            let async_env = napi::bindgen_prelude::Env::from_raw(env);
            let promise = async_env.spawn_promise_with(
              async move { #call },
              move |scope, result| {
                napi_args_ref.finalize(*scope.env());
                result.and_then(|#receiver_ret_name| #async_completion)
              },
            )?;
            Ok(napi::bindgen_prelude::JsValue::raw(&promise))
          }
        }
      }
    };

    let internal_construction = if matches!(self.kind, FnKind::Constructor { .. }) {
      let parent = self.parent().ok_or_else(|| {
        Diagnostic::span_error(
          self.name.span(),
          "class constructor codegen requires a parent class",
        )
      })?;
      quote! {
        let __napi_constructor_receiver = frame.constructor_receiver::<#parent>()?;
        match unsafe {
          <#parent as napi::bindgen_prelude::NapiClass>::CLASS
            .try_wrap_internal_construction(__napi_constructor_receiver)?
        } {
          napi::bindgen_prelude::InternalConstructionResult::Wrapped(value) => return Ok(value),
          napi::bindgen_prelude::InternalConstructionResult::Absent => {}
        }
      }
    } else {
      quote! {}
    };

    let function_call = quote! {
      #internal_construction
      #build_ref_container
      #(#arg_conversions)*
      #native_call
    };

    let function_call = if args_len == 0
      && self.fn_self().is_none()
      && !matches!(self.kind, FnKind::Constructor { .. })
      && !matches!(self.kind, FnKind::Factory)
      && !self.is_async
      && !needs_class_context
    {
      quote! { #native_call }
    } else {
      function_call
    };

    let function_call = if self.catch_unwind {
      quote! {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
          #function_call
        }))
        .map_err(|payload| {
          let message = {
            if let Some(string) = payload.downcast_ref::<String>() {
              string.clone()
            } else if let Some(string) = payload.downcast_ref::<&str>() {
              string.to_string()
            } else {
              "panic from Rust code".to_owned()
            }
          };
          napi::Error::new(napi::Status::GenericFailure, message)
        })
        .and_then(|result| result)
      }
    } else {
      function_call
    };

    let entry_call = if has_rest {
      quote! {
        napi::__private::__napi_binding_entry_variadic(env, cb, #args_len, |mut frame| {
          #tracing_debug
          let mut env_wrapper = frame.env();
          let env = napi::__private::callback_frame_env(&frame);
          #function_call
        })
      }
    } else {
      quote! {
        napi::__private::__napi_binding_entry::<#args_len>(env, cb, |mut frame| {
          #tracing_debug
          let mut env_wrapper = frame.env();
          let env = napi::__private::callback_frame_env(&frame);
          #function_call
        })
      }
    };

    let type_def_register = self.gen_type_def_register();

    (quote! {
      #(#attrs)*
      #[doc(hidden)]
      #[allow(non_snake_case)]
      #[allow(clippy::all)]
      extern "C" fn #intermediate_ident(
        env: napi::bindgen_prelude::sys::napi_env,
        cb: napi::bindgen_prelude::sys::napi_callback_info
      ) -> napi::bindgen_prelude::sys::napi_value {
        unsafe {
          #entry_call
        }
      }

      #register
      #type_def_register
    })
    .to_tokens(tokens);

    Ok(())
  }
}

impl NapiFn {
  fn needs_class_context(&self) -> bool {
    (self.parent().is_some()
      && (self.fn_self().is_some()
        || matches!(self.kind, FnKind::Constructor { .. })
        || matches!(self.kind, FnKind::Factory)))
      || self
        .ret
        .as_ref()
        .is_some_and(|ty| ty.as_class_initializer(self.parent()).is_some())
      || self.args.iter().any(arg_needs_class_context)
  }

  pub(crate) fn gen_arg_conversions(&self) -> BindgenResult<ArgConversions> {
    let needs_class_context = self.needs_class_context();
    let cb_this = callback_this_expr();
    let scope_arg = self.args.iter().enumerate().find_map(|(arg_index, arg)| {
      (arg.inject == Some(crate::InjectKind::Scope))
        .then(|| Ident::new(&format!("arg{arg_index}"), Span::call_site()))
    });

    let mut resolved = Vec::with_capacity(self.args.len());
    let mut js_arg_index = 0usize;
    for (arg_index, arg) in self.args.iter().enumerate() {
      let ident = Ident::new(&format!("arg{arg_index}"), Span::call_site());
      let r = if let Some(inject) = arg.inject {
        let path = match &arg.kind {
          NapiFnArgKind::PatType(p) => p,
          _ => unreachable!("inject attributes are not valid on callback arguments"),
        };
        match inject {
          crate::InjectKind::Scope => ResolvedArg::injected(quote! { #ident }),
          crate::InjectKind::Env => {
            self.resolve_env_arg(&ident, path, needs_class_context, scope_arg.as_ref())
          }
          crate::InjectKind::This => self.resolve_this_arg(&ident, path, &cb_this)?,
          crate::InjectKind::Rest => {
            self.resolve_rest_arg(&ident, path, js_arg_index, scope_arg.as_ref())?
          }
        }
      } else {
        self.resolve_regular_arg(&ident, js_arg_index, arg, scope_arg.as_ref())?
      };
      if !r.is_injected {
        js_arg_index += 1;
      }
      resolved.push(r);
    }

    // Assemble: self receiver first, then flatten resolved args
    let mut arg_conversions = vec![];
    let mut deferred_conversions = vec![];

    if let Some(parent) = self.parent() {
      match self.fn_self() {
        Some(&FnSelf::Ref) => arg_conversions.push(quote! {
          let this = frame.this_class::<#parent>()?;
        }),
        Some(&FnSelf::MutRef) => arg_conversions.push(quote! {
          let mut this = frame.this_class_mut::<#parent>()?;
        }),
        _ => {}
      };
    }

    let mut args = Vec::with_capacity(resolved.len());
    let mut refs = vec![];
    let mut mut_ref_spans = vec![];

    for r in resolved {
      args.push(r.arg_expr);
      arg_conversions.extend(r.conversion);
      deferred_conversions.extend(r.deferred);
      refs.extend(r.reference);
      mut_ref_spans.extend(r.mut_ref_span);
    }

    if let Some(scope_arg) = scope_arg {
      arg_conversions.push(quote! {
        let #scope_arg = frame.scope_mut();
      });
      arg_conversions.extend(deferred_conversions);
    }

    Ok(ArgConversions {
      arg_conversions,
      args,
      refs,
      mut_ref_spans,
      unsafe_: self.unsafe_,
    })
  }

  fn resolve_env_arg(
    &self,
    ident: &Ident,
    path: &syn::PatType,
    needs_class_context: bool,
    scope_arg: Option<&Ident>,
  ) -> ResolvedArg {
    let is_ref = matches!(&*path.ty, syn::Type::Reference(_));
    if !is_ref {
      return if let Some(scope_arg) = scope_arg {
        let mut r = ResolvedArg::injected(quote! { #ident });
        r.deferred = Some(quote! { let #ident = *#scope_arg.env(); });
        r
      } else if needs_class_context {
        let mut r = ResolvedArg::injected(quote! { #ident });
        r.conversion = Some(quote! { let #ident = frame.env(); });
        r
      } else {
        ResolvedArg::injected(quote! { env_wrapper })
      };
    }

    let mutability = matches!(
      &*path.ty,
      syn::Type::Reference(syn::TypeReference {
        mutability: Some(_),
        ..
      })
    );
    let env_holder = Ident::new(&format!("{ident}_env"), Span::call_site());
    let conversion = if let Some(scope_arg) = scope_arg {
      if mutability {
        quote! {
          let mut #env_holder = *#scope_arg.env();
          let #ident = &mut #env_holder;
        }
      } else {
        quote! {
          let #ident = #scope_arg.env();
        }
      }
    } else if needs_class_context {
      if mutability {
        quote! {
          let mut #env_holder = frame.env();
          let #ident = &mut #env_holder;
        }
      } else {
        quote! {
          let #env_holder = frame.env();
          let #ident = &#env_holder;
        }
      }
    } else if mutability {
      quote! { let #ident = &mut env_wrapper; }
    } else {
      quote! { let #ident = &env_wrapper; }
    };

    let mut r = ResolvedArg::injected(quote! { #ident });
    if scope_arg.is_some() {
      r.deferred = Some(conversion);
    } else {
      r.conversion = Some(conversion);
    }
    r
  }

  fn resolve_this_arg(
    &self,
    ident: &Ident,
    path: &syn::PatType,
    cb_this: &TokenStream,
  ) -> BindgenResult<ResolvedArg> {
    if let Some(input) = path.ty.as_class_input() {
      let class = input.class_type(self.parent()).ok_or_else(|| {
        Diagnostic::span_error(
          path.ty.span(),
          "receiver-position class argument requires a concrete class type",
        )
      })?;
      let receiver = class_receiver_expr(&input, &class);
      let mut r = ResolvedArg::injected(quote! { #ident });
      r.conversion = Some(quote! { let #ident = #receiver; });
      if input.is_mut() {
        r.mut_ref_span = Some(path.ty.span());
      }
      return Ok(r);
    }

    if let Some(this_ty) = path.ty.this_inner() {
      return self.resolve_this_inner(this_ty, cb_this);
    }

    if path.ty.is_bare_this() {
      return Ok(ResolvedArg::injected(
        quote! { frame.this::<napi::bindgen_prelude::This>()? },
      ));
    }

    bail_span!(
      path.ty,
      "#[napi(this)] requires a This<T>, Ref<Class<T>>, ClassRef<T>, ClassBorrow<T>, ClassBorrowMut<T>, or similar receiver type"
    );
  }

  fn resolve_this_inner(
    &self,
    this_ty: &syn::Type,
    cb_this: &TokenStream,
  ) -> BindgenResult<ResolvedArg> {
    if let syn::Type::Path(ty_path) = this_ty {
      if let Some(segment) = ty_path.path.segments.first() {
        if let Some((primitive_type, _)) = crate::PRIMITIVE_TYPES
          .iter()
          .find(|(p, _)| segment.ident == *p)
        {
          bail_span!(
            segment.ident,
            "This type must not be {} \nthis in JavaScript function must be `Object` type or `undefined`",
            primitive_type
          );
        }
        return Ok(ResolvedArg::injected(
          quote! { frame.this::<napi::bindgen_prelude::This<#this_ty>>()? },
        ));
      }
    }

    if let syn::Type::Reference(syn::TypeReference {
      elem, mutability, ..
    }) = this_ty
    {
      if is_external(elem) {
        let mut r = ResolvedArg::injected(if mutability.is_some() {
          quote! { frame.this::<napi::bindgen_prelude::This<&mut #elem>>()? }
        } else {
          quote! { frame.this::<napi::bindgen_prelude::This<&#elem>>()? }
        });
        r.reference = Some(make_ref(quote! { #cb_this }));
        if mutability.is_some() {
          r.mut_ref_span = Some(this_ty.span());
        }
        return Ok(r);
      }

      let class = resolve_class_type(elem, self.parent()).ok_or_else(|| {
        Diagnostic::span_error(
          elem.span(),
          "napi class receiver requires a concrete class type",
        )
      })?;
      let mut r = if mutability.is_some() {
        ResolvedArg::injected(quote! {{
          let mut this_class_ref = frame.this_class_mut::<#class>()?;
          napi::bindgen_prelude::This::from(&mut *this_class_ref)
        }})
      } else {
        ResolvedArg::injected(quote! {{
          let this_class_ref = frame.this_class::<#class>()?;
          napi::bindgen_prelude::This::from(&*this_class_ref)
        }})
      };
      if mutability.is_some() {
        r.mut_ref_span = Some(this_ty.span());
      }
      return Ok(r);
    }

    let mut r = ResolvedArg::injected(quote! { frame.this::<napi::bindgen_prelude::This>()? });
    r.reference = Some(make_ref(quote! { #cb_this }));
    Ok(r)
  }

  fn resolve_rest_arg(
    &self,
    ident: &Ident,
    path: &syn::PatType,
    js_arg_index: usize,
    scope_arg: Option<&Ident>,
  ) -> BindgenResult<ResolvedArg> {
    let rest_from = js_arg_index;
    let conversion = if is_js_arg_slice(&path.ty) {
      quote! { let #ident = frame.rest_args(#rest_from); }
    } else {
      let elem = extract_vec_element_type(&path.ty).ok_or_else(|| {
        Diagnostic::spanned_error(
          &path.ty,
          "#[napi(rest)] parameter must be Vec<T> or JsArgSlice",
        )
      })?;
      if let Some(scope_arg) = scope_arg {
        quote! { let #ident = frame.rest_args(#rest_from).collect::<#elem>(#scope_arg)?; }
      } else {
        quote! {
          let __rest_slice = frame.rest_args(#rest_from);
          let #ident = __rest_slice.collect::<#elem>(frame.scope_mut())?;
        }
      }
    };

    let mut r = ResolvedArg::injected(quote! { #ident });
    if scope_arg.is_some() {
      r.deferred = Some(conversion);
    } else {
      r.conversion = Some(conversion);
    }
    Ok(r)
  }

  fn resolve_regular_arg(
    &self,
    ident: &Ident,
    js_arg_index: usize,
    arg: &crate::NapiFnArg,
    scope_arg: Option<&Ident>,
  ) -> BindgenResult<ResolvedArg> {
    match &arg.kind {
      NapiFnArgKind::PatType(path) => {
        let mut refs = vec![];
        let mut mut_ref_spans = vec![];
        let (arg_conversion, decode_after_scope) = self.gen_ty_arg_conversion(
          ident,
          js_arg_index,
          path,
          scope_arg,
          &mut refs,
          &mut mut_ref_spans,
        )?;
        let mut r = ResolvedArg::regular(quote! { #ident }, quote! {});
        r.reference = refs.into_iter().next();
        r.mut_ref_span = mut_ref_spans.into_iter().next();
        if decode_after_scope {
          let raw_arg_ident = Ident::new(&format!("js_arg{js_arg_index}_raw"), Span::call_site());
          let cb_arg = callback_arg_expr(js_arg_index);
          r.conversion = Some(quote! { let #raw_arg_ident = #cb_arg; });
          r.deferred = Some(arg_conversion);
        } else {
          r.conversion = Some(arg_conversion);
        }
        Ok(r)
      }
      NapiFnArgKind::Callback(cb) => {
        let conversion = self.gen_cb_arg_conversion(ident, js_arg_index, cb)?;
        Ok(ResolvedArg::regular(quote! { #ident }, conversion))
      }
    }
  }

  fn gen_ty_arg_conversion(
    &self,
    arg_name: &Ident,
    index: usize,
    path: &syn::PatType,
    scope_arg: Option<&Ident>,
    refs: &mut Vec<TokenStream>,
    mut_ref_spans: &mut Vec<Span>,
  ) -> BindgenResult<(TokenStream, bool)> {
    let cb_arg = callback_arg_expr(index);
    let raw_arg_ident = Ident::new(&format!("js_arg{index}_raw"), Span::call_site());
    let scoped_cb_arg = if scope_arg.is_some() {
      quote! { #raw_arg_ident }
    } else {
      cb_arg.clone()
    };

    let resolved = crate::types::resolve::resolve_arg_type(&path.ty, self.parent());

    // Class input types: ClassRef, ClassBorrow, &Class, Option<ClassRef> etc.
    if resolved.kind.needs_class_context() {
      let conversion = resolved.kind.emit_from_js(quote! { #index });
      let is_bare_ref = matches!(&*path.ty, Type::Reference(_));
      let q = if is_bare_ref {
        if let Some((kind, _)) = resolved.kind.as_class_input_info() {
          match kind {
            crate::types::classify::ClassInputKind::Borrow => {
              let borrow_ident = Ident::new(&format!("{arg_name}_borrow"), Span::call_site());
              quote! {
                let #borrow_ident = #conversion;
                let #arg_name = &*#borrow_ident;
              }
            }
            crate::types::classify::ClassInputKind::BorrowMut => {
              let borrow_ident = Ident::new(&format!("{arg_name}_borrow"), Span::call_site());
              quote! {
                let mut #borrow_ident = #conversion;
                let #arg_name = &mut *#borrow_ident;
              }
            }
            _ => quote! { let #arg_name = #conversion; },
          }
        } else {
          quote! { let #arg_name = #conversion; }
        }
      } else {
        quote! { let #arg_name = #conversion; }
      };
      if resolved.kind.needs_async_ref() {
        refs.push(make_ref(quote! { #cb_arg }));
      }
      if resolved.kind.needs_mut_ref() {
        mut_ref_spans.push(path.ty.span());
      }
      return Ok((q, false));
    }

    // Borrowed ref types (typed arrays, &External) — need ref tracking
    if matches!(resolved.tag(), ResolvedTag::BorrowedRef) {
      let conversion = resolved.kind.emit_from_js(quote! { #index });
      let q = quote! { let #arg_name = #conversion; };
      refs.push(make_ref(quote! { #cb_arg }));
      if resolved.kind.needs_mut_ref() {
        mut_ref_spans.push(path.ty.span());
      }
      return Ok((q, false));
    }

    // Special: External (non-ref, through FromJs)
    if matches!(resolved.tag(), ResolvedTag::Special(SpecialKind::External)) {
      let ty = &resolved.tokens;
      let q = quote! { let #arg_name = frame.arg::<#ty>(#index)?; };
      return Ok((q, false));
    }

    // Generic path: FromJs trait dispatch
    let mut ty = *path.ty.clone();
    hidden_ty_lifetime(&mut ty)?;

    // Vec<&T> needs async ref tracking
    if let syn::Type::Path(path) = &ty {
      if let Some(syn::PathSegment { ident, arguments }) = path.path.segments.first() {
        if ident == "Vec" {
          if let syn::PathArguments::AngleBracketed(args) = &arguments {
            if let Some(syn::GenericArgument::Type(syn::Type::Reference(_))) = args.args.first() {
              refs.push(make_ref(quote! { #scoped_cb_arg }));
            }
          }
        }
      }
    }

    let arg_conversion = if self.is_module_exports() {
      quote! { _napi_module_exports_ }
    } else {
      cb_arg.clone()
    };
    let from_js = if self.is_module_exports() {
      quote! {
        {
          let value = unsafe {
            napi::bindgen_prelude::Local::from_raw(#arg_conversion)
          };
          <#ty as napi::bindgen_prelude::FromJs>::from_js(scope, value)?
        }
      }
    } else if let Some(scope_arg) = scope_arg {
      quote! {
        {
          let value = unsafe {
            napi::bindgen_prelude::Local::from_raw(#raw_arg_ident)
          };
          <#ty as napi::bindgen_prelude::FromJs>::from_js(#scope_arg, value)?
        }
      }
    } else {
      quote! { frame.arg::<#ty>(#index)? }
    };

    let q = quote! { let #arg_name = #from_js; };
    Ok((q, scope_arg.is_some()))
  }

  fn gen_cb_arg_conversion(
    &self,
    arg_name: &Ident,
    index: usize,
    cb: &CallbackArg,
  ) -> BindgenResult<TokenStream> {
    let mut inputs = vec![];
    let mut arg_conversions = vec![];

    for (i, ty) in cb.args.iter().enumerate() {
      let cb_arg_ident = Ident::new(&format!("callback_arg_{i}"), Span::call_site());
      inputs.push(quote! { #cb_arg_ident: #ty });
      let arg_conversion = into_js_raw(quote! { env }, quote! { #cb_arg_ident });
      arg_conversions.push(quote! {
        #arg_conversion?
      });
    }

    let ret = match &cb.ret {
      Some(ty) => {
        quote! {
          let ret = unsafe {
            napi::bindgen_prelude::EnvRecord::enter_scope(env, |scope| {
                let value = napi::bindgen_prelude::Local::from_raw(ret_ptr);
                <#ty as napi::bindgen_prelude::FromJs>::from_js(scope, value)
            })
          }?;

          Ok(ret)
        }
      }
      None => quote! { Ok(()) },
    };
    let cb_this = Ident::new(&format!("{arg_name}_this_raw"), Span::call_site());
    let cb_arg = Ident::new(&format!("{arg_name}_raw"), Span::call_site());
    let cb_this_expr = callback_this_expr();
    let cb_arg_expr = callback_arg_expr(index);

    Ok(quote! {
      let #cb_this = #cb_this_expr;
      let #cb_arg = #cb_arg_expr;
      napi::__private::callback_frame_assert_value_type(&frame, #cb_arg, napi::bindgen_prelude::ValueType::Function)?;
      let #arg_name = |#(#inputs),*| {
        let args = vec![
          #(#arg_conversions),*
        ];

        let mut ret_ptr = std::ptr::null_mut();

        napi::bindgen_prelude::check_pending_exception!(
          env,
          napi::bindgen_prelude::sys::napi_call_function(
            env,
            #cb_this,
            #cb_arg,
            args.len(),
            args.as_ptr(),
            &mut ret_ptr
          )
        )?;

        #ret
      };
    })
  }

  pub(crate) fn gen_fn_receiver(&self) -> BindgenResult<TokenStream> {
    let name = &self.name;

    match self.fn_self() {
      Some(&FnSelf::Value) => Err(Diagnostic::span_error(
        self.name.span(),
        "napi class methods cannot move self; use &self or &mut self",
      )),
      Some(&FnSelf::Ref) | Some(&FnSelf::MutRef) => Ok(quote! { this.#name }),
      None => match self.parent() {
        Some(class) => Ok(quote! { #class::#name }),
        None => Ok(quote! { #name }),
      },
    }
  }

  fn gen_fn_return(&self, ret: &Ident, raw_env: TokenStream) -> BindgenResult<TokenStream> {
    let needs_class_context = self.needs_class_context();
    let cb_access = quote! { frame };
    let cb_this = callback_this_expr();
    let js_name = &self.js_name;
    let has_scope_arg = self
      .args
      .iter()
      .any(|arg| arg.inject == Some(crate::InjectKind::Scope));
    let select_into_js = |value: TokenStream| -> TokenStream {
      if has_scope_arg {
        into_js_reuse_scope(value)
      } else {
        into_js_frame(value)
      }
    };

    if let Some(ty) = &self.ret {
      let resolved_ret = resolve_return_type(ty, self.parent());
      let is_return_self = matches!(
        resolved_ret.tag(),
        ResolvedTag::Special(SpecialKind::ReturnThis)
      );
      let class_initializer_return = match resolved_ret.tag() {
        ResolvedTag::Special(SpecialKind::ClassInitializer) => {
          resolved_ret.kind.special_tokens().cloned()
        }
        _ => None,
      };
      if matches!(self.kind, FnKind::Constructor { .. }) {
        let parent = self.parent().ok_or_else(|| {
          Diagnostic::span_error(
            self.name.span(),
            "class constructor return codegen requires a parent class",
          )
        })?;
        if self.is_ret_result {
          if self.parent_is_generator() {
            Ok(quote! { #cb_access.construct_generator::<false, _>(#js_name, #ret?) })
          } else if self.parent_is_async_generator() {
            Ok(quote! { #cb_access.construct_async_generator::<false, _>(#js_name, #ret?) })
          } else {
            let post_init_call = self.gen_post_init_call()?;
            Ok(quote! {
              match #ret {
                Ok(value) => {
                  let __class_init =
                    napi::bindgen_prelude::IntoClassInitializer::<#parent>::into_class_initializer(value);
                  let __napi_constructor_receiver = #cb_access.constructor_receiver::<#parent>()?;
                  let __napi_result = <#parent as napi::bindgen_prelude::NapiClass>::CLASS.wrap_receiver(
                    __napi_constructor_receiver,
                    __class_init,
                  )?;
                  #post_init_call
                  Ok(__napi_result)
                }
                Err(err) => {
                  napi::bindgen_prelude::JsError::from(err).throw_into(#raw_env);
                  Ok(std::ptr::null_mut())
                }
              }
            })
          }
        } else if self.parent_is_generator() {
          Ok(quote! { #cb_access.construct_generator::<false, #parent>(#js_name, #ret) })
        } else if self.parent_is_async_generator() {
          Ok(quote! { #cb_access.construct_async_generator::<false, #parent>(#js_name, #ret) })
        } else {
          let post_init_call = self.gen_post_init_call()?;
          Ok(quote! {
            {
              let __class_init =
                napi::bindgen_prelude::IntoClassInitializer::<#parent>::into_class_initializer(#ret);
              let __napi_constructor_receiver = #cb_access.constructor_receiver::<#parent>()?;
              let __napi_result = <#parent as napi::bindgen_prelude::NapiClass>::CLASS.wrap_receiver(
                __napi_constructor_receiver,
                __class_init,
              )?;
              #post_init_call
              Ok(__napi_result)
            }
          })
        }
      } else if matches!(self.kind, FnKind::Factory) {
        let parent = self.parent().ok_or_else(|| {
          Diagnostic::span_error(
            self.name.span(),
            "class factory return codegen requires a parent class",
          )
        })?;
        if self.is_ret_result {
          if self.parent_is_generator() {
            Ok(quote! { #cb_access.generator_factory(#js_name, #ret?) })
          } else if self.parent_is_async_generator() {
            Ok(quote! { #cb_access.async_generator_factory(#js_name, #ret?) })
          } else if self.is_async {
            Ok(quote! {
              {
                let __class_init =
                  napi::bindgen_prelude::IntoClassInitializer::<#parent>::into_class_initializer(#ret);
                napi::bindgen_prelude::EnvRecord::enter_scope(#raw_env, |scope| unsafe {
                    <#parent as napi::bindgen_prelude::NapiClass>::CLASS
                      .new_object_from_scope(scope, __class_init)
                })
              }
            })
          } else {
            Ok(quote! {
              match #ret {
                Ok(value) => {
                  let __class_init =
                    napi::bindgen_prelude::IntoClassInitializer::<#parent>::into_class_initializer(value);
                  unsafe {
                    <#parent as napi::bindgen_prelude::NapiClass>::CLASS
                      .new_object_from_initializer(frame.context_mut(), __class_init)
                  }
                }
                Err(err) => {
                  napi::bindgen_prelude::JsError::from(err).throw_into(#raw_env);
                  Ok(std::ptr::null_mut())
                }
              }
            })
          }
        } else if self.parent_is_generator() {
          Ok(quote! { #cb_access.generator_factory(#js_name, #ret) })
        } else if self.parent_is_async_generator() {
          Ok(quote! { #cb_access.async_generator_factory(#js_name, #ret) })
        } else if self.is_async {
          Ok(quote! {
            {
              let __class_init =
                napi::bindgen_prelude::IntoClassInitializer::<#parent>::into_class_initializer(#ret);
              napi::bindgen_prelude::EnvRecord::enter_scope(#raw_env, |scope| unsafe {
                  <#parent as napi::bindgen_prelude::NapiClass>::CLASS
                    .new_object_from_scope(scope, __class_init)
              })
            }
          })
        } else {
          Ok(quote! {
            {
              let __class_init =
                napi::bindgen_prelude::IntoClassInitializer::<#parent>::into_class_initializer(#ret);
              unsafe {
                <#parent as napi::bindgen_prelude::NapiClass>::CLASS
                  .new_object_from_initializer(frame.context_mut(), __class_init)
              }
            }
          })
        }
      } else if self.is_ret_result {
        if self.is_async {
          if let Some(class) = class_initializer_return.as_ref() {
            Ok(quote! {
              {
                let __class_init =
                  napi::bindgen_prelude::IntoClassInitializer::<#class>::into_class_initializer(#ret);
                napi::bindgen_prelude::EnvRecord::enter_scope(#raw_env, |scope| unsafe {
                    <#class as napi::bindgen_prelude::NapiClass>::CLASS
                      .new_object_from_scope(scope, __class_init)
                })
              }
            })
          } else {
            let ret_into_js = into_js_raw(raw_env.clone(), quote! { #ret });
            Ok(quote! {
              #ret_into_js
            })
          }
        } else if is_return_self {
          Ok(quote! { #ret.map(|_| #cb_this) })
        } else if needs_class_context {
          if let Some(class) = class_initializer_return.as_ref() {
            Ok(quote! {
              match #ret {
                Ok(value) => {
                  let __class_init =
                    napi::bindgen_prelude::IntoClassInitializer::<#class>::into_class_initializer(value);
                  unsafe {
                    <#class as napi::bindgen_prelude::NapiClass>::CLASS
                      .new_object_from_initializer(frame.context_mut(), __class_init)
                  }
                }
                Err(err) => {
                  napi::bindgen_prelude::JsError::from(err).throw_into(#raw_env);
                  Ok(std::ptr::null_mut())
                },
              }
            })
          } else {
            let value_into_js = select_into_js(quote! { value });
            Ok(quote! {
              match #ret {
                Ok(value) => #value_into_js,
                Err(err) => {
                  napi::bindgen_prelude::JsError::from(err).throw_into(#raw_env);
                  Ok(std::ptr::null_mut())
                },
              }
            })
          }
        } else {
          let value_into_js = select_into_js(quote! { value });
          Ok(quote! {
            match #ret {
              Ok(value) => #value_into_js,
              Err(err) => {
                napi::bindgen_prelude::JsError::from(err).throw_into(#raw_env);
                Ok(std::ptr::null_mut())
              },
            }
          })
        }
      } else if is_return_self {
        Ok(quote! { Ok(#cb_this) })
      } else if self.is_async {
        if let Some(class) = class_initializer_return.as_ref() {
          Ok(quote! {
            {
              let __class_init =
                napi::bindgen_prelude::IntoClassInitializer::<#class>::into_class_initializer(#ret);
              napi::bindgen_prelude::EnvRecord::enter_scope(#raw_env, |scope| unsafe {
                  <#class as napi::bindgen_prelude::NapiClass>::CLASS
                    .new_object_from_scope(scope, __class_init)
              })
            }
          })
        } else {
          let ret_into_js = into_js_raw(raw_env.clone(), quote! { #ret });
          Ok(quote! {
            #ret_into_js
          })
        }
      } else if needs_class_context {
        if let Some(class) = class_initializer_return.as_ref() {
          Ok(quote! {
            {
              let __class_init =
                napi::bindgen_prelude::IntoClassInitializer::<#class>::into_class_initializer(#ret);
              unsafe {
                <#class as napi::bindgen_prelude::NapiClass>::CLASS
                  .new_object_from_initializer(frame.context_mut(), __class_init)
              }
            }
          })
        } else {
          let ret_into_js = select_into_js(quote! { #ret });
          Ok(quote! {
            #ret_into_js
          })
        }
      } else {
        let ret_into_js = select_into_js(quote! { #ret });
        Ok(quote! {
          #ret_into_js
        })
      }
    } else {
      let unit_into_js = if self.is_async {
        into_js_raw(raw_env.clone(), quote! { () })
      } else {
        select_into_js(quote! { () })
      };
      Ok(quote! {
        #unit_into_js
      })
    }
  }

  fn gen_post_init_call(&self) -> BindgenResult<TokenStream> {
    if self.post_init_chain().is_empty() {
      return Ok(quote! {});
    }

    let calls: Vec<_> = self
      .post_init_chain()
      .iter()
      .map(|cls| quote! { #cls::__napi_post_init(&mut frame)?; })
      .collect();

    Ok(quote! { #(#calls)* })
  }

  fn gen_fn_register(&self) -> TokenStream {
    if self.parent().is_some() || cfg!(test) {
      quote! {}
    } else {
      let name_str = self.name.to_string();
      let js_name = Literal::string(&format!("{}\0", &self.js_name));
      let name_len = self.js_name.len();
      let module_register_name = &self.register_name;
      let intermediate_ident = get_intermediate_ident(&name_str);
      let js_mod_ident = js_mod_to_token_stream(self.js_mod.as_ref());
      let cb_name = Ident::new(
        &format!("_napi_rs_internal_register_{name_str}"),
        Span::call_site(),
      );

      if self.is_module_exports() {
        return quote! {
          #[doc(hidden)]
          #[allow(non_snake_case)]
          #[allow(clippy::all)]
          unsafe fn #cb_name(env: napi::bindgen_prelude::sys::napi_env, exports: napi::bindgen_prelude::sys::napi_value) -> napi::bindgen_prelude::Result<napi::bindgen_prelude::sys::napi_value> {
            #intermediate_ident(env, exports)?;
            Ok(exports)
          }

          #[cfg(not(test))]
          #[doc(hidden)]
          #[allow(non_upper_case_globals)]
          #[napi::__private::linkme::distributed_slice(napi::__private::MODULE_EXPORT_HOOK_DESCRIPTORS)]
          #[linkme(crate = napi::__private::linkme)]
          static #module_register_name: napi::__private::ModuleExportHookDescriptor =
            napi::__private::ModuleExportHookDescriptor {
              callback: #cb_name,
            };

        };
      }

      let register_module_export_tokens = if self.no_export {
        quote! {}
      } else {
        quote! {
          #[cfg(not(test))]
          #[doc(hidden)]
          #[allow(non_upper_case_globals)]
          #[napi::__private::linkme::distributed_slice(napi::__private::MODULE_EXPORT_DESCRIPTORS)]
          #[linkme(crate = napi::__private::linkme)]
          static #module_register_name: napi::__private::ModuleExportDescriptor =
            napi::__private::ModuleExportDescriptor {
              js_mod: #js_mod_ident,
              js_name: #js_name,
              callback: #cb_name,
            };

        }
      };

      quote! {
        #[doc(hidden)]
        #[allow(non_snake_case)]
        #[allow(clippy::all)]
        unsafe fn #cb_name(env: napi::bindgen_prelude::sys::napi_env) -> napi::bindgen_prelude::Result<napi::bindgen_prelude::sys::napi_value> {
          let mut fn_ptr = std::ptr::null_mut();

          napi::bindgen_prelude::check_status!(
            napi::bindgen_prelude::sys::napi_create_function(
              env,
              #js_name.as_ptr().cast(),
              #name_len as isize,
              Some(#intermediate_ident),
              std::ptr::null_mut(),
              &mut fn_ptr,
            ),
            "Failed to register function `{}`",
            #name_str,
          )?;
          Ok(fn_ptr)
        }

        #register_module_export_tokens
      }
    }
  }

  #[cfg(feature = "type-def")]
  fn gen_type_def_register(&self) -> TokenStream {
    if self.ts.skip_typescript
      || self.is_module_exports()
      || self.no_export
      || matches!(self.kind, FnKind::PostInit)
      || cfg!(test)
    {
      return quote! {};
    }

    let js_name = &self.js_name;
    let is_impl_method = self.parent().is_some();

    let kind = if is_impl_method { "impl" } else { "fn" };

    let name = if is_impl_method {
      let parent_str = self.parent().map(|p| p.to_string()).unwrap_or_default();
      self
        .parent_js_name()
        .map(|s| s.to_owned())
        .unwrap_or(parent_str)
    } else {
      js_name.clone()
    };

    let prefix = self.gen_ts_func_prefix_str();

    let def_body = if let Some(ts_type) = self.ts.ts_type.as_ref() {
      let ts_type_str = ts_type.clone();
      quote! { format!("{} {}{}", #prefix, #js_name, #ts_type_str) }
    } else {
      let generic = self
        .ts
        .ts_generic_types
        .as_ref()
        .map(|g| format!("<{g}>"))
        .unwrap_or_default();

      let args_body = if let Some(ts_args) = self.ts.ts_args_type.as_ref() {
        let ts_args_str = ts_args.clone();
        quote! { #ts_args_str.to_owned() }
      } else {
        self.gen_ts_func_args_tokens()
      };

      let ret_body = if let Some(ts_ret) = self.ts.ts_return_type.as_ref() {
        let ts_ret_str = format!(": {ts_ret}");
        quote! { #ts_ret_str.to_owned() }
      } else {
        self.gen_ts_func_ret_tokens()
      };

      quote! {
        {
          let args = #args_body;
          let ret = #ret_body;
          format!("{} {}{generic}({args}){ret}", #prefix, #js_name, generic = #generic, args = args, ret = ret)
        }
      }
    };

    let js_doc = crate::typegen::JSDoc::new(&self.comments);
    let js_doc_str = js_doc.to_string();

    let (final_def_body, final_js_doc) = if is_impl_method {
      let body = if !js_doc_str.is_empty() {
        quote! { format!("{}{}", #js_doc_str, #def_body) }
      } else {
        def_body
      };
      (body, crate::typegen::JSDoc::default())
    } else {
      (def_body, js_doc)
    };

    super::emit_type_def_descriptor(
      kind,
      &name,
      None,
      final_def_body,
      self.js_mod.as_ref(),
      &final_js_doc,
      None,
      None,
      &self.register_name,
      self.name.span(),
    )
  }

  #[cfg(not(feature = "type-def"))]
  fn gen_type_def_register(&self) -> TokenStream {
    quote! {}
  }

  fn gen_ts_func_prefix_str(&self) -> &'static str {
    if self.parent().is_some() {
      match &self.kind {
        FnKind::Normal => match self.fn_self() {
          Some(_) => "",
          None
            if self
              .args
              .iter()
              .any(|arg| arg.inject == Some(crate::InjectKind::This)) =>
          {
            ""
          }
          None => "static",
        },
        FnKind::Factory => "static",
        FnKind::Constructor { .. } => "",
        FnKind::Getter => "get",
        FnKind::Setter => "set",
        FnKind::PostInit => "",
        FnKind::ModuleExport => "",
      }
    } else {
      "function"
    }
  }

  #[cfg(feature = "type-def")]
  fn gen_ts_func_args_tokens(&self) -> TokenStream {
    use crate::typegen::tokens::{callback_to_ts_type_tokens, ty_to_ts_type_tokens};
    use crate::util::to_case;
    use convert_case::Case;

    let parent = self.parent();
    let is_setter = matches!(self.kind, FnKind::Setter);

    let mut arg_entries = Vec::new();

    for arg in &self.args {
      if let Some(inject) = arg.inject {
        match inject {
          crate::InjectKind::Env | crate::InjectKind::Scope => continue,
          crate::InjectKind::Rest => {
            let crate::NapiFnArgKind::PatType(path) = &arg.kind else {
              continue;
            };
            let mut path = path.clone();
            if let syn::Pat::Ident(i) = path.pat.as_mut() {
              i.mutability = None;
            }
            let arg_name = crate::typegen::gen_ts_func_arg_pub(&path.pat);

            let ts_override = arg.ts_arg_type.as_ref();
            let type_tokens = if let Some(ovr) = ts_override {
              let s = ovr.clone();
              quote! { #s.to_owned() }
            } else if is_js_arg_slice(&path.ty) {
              quote! { "unknown".to_owned() }
            } else if let Some(elem) = extract_vec_element_type(&path.ty) {
              ty_to_ts_type_tokens(elem, false, false, parent).0
            } else {
              quote! { "unknown".to_owned() }
            };

            arg_entries.push(quote! {
              parts.push(format!("...{}: {}[]", #arg_name, #type_tokens));
            });
            continue;
          }
          crate::InjectKind::This => {
            let crate::NapiFnArgKind::PatType(path) = &arg.kind else {
              continue;
            };
            if parent.is_some() || !matches!(self.kind, FnKind::Normal) {
              continue;
            }
            let ts_override = arg.ts_arg_type.as_ref();
            if let Some(ovr) = ts_override {
              let s = ovr.clone();
              arg_entries.push(quote! { this_arg = Some(format!("this: {}", #s)); });
              continue;
            }
            if let Some(input) = path.ty.as_class_input() {
              let type_tokens = ty_to_ts_type_tokens(input.inner(), false, false, parent).0;
              arg_entries.push(quote! { this_arg = Some(format!("this: {}", #type_tokens)); });
              continue;
            }
            if let Some(this_ty) = path.ty.this_inner() {
              let type_tokens = ty_to_ts_type_tokens(this_ty, false, false, parent).0;
              arg_entries.push(quote! { this_arg = Some(format!("this: {}", #type_tokens)); });
              continue;
            }
            if path.ty.is_bare_this() {
              arg_entries.push(quote! { this_arg = Some("this: this".to_owned()); });
              continue;
            }
            continue;
          }
        }
      }

      match &arg.kind {
        crate::NapiFnArgKind::PatType(path) => {
          let mut path = path.clone();
          if let syn::Pat::Ident(i) = path.pat.as_mut() {
            i.mutability = None;
          }
          let arg_name = crate::typegen::gen_ts_func_arg_pub(&path.pat);

          let ts_override = arg.ts_arg_type.as_ref();
          let (type_tokens, is_optional) = if let Some(ovr) = ts_override {
            let s = ovr.clone();
            (quote! { #s.to_owned() }, false)
          } else {
            ty_to_ts_type_tokens(&path.ty, false, false, parent)
          };

          arg_entries.push(quote! {
            args.push((#arg_name.to_owned(), #type_tokens, #is_optional));
          });
        }
        crate::NapiFnArgKind::Callback(cb) => {
          let ts_override = arg.ts_arg_type.as_ref();
          let type_tokens = if let Some(ovr) = ts_override {
            let s = ovr.clone();
            quote! { #s.to_owned() }
          } else {
            callback_to_ts_type_tokens(cb, parent)
          };
          let cb_name = to_case(cb.pat.to_token_stream().to_string(), Case::Camel);
          arg_entries.push(quote! {
            args.push((#cb_name.to_owned(), #type_tokens, false));
          });
        }
      }
    }

    let is_setter_lit = is_setter;

    quote! {
      {
        let mut this_arg: Option<String> = None;
        let mut args: Vec<(String, String, bool)> = Vec::new();
        let mut parts: Vec<String> = Vec::new();
        #( #arg_entries )*

        let last_required = args.iter().enumerate().rfind(|(_, (_, _, opt))| !opt).map(|(i, _)| i);

        let mut result = String::new();
        if let Some(this) = &this_arg {
          result.push_str(this);
        }
        for (i, (name, ty, optional)) in args.iter().enumerate() {
          if i != 0 || this_arg.is_some() {
            result.push_str(", ");
          }
          let show_optional = !#is_setter_lit && *optional && last_required.is_none_or(|lr| i > lr);
          if show_optional {
            result.push_str(&format!("{}?: {}", name, ty));
          } else {
            result.push_str(&format!("{}: {}", name, ty));
          }
        }
        for p in &parts {
          if !result.is_empty() { result.push_str(", "); }
          result.push_str(p);
        }
        result
      }
    }
  }

  #[cfg(feature = "type-def")]
  fn gen_ts_func_ret_tokens(&self) -> TokenStream {
    use crate::typegen::tokens::ty_to_ts_type_tokens;

    let parent = self.parent();

    match &self.kind {
      FnKind::Constructor { .. } | FnKind::Setter => {
        return quote! { String::new() };
      }
      FnKind::Factory => {
        if let Some(parent) = parent {
          let is_async = self.is_async;
          return quote! {
            {
              let parent_name = <#parent as napi::bindgen_prelude::TypeName>::ts_type();
              if #is_async {
                format!(": Promise<{}>", parent_name)
              } else {
                format!(": {}", parent_name)
              }
            }
          };
        }
        return quote! { String::new() };
      }
      _ => {}
    }

    let is_async = self.is_async;

    if let Some(ret) = &self.ret {
      let (type_tokens, _) = ty_to_ts_type_tokens(ret, true, false, parent);
      quote! {
        {
          let ts_type = #type_tokens;
          let ret = if ts_type == "undefined" { "void".to_owned() } else { ts_type };
          if #is_async {
            format!(": Promise<{}>", ret)
          } else {
            format!(": {}", ret)
          }
        }
      }
    } else {
      quote! {
        if #is_async {
          ": Promise<void>".to_owned()
        } else {
          ": void".to_owned()
        }
      }
    }
  }
}

fn hidden_ty_lifetime(ty: &mut syn::Type) -> BindgenResult<()> {
  match ty {
    Type::Path(TypePath {
      path: syn::Path { segments, .. },
      ..
    }) => {
      if let Some(syn::PathSegment {
        arguments:
          syn::PathArguments::AngleBracketed(syn::AngleBracketedGenericArguments { args, .. }),
        ..
      }) = segments.last_mut()
      {
        let mut has_lifetime = false;
        if let Some(syn::GenericArgument::Lifetime(lt)) = args.first_mut() {
          *lt = syn::Lifetime::new("'_", Span::call_site());
          has_lifetime = true;
        }
        for arg in args.iter_mut().skip(if has_lifetime { 1 } else { 0 }) {
          if let syn::GenericArgument::Type(ty) = arg {
            hidden_ty_lifetime(ty)?;
          }
        }
      }
    }
    Type::Reference(TypeReference {
      lifetime: Some(lt), ..
    }) => {
      *lt = syn::Lifetime::new("'_", Span::call_site());
    }
    _ => {}
  }
  Ok(())
}

fn make_ref(input: TokenStream) -> TokenStream {
  quote! {
    napi::__private::callback_frame_retain_value(&frame, &mut napi_args_ref, #input)?;
  }
}

pub(crate) struct ArgConversions {
  pub args: Vec<TokenStream>,
  pub arg_conversions: Vec<TokenStream>,
  pub refs: Vec<TokenStream>,
  pub mut_ref_spans: Vec<Span>,
  pub unsafe_: bool,
}

struct ResolvedArg {
  arg_expr: TokenStream,
  conversion: Option<TokenStream>,
  deferred: Option<TokenStream>,
  reference: Option<TokenStream>,
  mut_ref_span: Option<Span>,
  is_injected: bool,
}

impl ResolvedArg {
  fn injected(arg_expr: TokenStream) -> Self {
    Self {
      arg_expr,
      conversion: None,
      deferred: None,
      reference: None,
      mut_ref_span: None,
      is_injected: true,
    }
  }

  fn regular(arg_expr: TokenStream, conversion: TokenStream) -> Self {
    Self {
      arg_expr,
      conversion: Some(conversion),
      deferred: None,
      reference: None,
      mut_ref_span: None,
      is_injected: false,
    }
  }
}
