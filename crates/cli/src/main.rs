mod build;
mod config;
mod js_binding;
mod target;
mod typegen;

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
            let outputs = build::run_build(options)?;
            for output in &outputs {
                eprintln!("  {:?} → {}", output.kind, output.path.display());
            }
            Ok(())
        }
    }
}
