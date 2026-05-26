use napi::{bindgen_prelude::*, Either};
use napi_derive::napi;

#[napi(object)]
pub struct Shared {
  pub value: u32,
}

// Test fixture for GitHub issue #2722: Complex struct with constructor and multiple methods
#[napi]
pub struct ComplexClass {
  pub value: String,
  pub number: i32,
}

impl From<(String, i32)> for ComplexClass {
  fn from(value: (String, i32)) -> Self {
    ComplexClass {
      value: value.0,
      number: value.1,
    }
  }
}

#[napi]
impl ComplexClass {
  #[napi(constructor)]
  pub fn new(
    value: Either<String, Ref<Class<ComplexClass>>>,
    number: i32,
    #[napi(env)] mut env: Env,
  ) -> Result<Self> {
    let value_str = match value {
      Either::A(s) => s,
      Either::B(reference) => env.with_scope(|scope| {
        let reference = reference.as_class_local(scope)?;
        let instance = reference.borrow()?;
        Ok(format!("cloned:{}", instance.value))
      })?,
    };
    Ok(ComplexClass {
      value: value_str,
      number,
    })
  }

  #[napi]
  pub fn method_one(&self) -> String {
    format!("method_one: {}", self.value)
  }

  #[napi]
  pub fn method_two(&self) -> i32 {
    self.number * 2
  }

  #[napi]
  pub fn method_three(&self) -> String {
    format!("method_three: {} - {}", self.value, self.number)
  }

  #[napi]
  pub fn method_four(&self) -> bool {
    self.number > 0
  }

  #[napi]
  pub fn method_five(&self) -> String {
    self.value.to_uppercase()
  }
}
