use napi_derive::napi;

#[napi]
pub fn raw_value_arg(value: napi::sys::napi_value) -> u32 {
  value as usize as u32
}

#[napi]
pub fn raw_ref_return() -> napi::sys::napi_ref {
  std::ptr::null_mut()
}

fn main() {}
