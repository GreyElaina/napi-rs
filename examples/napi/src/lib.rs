#![allow(dead_code)]
#![allow(unreachable_code)]
#![allow(clippy::disallowed_names)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::new_without_default)]
#![allow(non_snake_case)]
#![allow(deprecated)]

use napi::bindgen_prelude::{Env, JsObjectValue, Object, Result, Symbol};
pub use napi_shared::*;

#[macro_use]
extern crate napi_derive;
#[macro_use]
extern crate serde_derive;

#[cfg(feature = "snmalloc")]
#[global_allocator]
static ALLOC: snmalloc_rs::SnMalloc = snmalloc_rs::SnMalloc;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static ALLOC: mimalloc_safe::MiMalloc = mimalloc_safe::MiMalloc;

#[napi]
/// This is a const
pub const DEFAULT_COST: u32 = 12;

#[napi(skip_typescript)]
pub const TYPE_SKIPPED_CONST: u32 = 12;

#[napi]
pub fn shutdown_runtime() {}

#[napi(module_exports)]
pub fn exports(#[napi(env)] env: Env, mut export: Object) -> Result<()> {
  napi_runtime_tokio::install_factory(&env, || {
    tokio::runtime::Builder::new_multi_thread()
      .enable_all()
      .build()
      .expect("Create Tokio runtime failed")
  })?;

  let symbol = Symbol::for_desc("NAPI_RS_SYMBOL");
  export.set_named_property("NAPI_RS_SYMBOL", symbol)?;
  Ok(())
}

mod array;
mod r#async;
mod async_generator_repro;
mod async_regression;
mod bigint;
mod callback;
mod class;
mod class_factory;
mod constructor;
mod date;
mod either;
mod r#enum;
mod env;
mod error;
mod external;
mod fetch;
mod fn_ts_override;
mod function;
mod generator;
mod js_mod;
mod late_class_registration;
mod lifetime;
mod map;
mod nullable;
mod number;
mod object;
mod promise;
mod reference;
mod scope;
mod serde;
mod set;
mod shared;
mod stream;
mod string;
mod symbol;
mod task;
mod transparent;
mod r#type;
mod typed_array;
