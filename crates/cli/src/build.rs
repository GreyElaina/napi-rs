use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use cargo_metadata::Package;
use sha2::{Digest, Sha256};

use crate::config::{NapiConfig, read_napi_config};
use crate::shell;
use crate::js_binding::{create_cjs_binding, create_esm_binding};
use crate::target::{Target, get_target_linker, get_system_default_target, parse_triple, target_to_env_var};
use crate::typegen::{DEFAULT_TYPE_DEF_HEADER, process_type_def};

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, clap::Args)]
pub struct BuildOptions {
    /// Build target triple
    #[arg(short, long)]
    pub target: Option<String>,

    /// Working directory
    #[arg(long, default_value = ".")]
    pub cwd: PathBuf,

    /// Path to Cargo.toml
    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    /// Path to napi config JSON
    #[arg(short = 'c', long)]
    pub config_path: Option<PathBuf>,

    /// Path to package.json
    #[arg(long, default_value = "package.json")]
    pub package_json_path: PathBuf,

    /// Output directory for .node files
    #[arg(short, long)]
    pub output_dir: Option<PathBuf>,

    /// Add platform triple to binary name
    #[arg(long)]
    pub platform: bool,

    /// Custom package name in JS binding
    #[arg(long)]
    pub js_package_name: Option<String>,

    /// Disable const enum generation (const enums are enabled by default)
    #[arg(long)]
    pub no_const_enum: bool,

    /// Emit string enums as runtime enums
    #[arg(long)]
    pub runtime_string_enum: bool,

    /// Custom JS binding filename
    #[arg(long)]
    pub js: Option<String>,

    /// Skip JS binding generation
    #[arg(long)]
    pub no_js: bool,

    /// Custom type-def filename
    #[arg(long)]
    pub dts: Option<String>,

    /// Custom header for .d.ts files
    #[arg(long)]
    pub dts_header: Option<String>,

    /// Skip default header
    #[arg(long)]
    pub no_dts_header: bool,

    /// Disable DTS caching (caching is enabled by default)
    #[arg(long)]
    pub no_dts_cache: bool,

    /// Generate ESM binding instead of CJS
    #[arg(long)]
    pub esm: bool,

    /// Strip symbols
    #[arg(short, long)]
    pub strip: bool,

    /// Build in release mode
    #[arg(long)]
    pub release: bool,

    /// Verbose output
    #[arg(long)]
    pub verbose: bool,

    /// Cargo package to build
    #[arg(short, long)]
    pub package: Option<String>,

    /// Binary name (for bin targets)
    #[arg(long)]
    pub bin: Option<String>,

    /// Cargo features
    #[arg(long)]
    pub features: Vec<String>,

    /// Enable all features
    #[arg(long)]
    pub all_features: bool,

    /// Disable default features
    #[arg(long)]
    pub no_default_features: bool,

    /// Build profile
    #[arg(long)]
    pub profile: Option<String>,

    /// Target directory
    #[arg(long)]
    pub target_dir: Option<PathBuf>,

    /// Cross-compile with zigbuild/xwin
    #[arg(long)]
    pub cross_compile: bool,

    /// Use `cross` crate
    #[arg(long)]
    pub use_cross: bool,

    /// Pipe output to command (e.g. prettier)
    #[arg(long)]
    pub pipe: Option<String>,

    /// Extra cargo args (passed after --)
    #[arg(last = true)]
    pub cargo_options: Vec<String>,
}

pub fn run_build(options: BuildOptions) -> anyhow::Result<Vec<OutputFile>> {
    let cwd = fs::canonicalize(&options.cwd)
        .with_context(|| format!("Cannot resolve cwd: {}", options.cwd.display()))?;

    let manifest_path = fs::canonicalize(
        options
            .manifest_path
            .as_deref()
            .unwrap_or(&cwd.join("Cargo.toml")),
    )
    .context("Cannot resolve manifest path")?;

    let metadata = cargo_metadata::MetadataCommand::new()
        .manifest_path(&manifest_path)
        .exec()
        .context("Failed to run `cargo metadata`")?;

    let crate_pkg = if let Some(ref pkg_name) = options.package {
        metadata
            .packages
            .iter()
            .find(|p| p.name.as_ref() == pkg_name.as_str())
    } else {
        metadata
            .packages
            .iter()
            .find(|p| p.manifest_path.as_std_path() == manifest_path)
    }
    .context(
        "Unable to find crate to build. \
         If in a workspace, try `--package`.",
    )?;

    let pkg_json_path = cwd.join(&options.package_json_path);
    let config = read_napi_config(
        &pkg_json_path,
        options.config_path.as_ref().map(|p| cwd.join(p)).as_deref(),
    )?;

    let mut builder = Builder::new(&metadata, crate_pkg, config, &options, &cwd)?;
    builder.build()
}

struct Builder<'a> {
    args: Vec<String>,
    envs: HashMap<String, String>,
    target: Target,
    output_dir: PathBuf,
    target_dir: PathBuf,
    enable_type_def: bool,
    crate_pkg: &'a Package,
    config: NapiConfig,
    options: &'a BuildOptions,
    cwd: &'a Path,
    metadata: &'a cargo_metadata::Metadata,
}

impl<'a> Builder<'a> {
    fn new(
        metadata: &'a cargo_metadata::Metadata,
        crate_pkg: &'a Package,
        config: NapiConfig,
        options: &'a BuildOptions,
        cwd: &'a Path,
    ) -> anyhow::Result<Self> {
        let target = if let Some(ref t) = options.target {
            parse_triple(t)?
        } else if let Ok(t) = env::var("CARGO_BUILD_TARGET") {
            parse_triple(&t)?
        } else {
            get_system_default_target()?
        };

        let crate_dir: PathBuf = crate_pkg.manifest_path.parent().unwrap().into();
        let output_dir = options
            .output_dir
            .as_ref()
            .map(|p| cwd.join(p))
            .unwrap_or_else(|| crate_dir.clone());
        let target_dir = options
            .target_dir
            .clone()
            .or_else(|| env::var("CARGO_BUILD_TARGET_DIR").ok().map(PathBuf::from))
            .unwrap_or_else(|| metadata.target_directory.clone().into());

        let enable_type_def = crate_pkg.dependencies.iter().any(|dep| {
            dep.name == "napi-derive"
                && (dep.uses_default_features || dep.features.contains(&"type-def".to_string()))
        });

        if !enable_type_def {
            shell::warn(
                "`napi-derive` crate is not used or `type-def` feature is not enabled, \
                 skipping type definition generation",
            );
        }

        Ok(Self {
            args: Vec::new(),
            envs: HashMap::new(),
            target,
            output_dir,
            target_dir,
            enable_type_def,
            crate_pkg,
            config,
            options,
            cwd,
            metadata,
        })
    }

    fn cdylib_name(&self) -> Option<String> {
        self.crate_pkg
            .targets
            .iter()
            .find(|t| t.crate_types.contains(&cargo_metadata::CrateType::CDyLib))
            .map(|t| t.name.clone())
    }

    fn bin_name(&self) -> Option<String> {
        self.options.bin.clone().or_else(|| {
            if self.cdylib_name().is_some() {
                None
            } else {
                self.crate_pkg
                    .targets
                    .iter()
                    .find(|t| t.crate_types.contains(&cargo_metadata::CrateType::Bin))
                    .map(|t| t.name.clone())
            }
        })
    }

    fn build(&mut self) -> anyhow::Result<Vec<OutputFile>> {
        let cdylib = self.cdylib_name();
        if cdylib.is_none() {
            let msg = "Missing `crate-type = [\"cdylib\"]` in [lib] config. \
                       The build result will not be available as node addon.";
            if self.bin_name().is_some() {
                shell::warn(msg);
            } else {
                bail!("{msg}");
            }
        }

        self.pick_binary();
        self.set_package();
        self.set_features()?;
        self.set_target();
        self.set_envs();
        self.set_bypass_args();
        self.exec()?;
        self.post_build()
    }

    fn pick_binary(&mut self) {
        if self.options.cross_compile {
            if self.target.platform == "win32" {
                self.args.extend(["xwin".into(), "build".into()]);
                if self.target.arch == "ia32" {
                    self.envs.insert("XWIN_ARCH".into(), "x86".into());
                }
                return;
            }
            self.args.push("zigbuild".into());
            return;
        }

        self.args.push("build".into());
    }

    fn set_package(&mut self) {
        if let Some(ref pkg) = self.options.package {
            self.args.extend(["--package".into(), pkg.clone()]);
        }
        if let Some(ref bin) = self.bin_name() {
            self.args.extend(["--bin".into(), bin.clone()]);
        }
    }

    fn set_target(&mut self) {
        self.args
            .extend(["--target".into(), self.target.triple.clone()]);
    }

    fn set_features(&mut self) -> anyhow::Result<()> {
        if self.options.all_features && self.options.no_default_features {
            bail!("Cannot specify --all-features and --no-default-features together");
        }
        if self.options.all_features {
            self.args.push("--all-features".into());
        } else if self.options.no_default_features {
            self.args.push("--no-default-features".into());
        }
        if !self.options.features.is_empty() {
            self.args.push("--features".into());
            self.args.extend(self.options.features.clone());
        }
        Ok(())
    }

    fn set_envs(&mut self) {
        if self.enable_type_def {
            let folder = self.generate_intermediate_type_def_folder();
            self.set_force_build_envs(&folder);
            self.envs
                .insert("NAPI_TYPE_DEF_TMP_FOLDER".into(), folder);
        }

        // RUSTFLAGS
        let mut rustflags = env::var("RUSTFLAGS")
            .or_else(|_| env::var("CARGO_BUILD_RUSTFLAGS"))
            .unwrap_or_default();

        if self
            .target
            .abi
            .as_deref()
            .is_some_and(|a| a.contains("musl"))
            && !rustflags.contains("target-feature=-crt-static")
        {
            rustflags.push_str(" -C target-feature=-crt-static");
        }

        if self.options.strip && !rustflags.contains("link-arg=-s") {
            rustflags.push_str(" -C link-arg=-s");
        }

        if !rustflags.is_empty() {
            self.envs.insert("RUSTFLAGS".into(), rustflags);
        }

        // LINKER
        if !self.options.cross_compile {
            if let Some(linker) = get_target_linker(&self.target.triple) {
                let env_key = format!(
                    "CARGO_TARGET_{}_LINKER",
                    target_to_env_var(&self.target.triple)
                );
                if env::var(&env_key).is_err() && !self.envs.contains_key(&env_key) {
                    self.envs.insert(env_key, linker.to_string());
                }
            }
        }
    }

    fn set_force_build_envs(&mut self, type_def_folder: &str) {
        for pkg in &self.metadata.packages {
            if pkg
                .dependencies
                .iter()
                .any(|d| d.name == "napi-derive")
                && !Path::new(type_def_folder).join(pkg.name.as_ref()).exists()
            {
                let env_key = format!(
                    "NAPI_FORCE_BUILD_{}",
                    pkg.name.replace('-', "_").to_uppercase()
                );
                self.envs
                    .insert(env_key, chrono_now_millis().to_string());
            }
        }
    }

    fn set_bypass_args(&mut self) {
        if self.options.release {
            self.args.push("--release".into());
        }
        if self.options.verbose {
            self.args.push("--verbose".into());
        }
        if let Some(ref dir) = self.options.target_dir {
            self.args
                .extend(["--target-dir".into(), dir.display().to_string()]);
        }
        if let Some(ref profile) = self.options.profile {
            self.args.extend(["--profile".into(), profile.clone()]);
        }
        if self.options.manifest_path.is_some() {
            self.args.extend([
                "--manifest-path".into(),
                self.crate_pkg.manifest_path.to_string(),
            ]);
        }
        if !self.options.cargo_options.is_empty() {
            self.args.extend(self.options.cargo_options.clone());
        }
    }

    fn generate_intermediate_type_def_folder(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.crate_pkg.manifest_path.as_str().as_bytes());
        hasher.update(CLI_VERSION.as_bytes());
        let hash = hex::encode(hasher.finalize());
        let hash_short = &hash[..8];

        let folder = self
            .target_dir
            .join("napi-rs")
            .join(format!("{}-{hash_short}", self.crate_pkg.name));

        let mut folder_str = folder.display().to_string();

        if self.options.no_dts_cache {
            let _ = fs::remove_dir_all(&folder);
            folder_str.push_str(&format!("_{}", chrono_now_millis()));
        }

        fs::create_dir_all(&folder_str).ok();
        folder_str
    }

    fn exec(&self) -> anyhow::Result<()> {
        let cargo = env::var("CARGO").unwrap_or_else(|_| {
            if self.options.use_cross {
                "cross".into()
            } else {
                "cargo".into()
            }
        });

        if self.options.use_cross && self.options.cross_compile {
            bail!("`--use-cross` and `--cross-compile` cannot be used together");
        }

        shell::status("Running", format_args!("`{cargo} {}`", self.args.join(" ")));

        let mut full_env: HashMap<String, String> = env::vars().collect();
        full_env.extend(self.envs.clone());

        let status = Command::new(&cargo)
            .args(&self.args)
            .envs(&full_env)
            .current_dir(self.cwd)
            .status()
            .with_context(|| format!("Failed to execute `{cargo}`"))?;

        if !status.success() {
            bail!("Build failed with exit code {}", status.code().unwrap_or(-1));
        }

        Ok(())
    }

    fn post_build(&self) -> anyhow::Result<Vec<OutputFile>> {
        fs::create_dir_all(&self.output_dir)
            .with_context(|| format!("Failed to create output dir {}", self.output_dir.display()))?;

        let mut outputs = Vec::new();

        self.copy_artifact(&mut outputs)?;

        if self.cdylib_name().is_some() {
            let idents = self.generate_type_def(&mut outputs)?;
            self.write_js_binding(&idents, &mut outputs)?;
        }

        Ok(outputs)
    }

    fn copy_artifact(&self, outputs: &mut Vec<OutputFile>) -> anyhow::Result<()> {
        let (src_name, dest_name) = match self.get_artifact_names() {
            Some(names) => names,
            None => return Ok(()),
        };

        let profile = self
            .options
            .profile
            .as_deref()
            .unwrap_or(if self.options.release { "release" } else { "debug" });

        let src = self
            .target_dir
            .join(&self.target.triple)
            .join(profile)
            .join(&src_name);
        let dest = self.output_dir.join(&dest_name);

        if dest.exists() {
            fs::remove_file(&dest).ok();
        }

        fs::copy(&src, &dest).with_context(|| {
            format!(
                "Failed to copy artifact from {} to {}",
                src.display(),
                dest.display()
            )
        })?;

        let kind = if dest_name.ends_with(".node") {
            OutputKind::Node
        } else {
            OutputKind::Exe
        };
        outputs.push(OutputFile {
            kind,
            path: dest,
        });

        Ok(())
    }

    fn get_artifact_names(&self) -> Option<(String, String)> {
        if let Some(ref cdylib) = self.cdylib_name() {
            let cdylib_snake = cdylib.replace('-', "_");
            let src_name = match self.target.platform.as_str() {
                "darwin" => format!("lib{cdylib_snake}.dylib"),
                "win32" => format!("{cdylib_snake}.dll"),
                _ => format!("lib{cdylib_snake}.so"),
            };

            let mut dest_name = self.config.binary_name.clone();
            if self.options.platform {
                dest_name.push('.');
                dest_name.push_str(&self.target.platform_arch_abi);
            }
            dest_name.push_str(".node");

            Some((src_name, dest_name))
        } else {
            let bin_name = self.bin_name()?;
            let src_name = if self.target.platform == "win32" {
                format!("{bin_name}.exe")
            } else {
                bin_name.clone()
            };
            Some((src_name.clone(), src_name))
        }
    }

    fn generate_type_def(
        &self,
        outputs: &mut Vec<OutputFile>,
    ) -> anyhow::Result<Vec<String>> {
        if !self.enable_type_def {
            return Ok(Vec::new());
        }

        let type_def_dir = match self.envs.get("NAPI_TYPE_DEF_TMP_FOLDER") {
            Some(d) => d.clone(),
            None => return Ok(Vec::new()),
        };

        let dir_path = Path::new(&type_def_dir);
        if !dir_path.exists() {
            return Ok(Vec::new());
        }

        let mut header = String::new();
        if !self.options.no_dts_header {
            let dts_header = self
                .options
                .dts_header
                .as_deref()
                .or(self.config.dts_header.as_deref());
            if let Some(ref header_file) = self.config.dts_header_file {
                match fs::read_to_string(self.cwd.join(header_file)) {
                    Ok(content) => header = content,
                    Err(e) => shell::warn(format_args!("failed to read dts header file {header_file}: {e}")),
                }
            } else if let Some(h) = dts_header {
                header = h.to_string();
            } else {
                header = DEFAULT_TYPE_DEF_HEADER.to_string();
            }
        }

        let mut entries: Vec<_> = fs::read_dir(dir_path)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .collect();
        entries.sort_by_key(|e| e.file_name());

        if entries.is_empty() {
            return Ok(Vec::new());
        }

        let const_enum = !self.options.no_const_enum;
        let runtime_string_enum = self.options.runtime_string_enum;

        let mut all_dts = String::new();
        let mut all_exports = Vec::new();

        for entry in &entries {
            let result = process_type_def(&entry.path(), const_enum, runtime_string_enum)?;
            all_dts.push_str(&result.dts);
            all_exports.extend(result.exports);
        }

        if all_dts.contains("ExternalObject<") {
            header.push_str(
                "\nexport declare class ExternalObject<T> {\n\
                 \x20 readonly '': {\n\
                 \x20   readonly '': unique symbol\n\
                 \x20   [K: symbol]: T\n\
                 \x20 }\n\
                 }\n",
            );
        }

        if all_dts.contains("TypedArray") {
            header.push_str(
                "\nexport type TypedArray = Int8Array | Uint8Array | Uint8ClampedArray | Int16Array | Uint16Array | Int32Array | Uint32Array | Float32Array | Float64Array | BigInt64Array | BigUint64Array\n",
            );
        }

        let full_dts = format!("{header}{all_dts}");
        let dest_name = self.options.dts.as_deref().unwrap_or("index.d.ts");
        let dest = self.output_dir.join(dest_name);
        fs::write(&dest, &full_dts)
            .with_context(|| format!("Failed to write type def to {}", dest.display()))?;

        if let Some(ref pipe_cmd) = self.options.pipe {
            pipe_through_command(pipe_cmd, &dest)?;
        }

        if !all_exports.is_empty() {
            outputs.push(OutputFile {
                kind: OutputKind::Dts,
                path: dest,
            });
        }

        Ok(all_exports)
    }

    fn write_js_binding(
        &self,
        idents: &[String],
        outputs: &mut Vec<OutputFile>,
    ) -> anyhow::Result<()> {
        if !self.options.platform || self.options.no_js || idents.is_empty() {
            return Ok(());
        }

        let name = self.options.js.as_deref().unwrap_or("index.js");
        let pkg_name = self
            .options
            .js_package_name
            .as_deref()
            .unwrap_or(&self.config.package_name);
        let version = env::var("npm_new_version")
            .ok()
            .or_else(|| Some(self.config.package_json.version.clone()));

        let binding = if self.options.esm {
            create_esm_binding(
                &self.config.binary_name,
                pkg_name,
                idents,
                version.as_deref(),
            )
        } else {
            create_cjs_binding(
                &self.config.binary_name,
                pkg_name,
                idents,
                version.as_deref(),
            )
        };

        let dest = self.output_dir.join(name);
        fs::write(&dest, &binding)
            .with_context(|| format!("Failed to write js binding to {}", dest.display()))?;

        if let Some(ref pipe_cmd) = self.options.pipe {
            pipe_through_command(pipe_cmd, &dest)?;
        }

        outputs.push(OutputFile {
            kind: OutputKind::Js,
            path: dest,
        });

        Ok(())
    }
}

#[derive(Debug)]
pub enum OutputKind {
    Js,
    Dts,
    Node,
    Exe,
}

impl OutputKind {
    pub fn label(&self) -> &'static str {
        match self {
            OutputKind::Node | OutputKind::Exe => "Copied",
            OutputKind::Dts | OutputKind::Js => "Generated",
        }
    }
}

#[derive(Debug)]
pub struct OutputFile {
    pub kind: OutputKind,
    pub path: PathBuf,
}

fn chrono_now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

fn pipe_through_command(cmd: &str, file: &Path) -> anyhow::Result<()> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let (program, args) = parts
        .split_first()
        .context("Empty pipe command")?;

    let status = Command::new(program)
        .args(args)
        .arg(file)
        .status()
        .with_context(|| format!("Failed to run pipe command: {cmd}"))?;

    if !status.success() {
        shell::warn(format_args!(
            "pipe command `{cmd}` exited with code {}",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}
