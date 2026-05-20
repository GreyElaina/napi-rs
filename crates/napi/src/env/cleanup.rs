use std::os::raw::c_void;

use crate::{check_status, sys, Result};

use super::Env;

#[cfg(feature = "napi3")]
pub(crate) struct CleanupEnvHookData<T: 'static> {
  pub(crate) data: T,
  pub(crate) hook: Box<dyn FnOnce(T)>,
}

/// Created by `Env::add_env_cleanup_hook`
/// And used by `Env::remove_env_cleanup_hook`
#[cfg(feature = "napi3")]
#[derive(Clone, Copy)]
pub struct CleanupEnvHook<T: 'static>(pub(crate) *mut CleanupEnvHookData<T>);

impl<'env> Env<'env> {
  #[cfg(feature = "napi3")]
  pub fn add_env_cleanup_hook<T, F>(
    &self,
    cleanup_data: T,
    cleanup_fn: F,
  ) -> Result<CleanupEnvHook<T>>
  where
    T: 'static,
    F: 'static + FnOnce(T),
  {
    let hook = CleanupEnvHookData {
      data: cleanup_data,
      hook: Box::new(cleanup_fn),
    };
    let hook_ref = Box::leak(Box::new(hook));
    #[cfg(not(target_family = "wasm"))]
    {
      check_status!(unsafe {
        sys::napi_add_env_cleanup_hook(
          self.0,
          Some(cleanup_env::<T>),
          (hook_ref as *mut CleanupEnvHookData<T>).cast(),
        )
      })?;
    }

    #[cfg(all(target_family = "wasm", not(feature = "noop")))]
    {
      check_status!(unsafe {
        crate::napi_add_env_cleanup_hook(
          self.0,
          Some(cleanup_env::<T>),
          (hook_ref as *mut CleanupEnvHookData<T>).cast(),
        )
      })?;
    }
    Ok(CleanupEnvHook(hook_ref))
  }

  #[cfg(feature = "napi3")]
  pub fn remove_env_cleanup_hook<T>(&self, hook: CleanupEnvHook<T>) -> Result<()>
  where
    T: 'static,
  {
    check_status!(unsafe {
      sys::napi_remove_env_cleanup_hook(self.0, Some(cleanup_env::<T>), hook.0 as *mut _)
    })
  }
}

#[cfg(feature = "napi3")]
unsafe extern "C" fn cleanup_env<T: 'static>(hook_data: *mut c_void) {
  let cleanup_env_hook = unsafe { Box::from_raw(hook_data as *mut CleanupEnvHookData<T>) };
  (cleanup_env_hook.hook)(cleanup_env_hook.data);
}
