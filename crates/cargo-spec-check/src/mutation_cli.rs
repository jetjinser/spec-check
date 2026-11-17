//! Stable CLI surface for the mutation workflow.
//!
//! `mutation generate|run|triage` are the supported forms. The older
//! `mutation-generate`, `analyses-mutation`, and `mutation-triage` spellings
//! remain compatibility aliases.

use spec_check_analyses::{mutation, mutation_generate, mutation_triage};
use spec_check_core::support::{Result, write_json};
use std::{collections::BTreeMap, fs, path::Path};

const TOP_HELP: &str = "cargo-spec-check mutation\n\
\n\
  mutation generate --config FILE --out FILE [--report FILE]\n\
  mutation run --manifest FILE --cargo-verus PATH --z3 PATH --vstd PATH --out DIR [--dry-run] [--mode advisory|strict]\n\
  mutation triage --manifest FILE --results FILE --source FILE --out FILE [--registry FILE]\n\
\n\
Aliases: mutation-generate, analyses-mutation, mutation-triage.";

enum Command {
    Generate,
    Run,
    Triage,
    Help,
}

fn parse(
    args: &[String],
    values: &[&str],
    flags: &[&str],
    usage: &str,
) -> Result<(BTreeMap<String, String>, BTreeMap<String, bool>)> {
    let mut map = BTreeMap::new();
    let mut seen_flags = BTreeMap::new();
    let mut iter = args.iter();
    while let Some(key) = iter.next() {
        if flags.contains(&key.as_str()) {
            seen_flags.insert(key.clone(), true);
        } else if values.contains(&key.as_str()) {
            let value = iter.next().ok_or(usage)?.clone();
            if map.insert(key.clone(), value).is_some() {
                return Err(usage.into());
            }
        } else {
            return Err(usage.into());
        }
    }
    Ok((map, seen_flags))
}

fn value<'a>(map: &'a BTreeMap<String, String>, key: &str, usage: &str) -> Result<&'a str> {
    map.get(key).map(String::as_str).ok_or(usage.into())
}

fn head(args: &[String]) -> (Command, &[String]) {
    let Some(first) = args.first().map(String::as_str) else {
        return (Command::Help, &[]);
    };
    let rest = &args[1..];
    match first {
        "mutation-generate" => (Command::Generate, rest),
        "analyses-mutation" => (Command::Run, rest),
        "mutation-triage" => (Command::Triage, rest),
        "mutation" => match rest.first().map(String::as_str) {
            Some("generate") => (Command::Generate, &rest[1..]),
            Some("run") => (Command::Run, &rest[1..]),
            Some("triage") => (Command::Triage, &rest[1..]),
            Some("--help") | Some("-h") | None => (Command::Help, &[]),
            // Preserve the pre-v0.3 bare `mutation --manifest ...` form.
            Some(other) if other.starts_with("--") => (Command::Run, rest),
            Some(_) => (Command::Help, &[]),
        },
        _ => (Command::Help, &[]),
    }
}

fn generate(args: &[String]) -> Result<()> {
    let usage = "mutation generate --config FILE --out FILE [--report FILE]";
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{usage}");
        return Ok(());
    }
    let (map, _) = parse(args, &["--config", "--out", "--report"], &[], usage)?;
    let config: mutation_generate::Config =
        serde_json::from_slice(&fs::read(value(&map, "--config", usage)?)?)?;
    let (manifest, report) = mutation_generate::generate(&config)?;
    write_json(Path::new(value(&map, "--out", usage)?), &manifest)?;
    if let Some(path) = map.get("--report") {
        write_json(Path::new(path), &report)?;
    }
    println!(
        "mutation generate: {} mutants from {} functions ({} non-unique, {} same, {} truncated)",
        report.generated,
        report.functions.len(),
        report.skipped_non_unique,
        report.skipped_same,
        report.truncated,
    );
    for (operator, count) in &report.by_operator {
        println!("  {operator}: {count}");
    }
    Ok(())
}

fn run(args: &[String]) -> Result<()> {
    let usage = "mutation run --manifest FILE --cargo-verus PATH --z3 PATH --vstd PATH --out DIR [--dry-run] [--mode advisory|strict]";
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{usage}");
        return Ok(());
    }
    let (map, flags) = parse(
        args,
        &[
            "--manifest",
            "--cargo-verus",
            "--z3",
            "--vstd",
            "--out",
            "--mode",
        ],
        &["--dry-run"],
        usage,
    )?;
    let manifest: mutation::Manifest =
        serde_json::from_slice(&fs::read(value(&map, "--manifest", usage)?)?)?;
    if flags.contains_key("--dry-run") {
        let report = mutation::validate_manifest(&manifest)?;
        println!(
            "mutation dry-run: {} valid mutants in {} (source {})",
            report.mutants,
            report.target_workspace.display(),
            report.source.display()
        );
        for warning in report.warnings {
            println!("warning: {warning}");
        }
        return Ok(());
    }
    let mode = map.get("--mode").map(String::as_str).unwrap_or("advisory");
    if mode != "advisory" && mode != "strict" {
        return Err("--mode must be advisory or strict".into());
    }
    let out = Path::new(value(&map, "--out", usage)?);
    let report = mutation::run(
        &manifest,
        Path::new(value(&map, "--cargo-verus", usage)?),
        Path::new(value(&map, "--z3", usage)?),
        out,
    )?;
    fs::create_dir_all(out)?;
    write_json(&out.join("report.json"), &report)?;
    println!(
        "mutation run: {} mutants, {} survivors, {} rejected, {} compile errors, {} timeouts, {} unknown",
        report.summary.mutants,
        report.summary.verified_survivors,
        report.summary.rejected,
        report.summary.compile_errors,
        report.summary.timeouts,
        report.summary.unknown,
    );
    for result in &report.results {
        println!("{}: {}", result.id, result.final_status);
    }
    if mode == "strict"
        && (report.summary.verified_survivors > 0
            || report.summary.timeouts > 0
            || report.summary.unknown > 0)
    {
        return Err(format!(
            "strict mode: {} survivors, {} timeouts, {} unknown; report kept at {}",
            report.summary.verified_survivors,
            report.summary.timeouts,
            report.summary.unknown,
            out.display()
        )
        .into());
    }
    Ok(())
}

fn triage(args: &[String]) -> Result<()> {
    let usage =
        "mutation triage --manifest FILE --results FILE --source FILE --out FILE [--registry FILE]";
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{usage}");
        return Ok(());
    }
    let (map, _) = parse(
        args,
        &["--manifest", "--results", "--source", "--out", "--registry"],
        &[],
        usage,
    )?;
    let report = mutation_triage::run(
        Path::new(value(&map, "--manifest", usage)?),
        Path::new(value(&map, "--results", usage)?),
        Path::new(value(&map, "--source", usage)?),
        map.get("--registry").map(Path::new),
        Path::new(value(&map, "--out", usage)?),
    )?;
    println!("mutation triage: {} survivors", report.survivors);
    for (route, count) in &report.routes {
        println!("  {route}: {count}");
    }
    Ok(())
}

/// Dispatch the mutation CLI surface. `None` means "not a mutation command".
pub fn dispatch(args: &[String]) -> Option<Result<()>> {
    let first = args.first()?.as_str();
    if !matches!(
        first,
        "mutation" | "mutation-generate" | "analyses-mutation" | "mutation-triage"
    ) {
        return None;
    }
    let (command, rest) = head(args);
    Some(match command {
        Command::Help => {
            println!("{TOP_HELP}");
            Ok(())
        }
        Command::Generate => generate(rest),
        Command::Run => run(rest),
        Command::Triage => triage(rest),
    })
}
