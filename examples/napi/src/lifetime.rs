use std::path::{Path, PathBuf};

use napi::{bindgen_prelude::*, threadsafe_function::UnknownReturnValue, JsString};

use crate::{class::Animal, r#enum::Kind};

#[napi]
pub struct ClassWithLifetime {
  inner: Reference<Animal>,
}

#[napi]
impl ClassWithLifetime {
  #[napi(constructor)]
  pub fn new(mut env: Env, mut this: This) -> Result<Self> {
    env.with_scope(|scope| {
      let inner = scope.reference(Animal::new(Kind::Cat, "alie".to_owned()))?;
      this.set("inner", scope.clone_reference(&inner)?)?;
      Ok(Self { inner })
    })
  }

  #[napi]
  pub fn get_name(&self, mut env: Env) -> Result<String> {
    env.with_scope(|scope| {
      let inner = scope.bind_reference(&self.inner)?;
      let name = {
        let animal = scope.borrow_class(&inner)?;
        animal.get_name().to_owned()
      };
      Ok(name)
    })
  }
}

#[napi]
pub struct CreateStringClass {
  inner: PathBuf,
}

#[napi]
impl CreateStringClass {
  #[napi]
  pub fn new() -> ClassInitializer<Self> {
    ClassInitializer::from(Self {
      inner: PathBuf::from(""),
    })
  }

  #[napi]
  pub fn create_string<'env>(&self, env: &'env Env) -> Option<JsString<'env>> {
    create_string(env, &self.inner).ok()
  }

  #[napi]
  pub fn create_string_result<'env>(&self, env: &'env Env) -> Result<JsString<'env>> {
    create_string(env, &self.inner)
  }
}

fn create_string<'env>(env: &'env Env, path: &Path) -> Result<JsString<'env>> {
  let path = path.to_string_lossy();
  env.create_string(path.as_ref())
}

#[napi]
pub fn callback_in_spawn<'env>(
  env: &mut Env<'env>,
  callback: Function<Object, UnknownReturnValue>,
) -> Result<()> {
  env.with_scope(|scope| {
    let callback_ref = scope.create_ref(&callback)?;
    scope
      .blocking(|| Ok(()))
      .promise(|_, ()| Ok(()))?
      .then(move |scope, ctx| {
        let mut obj = Object::new(scope.env())?;
        obj.set("foo", "bar")?;
        let cb = scope.borrow_function(&callback_ref)?;
        scope.call(&cb, obj)?;
        drop(ctx);
        Ok(())
      })?;
    Ok(())
  })?;
  Ok(())
}

#[napi]
pub fn compress_sync<'env>(
  env: &'env Env,
  _: Either<String, &'env [u8]>,
) -> Result<BufferSlice<'env>> {
  BufferSlice::from_data(env, vec![])
}
