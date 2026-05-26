#[allow(unused_imports)]
use napi::bindgen_prelude::{Class, WeakRef};
use napi_derive::napi;

#[napi]
pub struct WeakReferenceOwner;

#[napi]
pub fn weak_reference_arg(value: WeakRef<Class<WeakReferenceOwner>>) {
  drop(value);
}

#[napi]
pub fn weak_reference_return() -> Option<WeakRef<Class<WeakReferenceOwner>>> {
  None
}

fn main() {}
