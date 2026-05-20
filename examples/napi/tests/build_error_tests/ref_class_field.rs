use napi_derive::napi;

pub struct Ref<T>(std::marker::PhantomData<T>);
pub struct Unknown<'env>(std::marker::PhantomData<&'env ()>);

#[napi]
pub struct RefClassField {
  pub value: Ref<Unknown<'static>>,
}

fn main() {}
