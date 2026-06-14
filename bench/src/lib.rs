use std::str::FromStr;

use napi::bindgen_prelude::*;
use serde_json::{from_str, to_string};

#[macro_use]
extern crate napi_derive;

#[cfg(all(
  target_arch = "x86_64",
  not(target_env = "musl"),
  not(debug_assertions)
))]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[napi]
pub fn noop() {}

#[napi]
pub fn plus(a: u32, b: u32) -> u32 {
  a + b
}

#[napi]
pub fn bench_create_buffer() -> Buffer {
  Buffer::from(vec![1, 2])
}

#[napi]
pub fn create_array_json() -> Result<String> {
  let a: Vec<u32> = vec![42; 1000];
  Ok(to_string(&a)?)
}

#[napi]
pub fn create_array() -> Vec<u32> {
  vec![42; 1000]
}

#[napi]
pub fn create_array_with_serde_trait<'env>(
  #[napi(env)] env: &'env Env<'env>,
) -> Result<Unknown<'env>> {
  let a: Vec<u32> = vec![42; 1000];
  env.to_js_value(&a)
}

#[napi]
pub fn get_array_from_json(input: String) -> Result<()> {
  let _: Vec<u32> = from_str(&input)?;
  Ok(())
}

#[napi]
pub fn get_array_from_js_array(input: Vec<u32>) {
  drop(input);
}

#[napi]
pub fn get_array_with_for_loop<'env>(
  #[napi(env)] env: &mut Env<'env>,
  input: Object<'env>,
) -> Result<()> {
  env.with_scope(|scope| {
    let array_length = input.get_array_length_unchecked()? as usize;
    let mut result = Vec::with_capacity(array_length);
    for index in 0..array_length {
      result.push(scope.get_element::<Unknown, _>(&input, index as u32)?);
    }
    drop(result);
    Ok(())
  })
}

#[napi]
pub async fn bench_blocking(buffer: Buffer) -> Result<u32> {
  Ok(buffer.len() as u32 + 1)
}

#[napi]
pub fn bench_tokio_future<'env>(
  #[napi(env)] env: &'env Env<'env>,
  buffer: Buffer,
) -> Result<Promise<'env, u32>> {
  let len = buffer.len() as u32;
  env.spawn_promise(async move { Ok(len + 1) })
}

pub struct QueryEngine {
  datamodel: String,
}

impl QueryEngine {
  async fn query(&self) -> String {
    let data = serde_json::json!({
      "datamodel": self.datamodel,
      "findFirstBooking": {
        "id": "ckovh15xa104945sj64rdk8oas",
        "name": "1883da9ff9152",
        "forename": "221c99bedc6a4",
        "description": "8bf86b62ce6a",
        "email": "9d57a869661cc",
        "phone": "7e0c58d147215",
        "arrivalDate": -92229669,
        "departureDate": 202138795,
        "price": -1592700387,
        "advance": -369294193,
        "advanceDueDate": 925000428,
        "kids": 520124290,
        "adults": 1160258464,
        "status": "NO_PAYMENT",
        "nourishment": "BB",
        "createdAt": "2021-05-19T12:58:37.246Z",
        "room": { "id": "ckovh15xa104955sj6r2tqaw1c", "name": "38683b87f2664" }
      }
    });

    to_string(&data).expect("benchmark fixture must serialize")
  }
}

#[napi]
pub fn engine(datamodel: String) -> External<QueryEngine> {
  let size_hint = datamodel.len();
  External::new_with_size_hint(QueryEngine { datamodel }, size_hint)
}

#[napi]
pub async fn query(datamodel: String) -> String {
  let engine = QueryEngine { datamodel };
  engine.query().await
}

enum LineJoin {
  Miter,
  Round,
  Bevel,
}

impl LineJoin {
  fn as_str(&self) -> &str {
    match self {
      Self::Bevel => "bevel",
      Self::Miter => "miter",
      Self::Round => "round",
    }
  }
}

impl FromStr for LineJoin {
  type Err = Error;

  fn from_str(value: &str) -> Result<Self> {
    match value {
      "bevel" => Ok(Self::Bevel),
      "round" => Ok(Self::Round),
      "miter" => Ok(Self::Miter),
      _ => Err(Error::new(
        Status::InvalidArg,
        format!("[{value}] is not valid LineJoin value"),
      )),
    }
  }
}

#[napi]
pub struct TestClass {
  miter_limit: u32,
  line_join: LineJoin,
}

#[napi]
impl TestClass {
  #[napi(constructor)]
  pub fn new() -> Self {
    Self {
      miter_limit: 10,
      line_join: LineJoin::Miter,
    }
  }

  #[napi(getter, js_name = "miterNative")]
  pub fn miter_native(&self) -> u32 {
    self.miter_limit
  }

  #[napi(setter, js_name = "miterNative")]
  pub fn set_miter_native(&mut self, value: u32) {
    self.miter_limit = value;
  }

  #[napi(getter)]
  pub fn miter(&self) -> u32 {
    self.miter_limit
  }

  #[napi(setter)]
  pub fn set_miter(&mut self, value: u32) {
    self.miter_limit = value;
  }

  #[napi(getter, js_name = "lineJoinNative")]
  pub fn line_join_native(&self) -> &str {
    self.line_join.as_str()
  }

  #[napi(setter, js_name = "lineJoinNative")]
  pub fn set_line_join_native(&mut self, value: String) -> Result<()> {
    self.line_join = LineJoin::from_str(&value)?;
    Ok(())
  }

  #[napi(getter, js_name = "lineJoin")]
  pub fn line_join(&self) -> &str {
    self.line_join.as_str()
  }

  #[napi(setter, js_name = "lineJoin")]
  pub fn set_line_join(&mut self, value: String) -> Result<()> {
    self.line_join = LineJoin::from_str(&value)?;
    Ok(())
  }
}
