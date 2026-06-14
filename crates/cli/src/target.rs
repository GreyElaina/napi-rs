use std::collections::HashMap;
use std::process::Command;
use std::sync::LazyLock;

use anyhow::{Context, bail};
use target_lexicon::{Architecture, Environment, OperatingSystem, Triple};

static TARGET_LINKER: LazyLock<HashMap<&str, &str>> = LazyLock::new(|| {
  HashMap::from([
    ("aarch64-unknown-linux-musl", "aarch64-linux-musl-gcc"),
    (
      "loongarch64-unknown-linux-gnu",
      "loongarch64-linux-gnu-gcc-13",
    ),
    ("riscv64gc-unknown-linux-gnu", "riscv64-linux-gnu-gcc"),
    ("powerpc64le-unknown-linux-gnu", "powerpc64le-linux-gnu-gcc"),
    ("s390x-unknown-linux-gnu", "s390x-linux-gnu-gcc"),
  ])
});

#[derive(Debug, Clone)]
pub struct Target {
  pub triple: String,
  pub platform_arch_abi: String,
  pub platform: String,
  pub arch: String,
  pub abi: Option<String>,
}

pub fn parse_triple(raw_triple: &str) -> anyhow::Result<Target> {
  if raw_triple.starts_with("wasm32-") {
    bail!("WASM targets are no longer supported by napi-rs: {raw_triple}");
  }

  let parsed: Triple = raw_triple
    .parse()
    .map_err(|e| anyhow::anyhow!("Invalid target triple `{raw_triple}`: {e}"))?;

  let platform = node_platform(&parsed.operating_system, &parsed.environment);
  let arch = node_arch(&parsed.architecture);
  let abi = node_abi(&parsed.environment);

  let platform_arch_abi = match &abi {
    Some(a) => format!("{platform}-{arch}-{a}"),
    None => format!("{platform}-{arch}"),
  };

  Ok(Target {
    triple: raw_triple.to_string(),
    platform_arch_abi,
    platform,
    arch,
    abi,
  })
}

fn node_platform(os: &OperatingSystem, env: &Environment) -> String {
  match env {
    Environment::Android | Environment::Androideabi => return "android".into(),
    Environment::Ohos => return "openharmony".into(),
    _ => {}
  }
  match os {
    OperatingSystem::Linux => "linux",
    OperatingSystem::Darwin(_) | OperatingSystem::MacOSX(_) => "darwin",
    OperatingSystem::Windows => "win32",
    OperatingSystem::Freebsd => "freebsd",
    other => return other.to_string(),
  }
  .into()
}

fn node_arch(arch: &Architecture) -> String {
  match arch {
    Architecture::X86_64 | Architecture::X86_64h => "x64",
    Architecture::Aarch64(_) => "arm64",
    Architecture::X86_32(_) => "ia32",
    Architecture::Arm(_) => "arm",
    Architecture::LoongArch64 => "loong64",
    Architecture::Riscv64(_) => "riscv64",
    Architecture::Powerpc64le => "ppc64",
    Architecture::S390x => "s390x",
    other => return other.to_string(),
  }
  .into()
}

fn node_abi(env: &Environment) -> Option<String> {
  match env {
    Environment::Gnu | Environment::Gnueabihf | Environment::GnuLlvm => Some("gnu".into()),
    Environment::Gnueabi => Some("gnueabi".into()),
    Environment::Musl => Some("musl".into()),
    Environment::Musleabihf => Some("musleabihf".into()),
    Environment::Musleabi => Some("musleabi".into()),
    Environment::Msvc => Some("msvc".into()),
    Environment::Android | Environment::Androideabi | Environment::Ohos => None,
    _ => None,
  }
}

pub fn get_system_default_target() -> anyhow::Result<Target> {
  let output = Command::new("rustc")
    .arg("-vV")
    .output()
    .context("Failed to run `rustc -vV`")?;

  let stdout = String::from_utf8(output.stdout)?;
  let triple = stdout
    .lines()
    .find_map(|line| line.strip_prefix("host: "))
    .context("Cannot parse target triple from `rustc -vV`")?;

  parse_triple(triple)
}

pub fn get_target_linker(target: &str) -> Option<&'static str> {
  TARGET_LINKER.get(target).copied()
}

pub fn target_to_env_var(target: &str) -> String {
  target.replace('-', "_").to_uppercase()
}
