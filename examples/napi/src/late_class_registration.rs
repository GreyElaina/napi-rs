use napi::bindgen_prelude::*;

#[napi]
impl LateRegisteredBase {
  #[napi]
  pub fn base_value(&self) -> u32 {
    self.value
  }
}

#[napi(subclass)]
pub struct LateRegisteredBase {
  value: u32,
}

impl LateRegisteredBase {
  fn new(value: u32) -> Self {
    Self { value }
  }
}

#[napi(extends = LateRegisteredBase)]
pub struct LateRegisteredChild {
  child_value: u32,
}

#[napi]
impl LateRegisteredChild {
  #[napi(constructor)]
  pub fn new(value: u32, child_value: u32) -> ClassInitializer<Self> {
    ClassInitializer::from_parent(
      ClassInitializer::from(LateRegisteredBase::new(value)),
      Self { child_value },
    )
  }

  #[napi]
  pub fn child_value(&self) -> u32 {
    self.child_value
  }
}
