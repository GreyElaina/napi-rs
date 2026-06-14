mod build;
mod config;
mod js_binding;
mod shell;
mod target;

use std::env;

use clap::{Parser, Subcommand};

use crate::build::BuildOptions;

#[derive(Parser)]
#[command(name = "cargo", bin_name = "cargo")]
enum Cargo {
  Napi(NapiArgs),
}

#[derive(clap::Args)]
#[command(version, about = "NAPI-RS build tool")]
struct NapiArgs {
  #[command(subcommand)]
  command: NapiCommand,
}

#[derive(Subcommand)]
enum NapiCommand {
  /// Build the NAPI-RS project
  Build(BuildOptions),
}

fn main() -> anyhow::Result<()> {
  let Cargo::Napi(args) = Cargo::parse();

  match args.command {
    NapiCommand::Build(options) => {
      let cwd = env::current_dir().ok();
      let outputs = build::run_build(options)?;
      for output in &outputs {
        let display_path = cwd
          .as_deref()
          .and_then(|cwd| output.path.strip_prefix(cwd).ok())
          .unwrap_or(&output.path);
        shell::status(output.kind.label(), display_path.display());
      }
      Ok(())
    }
  }
}
