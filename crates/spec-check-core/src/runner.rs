use crate::{Result, capture::Invocation, execute, guidance, hash, ir, review, vir, write_json};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    env, fs,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
const SEP: &str = "__VERUS_DRIVER_ARGS_SEP__";
pub(crate) fn environment(command: &mut Command, inv: &Invocation) {
    command.envs(&inv.env);
    for key in &inv.removed_env {
        command.env_remove(key);
    }
}
pub(crate) fn execute_limited(
    command: &mut Command,
    out: &Path,
    name: &str,
    limit: Duration,
) -> Result<(std::process::Output, bool)> {
    write_json(
        &out.join(format!("{name}.command.json")),
        &json!({"program":command.get_program().to_string_lossy(),"args":command.get_args().map(|a|a.to_string_lossy()).collect::<Vec<_>>(),"cwd":command.get_current_dir(),"wall_seconds":limit.as_secs_f64(),"process_group":"isolated"}),
    )?;
    let stdout = out.join(format!("{name}.stdout"));
    let stderr = out.join(format!("{name}.stderr"));
    command
        .stdout(Stdio::from(fs::File::create(&stdout)?))
        .stderr(Stdio::from(fs::File::create(&stderr)?))
        .process_group(0);
    let mut child = command.spawn()?;
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= limit {
            unsafe extern "C" {
                fn kill(pid: i32, signal: i32) -> i32;
            }
            // The group is created solely for this verification and its solver children.
            let group = i32::try_from(child.id())?;
            if unsafe { kill(-group, 9) } != 0 {
                child.kill()?;
            }
            timed_out = true;
            break child.wait()?;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    write_json(
        &out.join(format!("{name}.wall.json")),
        &json!({"timed_out":timed_out,"seconds":start.elapsed().as_secs_f64()}),
    )?;
    Ok((
        std::process::Output {
            status,
            stdout: fs::read(stdout)?,
            stderr: fs::read(stderr)?,
        },
        timed_out,
    ))
}
pub(crate) fn compile(
    command: &mut Command,
    out: &Path,
    name: &str,
    original: &[String],
) -> Result<std::process::Output> {
    Ok(compile_with_limit(command, out, name, original, None)?.0)
}
fn compile_with_limit(
    command: &mut Command,
    out: &Path,
    name: &str,
    original: &[String],
    limit: Option<Duration>,
) -> Result<(std::process::Output, bool)> {
    let deadline = limit.map(|duration| Instant::now() + duration);
    let program = command.get_program().to_owned();
    let cwd = command.get_current_dir().map(Path::to_owned);
    let environment = command
        .get_envs()
        .map(|(k, v)| (k.to_owned(), v.map(std::ffi::OsStr::to_owned)))
        .collect::<Vec<_>>();
    let mut args = command
        .get_args()
        .map(std::ffi::OsStr::to_owned)
        .collect::<Vec<_>>();
    let feature = regex::Regex::new(r"the feature `([a-z_][a-z_0-9]*)` has already been enabled")?;
    let tool = regex::Regex::new(r"tool `([a-z_]+)` (?:was|is) already registered")?;
    for attempt in 0..16 {
        let mut command = Command::new(&program);
        command.args(&args);
        if let Some(cwd) = &cwd {
            command.current_dir(cwd);
        }
        for (k, v) in &environment {
            if let Some(v) = v {
                command.env(k, v);
            } else {
                command.env_remove(k);
            }
        }
        let label = format!("{name}-attempt-{attempt}");
        let (result, timed_out) = if let Some(deadline) = deadline {
            execute_limited(
                &mut command,
                out,
                &label,
                deadline.saturating_duration_since(Instant::now()),
            )?
        } else {
            (execute(&mut command, out, &label)?, false)
        };
        let redundant = String::from_utf8_lossy(&result.stderr)
            .lines()
            .filter_map(|s| serde_json::from_str::<Value>(s).ok())
            .filter_map(|d| {
                let m = d["message"].as_str()?;
                feature
                    .captures(m)
                    .map(|c| format!("-Zcrate-attr=feature({})", &c[1]))
                    .or_else(|| {
                        tool.captures(m)
                            .map(|c| format!("-Zcrate-attr=register_tool({})", &c[1]))
                    })
            })
            .filter_map(|flag| {
                let actual = args.iter().filter(|arg| *arg == flag.as_str()).count();
                let inherited = original.iter().filter(|arg| **arg == flag).count();
                if actual > inherited {
                    args.iter().rposition(|arg| arg == flag.as_str())
                } else {
                    None
                }
            })
            .collect::<std::collections::BTreeSet<_>>();
        if !redundant.is_empty() && !timed_out {
            for index in redundant.into_iter().rev() {
                args.remove(index);
            }
            continue;
        }
        for suffix in ["command.json", "stdout", "stderr"] {
            fs::copy(
                out.join(format!("{label}.{suffix}")),
                out.join(format!("{name}.{suffix}")),
            )?;
        }
        return Ok((result, timed_out));
    }
    Err("compiler adapter retry limit reached".into())
}
pub(crate) fn clean(
    args: &[String],
    out: &Path,
    source: Option<(&Path, &Path)>,
    cwd: &Path,
) -> Result<Vec<String>> {
    fs::create_dir_all(out)?;
    let mut result = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let s = &args[i];
        if ["--out-dir", "--emit"].contains(&s.as_str()) {
            i += 2;
            continue;
        }
        if s == "-C"
            && args
                .get(i + 1)
                .is_some_and(|s| s.starts_with("incremental="))
        {
            i += 2;
            continue;
        }
        if s.starts_with("--emit=") {
            i += 1;
            continue;
        }
        let replacement = source.and_then(|(from, to)| {
            let p = Path::new(s);
            let p = if p.is_absolute() {
                p.to_owned()
            } else {
                cwd.join(p)
            };
            if p == from {
                Some(to.to_string_lossy().into_owned())
            } else {
                None
            }
        });
        result.push(replacement.unwrap_or_else(|| s.clone()));
        i += 1;
    }
    result.extend([
        "--emit=metadata,dep-info".into(),
        "--out-dir".into(),
        out.to_string_lossy().into_owned(),
        "-Zcrate-attr=feature(proc_macro_hygiene)".into(),
    ]);
    Ok(result)
}
fn verifier(root: &Path, inv: &Invocation, args: &[String]) -> Command {
    let mut c = Command::new(root.join("verus"));
    c.current_dir(&inv.cwd).arg(&inv.rustc).args(args);
    environment(&mut c, inv);
    for (key, value) in &inv.env {
        if key.trim_start().starts_with("__VERUS_DRIVER_ARGS") {
            let tokens = value.split(SEP).skip(1).collect::<Vec<_>>();
            let mut kept = Vec::new();
            let mut i = 0;
            while i < tokens.len() {
                // Captured Cargo log destinations belong to immutable inputs.
                // This invocation receives its own log directory below.
                if matches!(tokens[i], "--log" | "--log-dir") {
                    i += 2;
                    continue;
                }
                if tokens[i] == "--no-verify" {
                    i += 1;
                    continue;
                }
                if tokens[i] == "--VIA-CARGO"
                    && tokens
                        .get(i + 1)
                        .is_some_and(|s| s.starts_with("compile-when-"))
                {
                    i += 2;
                    continue;
                }
                kept.push(tokens[i]);
                i += 1;
            }
            c.env(
                key,
                kept.into_iter()
                    .map(|s| format!("{SEP}{s}"))
                    .collect::<String>(),
            );
        }
    }
    c
}
fn diagnostics(output: &std::process::Output) -> Value {
    proof_summary(output.status.code(), &output.stdout, &output.stderr)
}
pub(crate) fn unsupported_message(message: &str) -> bool {
    [
        "not supported",
        "does not yet support",
        "does not currently support",
        "which is ignored",
    ]
    .iter()
    .any(|pattern| message.contains(pattern))
}

fn proof_summary(exit_code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> Value {
    let messages = String::from_utf8_lossy(stderr)
        .lines()
        .filter_map(|s| serde_json::from_str::<Value>(s).ok())
        .filter(|v| v["level"] == "error")
        .collect::<Vec<_>>();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    );
    let counts = regex::Regex::new(r"verification results:: (\d+) verified, (\d+) errors")
        .unwrap()
        .captures_iter(&combined)
        .last()
        .map(|c| vec![c[1].parse::<u64>().unwrap(), c[2].parse().unwrap()]);
    let unsupported = messages.iter().any(|v| {
        let m = v["message"].as_str().unwrap_or("");
        unsupported_message(m)
    });
    let unknown = messages.iter().any(|v| {
        let m = v["message"].as_str().unwrap_or("");
        m.contains("Resource limit") || m.to_lowercase().contains("timed out")
    });
    let tool_failure = exit_code != Some(0)
        && String::from_utf8_lossy(stderr)
            .lines()
            .any(|line| line.starts_with("thread '") && line.contains("panicked at "));
    let status = if tool_failure {
        "tool_failure"
    } else if unsupported {
        "unsupported"
    } else if unknown {
        "unknown"
    } else if exit_code == Some(0) && counts.as_ref().is_some_and(|c| c[1] == 0) {
        "proved"
    } else if counts.is_some() {
        "not_proved"
    } else {
        "not_analyzed"
    };
    json!({"exit_code":exit_code,"summary":counts,"status":status,"diagnostics":messages})
}
pub(crate) fn recorded_proof(dir: &Path, name: &str) -> Result<Value> {
    let value: Value = serde_json::from_slice(&fs::read(dir.join(format!("{name}.json")))?)?;
    let code = value["exit_code"].as_i64().map(i32::try_from).transpose()?;
    let rebuilt = proof_summary(
        code,
        &fs::read(dir.join(format!("{name}.stdout")))?,
        &fs::read(dir.join(format!("{name}.stderr")))?,
    );
    if rebuilt != value {
        return Err(format!("{name} proof report differs from its raw diagnostics").into());
    }
    Ok(value)
}
pub(crate) fn copy_package(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_symlink() {
            return Err("package symlinks require an explicit snapshot binding".into());
        }
        if path.is_dir() {
            copy_package(&path, &to.join(name))?
        } else {
            fs::copy(path, to.join(name))?;
        }
    }
    Ok(())
}
pub fn source_hashes(dir: &Path, hashes: &mut BTreeMap<PathBuf, String>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_name() == "target"
            || entry.file_name() == ".git"
            || entry.file_name() == "Cargo.lock"
        {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_symlink() {
            return Err("source fingerprinting does not follow package symlinks".into());
        }
        if path.is_dir() {
            source_hashes(&path, hashes)?
        } else {
            hashes.insert(path, hash(&fs::read(entry.path())?)?);
        }
    }
    Ok(())
}
pub(crate) fn depfiles(text: &str, cwd: &Path) -> Result<Vec<PathBuf>> {
    let text = text.replace("\\\n", " ");
    let line = text
        .lines()
        .find(|line| !line.starts_with('#') && line.contains(": "))
        .ok_or("missing Rustc dep-info rule")?;
    let (_, words) = line.split_once(": ").ok_or("invalid dep-info rule")?;
    let mut result = Vec::new();
    let mut token = String::new();
    let mut escaped = false;
    for c in words.chars().chain(std::iter::once(' ')) {
        if escaped {
            token.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c.is_whitespace() {
            if !token.is_empty() {
                result.push(cwd.join(token.replace("$$", "$")));
                token.clear();
            }
        } else {
            token.push(c);
        }
    }
    if escaped {
        return Err("truncated dep-info escape".into());
    }
    Ok(result)
}
pub(crate) fn runtime(
    executable: &str,
    function: &str,
    inv: &Invocation,
    pkg: &Path,
    out: &Path,
    index: usize,
) -> Result<Value> {
    let name = format!("runtime-{index}");
    let mut command = Command::new(executable);
    command
        .args(["--exact", function, "--nocapture"])
        .current_dir(pkg);
    environment(&mut command, inv);
    write_json(
        &out.join(format!("{name}.command.json")),
        &json!({"program":executable,"args":["--exact",function,"--nocapture"],"cwd":pkg}),
    )?;
    command
        .stdout(Stdio::from(fs::File::create(
            out.join(format!("{name}.stdout")),
        )?))
        .stderr(Stdio::from(fs::File::create(
            out.join(format!("{name}.stderr")),
        )?));
    let mut child = command.spawn()?;
    let start = Instant::now();
    let mut timeout = false;
    let code = loop {
        if let Some(status) = child.try_wait()? {
            break status.code();
        }
        if start.elapsed() > Duration::from_secs(20) {
            child.kill()?;
            child.wait()?;
            timeout = true;
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = fs::read_to_string(out.join(format!("{name}.stdout")))?;
    let status = if timeout {
        "timeout"
    } else if code == Some(0) && stdout.contains("1 passed") {
        "passed"
    } else if code == Some(0) && stdout.contains("1 ignored") {
        "ignored"
    } else {
        "failed"
    };
    Ok(json!({"status":status,"exit_code":code}))
}
fn extract(
    probe: &Path,
    inv: &Invocation,
    args: &[String],
    function: &str,
    out: &Path,
    name: &str,
) -> Result<Vec<ir::Record>> {
    extract_context(probe, inv, args, function, out, name, true)
}
pub(crate) fn extract_context(
    probe: &Path,
    inv: &Invocation,
    args: &[String],
    function: &str,
    out: &Path,
    name: &str,
    verus_cfg: bool,
) -> Result<Vec<ir::Record>> {
    let mut command = probe_command(probe, inv, args, function, verus_cfg);
    let result = compile(&mut command, out, name, &inv.args)?;
    let records = ir::parse(&String::from_utf8_lossy(&result.stderr))?;
    let selected = inv
        .env
        .get("CLIENT_NESTED_FUNCTION")
        .filter(|s| !s.is_empty())
        .map(String::as_str)
        .unwrap_or(function);
    write_json(
        &out.join(format!("{name}.json")),
        &json!({"status":if result.status.success(){"typed"}else{"not_analyzed"},"compiler_exit_code":result.status.code(),"records":records}),
    )?;
    if !result.status.success()
        || (!(function == "*" && inv.env.contains_key("CLIENT_PROJECT_INVENTORY"))
            && !records
                .iter()
                .any(|r| r["kind"] == "function" && r["path"] == selected))
    {
        return Err("Rustc could not typecheck the selected function".into());
    }
    Ok(records)
}
pub(crate) fn probe_command(
    probe: &Path,
    inv: &Invocation,
    args: &[String],
    function: &str,
    verus_cfg: bool,
) -> Command {
    let mut command = Command::new(probe);
    environment(&mut command, inv);
    command
        .current_dir(&inv.cwd)
        .args(args)
        .env("CLIENT_FUNCTION", function)
        .env("CLIENT_RESOLVE_INSTANCES", "1");
    if verus_cfg {
        command.args(["--cfg", "verus_only", "--cfg", "verus_keep_ghost"]);
    }
    command.args(["-Aunexpected_cfgs", "-Aunused"]);
    for feature in [
        "stmt_expr_attributes",
        "box_patterns",
        "negative_impls",
        "rustc_attrs",
        "unboxed_closures",
        "register_tool",
        "tuple_trait",
        "custom_inner_attributes",
        "try_trait_v2",
    ] {
        command.arg(format!("-Zcrate-attr=feature({feature})"));
    }
    for tool in ["verus", "verifier", "verusfmt"] {
        command.arg(format!("-Zcrate-attr=register_tool({tool})"));
    }
    command
}
pub(crate) fn prove(
    root: &Path,
    inv: &Invocation,
    args: &[String],
    out: &Path,
    name: &str,
) -> Result<Value> {
    prove_with_budget(root, inv, args, out, name, None)
}
fn prove_with_budget(
    root: &Path,
    inv: &Invocation,
    args: &[String],
    out: &Path,
    name: &str,
    limit: Option<u32>,
) -> Result<Value> {
    let logs = out.join(format!("{name}-vir"));
    let mut command = verifier(root, inv, args);
    if let Some(limit) = limit {
        command.args(["--rlimit", &limit.to_string()]);
    }
    command.args(["--log", "vir", "--log-dir"]).arg(logs);
    let (result, timed_out) = compile_with_limit(
        &mut command,
        out,
        name,
        &inv.args,
        limit.map(|_| Duration::from_secs(30)),
    )?;
    let mut report = diagnostics(&result);
    if timed_out {
        report["status"] = json!("timeout");
    }
    write_json(&out.join(format!("{name}.json")), &report)?;
    Ok(report)
}
pub(crate) fn build_probe(inv: &Invocation, dir: &Path) -> Result<Option<PathBuf>> {
    build_source_probe(
        inv,
        dir,
        include_str!("../../../experiments/rustc-ir/mir_probe.rs"),
        "mir-probe",
    )
}
pub(crate) fn build_source_probe(
    inv: &Invocation,
    dir: &Path,
    source: &str,
    name: &str,
) -> Result<Option<PathBuf>> {
    let compiler = || {
        let mut command = Command::new(&inv.rustc);
        environment(&mut command, inv);
        command.current_dir(&inv.cwd);
        command
    };
    let sysroot = compiler().args(["--print", "sysroot"]).output()?;
    if !sysroot.status.success() {
        return Err("Rustc sysroot discovery failed".into());
    }
    let sysroot = PathBuf::from(String::from_utf8(sysroot.stdout)?.trim());
    let version = compiler().arg("-vV").output()?;
    if !version.status.success() {
        return Err("Rustc version discovery failed".into());
    }
    let version = String::from_utf8(version.stdout)?;
    let host = version
        .lines()
        .find_map(|s| s.strip_prefix("host: "))
        .ok_or("missing compiler host")?;
    let macro_source = sysroot.join("lib/rustlib/src/rust/library/core/src/macros/mod.rs");
    write_json(
        &dir.join("compiler-rule.json"),
        &json!({
        "version":version,"release":version.lines().find_map(|s| s.strip_prefix("release: ")),
        "macro_source":macro_source,"macro_sha256":if macro_source.is_file(){Some(hash(&fs::read(&macro_source)?)?)}else{None},
        "scope":"Installed SDK source is fingerprinted; correspondence to the compiled core library remains trusted"}),
    )?;
    let code = dir.join("probe.rs");
    fs::write(&code, source)?;
    let binary = dir.join(name);
    let result = execute(
        compiler()
            .arg(&code)
            .arg("-L")
            .arg(sysroot.join(format!("lib/rustlib/{host}/lib")))
            .args(["-C", "linker=cc", "-C"])
            .arg(format!(
                "link-arg=-Wl,-rpath,{}",
                sysroot.join("lib").display()
            ))
            .args(["-C"])
            .arg(format!(
                "link-arg=-Wl,-rpath,{}/lib/rustlib/{host}/lib",
                sysroot.display()
            ))
            .arg("-o")
            .arg(&binary)
            .env("RUSTC_BOOTSTRAP", "1"),
        dir,
        "probe-build",
    )?;
    Ok(result.status.success().then_some(binary))
}

pub struct Options {
    pub verified: bool,
    pub host: bool,
    pub loop_guidance: bool,
}
pub fn check(
    root: &Path,
    pkg: &Path,
    out: &Path,
    artifacts: &[Value],
    invocations: &[Invocation],
    options: Options,
) -> Result<Vec<Value>> {
    let Options {
        verified,
        host,
        loop_guidance,
    } = options;
    let pkg = fs::canonicalize(pkg)?;
    if out.starts_with(&pkg) {
        return Err("output must be outside the selected package".into());
    }
    let mut inputs = BTreeMap::new();
    source_hashes(&pkg, &mut inputs)?;
    let package_inputs = inputs.clone();
    for p in [
        root.join("verus"),
        root.join("rust_verify"),
        root.join("cargo-verus"),
        env::current_exe()?,
    ] {
        inputs.insert(p.clone(), hash(&fs::read(p)?)?);
    }
    for inv in invocations {
        let name = inv
            .args
            .windows(2)
            .find(|w| w[0] == "--crate-name")
            .ok_or("missing crate name")?[1]
            .clone();
        let suffix = inv
            .args
            .windows(2)
            .find(|w| w[0] == "-C" && w[1].starts_with("extra-filename="))
            .map(|w| w[1].trim_start_matches("extra-filename="))
            .unwrap_or("");
        let dir = inv
            .args
            .windows(2)
            .find(|w| w[0] == "--out-dir")
            .ok_or("missing Cargo output directory")?[1]
            .clone();
        let dep = inv.cwd.join(dir).join(format!("{name}{suffix}.d"));
        for path in depfiles(&fs::read_to_string(dep)?, &inv.cwd)? {
            inputs.insert(path.clone(), hash(&fs::read(path)?)?);
        }
        for pair in inv.args.windows(2) {
            if pair[0] == "--extern"
                && let Some((_, path)) = pair[1].split_once('=')
            {
                let path = inv.cwd.join(path);
                for p in [path.clone(), path.with_extension("vir")] {
                    if p.is_file() {
                        inputs.insert(p.clone(), hash(&fs::read(p)?)?);
                    }
                }
            }
            if pair[0] == "-L"
                && let Some(path) = pair[1].strip_prefix("dependency=")
            {
                for entry in fs::read_dir(inv.cwd.join(path))? {
                    let p = entry?.path();
                    if p.is_file()
                        && p.extension()
                            .is_some_and(|s| ["rlib", "rmeta", "vir", "so"].iter().any(|e| s == *e))
                    {
                        inputs.insert(p.clone(), hash(&fs::read(p)?)?);
                    }
                }
            }
        }
        if let Some(path) = inv.env.get("OUT_DIR") {
            source_hashes(Path::new(path), &mut inputs)?;
        }
    }
    write_json(&out.join("inputs.json"), &inputs)?;
    let mut targets = Vec::new();
    for (target_index, artifact) in artifacts.iter().enumerate() {
        let dir = out.join(format!("target-{target_index}"));
        fs::create_dir(&dir)?;
        let source = fs::canonicalize(
            artifact["target"]["src_path"]
                .as_str()
                .ok_or("missing Cargo target source")?,
        )?;
        let matching = invocations
            .iter()
            .filter(|inv| {
                inv.args.iter().any(|arg| {
                    let path = inv.cwd.join(arg);
                    path.is_file() && fs::canonicalize(path).ok().as_ref() == Some(&source)
                })
            })
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            targets
                .push(json!({"target":artifact["target"],"status":"context_missing_or_ambiguous"}));
            continue;
        }
        let inv = matching[0];
        write_json(&dir.join("context.json"), inv)?;
        let executable = artifact["executable"]
            .as_str()
            .ok_or("missing executable")?;
        if !host {
            targets
                .push(json!({"target":artifact["target"],"status":"unsupported_runtime_target"}));
            continue;
        }
        let mut list_command = Command::new(executable);
        environment(&mut list_command, inv);
        let listing = execute(
            list_command
                .args(["--list", "--format", "terse"])
                .current_dir(&pkg),
            &dir,
            "census",
        )?;
        if !listing.status.success() {
            targets.push(json!({"target":artifact["target"],"status":"census_failed"}));
            continue;
        }
        let mut names = String::from_utf8(listing.stdout)?
            .lines()
            .filter_map(|s| s.strip_suffix(": test").map(str::to_owned))
            .collect::<Vec<_>>();
        names.sort();
        write_json(&dir.join("census.json"), &names)?;
        let filtered = inv.args.iter().chain(inv.env.values()).any(|s| {
            s.contains("--verify-only")
                || s.contains("--verify-function")
                || s.contains("--verify-module")
        });
        let context_supported = verified && inv.unsupported_env.is_empty() && !filtered;
        let args = clean(&inv.args, &dir.join("baseline-output"), None, &inv.cwd)?;
        let baseline = if context_supported {
            prove(root, inv, &args, &dir, "baseline")?
        } else {
            json!({"status":"unsupported_context","unrecorded_environment_changes":inv.unsupported_env,"verification_filter":filtered,"package_verification_enabled":verified})
        };
        let mut probe = None;
        if context_supported || (!names.is_empty() && inv.unsupported_env.is_empty() && !filtered) {
            probe = build_probe(inv, &dir)?;
        }
        let mut samples = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let actual = if inv.unsupported_env.is_empty() {
                runtime(executable, name, inv, &pkg, &dir, index)?
            } else {
                json!({"status":"not_run_context_incomplete"})
            };
            let mut row = json!({"function":name,"runtime":actual,"status":"unsupported_context","added_client_preconditions":0});
            if let Some(probe) = &probe {
                let case = dir.join(format!("sample-{index}"));
                fs::create_dir(&case)?;
                if !context_supported {
                    row["typed_extraction"] = match extract_context(
                        probe, inv, &args, name, &case, "original", false,
                    ) {
                        Ok(records) => json!({"status":"typed","function":name,
                            "calls":records.iter().filter(|r|r["kind"]=="call").count(),
                            "assertion_operands":records.iter().filter(|r|r["kind"]=="assert_operand").count(),
                            "scope":"Original Cargo cfg without injected Verus cfg; raw Rustc records only, no assertion conversion, proof or MIR correspondence claimed"}),
                        Err(error) => json!({"status":"not_analyzed","reason":error.to_string()}),
                    };
                    samples.push(row);
                    continue;
                }
                let result = (|| -> Result<Value> {
                    let original = extract(probe, inv, &args, name, &case, "original")?;
                    if original
                        .iter()
                        .any(|r| r["kind"] == "test_kind" && r["expected_panic"] == "true")
                    {
                        return Ok(
                            json!({"status":"unsupported_expected_panic","reason":"The test expects a panic; normal-return assertion checking does not encode that oracle."}),
                        );
                    }
                    if let Some(helper) = crate::helper::candidate(&original) {
                        let coverage = (|| -> Result<_> {
                            let parent =
                                extract(probe, inv, &args, helper, &case, "helper-source")?;
                            let callback = crate::helper::closure(&parent)?;
                            let mut nested = inv.clone();
                            nested.env.insert("CLIENT_NESTED_FUNCTION".into(), callback);
                            nested
                                .env
                                .insert("CLIENT_RESOLVE_INSTANCES".into(), "1".into());
                            nested
                                .env
                                .insert("CLIENT_EQUALITY_BODIES".into(), "1".into());
                            let callback =
                                extract(probe, &nested, &args, helper, &case, "helper-callback")?;
                            let runtime_parent = extract_context(
                                probe,
                                inv,
                                &args,
                                helper,
                                &case,
                                "helper-runtime",
                                false,
                            )?;
                            let runtime_callback = extract_context(
                                probe,
                                &nested,
                                &args,
                                helper,
                                &case,
                                "callback-runtime",
                                false,
                            )?;
                            crate::helper::configuration(
                                &parent,
                                &callback,
                                &runtime_parent,
                                &runtime_callback,
                            )?;
                            let mut report =
                                crate::helper::inspect(&original, &parent, &callback, &inv.cwd)?;
                            report["configuration_correspondence"] = json!({
                                "helper": true, "closure": true,
                                "scope": "runtime MIR and borrow records under captured runtime and proof configurations"
                            });
                            Ok((report, parent, callback))
                        })();
                        let mut context = None;
                        let report = match coverage {
                            Ok((report, parent, callback)) => {
                                context = Some((parent, callback));
                                report
                            }
                            Err(error) => {
                                json!({"status":"unsupported","reason":error.to_string()})
                            }
                        };
                        write_json(&case.join("helper-context.json"), &report)?;
                        row["helper_context"] = report;
                        if let Some((parent, callback)) = context {
                            let crate_name = inv
                                .args
                                .windows(2)
                                .find(|w| w[0] == "--crate-name")
                                .ok_or("missing crate name")?[1]
                                .clone();
                            let mut plan = crate::helper::lower(
                                &original,
                                &parent,
                                &callback,
                                &inv.cwd,
                                &fs::read_to_string(dir.join("baseline-vir/crate.vir"))?,
                                &crate_name,
                            )?;
                            plan.report["configuration_correspondence"] =
                                row["helper_context"]["configuration_correspondence"].clone();
                            write_json(&case.join("helper-context.json"), &plan.report)?;
                            let producer_target = vir::checked_target_in(
                                &fs::read_to_string(dir.join("baseline-vir/crate.vir"))?,
                                plan.report["lowering"]["input_relation"]["producer"]
                                    .as_str()
                                    .ok_or("producer path missing")?,
                                &pkg,
                                &inv.cwd,
                            )?
                            .ok_or("producer is not a checked local executable")?;
                            row["helper_context"] = plan.report.clone();
                            let relative = plan
                                .file
                                .strip_prefix(&pkg)
                                .map_err(|_| "helper source is outside the package snapshot")?;
                            let snapshot = case.join("package");
                            copy_package(&pkg, &snapshot)?;
                            fs::write(snapshot.join(relative), &plan.source)?;
                            write_json(&case.join("edits.json"), &plan.edits)?;
                            let root_source = snapshot.join(source.strip_prefix(&pkg)?);
                            let candidate = clean(
                                &inv.args,
                                &case.join("compiler-output"),
                                Some((&source, &root_source)),
                                &inv.cwd,
                            )?;
                            let proof = prove(root, inv, &candidate, &case, "check")?;
                            let marked = extract(probe, inv, &candidate, name, &case, "checked")?;
                            let lowered_helper = extract(
                                probe,
                                inv,
                                &candidate,
                                &plan.helper,
                                &case,
                                "helper-checked",
                            )?;
                            let generated = crate::helper::generated(
                                &plan,
                                &fs::read_to_string(snapshot.join(relative))?,
                                &lowered_helper,
                            )?;
                            let caller_matches = ir::compare(&original, &marked, None)?;
                            let checked_vir = fs::read_to_string(case.join("check-vir/crate.vir"))?;
                            let semantic_models = crate::helper::checked_models(
                                &checked_vir,
                                &plan.helper,
                                &crate_name,
                                &snapshot,
                            )?;
                            let target = vir::checked_target(
                                &checked_vir,
                                &format!("{crate_name}::{name}"),
                                &snapshot,
                            )?;
                            let checked_helper = vir::checked_target(
                                &checked_vir,
                                &format!("{crate_name}::{}", plan.helper),
                                &snapshot,
                            )?;
                            let mut status = proof["status"].as_str().unwrap_or("not_analyzed");
                            if !caller_matches {
                                status = "correspondence_failed";
                            } else if status == "proved" {
                                if baseline["status"] != "proved" {
                                    status = "api_not_validated";
                                } else if target
                                    .as_ref()
                                    .is_none_or(|t| t["existing_preconditions"] != 0)
                                    || checked_helper.is_none()
                                {
                                    status = "target_not_checked";
                                } else if row["runtime"]["status"] == "failed" {
                                    status = "runtime_proof_disagreement";
                                }
                            }
                            write_json(&case.join("helper-correspondence.json"), &generated)?;
                            return Ok(
                                json!({"status":status,"verification":proof,"target":target,
                                "helper_target":checked_helper,"helper_correspondence":generated,
                                "producer_target":producer_target,"semantic_models":semantic_models,
                                "runtime_scope":"Original host-adapted package; transformed-runtime replay is separate",
                                "added_helper_preconditions":1,
                                "correspondence":{"matches":caller_matches,"scope":"Caller runtime MIR preserved; helper uses the separately gated read/assert rule"},
                                "added_client_preconditions":0}),
                            );
                        }
                    }
                    if original.iter().any(|r| {
                        r["kind"] == "assert_operand"
                            && r["macro"] == "std::assert_eq"
                            && r["type"] == "usize"
                    }) {
                        crate::assertion_rule::compiler_binding(&serde_json::from_slice::<Value>(
                            &fs::read(dir.join("compiler-rule.json"))?,
                        )?)?;
                    }
                    let (file, mut changed, mut edits) = ir::lower(&original, &inv.cwd)?;
                    write_json(
                        &case.join("assertion-rule.json"),
                        &crate::assertion_rule::report(&original)?,
                    )?;
                    let file = fs::canonicalize(file)?;
                    let guidance_report = if loop_guidance
                        && dir.join("baseline-vir/crate.vir").is_file()
                    {
                        let (extra, report) = guidance::propose(
                            &original,
                            &fs::read_to_string(dir.join("baseline-vir/crate.vir"))?,
                            &inv.cwd,
                            &pkg,
                        )?;
                        for edit in extra.iter().rev() {
                            if edits
                                .iter()
                                .any(|e| e.start <= edit.start && edit.start < e.end)
                            {
                                return Err("loop guidance overlaps assertion conversion".into());
                            }
                            let offset = usize::try_from(
                                edit.start as isize
                                    + edits
                                        .iter()
                                        .filter(|e| e.end <= edit.start)
                                        .map(|e| {
                                            e.replacement.len() as isize
                                                - (e.end - e.start) as isize
                                        })
                                        .sum::<isize>(),
                            )?;
                            changed.insert_str(offset, &edit.replacement);
                        }
                        edits.extend(extra);
                        edits.sort_by_key(|e| e.start);
                        report
                    } else {
                        json!({"enabled":false})
                    };
                    let relative = file
                        .strip_prefix(&pkg)
                        .map_err(|_| "test source is outside the package snapshot")?;
                    let snapshot = case.join("package");
                    copy_package(&pkg, &snapshot)?;
                    fs::write(snapshot.join(relative), changed)?;
                    write_json(&case.join("edits.json"), &edits)?;
                    let root_source = snapshot.join(source.strip_prefix(&pkg)?);
                    let candidate = clean(
                        &inv.args,
                        &case.join("compiler-output"),
                        Some((&source, &root_source)),
                        &inv.cwd,
                    )?;
                    let mut proof = prove(root, inv, &candidate, &case, "check")?;
                    if baseline["status"] == "proved"
                        && proof["diagnostics"].as_array().is_some_and(|ds| {
                            ds.iter().any(|d| {
                                d["message"] == "cannot find attribute `verus_spec` in this scope"
                            })
                        })
                    {
                        let body = original
                            .iter()
                            .find(|r| r["kind"] == "body")
                            .ok_or("missing body")?;
                        let (_, coords) = ir::span(&body["span"], &inv.cwd)?;
                        let original_text = fs::read_to_string(&file)?;
                        let (_, end) = ir::offsets(&original_text, coords)?;
                        let offset = usize::try_from(
                            end as isize
                                + edits
                                    .iter()
                                    .filter(|e| e.end <= end)
                                    .map(|e| {
                                        e.replacement.len() as isize - (e.end - e.start) as isize
                                    })
                                    .sum::<isize>(),
                        )?;
                        let path = snapshot.join(relative);
                        let mut text = fs::read_to_string(&path)?;
                        text.insert_str(offset, "\nuse vstd::prelude::verus_spec;\n");
                        fs::write(path, text)?;
                        proof = prove(root, inv, &candidate, &case, "check")?;
                    }
                    let mut call_guidance = json!({"status":"disabled"});
                    if loop_guidance
                        && baseline["status"] == "proved"
                        && proof["status"] == "not_proved"
                        && case.join("check-vir/crate.vir").is_file()
                    {
                        let crate_name = inv
                            .args
                            .windows(2)
                            .find(|w| w[0] == "--crate-name")
                            .ok_or("missing crate name")?[1]
                            .clone();
                        let initial_vir = fs::read_to_string(case.join("check-vir/crate.vir"))?;
                        match crate::call_guidance::plan(
                            &proof,
                            &initial_vir,
                            &format!("{crate_name}::{name}"),
                            &snapshot.join(relative),
                            &inv.cwd,
                            &inv.args,
                        ) {
                            Err(e) => {
                                call_guidance =
                                    json!({"status":"unsupported","reason":e.to_string()})
                            }
                            Ok(plan) => {
                                call_guidance = plan.report;
                                if !plan.paths.is_empty() {
                                    let body = original
                                        .iter()
                                        .find(|r| r["kind"] == "body")
                                        .ok_or("missing client body")?;
                                    let (body_file, coords) = ir::span(&body["span"], &inv.cwd)?;
                                    let original_source = fs::read_to_string(&file)?;
                                    let (body_start, _) = ir::offsets(&original_source, coords)?;
                                    if fs::canonicalize(body_file)? != file
                                        || original_source.as_bytes().get(body_start) != Some(&b'{')
                                    {
                                        return Err(
                                            "call guidance needs a source-bound Rust block".into(),
                                        );
                                    }
                                    let insertion = body_start + 1;
                                    if edits
                                        .iter()
                                        .any(|e| e.start < insertion && insertion < e.end)
                                    {
                                        return Err(
                                            "guidance insertion overlaps an existing edit".into()
                                        );
                                    }
                                    let mapped = usize::try_from(
                                        insertion as isize
                                            + edits
                                                .iter()
                                                .filter(|e| {
                                                    e.end <= insertion && e.start < insertion
                                                })
                                                .map(|e| {
                                                    e.replacement.len() as isize
                                                        - (e.end - e.start) as isize
                                                })
                                                .sum::<isize>(),
                                    )?;
                                    let path = snapshot.join(relative);
                                    let base_source = fs::read_to_string(&path)?;
                                    let saved = case.join("call-guidance-initial");
                                    fs::create_dir(&saved)?;
                                    fs::write(saved.join("source.rs"), &base_source)?;
                                    fs::write(saved.join("crate.vir"), &initial_vir)?;
                                    for suffix in ["command.json", "stdout", "stderr", "json"] {
                                        fs::copy(
                                            case.join(format!("check.{suffix}")),
                                            saved.join(format!("check.{suffix}")),
                                        )?;
                                    }
                                    let mut attempts = Vec::new();
                                    let mut accepted = false;
                                    let mut terminal = "budget_exhausted".to_owned();
                                    let trial_result = (|| -> Result<()> {
                                        for (index, fuel) in
                                            crate::call_guidance::FUELS.into_iter().enumerate()
                                        {
                                            let hint =
                                                crate::call_guidance::hint(&plan.paths, fuel);
                                            let mut text = base_source.clone();
                                            text.insert_str(mapped, &hint);
                                            fs::write(&path, &text)?;
                                            let prefix = format!("call-guidance-{index}");
                                            fs::write(
                                                case.join(format!("{prefix}.source.rs")),
                                                &text,
                                            )?;
                                            let attempt = prove_with_budget(
                                                root,
                                                inv,
                                                &candidate,
                                                &case,
                                                &prefix,
                                                Some(crate::call_guidance::RLIMIT),
                                            )?;
                                            attempts.push(json!({"fuel":fuel,"status":attempt["status"],"summary":attempt["summary"],"report":case.join(format!("{prefix}.json")),"source_sha256":hash(text.as_bytes())?,"source":case.join(format!("{prefix}.source.rs"))}));
                                            if attempt["status"] == "proved" {
                                                proof = attempt;
                                                for suffix in
                                                    ["command.json", "stdout", "stderr", "json"]
                                                {
                                                    fs::copy(
                                                        case.join(format!("{prefix}.{suffix}")),
                                                        case.join(format!("check.{suffix}")),
                                                    )?;
                                                }
                                                fs::copy(
                                                    case.join(format!("{prefix}-vir/crate.vir")),
                                                    case.join("check-vir/crate.vir"),
                                                )?;
                                                edits.push(ir::Edit {
                                                    start: insertion,
                                                    end: insertion,
                                                    replacement: hint,
                                                    origin: body["span"].clone(),
                                                });
                                                edits.sort_by_key(|e| e.start);
                                                accepted = true;
                                                break;
                                            }
                                            if attempt["status"] != "not_proved" {
                                                terminal = attempt["status"]
                                                    .as_str()
                                                    .unwrap_or("execution_error")
                                                    .to_owned();
                                                break;
                                            }
                                        }
                                        Ok(())
                                    })();
                                    if !accepted || trial_result.is_err() {
                                        accepted = false;
                                        fs::write(&path, &base_source)?;
                                        proof = serde_json::from_slice(&fs::read(
                                            saved.join("check.json"),
                                        )?)?;
                                        for suffix in ["command.json", "stdout", "stderr", "json"] {
                                            fs::copy(
                                                saved.join(format!("check.{suffix}")),
                                                case.join(format!("check.{suffix}")),
                                            )?;
                                        }
                                        fs::copy(
                                            saved.join("crate.vir"),
                                            case.join("check-vir/crate.vir"),
                                        )?;
                                    }
                                    if let Err(error) = trial_result {
                                        accepted = false;
                                        terminal = "execution_error".into();
                                        call_guidance["error"] = json!(error.to_string());
                                    }
                                    call_guidance["status"] =
                                        json!(if accepted { "proved" } else { &terminal });
                                    call_guidance["attempts"] = json!(attempts);
                                    call_guidance["initial_source"] =
                                        json!(saved.join("source.rs"));
                                    call_guidance["initial_vir_sha256"] =
                                        json!(hash(initial_vir.as_bytes())?);
                                    call_guidance["scope"] = json!(
                                        "Hints come from the initial failed call obligations. Failed trials are retained and reverted; only a proved trial is used. MIR correspondence, target checks and runtime status remain independent gates."
                                    );
                                }
                            }
                        }
                    }
                    write_json(&case.join("call-guidance.json"), &call_guidance)?;
                    write_json(&case.join("edits.json"), &edits)?;
                    let marked = extract(probe, inv, &candidate, name, &case, "checked")?;
                    let has_eq = original
                        .iter()
                        .any(|r| r["kind"] == "assert_operand" && r["macro"] == "std::assert_eq");
                    let helper = format!(
                        "{}checked_eq_failure",
                        name.rsplit_once("::")
                            .map_or(String::new(), |(p, _)| format!("{p}::"))
                    );
                    let same = ir::compare(&original, &marked, has_eq.then_some(helper.as_str()))?;
                    let crate_name = inv
                        .args
                        .windows(2)
                        .find(|w| w[0] == "--crate-name")
                        .ok_or("missing crate name")?[1]
                        .clone();
                    let target = if case.join("check-vir/crate.vir").is_file() {
                        vir::checked_target(
                            &fs::read_to_string(case.join("check-vir/crate.vir"))?,
                            &format!("{crate_name}::{name}"),
                            &snapshot,
                        )?
                    } else {
                        None
                    };
                    let contract_trace = crate::contracts::summarize(
                        &fs::read_to_string(case.join("check-vir/crate.vir")).unwrap_or_default(),
                        &format!("{crate_name}::{name}"),
                        &inv.cwd,
                    )?;
                    write_json(&case.join("contracts.json"), &contract_trace)?;
                    let review = review::summarize(
                        &proof,
                        &fs::read_to_string(case.join("check-vir/crate.vir")).unwrap_or_default(),
                        &inv.cwd,
                        &file,
                        &snapshot.join(relative),
                        &edits,
                        &original,
                    )?;
                    let mut status = proof["status"].as_str().unwrap_or("not_analyzed");
                    if !same {
                        status = "correspondence_failed"
                    } else if status == "proved" {
                        if let Some(target) = &target {
                            if baseline["status"] != "proved" {
                                status = "api_not_validated"
                            } else if target["existing_preconditions"].as_u64().unwrap_or(0) > 0 {
                                status = "conditional_target_preconditions"
                            }
                        } else {
                            status = "target_not_checked"
                        }
                    }
                    if status == "proved" && row["runtime"]["status"] == "failed" {
                        status = "runtime_proof_disagreement";
                    }
                    Ok(
                        json!({"status":status,"verification":proof,"review":review,"assertion_rule":case.join("assertion-rule.json"),"contracts":case.join("contracts.json"),"loop_guidance":guidance_report,"call_guidance":call_guidance,"correspondence":{"matches":same,"scope":"Pinned runtime MIR and borrow facts; selected assertion failure formatting/payload/cleanup excluded"},"target":target,"original_declaration":original.iter().find(|r|r["kind"]=="decl")}),
                    )
                })();
                match result {
                    Ok(details) => {
                        for (k, v) in details.as_object().unwrap() {
                            row[k] = v.clone();
                        }
                    }
                    Err(e) => {
                        row["status"] = json!("unsupported");
                        row["reason"] = json!(e.to_string());
                    }
                }
            } else if context_supported {
                row["status"] = json!("probe_unavailable")
            }
            samples.push(row);
        }
        let driver_source = root
            .parent()
            .and_then(Path::parent)
            .map(|p| p.join("rust_verify/src/driver.rs"));
        let driver_hash = driver_source
            .as_ref()
            .filter(|p| p.is_file())
            .map(|p| -> Result<String> { hash(&fs::read(p)?) })
            .transpose()?;
        let cfg = inv
            .args
            .windows(2)
            .filter(|w| w[0] == "--cfg")
            .map(|w| w[1].clone())
            .chain(
                inv.args
                    .iter()
                    .filter_map(|s| s.strip_prefix("--cfg=").map(str::to_owned)),
            )
            .collect::<Vec<_>>();
        targets.push(json!({"target":artifact["target"],"baseline":baseline,"samples":samples,"context":dir.join("context.json"),
            "compiler_rule":dir.join("compiler-rule.json"),"cfg_profiles":{
                "applies_to_verus":verified,"cargo_argument_cfg":cfg,
                "probe_added":if verified{vec!["verus_only","verus_keep_ghost"]}else{vec![]},
                "driver_compile_added":["verus_only","verus_keep_ghost"],
                "driver_verify_added":["verus_only","verus_keep_ghost","verus_keep_ghost_body"],
                "driver_source":driver_source,"driver_source_sha256":driver_hash,
                "scope":"Selected pinned-driver profiles; source/binary identity, ghost erasure and cfg-dependent execution correspondence remain trusted. This is not a proof that runtime and proof configurations agree."
            }}));
    }
    for (path, expected) in &inputs {
        if hash(&fs::read(path)?)? != *expected {
            return Err(format!("input changed during check: {}", path.display()).into());
        }
    }
    let mut after = BTreeMap::new();
    source_hashes(&pkg, &mut after)?;
    if package_inputs != after {
        return Err("package input inventory changed during check".into());
    }
    Ok(targets)
}
#[cfg(test)]
mod tests {
    #[test]
    fn actual_opaque_callback_panics_are_tool_failures() {
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "../../../experiments/test-spec/goal31-opaque-callback.json"
        ))
        .unwrap();
        for (name, expected) in [
            ("opaque", "tool_failure"),
            ("shared", "tool_failure"),
            ("concrete", "proved"),
            ("no_callback", "proved"),
        ] {
            let case = &data["cases"][name];
            let result = super::proof_summary(
                Some(case["exit_code"].as_i64().unwrap() as i32),
                case["stdout"].as_str().unwrap().as_bytes(),
                case["stderr"].as_str().unwrap().as_bytes(),
            );
            assert_eq!(result["status"], expected);
        }
    }
    #[test]
    fn actual_callback_diagnostics_separate_unsupported_from_unproved() {
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "../../../experiments/test-spec/goal31-callback-state.json"
        ))
        .unwrap();
        for (name, expected) in [
            ("captured", "unsupported"),
            ("explicit", "proved"),
            ("wrong", "not_proved"),
        ] {
            let case = &data["cases"][name];
            let result = super::proof_summary(
                Some(case["result"]["exit_code"].as_i64().unwrap() as i32),
                case["stdout"].as_str().unwrap().as_bytes(),
                case["stderr"].as_str().unwrap().as_bytes(),
            );
            assert_eq!(result["status"], expected);
        }
    }
    #[test]
    #[ignore = "requires PROJECTION_CONTEXT, PROJECTION_SELECTION and PROJECTION_OUT from Goal 26"]
    fn real_projection_uses_caller_constraints() {
        let context = std::path::PathBuf::from(std::env::var("PROJECTION_CONTEXT").unwrap());
        let selection =
            std::fs::read_to_string(std::env::var("PROJECTION_SELECTION").unwrap()).unwrap();
        let function = selection
            .lines()
            .find(|p| p.contains("non_null::either::") && p.ends_with("::raw_as_ref"))
            .unwrap();
        let out = std::path::PathBuf::from(std::env::var("PROJECTION_OUT").unwrap());
        std::fs::create_dir(&out).unwrap();
        let inv: crate::capture::Invocation =
            serde_json::from_slice(&std::fs::read(context).unwrap()).unwrap();
        let probe = super::build_probe(&inv, &out).unwrap().unwrap();
        let args = super::clean(&inv.args, &out.join("compiler-output"), None, &inv.cwd).unwrap();
        let records =
            super::extract_context(&probe, &inv, &args, function, &out, "projection", true)
                .unwrap();
        let fields = records
            .iter()
            .filter(|r| r["kind"] == "datatype_field")
            .collect::<Vec<_>>();
        assert!(
            fields
                .iter()
                .any(|r| r["type"].contains("Alias") && r["environment"].contains("NonNullPtrRef"))
        );
        assert!(
            records
                .iter()
                .any(|r| r["kind"] == "function" && r["path"] == function)
        );
    }
    use super::*;
    #[test]
    #[ignore = "requires SPEC_UNVERIFIED_RUN from a non-Verus Cargo check"]
    fn typed_records_do_not_imply_a_proof() {
        let root = PathBuf::from(env::var("SPEC_UNVERIFIED_RUN").unwrap());
        let report: Value =
            serde_json::from_slice(&fs::read(root.join("report.json")).unwrap()).unwrap();
        let target = &report["targets"][0];
        let context: Invocation =
            serde_json::from_slice(&fs::read(root.join("target-0/context.json")).unwrap()).unwrap();
        assert_eq!(target["baseline"]["package_verification_enabled"], false);
        let samples = target["samples"].as_array().unwrap();
        assert!(!samples.is_empty());
        for (index, sample) in samples.iter().enumerate() {
            assert_eq!(sample["status"], "unsupported_context");
            assert_eq!(sample["runtime"]["status"], "passed");
            assert_eq!(sample["typed_extraction"]["status"], "typed");
            assert!(sample.get("verification").is_none());
            assert!(sample.get("correspondence").is_none());
            let command: Value = serde_json::from_slice(
                &fs::read(root.join(format!("target-0/sample-{index}/original.command.json")))
                    .unwrap(),
            )
            .unwrap();
            let args = command["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a.as_str().unwrap().to_owned())
                .collect::<Vec<_>>();
            let cfgs = |args: &[String]| {
                args.windows(2)
                    .filter(|w| w[0] == "--cfg")
                    .map(|w| w[1].clone())
                    .collect::<Vec<_>>()
            };
            assert_eq!(cfgs(&args), cfgs(&context.args));
        }
    }
    #[test]
    #[ignore = "requires SPEC_CPU_RUNS from evaluate_cpu.py"]
    fn cpu_api_correspondence() {
        let runs = PathBuf::from(env::var("SPEC_CPU_RUNS").unwrap());
        let out = env::temp_dir().join(format!(
            "cpu-api-correspondence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&out).unwrap();
        let mut result = Vec::new();
        for (index, function, nested) in [
            (0, "cpu::all_cpus", None),
            (1, "cpu::all_cpus", Some("cpu::all_cpus::{closure#0}")),
            (2, "cpu::set::CpuSet::iter", None),
            (
                3,
                "cpu::set::CpuSet::iter",
                Some("cpu::set::CpuSet::iter::{closure#0}"),
            ),
            (
                4,
                "cpu::set::CpuSet::iter",
                Some("cpu::set::CpuSet::iter::{closure#1}"),
            ),
        ] {
            let mut extracted = Vec::new();
            for label in ["original", "candidate-guided"] {
                let target = runs.join(label).join("target-0");
                let mut inv: Invocation =
                    serde_json::from_slice(&fs::read(target.join("context.json")).unwrap())
                        .unwrap();
                if let Some(nested) = nested {
                    inv.env
                        .insert("CLIENT_NESTED_FUNCTION".into(), nested.into());
                }
                let case = out.join(format!("{index}-{label}"));
                fs::create_dir(&case).unwrap();
                let args = clean(&inv.args, &case.join("metadata"), None, &inv.cwd).unwrap();
                extracted.push(
                    extract(
                        &target.join("mir-probe"),
                        &inv,
                        &args,
                        function,
                        &case,
                        "extract",
                    )
                    .unwrap(),
                );
            }
            let matches = ir::compare(&extracted[0], &extracted[1], None).unwrap();
            result.push(json!({"function":function,"nested":nested,"matches":matches}));
            assert!(matches, "{function} {nested:?}");
        }
        write_json(&out.join("comparison.json"), &result).unwrap();
        println!("API correspondence evidence: {}", out.display());
    }

    #[test]
    #[ignore = "requires SPEC_RESOURCE_CONTEXT, SPEC_RESOURCE_SNAPSHOT, SPEC_RESOURCE_OUT; optional SPEC_RESOURCE_VIR"]
    fn range_allocator_existing_methods_mir_and_source_audit() {
        let context_path = PathBuf::from(env::var("SPEC_RESOURCE_CONTEXT").unwrap());
        let snapshot = PathBuf::from(env::var("SPEC_RESOURCE_SNAPSHOT").unwrap());
        let out = PathBuf::from(env::var("SPEC_RESOURCE_OUT").unwrap());
        fs::create_dir_all(&out).unwrap();

        let mut context_json: Value =
            serde_json::from_slice(&fs::read(&context_path).unwrap()).unwrap();
        // Some existing Cargo captures predate this field. Missing means no omissions were
        // recorded, and is normalized only in memory (the captured input remains untouched).
        let unsupported_env_was_missing = !context_json
            .as_object()
            .unwrap()
            .contains_key("unsupported_env");
        context_json
            .as_object_mut()
            .unwrap()
            .entry("unsupported_env")
            .or_insert_with(|| json!([]));
        let original_inv: Invocation = serde_json::from_value(context_json.clone()).unwrap();
        let snapshot_inv: Invocation = serde_json::from_value(context_json.clone()).unwrap();
        let original_source = original_inv.cwd.join("ostd/src/util/range_alloc.rs");
        let snapshot_source = snapshot.join("ostd/src/util/range_alloc.rs");
        let original_crate_root = original_inv.cwd.join("ostd/src/lib.rs");
        let snapshot_crate_root = snapshot.join("ostd/src/lib.rs");
        let original_bytes = fs::read(&original_source).unwrap();
        let snapshot_bytes = fs::read(&snapshot_source).unwrap();
        let original_text = std::str::from_utf8(&original_bytes).unwrap();
        let snapshot_text = std::str::from_utf8(&snapshot_bytes).unwrap();
        let appended = snapshot_text
            .strip_prefix(original_text)
            .unwrap_or_else(|| panic!("snapshot changed source before the appended witness"));
        assert!(appended.contains("resource_chain_witness"));

        // Match the native verifier replay: keep the captured working directory,
        // environment, and dependency artifacts, rewriting only the crate-root source arg.
        assert_eq!(original_inv.cwd, snapshot_inv.cwd);
        assert_eq!(original_inv.env, snapshot_inv.env);
        assert_eq!(original_inv.args, snapshot_inv.args);
        let probe = build_probe(&original_inv, &out)
            .unwrap()
            .expect("pinned Rustc probe must build");

        fn discover(
            probe: &Path,
            inv: &Invocation,
            out: &Path,
            label: &str,
            source: &Path,
            source_rewrite: Option<(&Path, &Path)>,
        ) -> Vec<ir::Record> {
            let case = out.join(label);
            fs::create_dir_all(&case).unwrap();
            let args = clean(&inv.args, &case.join("metadata"), source_rewrite, &inv.cwd).unwrap();
            let result = extract_context(probe, inv, &args, "*", &case, "discover", true);
            assert!(result.is_err(), "wildcard discovery is not a selected body");
            let report: Value =
                serde_json::from_slice(&fs::read(case.join("discover.json")).unwrap()).unwrap();
            let records: Vec<ir::Record> =
                serde_json::from_value(report["records"].clone()).unwrap();
            records
                .into_iter()
                .filter(|record| {
                    record["kind"] == "candidate"
                        && record.get("span").is_some_and(|span| {
                            ir::span(span, &inv.cwd).is_ok_and(|(path, _)| path == source)
                        })
                })
                .collect()
        }

        let original_candidates = discover(
            &probe,
            &original_inv,
            &out,
            "discover-original",
            &original_source,
            None,
        );
        let snapshot_candidates = discover(
            &probe,
            &snapshot_inv,
            &out,
            "discover-snapshot",
            &snapshot_source,
            Some((&original_crate_root, &snapshot_crate_root)),
        );
        let methods = ["new", "get_freelist_guard", "alloc_specific", "free"];
        let mut comparisons = Vec::new();
        fn strict_source_preserving_records_match(
            before: &[ir::Record],
            after: &[ir::Record],
            snapshot_root: &Path,
        ) -> bool {
            static SOURCE_CONTEXT: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
            fn normalize(value: &mut Value, snapshot_root: &Path) {
                match value {
                    Value::String(text) => {
                        let prefix = format!("{}/", snapshot_root.to_string_lossy());
                        *text = text.replace(&prefix, "");
                        let contexts = SOURCE_CONTEXT.get_or_init(|| {
                            regex::Regex::new(r"(\.rs:\d+:\d+: \d+:\d+ \(#)\d+(\))").unwrap()
                        });
                        *text = contexts.replace_all(text, "$1SOURCE_CTX$2").into_owned();
                    }
                    Value::Array(items) => {
                        items.iter_mut().for_each(|x| normalize(x, snapshot_root))
                    }
                    Value::Object(fields) => fields
                        .values_mut()
                        .for_each(|x| normalize(x, snapshot_root)),
                    _ => {}
                }
            }
            if before.len() != after.len() {
                return false;
            }
            before.iter().zip(after).all(|(left, right)| {
                let mut left = serde_json::to_value(left).unwrap();
                let mut right = serde_json::to_value(right).unwrap();
                normalize(&mut left, snapshot_root);
                normalize(&mut right, snapshot_root);
                left == right
            })
        }
        let mut method_coordinates = Vec::new();
        for method in methods {
            let select = |candidates: &[ir::Record], cwd: &Path, source: &Path| {
                let matching = candidates
                    .iter()
                    .filter(|record| {
                        record["path"].ends_with(&format!("::{method}"))
                            && record.get("span").is_some_and(|span| {
                                let Ok((path, coordinates)) = ir::span(span, cwd) else {
                                    return false;
                                };
                                if path != source {
                                    return false;
                                }
                                let Ok(text) = fs::read_to_string(&path) else {
                                    return false;
                                };
                                let Ok((start, end)) = ir::offsets(&text, coordinates) else {
                                    return false;
                                };
                                let excerpt = &text[start..end];
                                method != "new" || excerpt.contains("new(fullrange: Range<usize>)")
                            })
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    matching.len(),
                    1,
                    "{method}: source/path candidate ambiguity"
                );
                let record = matching[0];
                let (path, coordinates) = ir::span(record.get("span").unwrap(), cwd).unwrap();
                let source_text = fs::read_to_string(path).unwrap();
                let (start, end) = ir::offsets(&source_text, coordinates).unwrap();
                (
                    record["path"].clone(),
                    source_text[start..end].to_owned(),
                    coordinates,
                )
            };
            let (original_path, original_item, original_coordinates) =
                select(&original_candidates, &original_inv.cwd, &original_source);
            let (snapshot_path, snapshot_item, snapshot_coordinates) =
                select(&snapshot_candidates, &snapshot_inv.cwd, &snapshot_source);
            assert_eq!(
                original_path, snapshot_path,
                "{method}: Rustc identity changed"
            );
            assert_eq!(
                original_item, snapshot_item,
                "{method}: source item bytes changed"
            );
            assert_eq!(
                original_coordinates, snapshot_coordinates,
                "{method}: source item span changed"
            );
            method_coordinates.push(snapshot_coordinates);

            let original_case = out.join(format!("original-{method}"));
            let snapshot_case = out.join(format!("snapshot-{method}"));
            fs::create_dir_all(&original_case).unwrap();
            fs::create_dir_all(&snapshot_case).unwrap();
            let original_args = clean(
                &original_inv.args,
                &original_case.join("metadata"),
                None,
                &original_inv.cwd,
            )
            .unwrap();
            let snapshot_args = clean(
                &snapshot_inv.args,
                &snapshot_case.join("metadata"),
                Some((&original_crate_root, &snapshot_crate_root)),
                &snapshot_inv.cwd,
            )
            .unwrap();
            let before = extract_context(
                &probe,
                &original_inv,
                &original_args,
                &original_path,
                &original_case,
                "extract",
                true,
            )
            .unwrap_or_else(|error| panic!("original {method} extraction: {error}"));
            let after = extract_context(
                &probe,
                &snapshot_inv,
                &snapshot_args,
                &snapshot_path,
                &snapshot_case,
                "extract",
                true,
            )
            .unwrap_or_else(|error| panic!("snapshot {method} extraction: {error}"));
            let ordinary_comparison = match ir::compare(&before, &after, None) {
                Ok(matches) => json!({
                    "status":if matches {"matched"} else {"mismatch"},"matches":matches
                }),
                Err(error) => json!({
                    "status":"comparison_unsupported","error":error.to_string()
                }),
            };
            let strict_matches = strict_source_preserving_records_match(&before, &after, &snapshot);
            assert!(
                strict_matches,
                "{method}: raw MIR/borrow records differ beyond source paths and syntax contexts"
            );
            comparisons.push(json!({
                "method":method,"path":original_path,"source_item_bytes_identical":true,
                "existing_ir_compare":ordinary_comparison,
                "strict_source_preserving_record_comparison":{
                    "status":"matched","matches":true,
                    "normalizations":["snapshot source-root prefix in source locations", "Rustc syntax-context numbers in source spans"]
                }
            }));
        }

        let all_matched = comparisons
            .iter()
            .all(|entry| entry["strict_source_preserving_record_comparison"]["matches"] == true);
        let mut evidence = json!({
            "source": {
                "original": original_source,
                "snapshot": snapshot_source,
                "original_bytes": original_bytes.len(),
                "snapshot_bytes": snapshot_bytes.len(),
                "snapshot_is_original_plus_witness": true,
                "witness_name": "resource_chain_witness"
            },
            "actual_cargo_context": context_path,
            "captured_cwd_and_environment_preserved": true,
            "unsupported_env_missing_in_capture_synthesized_as_empty_for_deserialization_only": unsupported_env_was_missing,
            "same_mir_and_borrow_facts": comparisons,
            "comparison_status": if all_matched {"all_matched"} else {"incomplete_or_mismatch"},
            "probe_cfg_supplement": ["verus_only", "verus_keep_ghost"],
            "scope": "Source identity and Rustc MIR/borrow correspondence for four existing RangeAllocator methods; no API behavior proof and no claim about allocator success."
        });

        if let Ok(native_vir) = env::var("SPEC_RESOURCE_VIR") {
            let metadata = fs::metadata(&native_vir).unwrap();
            let vir_text = fs::read_to_string(&native_vir).unwrap();
            let functions = vir::functions(&vir_text).unwrap();
            let mut inventory = Vec::new();
            for (method, expected_coordinates) in methods.into_iter().zip(method_coordinates) {
                let mut selected = Vec::new();
                for function in &functions {
                    let path = function
                        .field("name")
                        .and_then(|name| name.field("path"))
                        .and_then(vir::Node::atom);
                    if !path.is_some_and(|path| path.ends_with(&format!("::{method}"))) {
                        continue;
                    }
                    let span = function.list().and_then(|items| {
                        (items.first().and_then(vir::Node::atom) == Some("@"))
                            .then(|| items.get(1).and_then(vir::Node::atom))
                            .flatten()
                    });
                    let Some(span) = span else { continue };
                    let Ok((source, coordinates)) = ir::span(span, &snapshot_inv.cwd) else {
                        continue;
                    };
                    if source == snapshot_source && coordinates == expected_coordinates {
                        selected.push(path.unwrap().to_owned());
                    }
                }
                assert_eq!(
                    selected.len(),
                    1,
                    "{method}: native VIR source identity ambiguity"
                );
                inventory.push(
                    crate::contracts::summarize_functions(
                        &functions,
                        &selected[0],
                        &snapshot_inv.cwd,
                    )
                    .unwrap(),
                );
            }
            let witness_path = "ostd::util::range_alloc::resource_chain_witness";
            let witness = functions
                .iter()
                .find(|f| {
                    f.field("name")
                        .and_then(|n| n.field("path"))
                        .and_then(vir::Node::atom)
                        == Some(witness_path)
                })
                .unwrap();
            assert_eq!(
                witness.field("mode").and_then(vir::Node::atom),
                Some("Exec")
            );
            assert_eq!(
                witness
                    .field("attrs")
                    .and_then(|a| a.field("is_external_body"))
                    .and_then(vir::Node::atom),
                Some("false")
            );
            assert!(witness.field("body").and_then(vir::Node::list).is_some());
            let witness_report =
                crate::contracts::summarize_functions(&functions, witness_path, &snapshot_inv.cwd)
                    .unwrap();
            assert_eq!(witness_report["function"]["params"], json!([]));
            assert_eq!(witness_report["function"]["requires"], json!([]));
            evidence["constructive_witness"] = json!({"body_checked_in_selected_module":true,
                "contract_trace":witness_report});
            evidence["native_vir_inventory"] = json!({"status":"summarized","path":native_vir,
                "bytes":metadata.len(),"parsed_once":true,"functions":inventory});
        } else {
            evidence["native_vir_inventory"] = json!({"status":"not_requested"});
        }
        write_json(
            &out.join("range-allocator-source-mir-audit.json"),
            &evidence,
        )
        .unwrap();
        println!("RangeAllocator source/MIR audit: {}", out.display());
    }
    #[test]
    fn dep_info_preserves_escaped_paths() {
        let paths = depfiles(
            "/tmp/out.d: src/lib.rs src/with\\ space.rs price$$.rs\n",
            Path::new("/pkg"),
        )
        .unwrap();
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/pkg/src/lib.rs"),
                PathBuf::from("/pkg/src/with space.rs"),
                PathBuf::from("/pkg/price$.rs")
            ]
        );
    }
    #[test]
    #[ignore = "requires SPEC_TEST_CONTEXT and SPEC_TEST_VERUS_ROOT from a real captured Cargo build"]
    fn runtime_translation() {
        let context = PathBuf::from(env::var("SPEC_TEST_CONTEXT").unwrap());
        let inv: Invocation = serde_json::from_slice(&fs::read(context).unwrap()).unwrap();
        let root = PathBuf::from(env::var("SPEC_TEST_VERUS_ROOT").unwrap());
        let dir = env::var_os("SPEC_ASSERTION_RUNTIME_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                env::temp_dir().join(format!(
                    "spec-rust-runtime-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ))
            });
        fs::create_dir(&dir).unwrap();
        let fixture=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../checker-evidence/test-spec/multiple-assertions/order/multiple/assert-order-original.rs");
        let source = dir.join("original.rs");
        fs::copy(fixture, &source).unwrap();
        let old_source = inv.args.iter().find(|s| s.ends_with(".rs")).unwrap();
        let old_source = inv.cwd.join(old_source);
        let args = clean(
            &inv.args,
            &dir.join("metadata"),
            Some((&old_source, &source)),
            &inv.cwd,
        )
        .unwrap();
        let probe = build_probe(&inv, &dir).unwrap().unwrap();
        let original = extract(
            &probe,
            &inv,
            &args,
            "cpu::set::test::exercise",
            &dir,
            "original",
        )
        .unwrap();
        let (_, mut lowered, _) = ir::lower(&original, &inv.cwd).unwrap();
        lowered = lowered.replacen("#[vstd::prelude::verus_verify]\n", "", 1);
        let generated = dir.join("lowered.rs");
        fs::write(&generated, lowered).unwrap();
        let checked_args = clean(
            &inv.args,
            &dir.join("metadata-checked"),
            Some((&old_source, &generated)),
            &inv.cwd,
        )
        .unwrap();
        let checked = extract(
            &probe,
            &inv,
            &checked_args,
            "cpu::set::test::exercise",
            &dir,
            "checked",
        )
        .unwrap();
        assert!(
            ir::compare(
                &original,
                &checked,
                Some("cpu::set::test::checked_eq_failure")
            )
            .unwrap()
        );
        let mut events = Vec::new();
        for (name, path) in [("original", &source), ("lowered", &generated)] {
            let flags = clean(
                &inv.args,
                &dir.join(format!("{name}-build")),
                Some((&old_source, path)),
                &inv.cwd,
            )
            .unwrap();
            let mut command = verifier(&root, &inv, &flags);
            let binary = dir.join(format!("{name}-binary"));
            command
                .args(["--compile", "--no-verify", "--emit=link", "-o"])
                .arg(&binary);
            let result =
                compile(&mut command, &dir, &format!("{name}-compile"), &inv.args).unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let metadata = String::from_utf8_lossy(&result.stderr)
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find(|v| v["emit"] == "link" && v["artifact"].is_string())
                .unwrap();
            let result = execute(
                Command::new(metadata["artifact"].as_str().unwrap())
                    .args(["--exact", "cpu::set::test::runtime_order", "--nocapture"])
                    .envs(&inv.env),
                &dir,
                &format!("{name}-runtime"),
            )
            .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            events.push(
                String::from_utf8(result.stdout)
                    .unwrap()
                    .lines()
                    .filter(|s| s.starts_with("ORDER_EQ "))
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
            );
        }
        assert_eq!(events[0].len(), 8);
        assert_eq!(events[0], events[1]);
        write_json(&dir.join("runtime-summary.json"), &json!({"cases":8,"events":events,
            "mir_matches":true,"rule_binding":crate::assertion_rule::stamp().unwrap(),
            "scope":"Actual generator and compiler over eight finite controls; panic payload and cleanup excluded"})).unwrap();
        println!("runtime evidence: {}", dir.display());
    }
}

#[cfg(test)]
mod bounded_process_tests {
    use super::*;
    #[test]
    fn solver_process_group_has_a_wall_budget() {
        let dir = std::env::temp_dir().join(format!(
            "spec-wall-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let start = Instant::now();
        let (result, timed) = execute_limited(
            Command::new("sh").args(["-c", "sleep 5 & wait"]),
            &dir,
            "timeout",
            Duration::from_millis(60),
        )
        .unwrap();
        assert!(timed && !result.status.success());
        assert!(start.elapsed() < Duration::from_secs(3));
        let (result, timed) = execute_limited(
            Command::new("sh").args(["-c", "printf ready"]),
            &dir,
            "normal",
            Duration::from_secs(2),
        )
        .unwrap();
        assert!(!timed && result.status.success());
        assert_eq!(result.stdout, b"ready");
        fs::remove_dir_all(dir).unwrap();
    }
}
