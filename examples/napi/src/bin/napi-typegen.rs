use std::env;
use std::fs;
use std::path::{Path, PathBuf};

// Force linker to include linkme section entries from the lib crate.
extern crate napi_examples;
extern crate napi_shared;

fn main() {
  let mut args = env::args().skip(1).collect::<Vec<_>>();
  let mut output = PathBuf::from("index.d.cts");
  let mut header: Option<String> = None;
  let mut exports_out: Option<PathBuf> = None;
  let mut const_enum = false;
  let mut runtime_string_enum = false;

  let mut i = 0;
  while i < args.len() {
    match args[i].as_str() {
      "--header-file" => {
        let path = args.get(i + 1).expect("missing value for --header-file");
        header = Some(fs::read_to_string(path).unwrap_or_else(|e| {
          eprintln!("Failed to read header file {path}: {e}");
          std::process::exit(1);
        }));
        args.drain(i..=i + 1);
        continue;
      }
      "--exports-out" => {
        exports_out = Some(PathBuf::from(
          args.get(i + 1).expect("missing value for --exports-out"),
        ));
        args.drain(i..=i + 1);
        continue;
      }
      "--const-enum" => {
        const_enum = true;
        args.remove(i);
        continue;
      }
      "--runtime-string-enum" => {
        runtime_string_enum = true;
        args.remove(i);
        continue;
      }
      arg if !arg.starts_with('-') && output == PathBuf::from("index.d.cts") => {
        output = PathBuf::from(arg);
        args.remove(i);
        continue;
      }
      _ => {
        i += 1;
      }
    }
  }

  if let Some(path) = args.first() {
    output = PathBuf::from(path);
  }

  run_typegen(
    &output,
    header.as_deref(),
    exports_out.as_deref(),
    const_enum,
    runtime_string_enum,
  );
}

fn run_typegen(
  output: &Path,
  header: Option<&str>,
  exports_out: Option<&Path>,
  const_enum: bool,
  runtime_string_enum: bool,
) {
  match napi::typegen::generate_dts_with_options(
    output,
    napi::typegen::GenerateDtsOptions {
      const_enum,
      runtime_string_enum,
      header,
    },
  ) {
    Ok(result) => {
      eprintln!(
        "Generated {} with {} exports",
        output.display(),
        result.exports.len()
      );
      if let Some(exports_path) = exports_out {
        let content = result.exports.join("\n");
        if let Err(e) = fs::write(exports_path, content) {
          eprintln!(
            "Failed to write exports file {}: {e}",
            exports_path.display()
          );
          std::process::exit(1);
        }
      }
    }
    Err(e) => {
      eprintln!("Failed to generate type definitions: {e}");
      std::process::exit(1);
    }
  }
}
