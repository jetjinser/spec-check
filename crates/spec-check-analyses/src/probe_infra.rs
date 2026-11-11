//! Shared probe-injection infrastructure for target-workspace analyses
//! (vacuity construction probes, divergence obligation probes).
//!
//! Pure mechanics only: workspace resolution, snapshot/restore of injected
//! files with replay copies, cargo-verus command construction with forwarded
//! cfgs, and the rustc/verus log model (span events grouped by `Compiling`
//! sections). No probe semantics live here — every analysis owns its own
//! manifest types, generators, and classifiers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Source utilities
// ---------------------------------------------------------------------------

pub(crate) fn sanitize(name: &str) -> String {
    let raw: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    // Identifiers must not start with a digit.
    match raw.chars().next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => raw,
        _ => format!("p_{raw}"),
    }
}

pub(crate) fn param_names(params: &str) -> Vec<String> {
    params
        .split(',')
        .filter_map(|p| p.split(':').next())
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

pub(crate) fn indent_block(src: &str) -> String {
    src.lines()
        .map(|l| if l.trim().is_empty() { String::new() } else { format!("    {l}") })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Region of a synthesized fn: from its `pub fn ...` declaration line to
/// the next line that is exactly `}` (convention: synthesized fn bodies
/// are indented and contain no column-0 closing braces).
pub(crate) fn fn_region(lines: &[&str], decl_prefix: &str) -> Option<(usize, usize)> {
    let start = lines
        .iter()
        .position(|l| l.starts_with(decl_prefix))?
        + 1;
    for (off, l) in lines[start..].iter().enumerate() {
        if l.trim() == "}" {
            return Some((start, start + off + 1));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Target workspace
// ---------------------------------------------------------------------------

/// Probe target workspace (manifest v1.1, shared by construction and
/// obligation probes).
#[derive(Debug, Deserialize)]
pub struct TargetWorkspace {
    /// Absolute path of the target workspace manifest (`Cargo.toml`).
    pub manifest_path: PathBuf,
    /// Probe-module injection directory, relative to the workspace root;
    /// its first path component names the crate (e.g. `ostd/src/probes`).
    pub inject_dir: PathBuf,
    /// cfg gate carried by the injected module declarations.
    pub gate_cfg: String,
    /// Additional cfgs forwarded to both build phases (target-side
    /// exclusion gates; the injected module itself is gated only by
    /// `gate_cfg`).
    #[serde(default)]
    pub extra_cfgs: Vec<String>,
    /// Target triple for the target-workspace builds (crates with
    /// target-specific dependencies do not build for the host).
    #[serde(default)]
    pub target: Option<String>,
}

/// One generated per-entry probe module plus its synthesized-fn line
/// regions (1-based, inclusive) used for log span attribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeModule {
    pub safe_name: String,
    pub index: usize,
    pub source: String,
    pub entry_region: (usize, usize),
    /// Optional second synthesized fn region (vacuity holder-face control;
    /// unused by divergence).
    pub live_region: Option<(usize, usize)>,
}

/// The full injection set for a manifest's target-workspace entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeInjection {
    /// Module name of the injection directory (file_name of inject_dir).
    pub module_name: String,
    /// Declaration appended to the crate root file.
    pub lib_decl: String,
    /// Content of `inject_dir/mod.rs` (wiring + cfg gates).
    pub mod_rs: String,
    pub modules: Vec<ProbeModule>,
}

/// Resolved paths for an injection into a target workspace.
pub(crate) struct InjectionTarget {
    pub workspace_root: PathBuf,
    pub crate_name: String,
    pub root_file: PathBuf,
    pub inject_dir_abs: PathBuf,
}

pub(crate) fn resolve_injection_target(tw: &TargetWorkspace) -> Result<InjectionTarget, String> {
    let workspace_root = tw
        .manifest_path
        .parent()
        .ok_or("target_workspace.manifest_path has no parent")?
        .to_path_buf();
    let crate_name = tw
        .inject_dir
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .filter(|s| s != "src")
        .ok_or("target_workspace.inject_dir must start with the crate directory")?;
    let crate_dir = workspace_root.join(&crate_name);
    let root_file = ["src/lib.rs", "src/main.rs"]
        .iter()
        .map(|rel| crate_dir.join(rel))
        .find(|p| p.is_file())
        .ok_or("target crate root file (src/lib.rs or src/main.rs) not found")?;
    let inject_dir_abs = workspace_root.join(&tw.inject_dir);
    Ok(InjectionTarget {
        workspace_root,
        crate_name,
        root_file,
        inject_dir_abs,
    })
}

// ---------------------------------------------------------------------------
// Injection install (replay copies + snapshot/restore guard)
// ---------------------------------------------------------------------------

/// Snapshot/restore bookkeeping for the injection. Drop restores the
/// workspace to its original state on every exit path.
pub(crate) struct InjectionGuard {
    /// Files we overwrote: (path, original bytes; None = did not exist).
    pub modified: Vec<(PathBuf, Option<Vec<u8>>)>,
    /// Directory we created wholesale (removed recursively on restore).
    pub created_dir: Option<PathBuf>,
}

impl Drop for InjectionGuard {
    fn drop(&mut self) {
        for (path, orig) in &self.modified {
            match orig {
                Some(bytes) => {
                    let _ = std::fs::write(path, bytes);
                },
                None => {
                    let _ = std::fs::remove_file(path);
                },
            }
        }
        if let Some(dir) = &self.created_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

pub(crate) fn snapshot(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Install an injection set into the target workspace: wire the crate root,
/// write `mod.rs` and the probe modules, and keep replay copies under
/// `<out>/injections/`. Returns the guard that restores the workspace on
/// drop.
pub(crate) fn write_injection(
    tw: &TargetWorkspace,
    out: &Path,
    injection: &ProbeInjection,
    tool_label: &str,
) -> Result<(InjectionGuard, InjectionTarget), String> {
    let target = resolve_injection_target(tw)?;

    // Replay copies under out/injections/.
    let replay = out.join("injections");
    std::fs::create_dir_all(&replay).map_err(|e| e.to_string())?;

    let dir_existed = target.inject_dir_abs.is_dir();
    let mut guard = InjectionGuard {
        modified: Vec::new(),
        created_dir: if dir_existed { None } else { Some(target.inject_dir_abs.clone()) },
    };

    // Crate root wiring: append the gated module declaration.
    let mut lib_injected = std::fs::read_to_string(&target.root_file).map_err(|e| e.to_string())?;
    if !lib_injected.ends_with('\n') {
        lib_injected.push('\n');
    }
    lib_injected.push_str(&format!(
        "\n// injected by spec-check {tool_label} — replay copy in the analysis out dir\n{}\n",
        injection.lib_decl
    ));
    let lib_orig = snapshot(&target.root_file)?;
    std::fs::write(&target.root_file, &lib_injected).map_err(|e| e.to_string())?;
    std::fs::write(replay.join("lib.rs.orig"), lib_orig.clone().unwrap_or_default())
        .map_err(|e| e.to_string())?;
    guard.modified.push((target.root_file.clone(), lib_orig));
    std::fs::write(replay.join("lib.rs.injected"), &lib_injected).map_err(|e| e.to_string())?;

    // Injection directory contents.
    let mod_rs_path = target.inject_dir_abs.join("mod.rs");
    let mod_rs_orig = snapshot(&mod_rs_path)?;
    std::fs::create_dir_all(&target.inject_dir_abs).map_err(|e| e.to_string())?;
    std::fs::write(&mod_rs_path, &injection.mod_rs).map_err(|e| e.to_string())?;
    guard.modified.push((mod_rs_path.clone(), mod_rs_orig));
    std::fs::write(replay.join("mod.rs"), &injection.mod_rs).map_err(|e| e.to_string())?;
    let mut created_records = vec![(
        mod_rs_path.to_string_lossy().to_string(),
        "mod.rs".to_string(),
    )];
    for m in &injection.modules {
        let path = target.inject_dir_abs.join(format!("{}.rs", m.safe_name));
        let orig = snapshot(&path)?;
        std::fs::write(&path, &m.source).map_err(|e| e.to_string())?;
        guard.modified.push((path.clone(), orig));
        let replay_name = format!("{}.rs", m.safe_name);
        std::fs::write(replay.join(&replay_name), &m.source).map_err(|e| e.to_string())?;
        created_records.push((path.to_string_lossy().to_string(), replay_name));
    }

    // replay.json: structured record for manual replay/audit.
    let manifest_path_json = tw.manifest_path.display().to_string();
    let records = created_records
        .iter()
        .map(|(p, r)| format!("{{\"path\": {p:?}, \"replay_copy\": {r:?}}}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut modified_records = vec![format!(
        "{{\"path\": {:?}, \"orig\": \"lib.rs.orig\", \"injected\": \"lib.rs.injected\"}}",
        target.root_file.display().to_string()
    )];
    if !dir_existed {
        modified_records.push(format!(
            "{{\"directory_created\": {:?}}}",
            target.inject_dir_abs.display().to_string()
        ));
    }
    let replay_json = format!(
        "{{\n  \"workspace_root\": {root:?},\n  \"manifest_path\": {mp:?},\n  \"created\": [{created}],\n  \"modified\": [{modified}],\n  \"restore\": \"write *.orig bytes back over each modified path; delete each created path (and the created directory)\"\n}}\n",
        root = target.workspace_root.display().to_string(),
        mp = manifest_path_json,
        created = records,
        modified = modified_records.join(", "),
    );
    std::fs::write(replay.join("replay.json"), replay_json).map_err(|e| e.to_string())?;

    Ok((guard, target))
}

// ---------------------------------------------------------------------------
// cargo-verus commands
// ---------------------------------------------------------------------------

/// Run one target-workspace phase with stdout/stderr interleaved into a
/// single log file (dup'd descriptors share the write offset, preserving
/// real-time ordering — required for `Compiling`-section attribution).
/// The log is truncated on open: one phase owns one log file, and a
/// reused output directory must not mix evidence across runs.
pub(crate) fn run_captured(
    cmd: &mut Command,
    log_path: &Path,
) -> Result<(std::process::ExitStatus, String), String> {
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(log_path)
        .map_err(|e| e.to_string())?;
    let err = file.try_clone().map_err(|e| e.to_string())?;
    cmd.stdout(file).stderr(err);
    let status = cmd.status().map_err(|e| {
        format!(
            "failed to execute {}: {e}",
            cmd.get_program().to_string_lossy()
        )
    })?;
    let mut log = std::fs::read_to_string(log_path).map_err(|e| e.to_string())?;
    log.push_str(&format!("\n[cargo-verus exit status: {status}]\n"));
    Ok((status, log))
}

pub(crate) fn verus_env(cmd: &mut Command, z3: &Path) {
    cmd.env("VERUS_USE_RUSTUP", "0")
        .env("VERUS_Z3_PATH", z3)
        .env("RUSTC_BOOTSTRAP", "1");
}

pub(crate) fn target_command(
    cargo_verus: &Path,
    phase: &str,
    workspace_root: &Path,
    crate_name: &str,
    target_dir: &Path,
    target: Option<&String>,
    verus_args: &[&str],
    z3: &Path,
) -> Command {
    let mut cmd = Command::new(cargo_verus);
    cmd.arg(phase);
    cmd.current_dir(workspace_root);
    cmd.arg("-p").arg(crate_name).arg("--offline");
    cmd.arg("--target-dir").arg(target_dir);
    if let Some(t) = target {
        cmd.arg("--target").arg(t);
    }
    cmd.arg("--lib");
    cmd.arg("--");
    for a in verus_args {
        cmd.arg(a);
    }
    verus_env(&mut cmd, z3);
    cmd
}

pub(crate) fn forwarded_cfgs(tw: &TargetWorkspace) -> Vec<String> {
    let mut args = vec!["--cfg".to_owned(), tw.gate_cfg.clone()];
    for cfg in &tw.extra_cfgs {
        args.push("--cfg".to_owned());
        args.push(cfg.clone());
    }
    args
}

/// Tail of a failed phase log (last 40 lines) for error reports.
pub(crate) fn log_tail(log: &str) -> String {
    log.lines()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .fold(String::new(), |acc, l| acc + l + "\n")
}

// ---------------------------------------------------------------------------
// Log model: rustc/verus diagnostics as span events, grouped by cargo
// `Compiling`/`Checking` sections, plus per-section verification summaries.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CMarker {
    Precondition,
    Postcondition,
    Assertion,
    Other,
}

#[derive(Debug, Clone)]
pub(crate) struct CEvent {
    pub marker: CMarker,
    pub file: String,
    pub line: usize,
}

pub(crate) fn marker_of(line: &str) -> CMarker {
    if line.contains("precondition not satisfied") {
        CMarker::Precondition
    } else if line.contains("postcondition not satisfied") {
        CMarker::Postcondition
    } else if line.contains("assertion failed") {
        CMarker::Assertion
    } else {
        CMarker::Other
    }
}

pub(crate) fn parse_target_log(
    log: &str,
) -> (Vec<CEvent>, Vec<(usize, String)>, HashMap<String, usize>) {
    let mut events = Vec::new();
    let mut summaries = Vec::new();
    let mut crate_sections: HashMap<String, usize> = HashMap::new();
    let mut section = 0usize;
    let mut pending: Option<CMarker> = None;
    for line in log.lines() {
        let trimmed_line = line.trim_start();
        if let Some(rest) = trimmed_line.strip_prefix("Compiling ").or_else(|| trimmed_line.strip_prefix("Checking ")) {
            section += 1;
            if let Some(name) = rest.split_whitespace().next() {
                crate_sections.entry(name.to_string()).or_insert(section);
            }
            pending = None;
            continue;
        }
        if let Some(text) = trimmed_line.strip_prefix("verification results::") {
            summaries.push((section, text.trim().to_string()));
            pending = None;
            continue;
        }
        if line.starts_with("warning") {
            pending = None;
            continue;
        }
        if line.starts_with("error:") || line.starts_with("error[") {
            pending = Some(marker_of(line));
            continue;
        }
        if let Some(rest) = line.trim_start().strip_prefix("--> ") {
            let mut parts = rest.split(':');
            let file = parts.next().unwrap_or("").trim().to_string();
            if let (Some(marker), Some(num)) = (
                pending.take(),
                parts.next().and_then(|s| s.trim().parse::<usize>().ok()),
            ) {
                events.push(CEvent { marker, file, line: num });
            }
            continue;
        }
    }
    (events, summaries, crate_sections)
}
