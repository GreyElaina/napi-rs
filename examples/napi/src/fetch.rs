use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use reqwest::{header::HeaderMap, Method};
use tokio_stream::StreamExt;

#[napi(object)]
pub struct RequestInit {
  pub method: Option<String>,
  pub headers: Option<HashMap<String, String>>,
}

pub struct FetchResponse(reqwest::Response);

impl<'scope> IntoJs<'scope> for FetchResponse {
  type Output = Object<'scope>;

  fn into_js(self, scope: &mut Scope<'_, 'scope>) -> Result<Local<'scope, Self::Output>> {
    let reqwest_stream = self.0.bytes_stream();
    let napi_stream = reqwest_stream.filter_map(|chunk| match chunk {
      Ok(bytes) => {
        if bytes.is_empty() {
          return None;
        }

        Some(Ok(bytes))
      }
      Err(e) => Some(Err(napi::Error::new(
        napi::Status::Unknown,
        format!("Error reading response stream: {e:?}"),
      ))),
    });
    let js_stream = ReadableStream::create_with_stream_bytes(scope.env(), napi_stream)?;
    let global = scope.env().get_global()?;
    let response_constructor: Function<ReadableStream<BufferSlice>, ()> =
      scope.get_named_property(&global, "Response")?;
    let response = scope.new_instance(&response_constructor, js_stream)?;
    let response = response.into_js(scope)?;
    let response = Object::from_js(scope, response)?;
    response.into_js(scope)
  }
}

#[napi(ts_return_type = "Promise<import('undici-types').Response>")]
pub fn fetch<'env>(
  env: &'env Env<'env>,
  url: String,
  request_init: Option<RequestInit>,
) -> Result<Promise<'env, FetchResponse>> {
  env.spawn_future(async move {
    let headers: HeaderMap =
      if let Some(headers) = request_init.as_ref().and_then(|init| init.headers.as_ref()) {
        headers
          .try_into()
          .map_err(|err| Error::new(Status::InvalidArg, format!("Invalid header: {err}")))?
      } else {
        HeaderMap::new()
      };
    let client = reqwest::Client::new();
    let request = client
      .request(Method::GET, url)
      .headers(headers)
      .build()
      .map_err(|e| Error::new(Status::InvalidArg, format!("Invalid request: {e}")))?;

    let response = client
      .execute(request)
      .await
      .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))?;
    Ok(FetchResponse(response))
  })
}
