use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi]
pub struct TsfnReferenceOwner {
  value: u32,
}

#[napi]
impl TsfnReferenceOwner {
  #[napi(constructor)]
  pub fn new(value: u32) -> Self {
    Self { value }
  }

  #[napi]
  pub fn capture_reference(this: Ref<Class<Self>>, callback: Function<(), ()>) -> Result<()> {
    let captured = Some(this);
    let tsfn = callback
      .build_threadsafe_function::<()>()
      .build_callback(move |context| {
        let has_reference = captured.is_some();
        match (context.value, has_reference) {
          ((), _) => Ok(()),
        }
      })?;
    tsfn.call(
      (),
      napi::threadsafe_function::ThreadsafeFunctionCallMode::NonBlocking,
    );
    Ok(())
  }
}

fn main() {}
