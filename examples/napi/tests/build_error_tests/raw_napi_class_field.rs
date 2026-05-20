#![allow(non_camel_case_types)]

use napi_derive::napi;

pub struct napi_threadsafe_function;

#[napi]
pub struct RawNapiClassField {
  pub tsfn: napi_threadsafe_function,
}

fn main() {}
