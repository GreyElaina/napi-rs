fn main() {
  napi_build::setup();

  #[cfg(not(feature = "noop"))]
  napi_build::setup_typegen("napi-typegen");
}
