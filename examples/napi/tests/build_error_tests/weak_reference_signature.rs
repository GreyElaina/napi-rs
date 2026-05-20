use napi::bindgen_prelude::WeakReference;
use napi_derive::napi;

#[napi]
pub struct WeakReferenceOwner;

#[napi]
pub fn weak_reference_arg(value: WeakReference<WeakReferenceOwner>) {
  drop(value);
}

#[napi]
pub fn weak_reference_return() -> Option<WeakReference<WeakReferenceOwner>> {
  None
}

fn main() {}
