use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

use crate::shell;
use crate::target::parse_triple;

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserNapiConfig {
  pub binary_name: Option<String>,
  pub package_name: Option<String>,
  pub targets: Option<Vec<String>>,
  pub npm_client: Option<String>,
  pub const_enum: Option<bool>,
  pub runtime_string_enum: Option<bool>,
  pub dts_header: Option<String>,
  pub dts_header_file: Option<String>,
  pub typegen_bin: Option<String>,
  // deprecated fields
  pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageJson {
  pub name: String,
  pub version: String,
  pub napi: Option<UserNapiConfig>,
}

#[derive(Debug)]
pub struct NapiConfig {
  pub binary_name: String,
  pub package_name: String,
  pub dts_header: Option<String>,
  pub dts_header_file: Option<String>,
  pub typegen_bin: Option<String>,
  pub package_json: PackageJson,
}

pub fn read_napi_config(
  package_json_path: &Path,
  config_path: Option<&Path>,
) -> anyhow::Result<NapiConfig> {
  let content = std::fs::read_to_string(package_json_path)
    .with_context(|| format!("Failed to read {}", package_json_path.display()))?;
  let pkg_json: PackageJson = serde_json::from_str(&content)
    .with_context(|| format!("Failed to parse {}", package_json_path.display()))?;

  let mut user_config = pkg_json
    .napi
    .as_ref()
    .map_or_else(UserNapiConfig::default, |c| UserNapiConfig {
      binary_name: c.binary_name.clone(),
      package_name: c.package_name.clone(),
      targets: c.targets.clone(),
      npm_client: c.npm_client.clone(),
      const_enum: c.const_enum,
      runtime_string_enum: c.runtime_string_enum,
      dts_header: c.dts_header.clone(),
      dts_header_file: c.dts_header_file.clone(),
      typegen_bin: c.typegen_bin.clone(),
      name: c.name.clone(),
    });

  if let Some(config_path) = config_path {
    let config_content = std::fs::read_to_string(config_path)
      .with_context(|| format!("Failed to read config {}", config_path.display()))?;
    let sep_config: UserNapiConfig = serde_json::from_str(&config_content)
      .with_context(|| format!("Failed to parse config {}", config_path.display()))?;

    if pkg_json.napi.is_some() {
      shell::warn(
        "both napi field in package.json and separate config file found, \
                 config file takes precedence",
      );
    }

    merge_config(&mut user_config, sep_config);
  }

  // deprecated name field
  if let Some(ref name) = user_config.name {
    if user_config.binary_name.is_none() {
      shell::warn("napi.name is deprecated, use napi.binaryName instead");
      user_config.binary_name = Some(name.clone());
    }
  }

  for target in user_config.targets.unwrap_or_default() {
    parse_triple(&target)?;
  }

  Ok(NapiConfig {
    binary_name: user_config
      .binary_name
      .unwrap_or_else(|| "index".to_string()),
    package_name: user_config
      .package_name
      .unwrap_or_else(|| pkg_json.name.clone()),
    dts_header: user_config.dts_header,
    dts_header_file: user_config.dts_header_file,
    typegen_bin: user_config.typegen_bin,
    package_json: pkg_json,
  })
}

fn merge_config(base: &mut UserNapiConfig, other: UserNapiConfig) {
  if other.binary_name.is_some() {
    base.binary_name = other.binary_name;
  }
  if other.package_name.is_some() {
    base.package_name = other.package_name;
  }
  if other.targets.is_some() {
    base.targets = other.targets;
  }
  if other.npm_client.is_some() {
    base.npm_client = other.npm_client;
  }
  if other.const_enum.is_some() {
    base.const_enum = other.const_enum;
  }
  if other.runtime_string_enum.is_some() {
    base.runtime_string_enum = other.runtime_string_enum;
  }
  if other.dts_header.is_some() {
    base.dts_header = other.dts_header;
  }
  if other.dts_header_file.is_some() {
    base.dts_header_file = other.dts_header_file;
  }
  if other.typegen_bin.is_some() {
    base.typegen_bin = other.typegen_bin;
  }
}
