//! Conservative recognition of literal `usize` tables and their simple loops.

use crate::{Result, ir};
use regex::Regex;
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ArrayLiteral {
    pub name: String,
    pub place: String,
    pub elements: Vec<usize>,
    /// Rustc source span string for the initializer expression.
    pub span: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IndexLoop {
    pub index: String,
    pub array_name: String,
    /// Rustc span for the range expression (`0..array.len()`).
    pub range_span: String,
    /// Rustc span for the complete `for` expression.
    pub span: String,
}

fn slice<'a>(span: &str, cwd: &Path, cache: &'a BTreeMap<String, String>) -> Result<&'a str> {
    let (path, coords) = ir::span(span, cwd)?;
    let text = cache
        .get(&path.to_string_lossy().into_owned())
        .ok_or("source file not cached")?;
    let (start, end) = ir::offsets(text, coords)?;
    text.get(start..end)
        .ok_or_else(|| "invalid source slice".into())
}

fn source_cache(records: &[ir::Record], cwd: &Path) -> Result<BTreeMap<String, String>> {
    let mut cache = BTreeMap::new();
    for record in records {
        if let Some(span) = record.get("span")
            && let Ok((path, _)) = ir::span(span, cwd)
        {
            let key = path.to_string_lossy().into_owned();
            if let std::collections::btree_map::Entry::Vacant(entry) = cache.entry(key) {
                entry.insert(fs::read_to_string(path)?);
            }
        }
    }
    Ok(cache)
}

fn literal_values(source: &str, mir: &str, length: usize) -> Option<Vec<usize>> {
    let literal_re = Regex::new(
        r"^\[\s*([0-9][0-9_]*)(?:\s*usize)?(?:\s*,\s*([0-9][0-9_]*)(?:\s*usize)?)*\s*,?\s*\]$",
    )
    .ok()?;
    if !literal_re.is_match(source.trim()) {
        return None;
    }
    let elem_re = Regex::new(r"([0-9][0-9_]*)(?:\s*usize)?").ok()?;
    let values = elem_re
        .captures_iter(source.trim())
        .map(|m| m[1].replace('_', "").parse::<usize>().ok())
        .collect::<Option<Vec<_>>>()?;
    let mir_elem_re = Regex::new(r"const ([0-9][0-9_]*)_usize").ok()?;
    let mir_values = mir_elem_re
        .captures_iter(mir)
        .map(|m| m[1].replace('_', "").parse::<usize>().ok())
        .collect::<Option<Vec<_>>>()?;
    (values.len() == length && values == mir_values).then_some(values)
}

/// Find directly initialized, immutable local `[usize; N_usize]` arrays.
/// Any uncertain syntax or evidence of mutation excludes that local.
pub(crate) fn arrays(records: &[ir::Record], cwd: &Path) -> Result<Vec<ArrayLiteral>> {
    let cache = source_cache(records, cwd)?;
    let bindings_by_place = records.iter().filter(|r| r["kind"] == "binding").fold(
        BTreeMap::<String, Vec<&ir::Record>>::new(),
        |mut map, record| {
            map.entry(record["place"].clone()).or_default().push(record);
            map
        },
    );
    let locals = records
        .iter()
        .filter(|r| r["kind"] == "local")
        .map(|r| (r["place"].as_str(), r["type"].as_str()))
        .collect::<BTreeMap<_, _>>();
    let type_re = Regex::new(r"^\[usize; ([0-9][0-9_]*)_usize\]$")?;
    let init_re = Regex::new(r"^(_[0-9]+) = (\[.*\])$")?;
    let decl_re = Regex::new(r"^\s*let\s+([A-Za-z_][A-Za-z0-9_]*)$")?;
    let mut found = Vec::new();
    for stmt in records.iter().filter(|r| r["kind"] == "stmt") {
        let Some(cap) = init_re.captures(&stmt["operation"]) else {
            continue;
        };
        let place = &cap[1];
        let Some(bindings) = bindings_by_place.get(place) else {
            continue;
        };
        if bindings.len() != 1 {
            continue;
        }
        let binding = bindings[0];
        let Some(ty) = locals.get(place) else {
            continue;
        };
        let Some(type_cap) = type_re.captures(ty) else {
            continue;
        };
        let length = type_cap[1].replace('_', "").parse::<usize>()?;
        let span = &stmt["span"];
        let text = slice(span, cwd, &cache)?.trim();
        let Some(values) = literal_values(text, &cap[2], length) else {
            continue;
        };
        let name = &binding["name"];
        let (binding_path, binding_coords) = ir::span(&binding["span"], cwd)?;
        let binding_source = cache
            .get(&binding_path.to_string_lossy().into_owned())
            .ok_or("binding source not cached")?;
        let (binding_start, binding_end) = ir::offsets(binding_source, binding_coords)?;
        let line_start = binding_source[..binding_start]
            .rfind('\n')
            .map_or(0, |offset| offset + 1);
        let prefix = &binding_source[line_start..binding_end];
        let let_start = prefix.rfind("let ");
        let Some(let_start) = let_start else { continue };
        let prefix = &prefix[let_start..];
        let Some(decl_cap) = decl_re.captures(prefix) else {
            continue;
        };
        if decl_cap[1] != *name {
            continue;
        }
        // Reject projected/base writes and mutable borrows. The exact initializer is allowed.
        let write_re = Regex::new(&format!(
            r"^{}(?:\[[^]]*\]|\.[A-Za-z_][A-Za-z0-9_]*)?\s*=",
            regex::escape(place)
        ))?;
        let stmt_is_initializer = |candidate: &ir::Record| {
            candidate["kind"] == "stmt"
                && candidate["block"] == stmt["block"]
                && candidate["statement_index"] == stmt["statement_index"]
                && candidate["span"] == stmt["span"]
        };
        let place_prefix_re = Regex::new(&format!(r"^{}(?:$|[.\[(])", regex::escape(place)))?;
        let raw_mut_re = Regex::new(&format!(
            r"(?:&\s*(?:raw\s+)?mut\s+{}\b|addr_of_mut!\s*\(\s*{}\b|{}\s+as\s+\*mut\b)",
            regex::escape(name),
            regex::escape(name),
            regex::escape(name)
        ))?;
        let body = records.iter().find(|r| r["kind"] == "body");
        let raw_mut_use = body
            .and_then(|r| slice(&r["span"], cwd, &cache).ok())
            .is_some_and(|body| raw_mut_re.is_match(body));
        let mutated = raw_mut_use
            || records.iter().any(|r| {
                (r["kind"] == "stmt"
                    && !stmt_is_initializer(r)
                    && write_re.is_match(&r["operation"]))
                    || (r["kind"] == "loan"
                        && place_prefix_re.is_match(&r["borrowed_place"])
                        && r["borrow_kind"].starts_with("Mut"))
            });
        if mutated {
            continue;
        }
        found.push(ArrayLiteral {
            name: name.clone(),
            place: place.to_owned(),
            elements: values,
            span: span.clone(),
        });
    }
    found.sort_by(|a, b| a.name.cmp(&b.name).then(a.span.cmp(&b.span)));
    found.dedup();
    if found.iter().enumerate().any(|(i, array)| {
        found[..i]
            .iter()
            .any(|prior| prior.name == array.name || prior.place == array.place)
    }) {
        return Ok(Vec::new());
    }
    Ok(found)
}

/// Find `for i in 0..name.len()` loops that index a recognized immutable array.
pub(crate) fn index_loops(
    records: &[ir::Record],
    cwd: &Path,
    arrays: &[ArrayLiteral],
) -> Result<Vec<IndexLoop>> {
    let cache = source_cache(records, cwd)?;
    let range_re = Regex::new(r"^0\s*\.\.\s*([A-Za-z_][A-Za-z0-9_]*)\s*\.\s*len\s*\(\s*\)$")?;
    let for_re = Regex::new(r"^\s*for\s+([A-Za-z_][A-Za-z0-9_]*)\s+in\b")?;
    let mut found = Vec::new();
    for arg in records
        .iter()
        .filter(|r| r["kind"] == "source_argument" && r["type"] == "std::ops::Range<usize>")
    {
        let range = slice(&arg["span"], cwd, &cache)?.trim();
        let Some(cap) = range_re.captures(range) else {
            continue;
        };
        let array_name = &cap[1];
        if arrays.iter().filter(|a| a.name == array_name).count() != 1 {
            continue;
        }
        let Some(expr) = records.iter().find(|r| {
            r["kind"] == "source_expr"
                && r["expression_kind"] == "loop"
                && r["span"] == arg["call_span"]
        }) else {
            continue;
        };
        let loop_text = slice(&expr["span"], cwd, &cache)?;
        let Some(for_cap) = for_re.captures(loop_text) else {
            continue;
        };
        let index = &for_cap[1];
        let (loop_path, loop_coords) = ir::span(&expr["span"], cwd)?;
        let (loop_start, loop_end) = ir::offsets(
            cache
                .get(&loop_path.to_string_lossy().into_owned())
                .ok_or("loop source not cached")?,
            loop_coords,
        )?;
        let index_bindings = records
            .iter()
            .filter(|r| {
                if r["kind"] != "binding" || r["name"] != index || r["type"] != "usize" {
                    return false;
                }
                let Ok((path, coords)) = ir::span(&r["span"], cwd) else {
                    return false;
                };
                if path != loop_path {
                    return false;
                }
                let Some(source) = cache.get(&path.to_string_lossy().into_owned()) else {
                    return false;
                };
                let Ok((start, end)) = ir::offsets(source, coords) else {
                    return false;
                };
                start >= loop_start && end <= loop_end
            })
            .count();
        if index_bindings != 1 {
            continue;
        }
        found.push(IndexLoop {
            index: index.to_owned(),
            array_name: array_name.to_owned(),
            range_span: arg["span"].clone(),
            span: expr["span"].clone(),
        });
    }
    found.sort_by(|a, b| a.span.cmp(&b.span).then(a.range_span.cmp(&b.range_span)));
    found.dedup();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn row(pairs: &[(&str, &str)]) -> ir::Record {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn sample_records(source: &str, extra: Vec<ir::Record>) -> (Vec<ir::Record>, String) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "literal-recognition-{}-{}.rs",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, source).unwrap();
        let path = path.to_string_lossy().into_owned();
        let binding_start = source.find("input").unwrap() + 1;
        let init_start = source.find("[1usize").unwrap() + 1;
        let init_end = init_start + "[1usize, 2]".len();
        let records = vec![
            row(&[
                ("kind", "local"),
                ("place", "_1"),
                ("type", "[usize; 2_usize]"),
            ]),
            row(&[
                ("kind", "binding"),
                ("name", "input"),
                ("place", "_1"),
                ("type", "[usize; 2_usize]"),
                (
                    "span",
                    &format!("{path}:1:{binding_start}: 1:{} (#0)", binding_start + 5),
                ),
            ]),
            row(&[
                ("kind", "stmt"),
                ("block", "bb0"),
                ("statement_index", "0"),
                ("operation", "_1 = [const 1_usize, const 2_usize]"),
                ("span", &format!("{path}:1:{init_start}: 1:{init_end} (#0)")),
            ]),
            row(&[
                ("kind", "body"),
                ("span", &format!("{path}:1:1: 1:{} (#0)", source.len() + 1)),
                ("snippet", ""),
            ]),
        ]
        .into_iter()
        .chain(extra)
        .collect();
        (records, path)
    }

    #[test]
    fn literal_syntax_is_narrow() {
        assert_eq!(
            literal_values(
                "[0usize, 1, 2]",
                "[const 0_usize, const 1_usize, const 2_usize]",
                3
            ),
            Some(vec![0, 1, 2])
        );
        assert_eq!(
            literal_values(
                "[1_000usize, 2usize,]",
                "[const 1000_usize, const 2_usize]",
                2
            ),
            Some(vec![1000, 2])
        );
        for source in ["[N, 2]", "[1 + 1, 2]", "make_values()", "[1usize; 3]"] {
            assert_eq!(
                literal_values(source, "[const 1_usize, const 2_usize]", 2),
                None
            );
        }
    }

    #[test]
    fn arrays_require_immutable_unique_and_unaliased_bindings() {
        let src = "fn t() { let input = [1usize, 2];                    }";
        let (records, path) = sample_records(src, vec![]);
        assert_eq!(
            arrays(&records, Path::new("/")).unwrap()[0].elements,
            vec![1, 2]
        );
        fs::remove_file(path).unwrap();

        let src = "fn t() { let mut input = [1usize, 2];                 }";
        let (records, path) = sample_records(src, vec![]);
        assert!(arrays(&records, Path::new("/")).unwrap().is_empty());
        fs::remove_file(path).unwrap();

        let src = "fn t() { let input = [1usize, 2]; let input = [1usize, 2]; }";
        let (mut records, path) = sample_records(src, vec![]);
        let binding_start = src.rfind("input").unwrap() + 1;
        let init_start = src.rfind("[1usize").unwrap() + 1;
        let init_end = init_start + "[1usize, 2]".len();
        records.push(row(&[
            ("kind", "binding"),
            ("name", "input"),
            ("place", "_2"),
            ("type", "[usize; 2_usize]"),
            (
                "span",
                &format!("{path}:1:{binding_start}: 1:{} (#0)", binding_start + 5),
            ),
        ]));
        records.push(row(&[
            ("kind", "local"),
            ("place", "_2"),
            ("type", "[usize; 2_usize]"),
        ]));
        records.push(row(&[
            ("kind", "stmt"),
            ("block", "bb0"),
            ("statement_index", "1"),
            ("operation", "_2 = [const 1_usize, const 2_usize]"),
            ("span", &format!("{path}:1:{init_start}: 1:{init_end} (#0)")),
        ]));
        assert!(arrays(&records, Path::new("/")).unwrap().is_empty());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn mutable_loans_match_whole_places_and_raw_mutation_is_rejected() {
        let src = "fn t() { let input = [1usize, 2];                    }";
        let unrelated_loan = row(&[
            ("kind", "loan"),
            ("borrowed_place", "_10"),
            ("borrow_kind", "Mut { kind: Default }"),
            ("assigned_place", "_11"),
            ("location", "bb0[1]"),
            ("region", "'?0"),
            ("activation", "NotTwoPhase"),
        ]);
        let (records, path) = sample_records(src, vec![unrelated_loan]);
        assert_eq!(arrays(&records, Path::new("/")).unwrap().len(), 1);
        fs::remove_file(path).unwrap();

        let selected_loan = row(&[
            ("kind", "loan"),
            ("borrowed_place", "_1"),
            ("borrow_kind", "Mut { kind: Default }"),
            ("assigned_place", "_11"),
            ("location", "bb0[1]"),
            ("region", "'?0"),
            ("activation", "NotTwoPhase"),
        ]);
        let (records, path) = sample_records(src, vec![selected_loan]);
        assert!(arrays(&records, Path::new("/")).unwrap().is_empty());
        fs::remove_file(path).unwrap();

        let src = "fn t() { let input = [1usize, 2]; let p = &raw mut input; }";
        let (records, path) = sample_records(src, vec![]);
        assert!(arrays(&records, Path::new("/")).unwrap().is_empty());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn repeated_identical_initializer_is_still_a_write() {
        let src = "fn t() { let input = [1usize, 2];                    }";
        let (mut records, path) = sample_records(src, vec![]);
        let mut second_write = records
            .iter()
            .find(|r| r["kind"] == "stmt")
            .unwrap()
            .clone();
        second_write.insert("statement_index".into(), "1".into());
        records.push(second_write);
        assert!(arrays(&records, Path::new("/")).unwrap().is_empty());
        fs::remove_file(path).unwrap();
    }
}
