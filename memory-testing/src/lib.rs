use std::thread::spawn;

use napi::{
  bindgen_prelude::*,
  threadsafe_function::{ThreadsafeCallContext, ThreadsafeFunctionCallMode},
};

#[macro_use]
extern crate napi_derive;
#[macro_use]
extern crate serde_derive;

#[derive(Serialize, Deserialize)]
pub struct Welcome {
  id: String,
  name: String,
  forename: String,
  description: String,
  email: String,
  phone: String,
  #[serde(rename = "arrivalDate")]
  arrival_date: i64,
  #[serde(rename = "departureDate")]
  departure_date: i64,
  price: i64,
  advance: i64,
  #[serde(rename = "advanceDueDate")]
  advance_due_date: i64,
  kids: i64,
  adults: i64,
  status: String,
  nourishment: String,
  #[serde(rename = "createdAt")]
  created_at: String,
  room: Room,
}

#[derive(Serialize, Deserialize)]
pub struct Room {
  id: String,
  name: String,
}

#[napi]
pub fn test_async<'env>(
  env: &'env Env<'env>,
) -> napi::Result<napi::bindgen_prelude::Promise<'env, String>> {
  let data = serde_json::json!({
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
  env.spawn_future_with_callback(
    async move { Ok(serde_json::to_string(&data).unwrap()) },
    |_, res| Ok(res),
  )
}

#[napi]
pub fn from_js(env: Env, input_object: Object) -> napi::Result<String> {
  let a: Welcome = env.from_js_value(input_object)?;
  Ok(serde_json::to_string(&a)?)
}

#[napi]
pub struct MemoryHolder(Vec<u8>);

#[napi]
impl MemoryHolder {
  #[napi(constructor)]
  pub fn new(len: u32) -> Result<Self> {
    Ok(Self(vec![42; len as usize]))
  }

  #[napi]
  pub fn create_reference(
    &self,
    this: Reference<MemoryHolder>,
  ) -> ClassInitializer<ChildReference> {
    ClassInitializer::from(ChildReference(this))
  }
}

#[napi]
pub struct ChildReference(Reference<MemoryHolder>);

#[napi]
impl ChildReference {
  #[napi]
  pub fn count(&self, mut env: Env) -> Result<u32> {
    env.with_scope(|scope| {
      let holder_ref = scope.bind_reference(&self.0)?;
      let holder = scope.borrow_class(&holder_ref)?;
      Ok(holder.0.len() as u32)
    })
  }
}

#[napi]
pub fn leaking_func(func: Function<String, String>) -> napi::Result<()> {
  let tsfn = func
    .build_threadsafe_function()
    .weak::<true>()
    .build_callback(|ctx: ThreadsafeCallContext<String>| Ok(ctx.value))?;

  spawn(move || {
    tsfn.call("foo".into(), ThreadsafeFunctionCallMode::Blocking);
  });

  Ok(())
}

#[napi]
pub fn buffer_convert(buffer: Buffer) -> Buffer {
  Buffer::from(vec![0; buffer.len()])
}

#[napi]
pub fn buffer_len() -> u32 {
  Buffer::from(vec![0; 1024 * 10240]).len() as u32
}

#[napi]
pub fn array_buffer_convert(array_buffer: Uint8Array) -> Uint8Array {
  Uint8Array::new(vec![1; array_buffer.len()])
}

#[napi]
pub fn array_buffer_create_from_vec_with_spare_capacity() -> Uint8Array {
  let mut v = vec![1; 1024 * 10240];
  v.truncate(1);
  Uint8Array::new(v)
}

#[napi]
pub fn array_buffer_len() -> u32 {
  Uint8Array::new(vec![1; 1024 * 10240]).len() as u32
}

#[napi]
pub fn buffer_pass_through(buffer: Buffer) -> Buffer {
  buffer
}

#[napi]
pub fn array_buffer_pass_through(array_buffer: Uint8Array) -> Uint8Array {
  array_buffer
}

#[napi]
pub async fn returns_future() {}
