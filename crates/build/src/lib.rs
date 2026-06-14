use std::env;

mod android;
mod windows;

pub fn setup() {
  // compatible with the v2 versions, will remove in the future
  {
    println!("cargo:rerun-if-env-changed=DEBUG_GENERATED_CODE");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_NAPI_RS_CLI_VERSION");
  }

  println!("cargo::rerun-if-env-changed=NAPI_DEBUG_GENERATED_CODE");
  println!(
    "cargo::rerun-if-env-changed=NAPI_FORCE_BUILD_{}",
    env::var("CARGO_PKG_NAME")
      .expect("CARGO_PKG_NAME is not set")
      .to_uppercase()
      .replace("-", "_")
  );

  let target_env = env::var("CARGO_CFG_TARGET_ENV").expect("CARGO_CFG_TARGET_ENV is not set");
  let target_os = env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS is not set");

  match target_os.as_str() {
    "android" if android::setup().is_ok() => {}
    "macos" => {
      // Keep the dynamic lookup behavior on macOS to avoid breaking changes.
      println!("cargo:rustc-cdylib-link-arg=-Wl");
      println!("cargo:rustc-cdylib-link-arg=-undefined");
      println!("cargo:rustc-cdylib-link-arg=dynamic_lookup");
    }
    "windows" => {
      if let Ok("gnu") = env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        windows::setup_gnu();
      }
    }
    _ => {}
  }

  if (target_env == "gnu" && target_os != "windows")
    || target_os == "freebsd"
    || target_os == "openbsd"
  {
    // https://sourceware.org/bugzilla/show_bug.cgi?id=21032
    // https://sourceware.org/bugzilla/show_bug.cgi?id=21031
    // https://github.com/rust-lang/rust/issues/134820
    // pthread_key_create() destructors and segfault after a DSO unloading
    println!("cargo:rustc-link-arg=-Wl,-z,nodelete");
  }
}

/// Set up linker flags for the typegen binary target.
///
/// The typegen binary links the crate as rlib to collect linkme-registered
/// type descriptors. The crate's FFI code references Node-API/libuv symbols
/// that are only available when loaded as a .node addon — not in a standalone
/// binary. This function tells the linker to allow those unresolved symbols
/// since the typegen binary never calls FFI code.
///
/// Call from build.rs:
/// ```no_run
/// napi_build::setup_typegen("my-typegen-bin");
/// ```
pub fn setup_typegen(bin_name: &str) {
  let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

  match target_os.as_str() {
    "macos" | "ios" => {
      println!("cargo:rustc-link-arg-bin={bin_name}=-Wl,-undefined,dynamic_lookup");
    }
    "linux" | "freebsd" | "openbsd" | "android" => {
      println!("cargo:rustc-link-arg-bin={bin_name}=-Wl,--allow-shlib-undefined");
    }
    _ => {}
  }
}
