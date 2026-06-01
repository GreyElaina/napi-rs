/// default enum values are continuos i32s start from 0
#[napi]
#[derive(Debug, Clone, Copy)]
pub enum Kind {
  /// Barks
  Dog,
  /// Kills birds
  Cat,
  /// Tasty
  Duck,
}

#[napi]
pub enum Empty {}

#[napi(string_enum)]
pub enum Status {
  Pristine,
  Loading,
  Ready,
}

#[allow(clippy::enum_variant_names)]
#[napi(string_enum = "lowercase")]
pub enum StringEnum {
  VariantOne,
  VariantTwo,
  VariantThree,
}

/// You could break the step and for an new continuous value.
#[napi]
pub enum CustomNumEnum {
  One = 1,
  Two,
  Three = 3,
  Four,
  #[doc(hidden)]
  Six = 6,
  Eight = 8,
  Nine, // would be 9
  Ten,  // 10
}

#[napi]
fn enum_to_i32(e: CustomNumEnum) -> i32 {
  e as i32
}

#[napi(skip_typescript)]
pub enum SkippedEnums {
  One = 1,
  Two,
  Tree,
}

#[napi(string_enum)]
pub enum CustomStringEnum {
  #[napi(value = "my-custom-value")]
  Foo,
  Bar,
  Baz,
}

#[napi(object, discriminant = "type2")]
pub enum StructuredKind {
  Hello,
  Greeting { name: String },
  Birthday { name: String, age: u8 },
  Tuple(u32, u32),
}

#[napi(object, discriminant_case = "lowercase")]
pub enum StructuredKindLowercase {
  Hello,
  Greeting { name: String },
  Birthday { name: String, age: u8 },
  Tuple(u32, u32),
}

#[napi]
pub fn validate_structured_enum(kind: StructuredKind) -> StructuredKind {
  kind
}

#[napi]
pub fn validate_structured_enum_lowercase(
  kind: StructuredKindLowercase,
) -> StructuredKindLowercase {
  kind
}

#[napi]
pub enum Shape {
  Circle { radius: f64 },
  Rectangle { width: f64, height: f64 },
}

#[napi]
impl Shape {
  #[napi(factory)]
  pub fn circle(radius: f64) -> Shape {
    Shape::Circle { radius }
  }

  #[napi(factory)]
  pub fn rectangle(width: f64, height: f64) -> Shape {
    Shape::Rectangle { width, height }
  }

  #[napi(getter)]
  pub fn kind(&self) -> &str {
    match self {
      Shape::Circle { .. } => "Circle",
      Shape::Rectangle { .. } => "Rectangle",
    }
  }

  #[napi]
  pub fn area(&self) -> f64 {
    match self {
      Shape::Circle { radius } => std::f64::consts::PI * radius * radius,
      Shape::Rectangle { width, height } => width * height,
    }
  }
}
