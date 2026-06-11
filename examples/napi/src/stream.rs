use bytes::BytesMut;
use futures::channel::mpsc;
use futures::stream::StreamExt;
use napi::bindgen_prelude::*;

pub struct AcceptedStream(BytesMut);

impl<'scope> IntoJs<'scope> for AcceptedStream {
  type Output = BufferSlice<'scope>;

  fn into_js(mut self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let value_ptr = self.0.as_mut_ptr();
    unsafe {
      BufferSlice::from_external(scope.env(), value_ptr, self.0.len(), self.0, move |bytes| {
        drop(bytes);
      })?
      .into_js(scope)
    }
  }
}

#[napi]
pub fn accept_stream<'env>(
  #[napi(env)] env: &'env Env<'env>,
  stream: ReadableStream<Uint8Array>,
) -> Result<Promise<'env, AcceptedStream>> {
  let web_readable_stream = stream.read()?;
  env.spawn_promise(async move {
    let mut bytes_mut = BytesMut::new();
    futures::pin_mut!(web_readable_stream);
    while let Some(chunk) = web_readable_stream.next().await {
      let chunk = chunk?;
      bytes_mut.extend_from_slice(&chunk);
    }
    Ok(AcceptedStream(bytes_mut))
  })
}

#[napi]
pub fn create_readable_stream<'env>(
  #[napi(env)] env: &'env Env<'env>,
) -> Result<ReadableStream<'env, BufferSlice<'env>>> {
  let (mut tx, rx) = mpsc::channel(100);
  std::thread::spawn(move || {
    for _ in 0..100 {
      if tx.try_send(Ok(b"hello".to_vec())).is_err() {
        break;
      }
    }
  });
  ReadableStream::create_with_stream_bytes(env, rx)
}

#[napi(object)]
#[derive(Default)]
pub struct NestedMetadata {
  pub hello: String,
}

#[napi(object)]
#[derive(Default)]
pub struct StreamItem {
  pub something: NestedMetadata,
  pub name: String,
  pub size: i32,
}

#[napi]
pub fn create_readable_stream_with_object<'env>(
  #[napi(env)] env: &'env Env<'env>,
) -> Result<ReadableStream<'env, StreamItem>> {
  let (mut tx, rx) = mpsc::channel(100);
  std::thread::spawn(move || {
    for i in 0..100 {
      let item = StreamItem {
        something: Default::default(),
        name: Default::default(),
        size: i,
      };
      if tx.try_send(Ok(item)).is_err() {
        break;
      }
    }
  });
  ReadableStream::new(env, rx)
}

#[napi(ts_args_type = "readableStreamClass: typeof ReadableStream")]
pub fn create_readable_stream_from_class<'env>(
  #[napi(env)] env: &Env,
  readable_stream_class: Unknown<'env>,
) -> Result<ReadableStream<'env, BufferSlice<'env>>> {
  let (mut tx, rx) = mpsc::channel(100);
  std::thread::spawn(move || {
    for _ in 0..100 {
      if tx.try_send(Ok(b"hello".to_vec())).is_err() {
        break;
      }
    }
  });
  ReadableStream::with_stream_bytes_and_readable_stream_class(env, &readable_stream_class, rx)
}
