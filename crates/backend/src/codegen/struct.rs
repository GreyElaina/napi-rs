use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use proc_macro2::{Ident, Literal, Span, TokenStream};
use quote::ToTokens;

use crate::util::to_case;

use crate::{
  codegen::{get_intermediate_ident, js_mod_to_token_stream},
  type_semantics::{resolve_class_type, ClassInputKind, NapiTypeExt},
  BindgenResult, FnKind, NapiImpl, NapiStruct, NapiStructKind, TryToTokens,
};
use crate::{NapiArray, NapiClass, NapiObject, NapiStructuredEnum, NapiTransparent};

static NAPI_IMPL_ID: AtomicU32 = AtomicU32::new(0);

fn has_receiver_frame_input_arg(item: &crate::NapiFn) -> bool {
  item.args.iter().any(|arg| {
    let crate::NapiFnArgKind::PatType(path) = &arg.kind else {
      return false;
    };
    let syn::Pat::Ident(pat) = path.pat.as_ref() else {
      return false;
    };
    if pat.ident != "this" {
      return false;
    }
    path.ty.as_class_input().is_some()
  })
}

fn is_reference_class_type(ty: &syn::Type) -> bool {
  ty.as_class_input()
    .is_some_and(|input| input.kind().is_reference())
}

fn class_field_from_frame(ty: &syn::Type, index: TokenStream, owner: &Ident) -> TokenStream {
  if let Some(input) = ty.as_class_input() {
    if let Some(class) = input.class_type(Some(owner)) {
      return match input.kind() {
        ClassInputKind::Ref => quote! { frame.arg_reference::<#class>(#index)? },
        ClassInputKind::ClassRef => quote! { frame.arg_class_ref::<#class>(#index)? },
        ClassInputKind::Borrow => quote! { frame.arg_class::<#class>(#index)? },
        ClassInputKind::BorrowMut => quote! { frame.arg_class_mut::<#class>(#index)? },
      };
    }
  }

  if let Some(input) = ty.as_optional_class_input() {
    if let Some(class) = resolve_class_type(input.inner(), Some(owner)) {
      return match input.kind() {
        ClassInputKind::Ref => quote! { frame.arg_opt_reference::<#class>(#index)? },
        ClassInputKind::ClassRef => quote! { frame.arg_opt_class_ref::<#class>(#index)? },
        ClassInputKind::Borrow => quote! { frame.arg_opt_class::<#class>(#index)? },
        ClassInputKind::BorrowMut => quote! { frame.arg_opt_class_mut::<#class>(#index)? },
      };
    }
  }

  quote! {{
    frame.arg::<#ty>(#index)?
  }}
}

fn object_field_getter_from_scope(
  ty: &syn::Type,
  target: TokenStream,
  field_js_name: &str,
  missing_is_none: bool,
) -> TokenStream {
  let decode_ty = if missing_is_none {
    ty.option_inner().unwrap_or(ty).clone()
  } else {
    ty.clone()
  };
  quote! {
    scope.get_optional_named_property::<#decode_ty, _>(&#target, #field_js_name)
  }
}

fn optional_reference_field_inner(ty: &syn::Type, owner: &Ident) -> Option<TokenStream> {
  let input = ty.as_optional_class_input()?;
  if !input.kind().is_reference() {
    return None;
  }
  let class = input.class_type(Some(owner))?;
  Some(match input.kind() {
    ClassInputKind::Ref => quote! { napi::bindgen_prelude::Ref<napi::bindgen_prelude::Class<#class>> },
    ClassInputKind::ClassRef => quote! { napi::bindgen_prelude::ClassRef<#class> },
    _ => unreachable!(),
  })
}

fn class_field_from_object_scope(
  ty: &syn::Type,
  field_js_name: &str,
  owner: &Ident,
  missing_is_none: bool,
  missing_context: TokenStream,
) -> Option<TokenStream> {
  let decode_ty = if is_reference_class_type(ty) {
    quote! { #ty }
  } else if let Some(inner) = optional_reference_field_inner(ty, owner) {
    if missing_is_none {
      return Some(quote! {
        scope.get_optional_named_property::<#inner, _>(&obj, #field_js_name)
      });
    }
    quote! { #ty }
  } else {
    return None;
  };

  Some(quote! {
    scope.get_optional_named_property::<#decode_ty, _>(&obj, #field_js_name)?.ok_or_else(|| napi::bindgen_prelude::Error::new(
      napi::bindgen_prelude::Status::InvalidArg,
      #missing_context,
    ))
  })
}

fn into_js_frame(value: TokenStream) -> TokenStream {
  quote! {
    {
      let mut return_env = frame.env();
      return_env.with_scope(|scope| {
        napi::bindgen_prelude::IntoJs::into_js(#value, scope).map(|local| local.raw())
      })
    }
  }
}

fn class_field_into_js(ty: &syn::Type, field: &syn::Member) -> Option<TokenStream> {
  if is_reference_class_type(ty) {
    return Some(quote! {
      let scope = frame.context_mut().scope_mut();
      let val = obj.#field.clone(scope)?;
      napi::bindgen_prelude::IntoJs::into_js(val, scope).map(|local| local.raw())
    });
  }

  if ty
    .as_optional_class_input()
    .is_some_and(|input| input.kind().is_reference())
  {
    let undefined = into_js_frame(quote! { () });
    return Some(quote! {
      match obj.#field.as_ref() {
        Some(reference) => {
          let scope = frame.context_mut().scope_mut();
          let val = reference.clone(scope)?;
          napi::bindgen_prelude::IntoJs::into_js(val, scope).map(|local| local.raw())
        }
        None => #undefined,
      }
    });
  }

  None
}

#[cfg(feature = "tracing")]
fn gen_tracing_debug(class_name: &str, method_name: &str) -> TokenStream {
  let full_name = format!("{}::{}", class_name, method_name);
  quote! {
    napi::bindgen_prelude::tracing::debug!(target: "napi", "{}", #full_name);
  }
}

#[cfg(not(feature = "tracing"))]
fn gen_tracing_debug(_class_name: &str, _method_name: &str) -> TokenStream {
  quote! {}
}

// Generate trait implementations for given Struct.
fn gen_napi_value_map_impl(
  name: &Ident,
  to_napi_val_impl: TokenStream,
  has_lifetime: bool,
) -> TokenStream {
  let name_str = name.to_string();
  let name = if has_lifetime {
    quote! { #name<'_> }
  } else {
    quote! { #name }
  };
  let validate = quote! {
    unsafe fn validate(env: napi::sys::napi_env, napi_val: napi::sys::napi_value) -> napi::Result<napi::sys::napi_value> {
      let mut env_wrapper = unsafe { napi::bindgen_prelude::Env::from_raw(env) };
      env_wrapper.with_scope(|scope| {
        unsafe {
          <#name as napi::bindgen_prelude::NapiReceiver>::validate_raw_object(scope, napi_val)?;
        }
        Ok(std::ptr::null_mut())
      })
    }
  };
  quote! {
    #[automatically_derived]
    impl napi::bindgen_prelude::TypeName for #name {
      fn type_name() -> &'static str {
        #name_str
      }

      fn value_type() -> napi::ValueType {
        napi::ValueType::Function
      }
    }

    #[automatically_derived]
    impl napi::bindgen_prelude::TypeName for &#name {
      fn type_name() -> &'static str {
        #name_str
      }

      fn value_type() -> napi::ValueType {
        napi::ValueType::Object
      }
    }

    #[automatically_derived]
    impl napi::bindgen_prelude::TypeName for &mut #name {
      fn type_name() -> &'static str {
        #name_str
      }

      fn value_type() -> napi::ValueType {
        napi::ValueType::Object
      }
    }

    #to_napi_val_impl

    #[automatically_derived]
    impl napi::bindgen_prelude::ValidateNapiValue for &#name {
      #validate
    }

    #[automatically_derived]
    impl napi::bindgen_prelude::ValidateNapiValue for &mut #name {
      #validate
    }
  }
}

impl TryToTokens for NapiStruct {
  fn try_to_tokens(&self, tokens: &mut TokenStream) -> BindgenResult<()> {
    let napi_value_map_impl = self.gen_napi_value_map_impl();

    let class_helper_mod = match &self.kind {
      NapiStructKind::Class(class) => self.gen_helper_mod(class),
      _ => quote! {},
    };

    (quote! {
      #napi_value_map_impl
      #class_helper_mod
    })
    .to_tokens(tokens);

    Ok(())
  }
}

impl NapiStruct {
  fn gen_helper_mod(&self, class: &NapiClass) -> TokenStream {
    let mod_name = Ident::new(&format!("__napi_helper__{}", self.name), Span::call_site());

    let ctor = self.gen_default_ctor(class);

    let mut getters_setters = self.gen_default_getters_setters(class);
    getters_setters.sort_by(|a, b| a.0.cmp(&b.0));
    let register = self.gen_register(class);
    let class_runtime = if self.has_lifetime {
      quote! {}
    } else {
      self.gen_class_runtime(class)
    };

    let getters_setters_token = getters_setters.into_iter().map(|(_, token)| token);

    quote! {
      #[allow(clippy::all)]
      #[allow(non_snake_case)]
      mod #mod_name {
        use std::ptr;
        use super::*;

        #ctor
        #(#getters_setters_token)*
        #class_runtime
        #register
      }
    }
  }

  fn gen_class_runtime(&self, class: &NapiClass) -> TokenStream {
    let name = &self.name;
    let js_name = &self.js_name;
    let rust_name = self.name.to_string();
    let subclassable = class.subclass;
    let info_name = Ident::new("__NAPI_CLASS_INFO", Span::call_site());
    let def_name = Ident::new("__NAPI_CLASS_DEF", Span::call_site());
    let layout_name = Ident::new("__NAPI_CLASS_LAYOUT", Span::call_site());
    let entry_name = Ident::new("__NAPI_CLASS_ENTRY", Span::call_site());
    let layout_ty = Ident::new("__NapiClassLayout", Span::call_site());
    let layout_fn = Ident::new("__napi_class_layout", Span::call_site());
    let drop_fn = Ident::new("__napi_drop_initialized", Span::call_site());

    let subclass_impl = if class.subclass {
      quote! {
        unsafe impl napi::bindgen_prelude::NapiSubclass for #name {}
      }
    } else {
      quote! {}
    };

    if let Some(parent) = &class.parent {
      let parent = &parent.rust_path;
      quote! {
        #[repr(C)]
        pub struct #layout_ty {
          parent: <#parent as napi::bindgen_prelude::ClassChain>::Layout,
          value: std::mem::MaybeUninit<#name>,
        }

        static #info_name: napi::bindgen_prelude::ClassInfo = unsafe {
          napi::bindgen_prelude::ClassInfo::new(#rust_name, #js_name, #subclassable)
        };

        static #entry_name: napi::bindgen_prelude::ClassEntry = unsafe {
          napi::bindgen_prelude::ClassEntry::new(
            &#info_name,
            std::mem::offset_of!(#layout_ty, value),
          )
        };

        static #layout_name: napi::bindgen_prelude::ClassLayout = unsafe {
          napi::bindgen_prelude::ClassLayout::new(
            Some(<#parent as napi::bindgen_prelude::ClassChain>::LAYOUT),
            #entry_name,
            std::mem::size_of::<#layout_ty>(),
            std::mem::align_of::<#layout_ty>(),
            #drop_fn,
          )
        };

        fn #layout_fn() -> &'static napi::bindgen_prelude::ClassLayout {
          &#layout_name
        }

        static #def_name: napi::bindgen_prelude::ClassDef<#name> = unsafe {
          napi::bindgen_prelude::ClassDef::new(&#info_name, #layout_fn)
        };

        unsafe impl napi::bindgen_prelude::NapiClass for #name {
          type Parent = #parent;

          const CLASS: &'static napi::bindgen_prelude::ClassDef<Self> = &#def_name;
        }

        unsafe impl napi::bindgen_prelude::NapiReceiver for #name {
          type Access = napi::bindgen_prelude::ClassAccess;

          type Borrow<'a> = napi::bindgen_prelude::ClassBorrow<'a, Self>
          where
            Self: 'a;

          type BorrowMut<'a> = napi::bindgen_prelude::ClassBorrowMut<'a, Self>
          where
            Self: 'a;

          unsafe fn validate_raw_object<'scope>(
            scope: &mut napi::bindgen_prelude::Scope<'_, 'scope>,
            object: napi::bindgen_prelude::sys::napi_value,
          ) -> napi::Result<(Self::Access, napi::bindgen_prelude::ClassStorageRef<'scope>)> {
            unsafe { napi::bindgen_prelude::ClassStorageRef::validate_raw_object(
              scope,
              object,
              <Self as napi::bindgen_prelude::NapiClass>::CLASS.info(),
            ) }
          }

          unsafe fn ref_from_validated_object<'scope>(
            storage: napi::bindgen_prelude::ClassStorageRef<'scope>,
            access: Self::Access,
          ) -> napi::Result<Self::Borrow<'scope>> {
            unsafe {
              napi::bindgen_prelude::ClassBorrow::from_validated_parts(storage, access)
            }
          }

          unsafe fn mut_from_validated_object<'scope>(
            storage: napi::bindgen_prelude::ClassStorageRef<'scope>,
            access: Self::Access,
          ) -> napi::Result<Self::BorrowMut<'scope>> {
            unsafe {
              napi::bindgen_prelude::ClassBorrowMut::from_validated_parts(storage, access)
            }
          }
        }

        #subclass_impl

        unsafe impl napi::bindgen_prelude::ClassChain for #name {
          type Layout = #layout_ty;

          const LAYOUT: &'static napi::bindgen_prelude::ClassLayout = &#layout_name;

          unsafe fn write_init(
            init: napi::bindgen_prelude::ClassInitializer<Self>,
            dst: std::ptr::NonNull<Self::Layout>,
          ) {
            let (value, parent) = init.into_value_and_parent();
            let layout = dst.as_ptr();
            unsafe {
              <#parent as napi::bindgen_prelude::ClassChain>::write_init(
                parent,
                std::ptr::NonNull::new_unchecked(&mut (*layout).parent),
              );
              (*layout).value.write(value);
            }
          }

          unsafe fn drop_segments(data: std::ptr::NonNull<Self::Layout>) {
            let layout = data.as_ptr();
            unsafe {
              napi::bindgen_prelude::drop_segment(
                std::ptr::NonNull::new_unchecked((*layout).value.as_mut_ptr()),
              );
              <#parent as napi::bindgen_prelude::ClassChain>::drop_segments(
                std::ptr::NonNull::new_unchecked(&mut (*layout).parent),
              );
            }
          }

          unsafe fn drop_initialized(data: std::ptr::NonNull<u8>) {
            unsafe {
              <Self as napi::bindgen_prelude::ClassChain>::drop_segments(data.cast());
            }
          }
        }

        unsafe fn #drop_fn(data: std::ptr::NonNull<u8>) {
          unsafe { <#name as napi::bindgen_prelude::ClassChain>::drop_initialized(data) }
        }
      }
    } else {
      quote! {
        #[repr(C)]
        pub struct #layout_ty {
          value: std::mem::MaybeUninit<#name>,
        }

        static #info_name: napi::bindgen_prelude::ClassInfo = unsafe {
          napi::bindgen_prelude::ClassInfo::new(#rust_name, #js_name, #subclassable)
        };

        static #entry_name: napi::bindgen_prelude::ClassEntry = unsafe {
          napi::bindgen_prelude::ClassEntry::new(
            &#info_name,
            std::mem::offset_of!(#layout_ty, value),
          )
        };

        static #layout_name: napi::bindgen_prelude::ClassLayout = unsafe {
          napi::bindgen_prelude::ClassLayout::new(
            None,
            #entry_name,
            std::mem::size_of::<#layout_ty>(),
            std::mem::align_of::<#layout_ty>(),
            #drop_fn,
          )
        };

        fn #layout_fn() -> &'static napi::bindgen_prelude::ClassLayout {
          &#layout_name
        }

        static #def_name: napi::bindgen_prelude::ClassDef<#name> = unsafe {
          napi::bindgen_prelude::ClassDef::new(&#info_name, #layout_fn)
        };

        unsafe impl napi::bindgen_prelude::NapiClass for #name {
          type Parent = ();

          const CLASS: &'static napi::bindgen_prelude::ClassDef<Self> = &#def_name;
        }

        unsafe impl napi::bindgen_prelude::NapiReceiver for #name {
          type Access = napi::bindgen_prelude::ClassAccess;

          type Borrow<'a> = napi::bindgen_prelude::ClassBorrow<'a, Self>
          where
            Self: 'a;

          type BorrowMut<'a> = napi::bindgen_prelude::ClassBorrowMut<'a, Self>
          where
            Self: 'a;

          unsafe fn validate_raw_object<'scope>(
            scope: &mut napi::bindgen_prelude::Scope<'_, 'scope>,
            object: napi::bindgen_prelude::sys::napi_value,
          ) -> napi::Result<(Self::Access, napi::bindgen_prelude::ClassStorageRef<'scope>)> {
            unsafe { napi::bindgen_prelude::ClassStorageRef::validate_raw_object(
              scope,
              object,
              <Self as napi::bindgen_prelude::NapiClass>::CLASS.info(),
            ) }
          }

          unsafe fn ref_from_validated_object<'scope>(
            storage: napi::bindgen_prelude::ClassStorageRef<'scope>,
            access: Self::Access,
          ) -> napi::Result<Self::Borrow<'scope>> {
            unsafe {
              napi::bindgen_prelude::ClassBorrow::from_validated_parts(storage, access)
            }
          }

          unsafe fn mut_from_validated_object<'scope>(
            storage: napi::bindgen_prelude::ClassStorageRef<'scope>,
            access: Self::Access,
          ) -> napi::Result<Self::BorrowMut<'scope>> {
            unsafe {
              napi::bindgen_prelude::ClassBorrowMut::from_validated_parts(storage, access)
            }
          }
        }

        #subclass_impl

        unsafe impl napi::bindgen_prelude::ClassChain for #name {
          type Layout = #layout_ty;

          const LAYOUT: &'static napi::bindgen_prelude::ClassLayout = &#layout_name;

          unsafe fn write_init(
            init: napi::bindgen_prelude::ClassInitializer<Self>,
            dst: std::ptr::NonNull<Self::Layout>,
          ) {
            let (value, parent) = init.into_value_and_parent();
            let layout = dst.as_ptr();
            std::mem::drop(parent);
            unsafe {
              (*layout).value.write(value);
            }
          }

          unsafe fn drop_segments(data: std::ptr::NonNull<Self::Layout>) {
            let layout = data.as_ptr();
            unsafe {
              napi::bindgen_prelude::drop_segment(
                std::ptr::NonNull::new_unchecked((*layout).value.as_mut_ptr()),
              );
            }
          }

          unsafe fn drop_initialized(data: std::ptr::NonNull<u8>) {
            unsafe {
              <Self as napi::bindgen_prelude::ClassChain>::drop_segments(data.cast());
            }
          }
        }

        unsafe fn #drop_fn(data: std::ptr::NonNull<u8>) {
          unsafe { <#name as napi::bindgen_prelude::ClassChain>::drop_initialized(data) }
        }
      }
    }
  }

  fn gen_default_ctor(&self, class: &NapiClass) -> TokenStream {
    if class.ctor {
      self.gen_field_default_ctor(class)
    } else {
      self.gen_hidden_constructor_shell(class)
    }
  }

  fn gen_hidden_constructor_shell(&self, _class: &NapiClass) -> TokenStream {
    let name = &self.name;
    let js_name_str = &self.js_name;
    let tracing_debug = gen_tracing_debug(js_name_str, "constructor");

    let constructor = quote! {
      let receiver = frame.constructor_receiver::<#name>()?;
      match unsafe {
        <#name as napi::bindgen_prelude::NapiClass>::CLASS
          .try_wrap_internal_construction(receiver)?
      } {
        napi::bindgen_prelude::InternalConstructionResult::Wrapped(value) => Ok(value),
        napi::bindgen_prelude::InternalConstructionResult::Absent => Err(
          napi::bindgen_prelude::Error::new(
            napi::bindgen_prelude::Status::InvalidArg,
            format!("Class `{}` is not constructible", #js_name_str),
          ),
        ),
      }
    };

    quote! {
      extern "C" fn constructor(
        env: napi::bindgen_prelude::sys::napi_env,
        cb: napi::bindgen_prelude::sys::napi_callback_info
      ) -> napi::bindgen_prelude::sys::napi_value {
        unsafe {
          napi::__private::__napi_binding_entry::<0>(env, cb, |mut frame| {
            #tracing_debug
            #constructor
          })
        }
      }
    }
  }

  fn gen_field_default_ctor(&self, class: &NapiClass) -> TokenStream {
    let name = &self.name;
    let js_name_str = &self.js_name;
    let fields_len = class.fields.len();
    let mut fields = vec![];

    for (i, field) in class.fields.iter().enumerate() {
      let ty = &field.ty;
      let field_value = class_field_from_frame(ty, quote! { #i }, name);
      match &field.name {
        syn::Member::Named(ident) => fields.push(quote! { #ident: #field_value }),
        syn::Member::Unnamed(_) => {
          fields.push(field_value);
        }
      }
    }

    let construct = if class.is_tuple {
      quote! { #name (#(#fields),*) }
    } else {
      quote! { #name {#(#fields),*} }
    };
    let wrap_from_public_constructor = if class.implement_iterator {
      quote! {
        let init =
          napi::bindgen_prelude::IntoClassInitializer::<#name>::into_class_initializer(#construct);
        frame.construct_generator::<false, #name>(
          #js_name_str,
          init,
        )
      }
    } else if class.implement_async_iterator {
      quote! {
        let init =
          napi::bindgen_prelude::IntoClassInitializer::<#name>::into_class_initializer(#construct);
        frame.construct_async_generator::<false, #name>(
          #js_name_str,
          init,
        )
      }
    } else {
      quote! {
        let init =
          napi::bindgen_prelude::IntoClassInitializer::<#name>::into_class_initializer(#construct);
        let receiver = frame.constructor_receiver::<#name>()?;
        <#name as napi::bindgen_prelude::NapiClass>::CLASS.wrap_receiver(
          receiver,
          init,
        )
      }
    };

    let constructor = quote! {
      let receiver = frame.constructor_receiver::<#name>()?;
      match unsafe {
        <#name as napi::bindgen_prelude::NapiClass>::CLASS
          .try_wrap_internal_construction(receiver)?
      } {
        napi::bindgen_prelude::InternalConstructionResult::Wrapped(value) => Ok(value),
        napi::bindgen_prelude::InternalConstructionResult::Absent => {
          let receiver = frame.constructor_receiver::<#name>()?;
          #wrap_from_public_constructor
        }
      }
    };

    let tracing_debug = gen_tracing_debug(js_name_str, "constructor");

    quote! {
      extern "C" fn constructor(
        env: napi::bindgen_prelude::sys::napi_env,
        cb: napi::bindgen_prelude::sys::napi_callback_info
      ) -> napi::bindgen_prelude::sys::napi_value {
        unsafe {
          napi::__private::__napi_binding_entry::<#fields_len>(env, cb, |mut frame| {
            #tracing_debug
            #constructor
          })
        }
      }
    }
  }
  fn gen_napi_value_map_impl(&self) -> TokenStream {
    match &self.kind {
      NapiStructKind::Array(array) => self.gen_napi_value_array_impl(array),
      NapiStructKind::Transparent(transparent) => self.gen_napi_value_transparent_impl(transparent),
      NapiStructKind::Class(_) => gen_napi_value_map_impl(&self.name, quote! {}, self.has_lifetime),
      NapiStructKind::Object(obj) => self.gen_into_js_obj_impl(obj),
      NapiStructKind::StructuredEnum(structured_enum) => {
        self.gen_into_js_structured_enum_impl(structured_enum)
      }
    }
  }

  fn gen_into_js_obj_impl(&self, obj: &NapiObject) -> TokenStream {
    let name = &self.name;
    let name_str = self.name.to_string();

    let mut js_obj_field_getters = vec![];
    let mut field_destructions = vec![];

    // For optimized object creation: separate always-set fields from conditionally-set fields
    let mut value_conversions = vec![];
    let mut property_descriptors = vec![];
    let mut conditional_setters = vec![];
    let mut value_names = vec![];

    for (idx, field) in obj.fields.iter().enumerate() {
      let field_js_name = &field.js_name;
      let field_js_name_lit = Literal::string(&format!("{}\0", field.js_name));
      let mut ty = field.ty.clone();
      remove_lifetime_in_type(&mut ty);
      let is_optional_field = if let syn::Type::Path(syn::TypePath {
        path: syn::Path { segments, .. },
        ..
      }) = &ty
      {
        if let Some(last_path) = segments.last() {
          last_path.ident == "Option"
        } else {
          false
        }
      } else {
        false
      };

      // Determine if this field is always set or conditionally set
      let is_always_set = !is_optional_field || self.use_nullable;

      match &field.name {
        syn::Member::Named(ident) => {
          let alias_ident = format_ident!("{}_", ident);
          field_destructions.push(quote! { #ident: #alias_ident });

          if is_always_set {
            // This field is always set - use batched approach
            let value_var = Ident::new(&format!("__obj_value_{}", idx), Span::call_site());
            value_names.push(value_var.clone());

            if is_optional_field {
              // Optional with use_nullable=true: set to value or null
              value_conversions.push(quote! {
                let #value_var = if let Some(inner) = #alias_ident {
                  napi::bindgen_prelude::IntoJs::into_js(inner, scope)?.raw()
                } else {
                  napi::bindgen_prelude::IntoJs::into_js(napi::bindgen_prelude::Null, scope)?.raw()
                };
              });
            } else {
              // Non-optional: always set
              value_conversions.push(quote! {
                let #value_var = napi::bindgen_prelude::IntoJs::into_js(#alias_ident, scope)?.raw();
              });
            }

            property_descriptors.push(quote! {
              napi::bindgen_prelude::sys::napi_property_descriptor {
                utf8name: unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(#field_js_name_lit.as_bytes()) }.as_ptr(),
                name: std::ptr::null_mut(),
                method: None,
                getter: None,
                setter: None,
                value: #value_var,
                attributes: napi::bindgen_prelude::sys::PropertyAttributes::writable
                  | napi::bindgen_prelude::sys::PropertyAttributes::enumerable
                  | napi::bindgen_prelude::sys::PropertyAttributes::configurable,
                data: std::ptr::null_mut(),
              }
            });
          } else {
            // Optional with use_nullable=false: conditionally set
            conditional_setters.push(quote! {
              if let Some(value) = #alias_ident {
                obj.set(#field_js_name, value)?;
              }
            });
          }

          if is_optional_field && !self.use_nullable {
            let js_getter = class_field_from_object_scope(
              &ty,
              field_js_name,
              name,
              true,
              quote! { format!("Missing field `{}`", #field_js_name) },
            )
            .unwrap_or_else(|| {
              object_field_getter_from_scope(&ty, quote! { obj }, field_js_name, true)
            });
            js_obj_field_getters.push(quote! {
              let #alias_ident: #ty = #js_getter.map_err(|mut err| {
                err.reason = format!("{} on {}.{}", err.reason, #name_str, #field_js_name);
                err
              })?;
            });
          } else if let Some(js_getter) = class_field_from_object_scope(
            &ty,
            field_js_name,
            name,
            false,
            quote! { format!("Missing field `{}`", #field_js_name) },
          ) {
            js_obj_field_getters.push(quote! {
              let #alias_ident: #ty = #js_getter.map_err(|mut err| {
                err.reason = format!("{} on {}.{}", err.reason, #name_str, #field_js_name);
                err
              })?;
            });
          } else {
            let js_getter =
              object_field_getter_from_scope(&ty, quote! { obj }, field_js_name, false);
            js_obj_field_getters.push(quote! {
              let #alias_ident: #ty = #js_getter.map_err(|mut err| {
                err.reason = format!("{} on {}.{}", err.reason, #name_str, #field_js_name);
                err
              })?.ok_or_else(|| napi::bindgen_prelude::Error::new(
                napi::bindgen_prelude::Status::InvalidArg,
                format!("Missing field `{}`", #field_js_name),
              ))?;
            });
          }
        }
        syn::Member::Unnamed(i) => {
          let arg_name = format_ident!("arg{}", i);
          field_destructions.push(quote! { #arg_name });

          if is_always_set {
            // This field is always set - use batched approach
            let value_var = Ident::new(&format!("__obj_value_{}", idx), Span::call_site());
            value_names.push(value_var.clone());

            if is_optional_field {
              // Optional with use_nullable=true: set to value or null
              value_conversions.push(quote! {
                let #value_var = if let Some(inner) = #arg_name {
                  napi::bindgen_prelude::IntoJs::into_js(inner, scope)?.raw()
                } else {
                  napi::bindgen_prelude::IntoJs::into_js(napi::bindgen_prelude::Null, scope)?.raw()
                };
              });
            } else {
              // Non-optional: always set
              value_conversions.push(quote! {
                let #value_var = napi::bindgen_prelude::IntoJs::into_js(#arg_name, scope)?.raw();
              });
            }

            property_descriptors.push(quote! {
              napi::bindgen_prelude::sys::napi_property_descriptor {
                utf8name: unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(#field_js_name_lit.as_bytes()) }.as_ptr(),
                name: std::ptr::null_mut(),
                method: None,
                getter: None,
                setter: None,
                value: #value_var,
                attributes: napi::bindgen_prelude::sys::PropertyAttributes::writable
                  | napi::bindgen_prelude::sys::PropertyAttributes::enumerable
                  | napi::bindgen_prelude::sys::PropertyAttributes::configurable,
                data: std::ptr::null_mut(),
              }
            });
          } else {
            // Optional with use_nullable=false: conditionally set
            conditional_setters.push(quote! {
              if let Some(value) = #arg_name {
                obj.set(#field_js_name, value)?;
              }
            });
          }

          if is_optional_field && !self.use_nullable {
            let js_getter = class_field_from_object_scope(
              &ty,
              field_js_name,
              name,
              true,
              quote! { format!("Missing field `{}`", #field_js_name) },
            )
            .unwrap_or_else(|| {
              object_field_getter_from_scope(&ty, quote! { obj }, field_js_name, true)
            });
            js_obj_field_getters.push(quote! { let #arg_name: #ty = #js_getter?; });
          } else if let Some(js_getter) = class_field_from_object_scope(
            &ty,
            field_js_name,
            name,
            false,
            quote! { format!("Missing field `{}`", #field_js_name) },
          ) {
            js_obj_field_getters.push(quote! { let #arg_name: #ty = #js_getter?; });
          } else {
            let js_getter =
              object_field_getter_from_scope(&ty, quote! { obj }, field_js_name, false);
            js_obj_field_getters.push(quote! {
              let #arg_name: #ty = #js_getter?.ok_or_else(|| napi::bindgen_prelude::Error::new(
                napi::bindgen_prelude::Status::InvalidArg,
                format!("Missing field `{}`", #field_js_name),
              ))?;
            });
          }
        }
      }
    }

    let destructed_fields = if obj.is_tuple {
      quote! {
        Self (#(#field_destructions),*)
      }
    } else {
      quote! {
        Self {#(#field_destructions),*}
      }
    };

    let (into_js_impl, validate_napi_value_impl, type_name_impl) = if self.has_lifetime {
      (
        quote! { impl <'scope, '_javascript_function_scope> napi::bindgen_prelude::IntoJs<'scope> for #name<'_javascript_function_scope> where '_javascript_function_scope: 'scope },
        quote! { impl <'_javascript_function_scope> napi::bindgen_prelude::ValidateNapiValue for #name<'_javascript_function_scope> },
        quote! { impl <'_javascript_function_scope> napi::bindgen_prelude::TypeName for #name<'_javascript_function_scope> },
      )
    } else {
      (
        quote! { impl <'scope> napi::bindgen_prelude::IntoJs<'scope> for #name },
        quote! { impl napi::bindgen_prelude::ValidateNapiValue for #name },
        quote! { impl napi::bindgen_prelude::TypeName for #name },
      )
    };

    // Generate object creation code
    let object_creation = if conditional_setters.is_empty() {
      // All fields are always set - use fully batched approach
      quote! {
        // Convert all values first, so error handling works correctly
        #(#value_conversions)*

        let properties = [
          #(#property_descriptors),*
        ];

        scope.create_object_with_properties(&properties)
      }
    } else {
      // Some fields are conditionally set - use batched for always-set, then add conditionals
      quote! {
        // Convert all always-set values first
        #(#value_conversions)*

        let properties = [
          #(#property_descriptors),*
        ];

        let mut obj = scope.create_object_with_properties(&properties)?;

        #(#conditional_setters)*

        Ok::<_, napi::bindgen_prelude::Error>(obj)
      }
    };

    let into_js = if obj.object_to_js {
      quote! {
        #[automatically_derived]
        #into_js_impl {
          type Output = napi::bindgen_prelude::Object<'scope>;

          fn into_js(
            self,
            scope: &mut napi::bindgen_prelude::Scope<'_, 'scope>,
          ) -> napi::bindgen_prelude::Result<napi::bindgen_prelude::Local<'scope, Self::Output>> {
            let #destructed_fields = self;
            let object = { #object_creation }?;
            napi::bindgen_prelude::IntoJs::into_js(object, scope)
          }
        }
      }
    } else {
      quote! {}
    };

    let from_js = if obj.object_from_js {
      let from_js_impl = if self.has_lifetime {
        quote! { impl<'env, 'scope> napi::bindgen_prelude::FromJs<'env, 'scope> for #name<'scope> }
      } else {
        quote! { impl<'env, 'scope> napi::bindgen_prelude::FromJs<'env, 'scope> for #name }
      };
      let js_field_decode = if js_obj_field_getters.is_empty() {
        quote! {
          scope.assert_value_type(value, napi::bindgen_prelude::ValueType::Object)?;
        }
      } else {
        quote! {
          let obj = <napi::bindgen_prelude::Object as napi::bindgen_prelude::FromJs>::from_js(
            scope,
            value,
          )?;

          #(#js_obj_field_getters)*
        }
      };
      quote! {
        #[automatically_derived]
        #validate_napi_value_impl {}

        #[automatically_derived]
        #from_js_impl {
          fn from_js(
            scope: &mut napi::bindgen_prelude::Scope<'env, 'scope>,
            value: napi::bindgen_prelude::Local<'scope, napi::bindgen_prelude::Unknown<'scope>>
          ) -> napi::bindgen_prelude::Result<Self> {
            #js_field_decode

            let val = #destructed_fields;

            Ok(val)
          }
        }
      }
    } else {
      quote! {}
    };

    quote! {
      #[automatically_derived]
      #type_name_impl {
        fn type_name() -> &'static str {
          #name_str
        }

        fn value_type() -> napi::ValueType {
          napi::ValueType::Object
        }
      }

      #into_js

      #from_js
    }
  }

  fn gen_default_getters_setters(&self, class: &NapiClass) -> Vec<(String, TokenStream)> {
    let mut getters_setters = vec![];
    let struct_name = &self.name;
    let js_name_str = &self.js_name;

    for field in class.fields.iter() {
      let field_ident = &field.name;
      let field_name = match &field.name {
        syn::Member::Named(ident) => ident.to_string(),
        syn::Member::Unnamed(i) => format!("field{}", i.index),
      };
      let ty = &field.ty;

      let getter_name = Ident::new(
        &format!("get_{}", rm_raw_prefix(&field_name)),
        Span::call_site(),
      );
      let setter_name = Ident::new(
        &format!("set_{}", rm_raw_prefix(&field_name)),
        Span::call_site(),
      );

      if field.getter {
        let default_into_js_convert = into_js_frame(quote! { val });
        let none_into_js = into_js_frame(quote! { () });
        let ok_into_js = into_js_frame(quote! { val });
        let default_into_js_convert = quote! {
          let val = &obj.#field_ident;
          #default_into_js_convert
        };
        let into_js_convert = if let Some(convert) = class_field_into_js(ty, field_ident) {
          convert
        } else if let syn::Type::Path(syn::TypePath {
          path: syn::Path { segments, .. },
          ..
        }) = ty
        {
          if let Some(syn::PathSegment { ident, .. }) = segments.last() {
            if ident == "Option" {
              quote! {
                match &obj.#field_ident {
                  Some(val) => #ok_into_js,
                  None => #none_into_js,
                }
              }
            } else if ident == "Result" {
              quote! {
                match &obj.#field_ident {
                  Ok(val) => #ok_into_js,
                  Err(err) => {
                    let scope = frame.context_mut().scope_mut();
                    let error = scope.create_error_value(format!("{:?}", err.status), err.reason.clone())?;
                    Ok(error.raw())
                  }
                }
              }
            } else {
              default_into_js_convert
            }
          } else {
            default_into_js_convert
          }
        } else {
          default_into_js_convert
        };
        let tracing_debug = gen_tracing_debug(js_name_str, &field.js_name);
        getters_setters.push((
          field.js_name.clone(),
          quote! {
            extern "C" fn #getter_name(
              env: napi::bindgen_prelude::sys::napi_env,
              cb: napi::bindgen_prelude::sys::napi_callback_info
            ) -> napi::bindgen_prelude::sys::napi_value {
              unsafe {
                napi::__private::__napi_binding_entry::<0>(env, cb, |mut frame| {
                  #tracing_debug
                  let obj = frame.this_class::<#struct_name>()?;
                  #into_js_convert
                })
              }
            }
          },
        ));
      }

      if field.setter {
        let setter_tracing_debug =
          gen_tracing_debug(js_name_str, &format!("set_{}", field.js_name));
        let class_field_from_frame = class_field_from_frame(ty, quote! { 0 }, struct_name);
        let setter_return = quote! { frame.return_value(()) };
        getters_setters.push((
          field.js_name.clone(),
          quote! {
            extern "C" fn #setter_name(
              env: napi::bindgen_prelude::sys::napi_env,
              cb: napi::bindgen_prelude::sys::napi_callback_info
            ) -> napi::bindgen_prelude::sys::napi_value {
              unsafe {
                napi::__private::__napi_binding_entry::<1>(env, cb, |mut frame| {
                  #setter_tracing_debug
                  let mut obj = frame.this_class_mut::<#struct_name>()?;
                  let val = #class_field_from_frame;
                  obj.#field_ident = val;
                  #setter_return
                })
              }
            }
          },
        ));
      }
    }

    getters_setters
  }

  fn gen_register(&self, class: &NapiClass) -> TokenStream {
    let name = &self.name;
    let struct_register_name = &self.register_name;
    let js_name = format!("{}\0", self.js_name);
    let implement_iterator = class.implement_iterator;
    let mut props = vec![];

    if class.ctor {
      props.push(quote! { napi::bindgen_prelude::Property::new().with_utf8_name("constructor").unwrap().with_ctor(constructor) });
    }

    for field in class.fields.iter() {
      let field_name = match &field.name {
        syn::Member::Named(ident) => ident.to_string(),
        syn::Member::Unnamed(i) => format!("field{}", i.index),
      };

      if !field.getter {
        continue;
      }

      let js_name = &field.js_name;
      let mut attribute = super::PROPERTY_ATTRIBUTE_DEFAULT;
      if field.writable {
        attribute |= super::PROPERTY_ATTRIBUTE_WRITABLE;
      }
      if field.enumerable {
        attribute |= super::PROPERTY_ATTRIBUTE_ENUMERABLE;
      }
      if field.configurable {
        attribute |= super::PROPERTY_ATTRIBUTE_CONFIGURABLE;
      }

      let mut prop = quote! {
        napi::bindgen_prelude::Property::new().with_utf8_name(#js_name)
          .unwrap()
          .with_property_attributes(napi::bindgen_prelude::PropertyAttributes::from_bits(#attribute).unwrap())
      };

      if field.getter {
        let getter_name = Ident::new(
          &format!("get_{}", rm_raw_prefix(&field_name)),
          Span::call_site(),
        );
        (quote! { .with_getter(#getter_name) }).to_tokens(&mut prop);
      }

      if field.writable && field.setter {
        let setter_name = Ident::new(
          &format!("set_{}", rm_raw_prefix(&field_name)),
          Span::call_site(),
        );
        (quote! { .with_setter(#setter_name) }).to_tokens(&mut prop);
      }

      props.push(prop);
    }
    let js_mod_ident = js_mod_to_token_stream(self.js_mod.as_ref());
    let constructible = class.ctor;
    quote! {
      #[cfg(all(not(test), not(target_family = "wasm")))]
      #[allow(non_snake_case)]
      #[allow(non_upper_case_globals)]
      #[allow(clippy::all)]
      mod #struct_register_name {
        use super::*;
        use napi::__private::linkme::distributed_slice;

        fn __class() -> napi::bindgen_prelude::ErasedClassDef {
          <#name as napi::bindgen_prelude::NapiClass>::CLASS.erase()
        }

        fn __parent() -> Option<napi::bindgen_prelude::ErasedClassDef> {
          <<#name as napi::bindgen_prelude::NapiClass>::Parent as napi::bindgen_prelude::NativeParent>::erased_class_def()
        }

        fn __props() -> Vec<napi::bindgen_prelude::Property> {
          vec![#(#props),*]
        }

        #[distributed_slice(napi::__private::CLASS_STRUCT_DESCRIPTORS)]
        #[linkme(crate = napi::__private::linkme)]
        static __DESCRIPTOR: napi::__private::ClassStructDescriptor =
          napi::__private::ClassStructDescriptor {
            class: __class,
            parent: __parent,
            js_mod: #js_mod_ident,
            js_name: #js_name,
            hidden_constructor: Some(constructor),
            constructible: #constructible,
            implement_iterator: #implement_iterator,
            props: __props,
          };
      }

      #[allow(non_snake_case)]
      #[allow(clippy::all)]
      #[cfg(all(not(test), target_family = "wasm"))]
      // Compatibility path only. Non-WASM registration is descriptor-driven.
      #[no_mangle]
      extern "C" fn #struct_register_name() {
        napi::__private::register_napi_class::<#name>(
          #js_mod_ident,
          #js_name,
          vec![#(#props),*],
          Some(constructor),
          #constructible,
          #implement_iterator,
        );
      }
    }
  }

  fn gen_into_js_structured_enum_impl(&self, structured_enum: &NapiStructuredEnum) -> TokenStream {
    let name = &self.name;
    let name_str = self.name.to_string();
    let discriminant = structured_enum.discriminant.as_str();
    let discriminant_null_terminated = format!("{}\0", discriminant);

    let mut variant_arm_setters = vec![];
    let mut variant_arm_js_getters = vec![];

    for variant in structured_enum.variants.iter() {
      let variant_name = &variant.name;
      let mut variant_name_str = variant_name.to_string();
      if let Some(case) = structured_enum.discriminant_case {
        variant_name_str = to_case(variant_name_str, case);
      }

      let mut js_obj_field_getters = vec![];
      let mut field_destructions = vec![];

      // For optimized object creation
      let mut value_conversions = vec![];
      let mut property_descriptors = vec![];
      let mut conditional_setters = vec![];

      // First property is always the discriminant
      let discriminant_value_var = Ident::new("__discriminant_value", Span::call_site());
      value_conversions.push(quote! {
        let #discriminant_value_var = napi::bindgen_prelude::IntoJs::into_js(#variant_name_str, scope)?.raw();
      });
      property_descriptors.push(quote! {
        napi::bindgen_prelude::sys::napi_property_descriptor {
          utf8name: unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(#discriminant_null_terminated.as_bytes()) }.as_ptr(),
          name: std::ptr::null_mut(),
          method: None,
          getter: None,
          setter: None,
          value: #discriminant_value_var,
          attributes: napi::bindgen_prelude::sys::PropertyAttributes::writable
                  | napi::bindgen_prelude::sys::PropertyAttributes::enumerable
                  | napi::bindgen_prelude::sys::PropertyAttributes::configurable,
          data: std::ptr::null_mut(),
        }
      });

      for (idx, field) in variant.fields.iter().enumerate() {
        let field_js_name = &field.js_name;
        let field_js_name_lit = Literal::string(&format!("{}\0", field.js_name));
        let mut ty = field.ty.clone();
        remove_lifetime_in_type(&mut ty);
        let is_optional_field = if let syn::Type::Path(syn::TypePath {
          path: syn::Path { segments, .. },
          ..
        }) = &ty
        {
          if let Some(last_path) = segments.last() {
            last_path.ident == "Option"
          } else {
            false
          }
        } else {
          false
        };

        // Determine if this field is always set or conditionally set
        let is_always_set = !is_optional_field || self.use_nullable;

        match &field.name {
          syn::Member::Named(ident) => {
            let alias_ident = format_ident!("{}_", ident);
            field_destructions.push(quote! { #ident: #alias_ident });

            if is_always_set {
              // This field is always set - use batched approach
              let value_var = Ident::new(&format!("__variant_value_{}", idx), Span::call_site());

              if is_optional_field {
                // Optional with use_nullable=true: set to value or null
                value_conversions.push(quote! {
                  let #value_var = if let Some(inner) = #alias_ident {
                    napi::bindgen_prelude::IntoJs::into_js(inner, scope)?.raw()
                  } else {
                    napi::bindgen_prelude::IntoJs::into_js(napi::bindgen_prelude::Null, scope)?.raw()
                  };
                });
              } else {
                // Non-optional: always set
                value_conversions.push(quote! {
                  let #value_var = napi::bindgen_prelude::IntoJs::into_js(#alias_ident, scope)?.raw();
                });
              }

              property_descriptors.push(quote! {
                napi::bindgen_prelude::sys::napi_property_descriptor {
                  utf8name: unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(#field_js_name_lit.as_bytes()) }.as_ptr(),
                  name: std::ptr::null_mut(),
                  method: None,
                  getter: None,
                  setter: None,
                  value: #value_var,
                  attributes: napi::bindgen_prelude::sys::PropertyAttributes::writable
                  | napi::bindgen_prelude::sys::PropertyAttributes::enumerable
                  | napi::bindgen_prelude::sys::PropertyAttributes::configurable,
                  data: std::ptr::null_mut(),
                }
              });
            } else {
              // Optional with use_nullable=false: conditionally set
              conditional_setters.push(quote! {
                if let Some(value) = #alias_ident {
                  obj.set(#field_js_name, value)?;
                }
              });
            }

            if is_optional_field && !self.use_nullable {
              let decode_ty = ty.option_inner().unwrap_or(&ty);
              js_obj_field_getters.push(quote! {
                let #alias_ident: #ty = scope.get_optional_named_property::<#decode_ty, _>(&obj, #field_js_name).map_err(|mut err| {
                  err.reason = format!("{} on {}.{}", err.reason, #name_str, #field_js_name);
                  err
                })?;
              });
            } else {
              js_obj_field_getters.push(quote! {
                let #alias_ident: #ty = scope.get_optional_named_property::<#ty, _>(&obj, #field_js_name).map_err(|mut err| {
                  err.reason = format!("{} on {}.{}", err.reason, #name_str, #field_js_name);
                  err
                })?.ok_or_else(|| napi::bindgen_prelude::Error::new(
                  napi::bindgen_prelude::Status::InvalidArg,
                  format!("Missing field `{}`", #field_js_name),
                ))?;
              });
            }
          }
          syn::Member::Unnamed(i) => {
            let arg_name = format_ident!("arg{}", i);
            field_destructions.push(quote! { #arg_name });

            if is_always_set {
              // This field is always set - use batched approach
              let value_var = Ident::new(&format!("__variant_value_{}", idx), Span::call_site());

              if is_optional_field {
                // Optional with use_nullable=true: set to value or null
                value_conversions.push(quote! {
                  let #value_var = if let Some(inner) = #arg_name {
                    napi::bindgen_prelude::IntoJs::into_js(inner, scope)?.raw()
                  } else {
                    napi::bindgen_prelude::IntoJs::into_js(napi::bindgen_prelude::Null, scope)?.raw()
                  };
                });
              } else {
                // Non-optional: always set
                value_conversions.push(quote! {
                  let #value_var = napi::bindgen_prelude::IntoJs::into_js(#arg_name, scope)?.raw();
                });
              }

              property_descriptors.push(quote! {
                napi::bindgen_prelude::sys::napi_property_descriptor {
                  utf8name: unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(#field_js_name_lit.as_bytes()) }.as_ptr(),
                  name: std::ptr::null_mut(),
                  method: None,
                  getter: None,
                  setter: None,
                  value: #value_var,
                  attributes: napi::bindgen_prelude::sys::PropertyAttributes::writable
                  | napi::bindgen_prelude::sys::PropertyAttributes::enumerable
                  | napi::bindgen_prelude::sys::PropertyAttributes::configurable,
                  data: std::ptr::null_mut(),
                }
              });
            } else {
              // Optional with use_nullable=false: conditionally set
              conditional_setters.push(quote! {
                if let Some(value) = #arg_name {
                  obj.set(#field_js_name, value)?;
                }
              });
            }

            if is_optional_field && !self.use_nullable {
              let decode_ty = ty.option_inner().unwrap_or(&ty);
              js_obj_field_getters.push(
                quote! { let #arg_name: #ty = scope.get_optional_named_property::<#decode_ty, _>(&obj, #field_js_name)?; },
              );
            } else {
              js_obj_field_getters.push(quote! {
                let #arg_name: #ty = scope.get_optional_named_property::<#ty, _>(&obj, #field_js_name)?.ok_or_else(|| napi::bindgen_prelude::Error::new(
                  napi::bindgen_prelude::Status::InvalidArg,
                  format!("Missing field `{}`", #field_js_name),
                ))?;
              });
            }
          }
        }
      }

      let destructed_fields = if variant.is_tuple {
        quote! {
          Self::#variant_name (#(#field_destructions),*)
        }
      } else {
        quote! {
          Self::#variant_name {#(#field_destructions),*}
        }
      };

      // Generate object creation for this variant
      let variant_object_creation = if conditional_setters.is_empty() {
        // All fields are always set - use fully batched approach
        quote! {
          #(#value_conversions)*

          let properties = [
            #(#property_descriptors),*
          ];

          scope.create_object_with_properties(&properties)
        }
      } else {
        // Some fields are conditionally set
        quote! {
          #(#value_conversions)*

          let properties = [
            #(#property_descriptors),*
          ];

          let mut obj = scope.create_object_with_properties(&properties)?;

          #(#conditional_setters)*

          Ok::<_, napi::bindgen_prelude::Error>(obj)
        }
      };

      variant_arm_setters.push(quote! {
        #destructed_fields => {
          #variant_object_creation
        },
      });

      variant_arm_js_getters.push(quote! {
        #variant_name_str => {
          #(#js_obj_field_getters)*
          #destructed_fields
        },
      })
    }

    let into_js = if structured_enum.object_to_js {
      quote! {
        impl<'scope> napi::bindgen_prelude::IntoJs<'scope> for #name {
          type Output = napi::bindgen_prelude::Object<'scope>;

          fn into_js(
            self,
            scope: &mut napi::bindgen_prelude::Scope<'_, 'scope>,
          ) -> napi::bindgen_prelude::Result<napi::bindgen_prelude::Local<'scope, Self::Output>> {
            let object = match self {
              #(#variant_arm_setters)*
            }?;
            napi::bindgen_prelude::IntoJs::into_js(object, scope)
          }
        }
      }
    } else {
      quote! {}
    };

    let from_js = if structured_enum.object_from_js {
      quote! {
        impl napi::bindgen_prelude::ValidateNapiValue for #name {}

        impl<'env, 'scope> napi::bindgen_prelude::FromJs<'env, 'scope> for #name {
          fn from_js(
            scope: &mut napi::bindgen_prelude::Scope<'env, 'scope>,
            value: napi::bindgen_prelude::Local<'scope, napi::bindgen_prelude::Unknown<'scope>>
          ) -> napi::bindgen_prelude::Result<Self> {
            let obj = <napi::bindgen_prelude::Object as napi::bindgen_prelude::FromJs>::from_js(
              scope,
              value,
            )?;
            let type_: String = scope.get_optional_named_property(&obj, #discriminant).map_err(|mut err| {
              err.reason = format!("{} on {}.{}", err.reason, #name_str, #discriminant);
              err
            })?.ok_or_else(|| napi::bindgen_prelude::Error::new(
              napi::bindgen_prelude::Status::InvalidArg,
              format!("Missing field `{}`", #discriminant),
            ))?;
            let val = match type_.as_str() {
              #(#variant_arm_js_getters)*
              _ => return Err(napi::bindgen_prelude::Error::new(
                napi::bindgen_prelude::Status::InvalidArg,
                format!("Unknown variant `{}`", type_),
              )),
            };

            Ok(val)
          }
        }
      }
    } else {
      quote! {}
    };

    quote! {
      impl napi::bindgen_prelude::TypeName for #name {
        fn type_name() -> &'static str {
          #name_str
        }

        fn value_type() -> napi::ValueType {
          napi::ValueType::Object
        }
      }

      #into_js

      #from_js
    }
  }

  fn gen_napi_value_transparent_impl(&self, transparent: &NapiTransparent) -> TokenStream {
    let name = &self.name;
    let name = if self.has_lifetime {
      quote! { #name<'_> }
    } else {
      quote! { #name }
    };
    let inner_type = transparent.ty.clone().into_token_stream();

    let into_js = if transparent.object_to_js {
      quote! {
        #[automatically_derived]
        impl<'scope> napi::bindgen_prelude::IntoJs<'scope> for #name {
          type Output = <#inner_type as napi::bindgen_prelude::IntoJs<'scope>>::Output;

          fn into_js(
            self,
            scope: &mut napi::bindgen_prelude::Scope<'_, 'scope>,
          ) -> napi::bindgen_prelude::Result<napi::bindgen_prelude::Local<'scope, Self::Output>> {
            napi::bindgen_prelude::IntoJs::into_js(self.0, scope)
          }
        }
      }
    } else {
      quote! {}
    };

    let from_js = if transparent.object_from_js {
      quote! {
        #[automatically_derived]
        impl<'env, 'scope> napi::bindgen_prelude::FromJs<'env, 'scope> for #name {
          fn from_js(
            scope: &mut napi::bindgen_prelude::Scope<'env, 'scope>,
            value: napi::bindgen_prelude::Local<'scope, napi::bindgen_prelude::Unknown<'scope>>
          ) -> napi::bindgen_prelude::Result<Self> {
            Ok(Self(<#inner_type as napi::bindgen_prelude::FromJs>::from_js(scope, value)?))
          }
        }
      }
    } else {
      quote! {}
    };

    quote! {
      #[automatically_derived]
      impl napi::bindgen_prelude::TypeName for #name {
        fn type_name() -> &'static str {
          <#inner_type>::type_name()
        }

        fn value_type() -> napi::ValueType {
          <#inner_type>::value_type()
        }
      }

      #[automatically_derived]
      impl napi::bindgen_prelude::ValidateNapiValue for #name {
        unsafe fn validate(
          env: napi::bindgen_prelude::sys::napi_env,
          napi_val: napi::bindgen_prelude::sys::napi_value
        ) -> napi::bindgen_prelude::Result<napi::sys::napi_value> {
          <#inner_type>::validate(env, napi_val)
        }
      }

      #into_js

      #from_js
    }
  }

  fn gen_napi_value_array_impl(&self, array: &NapiArray) -> TokenStream {
    let name = &self.name;
    let name_str = self.name.to_string();

    let mut obj_field_setters = vec![];
    let mut js_obj_field_getters = vec![];
    let mut field_destructions = vec![];

    for field in array.fields.iter() {
      let mut ty = field.ty.clone();
      remove_lifetime_in_type(&mut ty);
      let is_optional_field = if let syn::Type::Path(syn::TypePath {
        path: syn::Path { segments, .. },
        ..
      }) = &ty
      {
        if let Some(last_path) = segments.last() {
          last_path.ident == "Option"
        } else {
          false
        }
      } else {
        false
      };

      if let syn::Member::Unnamed(i) = &field.name {
        let arg_name = format_ident!("arg{}", i);
        let field_index = i.index;
        field_destructions.push(quote! { #arg_name });
        if is_optional_field {
          obj_field_setters.push(match self.use_nullable {
            false => quote! {
              if let Some(value) = #arg_name {
                array.set(#field_index, value)?;
              }
            },
            true => quote! {
              if let Some(#arg_name) = #arg_name {
                array.set(#field_index, #arg_name)?;
              } else {
                array.set(#field_index, napi::bindgen_prelude::Null)?;
              }
            },
          });
        } else {
          obj_field_setters.push(quote! { array.set(#field_index, #arg_name)?; });
        }
        if is_optional_field && !self.use_nullable {
          let decode_ty = ty.option_inner().unwrap_or(&ty);
          js_obj_field_getters.push(
            quote! { let #arg_name: #ty = scope.get_optional_element::<#decode_ty>(&array, #field_index)?; },
          );
        } else {
          js_obj_field_getters.push(quote! {
            let #arg_name: #ty = scope.get_optional_element::<#ty>(&array, #field_index)?.ok_or_else(|| napi::bindgen_prelude::Error::new(
              napi::bindgen_prelude::Status::InvalidArg,
              format!("Failed to get element with index `{}`", #field_index),
            ))?;
          });
        }
      }
    }

    let destructed_fields = quote! {
      Self (#(#field_destructions),*)
    };

    let (into_js_impl, validate_napi_value_impl, type_name_impl) = if self.has_lifetime {
      (
        quote! { impl <'scope, '_javascript_function_scope> napi::bindgen_prelude::IntoJs<'scope> for #name<'_javascript_function_scope> where '_javascript_function_scope: 'scope },
        quote! { impl <'_javascript_function_scope> napi::bindgen_prelude::ValidateNapiValue for #name<'_javascript_function_scope> },
        quote! { impl <'_javascript_function_scope> napi::bindgen_prelude::TypeName for #name<'_javascript_function_scope> },
      )
    } else {
      (
        quote! { impl <'scope> napi::bindgen_prelude::IntoJs<'scope> for #name },
        quote! { impl napi::bindgen_prelude::ValidateNapiValue for #name },
        quote! { impl napi::bindgen_prelude::TypeName for #name },
      )
    };

    let array_len = array.fields.len() as u32;

    let into_js = if array.object_to_js {
      quote! {
        #[automatically_derived]
        #into_js_impl {
          type Output = napi::bindgen_prelude::Array<'scope>;

          fn into_js(
            self,
            scope: &mut napi::bindgen_prelude::Scope<'_, 'scope>,
          ) -> napi::bindgen_prelude::Result<napi::bindgen_prelude::Local<'scope, Self::Output>> {
            #[allow(unused_mut)]
            let mut array = scope.create_array(#array_len)?;

            let #destructed_fields = self;
            #(#obj_field_setters)*

            napi::bindgen_prelude::IntoJs::into_js(array, scope)
          }
        }
      }
    } else {
      quote! {}
    };

    let from_js = if array.object_from_js {
      let from_js_impl = if self.has_lifetime {
        quote! { impl<'env, 'scope> napi::bindgen_prelude::FromJs<'env, 'scope> for #name<'scope> }
      } else {
        quote! { impl<'env, 'scope> napi::bindgen_prelude::FromJs<'env, 'scope> for #name }
      };
      quote! {
        #[automatically_derived]
        #validate_napi_value_impl {}

        #[automatically_derived]
        #from_js_impl {
          fn from_js(
            scope: &mut napi::bindgen_prelude::Scope<'env, 'scope>,
            value: napi::bindgen_prelude::Local<'scope, napi::bindgen_prelude::Unknown<'scope>>
          ) -> napi::bindgen_prelude::Result<Self> {
            let array = <napi::bindgen_prelude::Array as napi::bindgen_prelude::FromJs>::from_js(scope, value)?;

            #(#js_obj_field_getters)*

            let val = #destructed_fields;

            Ok(val)
          }
        }
      }
    } else {
      quote! {}
    };

    quote! {
      #[automatically_derived]
      #type_name_impl {
        fn type_name() -> &'static str {
          #name_str
        }

        fn value_type() -> napi::ValueType {
          napi::ValueType::Object
        }
      }

      #into_js

      #from_js
    }
  }
}

impl TryToTokens for NapiImpl {
  fn try_to_tokens(&self, tokens: &mut TokenStream) -> BindgenResult<()> {
    self.gen_helper_mod()?.to_tokens(tokens);
    self.gen_post_init_shim()?.to_tokens(tokens);

    Ok(())
  }
}

impl NapiImpl {
  fn gen_helper_mod(&self) -> BindgenResult<TokenStream> {
    if cfg!(test) {
      return Ok(quote! {});
    }

    let name = &self.name;
    let name_str = self.name.to_string();
    let js_name = format!("{}\0", self.js_name);
    let mod_name = Ident::new(
      &format!(
        "__napi_impl_helper_{}_{}",
        name_str,
        NAPI_IMPL_ID.fetch_add(1, Ordering::SeqCst)
      ),
      Span::call_site(),
    );

    let register_name = &self.register_name;

    let mut methods = vec![];
    let mut props: HashMap<String, TokenStream> = HashMap::new();

    for item in self.items.iter() {
      if item.kind == FnKind::PostInit {
        continue;
      }

      let js_name = Literal::string(&item.js_name);
      let item_str = item.name.to_string();
      let intermediate_name = get_intermediate_ident(&item_str);
      methods.push(item.try_to_token_stream()?);

      let mut attribute = super::PROPERTY_ATTRIBUTE_DEFAULT;
      if item.writable {
        attribute |= super::PROPERTY_ATTRIBUTE_WRITABLE;
      }
      if item.enumerable {
        attribute |= super::PROPERTY_ATTRIBUTE_ENUMERABLE;
      }
      if item.configurable {
        attribute |= super::PROPERTY_ATTRIBUTE_CONFIGURABLE;
      }

      let prop = props.entry(item.js_name.clone()).or_insert_with(|| {
        quote! {
          napi::bindgen_prelude::Property::new().with_utf8_name(#js_name).unwrap().with_property_attributes(napi::bindgen_prelude::PropertyAttributes::from_bits(#attribute).unwrap())
        }
      });

      let appendix = match item.kind {
        FnKind::Constructor => quote! { .with_ctor(#intermediate_name) },
        FnKind::Getter => quote! { .with_getter(#intermediate_name) },
        FnKind::Setter => quote! { .with_setter(#intermediate_name) },
        FnKind::PostInit => unreachable!(),
        _ => {
          if item.fn_self.is_some() || has_receiver_frame_input_arg(&item) {
            quote! { .with_method(#intermediate_name) }
          } else {
            quote! { .with_method(#intermediate_name).with_property_attributes(napi::bindgen_prelude::PropertyAttributes::Static) }
          }
        }
      };

      appendix.to_tokens(prop);
    }

    let mut props: Vec<_> = props.into_iter().collect();
    props.sort_by_key(|(_, prop)| prop.to_string());
    let props = props.into_iter().map(|(_, prop)| prop);
    let props_wasm = props.clone();
    let js_mod_ident = js_mod_to_token_stream(self.js_mod.as_ref());
    Ok(quote! {
      #[allow(non_snake_case)]
      #[allow(non_upper_case_globals)]
      #[allow(clippy::all)]
      mod #mod_name {
        use super::*;
        use napi::__private::linkme::distributed_slice;
        #(#methods)*

        #[cfg(all(not(test), not(target_family = "wasm")))]
        fn __class() -> napi::bindgen_prelude::ErasedClassDef {
          <#name as napi::bindgen_prelude::NapiClass>::CLASS.erase()
        }

        #[cfg(all(not(test), not(target_family = "wasm")))]
        fn __props() -> Vec<napi::bindgen_prelude::Property> {
          vec![#(#props),*]
        }

        #[cfg(all(not(test), not(target_family = "wasm")))]
        #[distributed_slice(napi::__private::CLASS_IMPL_DESCRIPTORS)]
        #[linkme(crate = napi::__private::linkme)]
        static #register_name: napi::__private::ClassImplDescriptor =
          napi::__private::ClassImplDescriptor {
            class: __class,
            js_mod: #js_mod_ident,
            js_name_hint: #js_name,
            implement_iterator: false,
            props: __props,
          };

        #[cfg(all(not(test), target_family = "wasm"))]
        // Compatibility path only. Non-WASM registration is descriptor-driven.
        #[no_mangle]
        extern "C" fn #register_name() {
          napi::__private::register_napi_class_impl::<#name>(
            #js_mod_ident,
            #js_name,
            vec![#(#props_wasm),*],
            false,
          );
        }
      }
    })
  }

  fn gen_post_init_shim(&self) -> BindgenResult<TokenStream> {
    let Some(post_init_fn) = self.items.iter().find(|item| item.kind == FnKind::PostInit) else {
      return Ok(quote! {});
    };

    let name = &self.name;

    let super::r#fn::ArgConversions {
      arg_conversions,
      args: arg_names,
      ..
    } = post_init_fn.gen_arg_conversions()?;
    let receiver = post_init_fn.gen_fn_receiver()?;

    let call = quote! { #receiver(#(#arg_names),*) };
    let call = if post_init_fn.is_ret_result {
      quote! { #call?; }
    } else {
      quote! { #call; }
    };

    Ok(quote! {
      impl #name {
        #[doc(hidden)]
        pub fn __napi_post_init<'env, 'scope>(
          frame: &mut napi::bindgen_prelude::CallbackFrame<'env, 'scope>,
        ) -> napi::Result<()> {
          #(#arg_conversions)*
          #call
          Ok(())
        }
      }
    })
  }
}

pub fn rm_raw_prefix(s: &str) -> &str {
  if let Some(stripped) = s.strip_prefix("r#") {
    stripped
  } else {
    s
  }
}

fn remove_lifetime_in_type(ty: &mut syn::Type) {
  if let syn::Type::Path(syn::TypePath { path, .. }) = ty {
    path.segments.iter_mut().for_each(|segment| {
      if let syn::PathArguments::AngleBracketed(ref mut args) = segment.arguments {
        args.args.iter_mut().for_each(|arg| match arg {
          syn::GenericArgument::Type(ref mut ty) => {
            remove_lifetime_in_type(ty);
          }
          syn::GenericArgument::Lifetime(lifetime) => {
            lifetime.ident = Ident::new("_", lifetime.ident.span());
          }
          _ => {}
        });
      }
    });
  }
}
