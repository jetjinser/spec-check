//! Shared support items used across engine modules and the CLI.
//!
//! Ported verbatim from the research lineage `main.rs` so that module
//! `crate::` paths resolve unchanged.

use serde::Serialize;
use serde_json::json;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

pub fn hash(bytes: &[u8]) -> Result<String> {
    let mut p = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    p.stdin
        .take()
        .ok_or("missing hash stdin")?
        .write_all(bytes)?;
    let r = p.wait_with_output()?;
    if !r.status.success() {
        return Err("sha256sum failed".into());
    }
    Ok(String::from_utf8(r.stdout)?[..64].to_owned())
}

pub fn execute(command: &mut Command, out: &Path, name: &str) -> Result<std::process::Output> {
    write_json(
        &out.join(format!("{name}.command.json")),
        &json!({"program":command.get_program().to_string_lossy(),"args":command.get_args().map(|s|s.to_string_lossy()).collect::<Vec<_>>(),"cwd":command.get_current_dir()}),
    )?;
    let r = command.output()?;
    fs::write(out.join(format!("{name}.stdout")), &r.stdout)?;
    fs::write(out.join(format!("{name}.stderr")), &r.stderr)?;
    Ok(r)
}

