use napi::{bindgen_prelude::*, JsString};

#[napi]
pub fn shorter_scope(env: &mut Env, arr: Array) -> Result<Vec<u32>> {
  let len = arr.len();
  let mut result = Vec::with_capacity(len as usize);
  for i in 0..len {
    let handle_scope = HandleScope::create(env)?;
    let len = env.with_scope(|scope| {
      let value = scope
        .get_optional_element::<Unknown>(&arr, i)?
        .ok_or_else(|| {
          Error::new(
            Status::InvalidArg,
            format!("Missing array element at index {i}"),
          )
        })?;
      match value.get_type()? {
        ValueType::String => {
          let value = value.into_js(scope)?;
          let string = JsString::from_js(scope, value)?;
          Ok(string.utf8_len()? as u32)
        }
        ValueType::Object => Ok(1),
        _ => Ok(0),
      }
    })?;
    unsafe { handle_scope.close(arr, |_| Ok(()))? };
    result.push(len);
  }
  Ok(result)
}

#[napi]
pub fn shorter_escapable_scope<'env>(
  env: &mut Env<'env>,
  create_string: Function<(), Option<String>>,
) -> Result<String> {
  let mut longest_string = String::new();
  let mut prev_len = 0;
  loop {
    let maybe_longest = env.with_scope(|scope| scope.call(&create_string, ()))?;
    match maybe_longest {
      Some(string) => {
        let len = string.len();
        if len <= longest_string.len() || len == prev_len {
          break;
        }
        prev_len = len;
        longest_string = string;
      }
      None => break,
    }
  }
  Ok(longest_string)
}
