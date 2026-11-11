use crate::{Result, hash, write_json};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const DIR: &str = "SPEC_CHECK_CAPTURE";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Invocation {
    pub cwd: PathBuf,
    pub rustc: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub unsupported_env: Vec<String>,
    #[serde(default)]
    pub env_hashes: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub env_origins: BTreeMap<String, String>,
    #[serde(default)]
    pub removed_env: Vec<String>,
}
fn retained(key: &str) -> bool {
    key.trim_start().starts_with("__VERUS_DRIVER")
        || [
            "CARGO_PKG_",
            "CARGO_CFG_",
            "CARGO_FEATURE_",
            "CARGO_BIN_EXE_",
            "DEP_",
        ]
        .iter()
        .any(|p| key.starts_with(p))
        || matches!(
            key,
            "CARGO"
                | "CARGO_CRATE_NAME"
                | "CARGO_MANIFEST_DIR"
                | "CARGO_MANIFEST_PATH"
                | "CARGO_PRIMARY_PACKAGE"
                | "CARGO_BIN_NAME"
                | "OUT_DIR"
                | "HOST"
                | "TARGET"
                | "OPT_LEVEL"
                | "DEBUG"
                | "PROFILE"
                | "PATH"
                | "LD_LIBRARY_PATH"
                | "RUSTC"
                | "RUSTDOC"
                | "RUSTC_BOOTSTRAP"
                | "RUSTFLAGS"
                | "CARGO_ENCODED_RUSTFLAGS"
                | "VERUS_USE_RUSTUP"
                | "VERUS_Z3_PATH"
                | "CARGO_UNSTABLE_CHECKSUM_FRESHNESS"
                | "__CARGO_DEFAULT_LIB_METADATA"
                | "PWD"
                | "OLDPWD"
                | "SSL_CERT_DIR"
                | "SSL_CERT_FILE"
                | "CARGO_TARGET_TMPDIR"
        )
}
fn ignored(key: &str) -> bool {
    matches!(
        key,
        "SPEC_CHECK_CAPTURE"
            | "SPEC_CHECK_CARGO"
            | "SPEC_CHECK_PACKAGE"
            | "SPEC_CHECK_UPSTREAM"
            | "SPEC_CHECK_CAPTURE_ALL"
            | "SPEC_CHECK_PROJECT_LOG"
            | "SPEC_CHECK_CFGS"
            | "RUSTC_WRAPPER"
            | "CARGO_MAKEFLAGS"
            | "MAKEFLAGS"
            | "MFLAGS"
            | "_"
            | "SHLVL"
    )
}
pub(crate) fn configured_args(
    args: &[String],
    cfgs: &[String],
    selected_package: bool,
) -> Result<Vec<String>> {
    for cfg in cfgs {
        let mut chars = cfg.chars();
        if !chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            || cfg.starts_with("verus")
            || cfg == "test"
        {
            return Err("project cfgs must be plain identifiers; test uses the tests profile and Verus cfgs come from the driver".into());
        }
    }
    let mut configured = args.to_vec();
    if selected_package {
        for cfg in cfgs {
            if !configured
                .windows(2)
                .any(|pair| pair[0] == "--cfg" && pair[1] == *cfg)
            {
                configured.extend(["--cfg".into(), cfg.clone()]);
            }
        }
    }
    Ok(configured)
}

pub fn wrapper(args: &[String]) -> Result<i32> {
    let dir = PathBuf::from(env::var(DIR)?);
    let is_rustc = args.first().is_some_and(|s| {
        Path::new(s)
            .file_name()
            .is_some_and(|s| s.to_string_lossy().contains("rustc"))
    });
    if !is_rustc {
        let cargo = env::var("SPEC_CHECK_CARGO")?;
        let mut command = Command::new(&cargo);
        command
            .args(args)
            .env("CARGO", &cargo)
            .env("RUSTC_WRAPPER", env::current_exe()?);
        command.env(
            "SPEC_CHECK_UPSTREAM",
            env::var("RUSTC_WRAPPER").unwrap_or_default(),
        );
        return Ok(command.status()?.code().unwrap_or(1));
    }
    let selected_package = env::var("CARGO_PKG_NAME").ok() == env::var("SPEC_CHECK_PACKAGE").ok();
    let cfgs: Vec<String> =
        serde_json::from_str(&env::var("SPEC_CHECK_CFGS").unwrap_or_else(|_| "[]".into()))?;
    let configured = configured_args(args, &cfgs, selected_package)?;
    let args = configured.as_slice();
    let mut driver_log_override = None;
    if (args.iter().any(|s| s == "--test") || env::var_os("SPEC_CHECK_CAPTURE_ALL").is_some())
        && env::var("CARGO_PKG_NAME").ok() == env::var("SPEC_CHECK_PACKAGE").ok()
    {
        let parent: BTreeMap<String, String> =
            serde_json::from_slice(&fs::read(dir.join("parent-env.json"))?)?;
        let mut values = BTreeMap::new();
        let mut unsupported = Vec::new();
        let mut env_hashes = BTreeMap::new();
        let mut removed_env = Vec::new();
        for (key, value) in env::vars() {
            if retained(&key) {
                values.insert(key, value);
            } else if !ignored(&key) && parent.get(&key) != Some(&hash(value.as_bytes())?) {
                env_hashes.insert(key.clone(), Some(hash(value.as_bytes())?));
                unsupported.push(key);
            }
        }
        for key in parent.keys() {
            if retained(key) && env::var_os(key).is_none() {
                removed_env.push(key.clone());
            }
            if !retained(key) && !ignored(key) && env::var_os(key).is_none() {
                env_hashes.insert(key.clone(), None);
                unsupported.push(key.clone());
            }
        }
        if env::var_os("SPEC_CHECK_PROJECT_LOG").is_some() {
            let key = values
                .keys()
                .find(|k| k.trim() == "__VERUS_DRIVER_ARGS__")
                .cloned()
                .ok_or("project logging requires actual Verus driver environment")?;
            let logs = dir
                .parent()
                .ok_or("capture profile directory missing")?
                .join(format!("native-vir-{}", std::process::id()));
            let flags = format!(
                "{}__VERUS_DRIVER_ARGS_SEP__--log__VERUS_DRIVER_ARGS_SEP__vir__VERUS_DRIVER_ARGS_SEP__--log-dir__VERUS_DRIVER_ARGS_SEP__{}",
                values[&key],
                logs.display()
            );
            values.insert(key.clone(), flags.clone());
            driver_log_override = Some((key, flags));
        }
        let invocation = Invocation {
            cwd: env::current_dir()?,
            rustc: args[0].clone(),
            args: args[1..].to_vec(),
            env: values,
            unsupported_env: unsupported,
            env_hashes,
            env_origins: BTreeMap::new(),
            removed_env,
        };
        write_json(
            &dir.join(format!("invocation-{}.json", std::process::id())),
            &invocation,
        )?;
    }
    let upstream = env::var("SPEC_CHECK_UPSTREAM").unwrap_or_default();
    let mut command = Command::new(if upstream.is_empty() {
        &args[0]
    } else {
        &upstream
    });
    if let Some((key, value)) = driver_log_override {
        command.env(key, value);
    }
    command.args(if upstream.is_empty() {
        &args[1..]
    } else {
        args
    });
    Ok(command.status()?.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_cfgs_preserve_flags_and_dependency_configuration() {
        let args = vec!["rustc".into(), "--cfg".into(), "existing".into()];
        let cfgs = vec!["ktest".into()];
        assert_eq!(configured_args(&args, &cfgs, false).unwrap(), args);
        let selected = configured_args(&args, &cfgs, true).unwrap();
        assert_eq!(&selected[..args.len()], args.as_slice());
        assert_eq!(configured_args(&selected, &cfgs, true).unwrap(), selected);
        for invalid in [
            "verus_keep_ghost",
            "test",
            "ktest --no-verify",
            "feature=abc",
            "",
        ] {
            assert!(configured_args(&args, &[invalid.into()], true).is_err());
        }
    }
}
