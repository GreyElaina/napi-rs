use napi_derive::napi;

pub struct CleanupEnvHook<T>(std::marker::PhantomData<T>);
pub struct Date<'env>(std::marker::PhantomData<&'env ()>);
pub struct EscapableHandleScope<'env>(std::marker::PhantomData<&'env ()>);
pub struct HandleScope;

#[napi]
pub struct EnvBoundClassField {
  pub cleanup: CleanupEnvHook<()>,
  pub date: Date<'static>,
  pub escapable: EscapableHandleScope<'static>,
  pub handle: HandleScope,
}

fn main() {}
