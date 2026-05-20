use napi_derive::napi;

#[napi]
pub struct NestedArrayBufferField {
  pub value: Option<napi::bindgen_prelude::ArrayBuffer<'static>>,
}

fn main() {}
