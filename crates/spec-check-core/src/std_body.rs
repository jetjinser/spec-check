//! Checked copies of pinned standard-library bodies unsupported by the SDK.
use crate::{Result, hash, ir};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub const HELPER: &str = "__spec_checked_option_map_or";
const TARGET: &str = "core::option::Option::<T>::map_or";

pub struct Copy {
    pub file: PathBuf,
    pub edits: Vec<ir::Edit>,
    pub evidence: Value,
}

pub fn index_propose(records: &[ir::Record], cwd: &Path) -> Result<Option<Copy>> {
    let indices = records
        .iter()
        .filter(|r| r["kind"] == "source_index")
        .collect::<Vec<_>>();
    if indices.is_empty() {
        return Ok(None);
    }
    let mut file = None;
    let mut edits = Vec::new();
    let mut evidence = Vec::new();
    for index in indices {
        let (path, coordinates) = ir::span(&index["span"], cwd)?;
        if file.as_ref().is_some_and(|f| f != &path) {
            return Err("index candidates span source files".into());
        }
        let source = fs::read_to_string(&path)?;
        let (start, end) = ir::offsets(&source, coordinates)?;
        let terms = records
            .iter()
            .filter(|r| {
                if r["kind"] != "term" {
                    return false;
                }
                let Ok((p, c)) = ir::span(&r["span"], cwd) else {
                    return false;
                };
                let Ok((a, b)) = ir::offsets(&source, c) else {
                    return false;
                };
                p == path
                    && a >= start
                    && a < end
                    && b == end
                    && records.iter().any(|call| {
                        call["kind"] == "call"
                            && call["block"] == r["block"]
                            && call["target"] == index["target"]
                    })
            })
            .collect::<Vec<_>>();
        let [term] = terms.as_slice() else {
            return Err("index needs a unique MIR call location".into());
        };
        let instances = records
            .iter()
            .filter(|r| r["kind"] == "instance" && r["block"] == term["block"])
            .collect::<Vec<_>>();
        let [instance] = instances.as_slice() else {
            return Err("index needs a resolved Rustc instance".into());
        };
        if instance["target"]
            != "<std::collections::BTreeMap<K, V, A> as std::ops::Index<&Q>>::index"
        {
            return Err("index implementation has no checked body candidate".into());
        }
        let (library, signature, body, digest) = pinned_body(&instance["definition_span"], cwd)?;
        if signature != "fnindex(&self,key:&Q)->&V"
            || body != "{self.get(key).expect(\"noentryfoundforkey\")}"
        {
            return Err("standard BTreeMap index body changed".into());
        }
        let library_source = fs::read_to_string(&library)?;
        let (_, coordinates) = ir::span(&instance["definition_span"], cwd)?;
        let (_, signature_end) = ir::offsets(&library_source, coordinates)?;
        if !library_source[signature_end..]
            .trim_start()
            .starts_with("{\n        self.get(key).expect(\"no entry found for key\")\n    }")
        {
            return Err("standard index literal or body formatting changed".into());
        }
        let argument_records = records
            .iter()
            .filter(|r| r["kind"] == "source_argument" && r["call_span"] == index["span"])
            .collect::<Vec<_>>();
        if argument_records.len() != 2 {
            return Err("index operands are missing or ambiguous".into());
        }
        let args = argument_records
            .into_iter()
            .map(|r| Ok((r["index"].parse::<usize>()?, r)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        if args.len() != 2 || !args.contains_key(&0) || !args.contains_key(&1) {
            return Err("index needs two HIR operands".into());
        }
        let mut operands = Vec::new();
        for arg in args.values() {
            let (p, c) = ir::span(&arg["span"], cwd)?;
            let (a, b) = ir::offsets(&source, c)?;
            if p != path || a < start || b > end || a >= b {
                return Err("index operand outside its expression".into());
            }
            operands.push(source[a..b].to_owned());
        }
        if source.contains("__spec_checked_btree_index") || source.contains("__SpecCheckedIndex") {
            return Err("index helper name already present".into());
        }
        edits.push(ir::Edit {
            start,
            end,
            replacement: format!(
                "*({}).__spec_checked_btree_index({})",
                operands[0], operands[1]
            ),
            origin: index["span"].clone(),
        });
        evidence.push(
            json!({"expression":index,"operands":args,"instance":instance,
            "library":library,"library_sha256":digest,"source_body":body}),
        );
        file = Some(path);
    }
    let file = file.unwrap();
    let source = fs::read_to_string(&file)?;
    edits.push(ir::Edit {
        start: 0,
        end: 0,
        replacement: "#![feature(allocator_api)]\n".into(),
        origin: "generic standard-library allocator parameter".into(),
    });
    edits.push(ir::Edit {
        start: source.len(),
        end: source.len(),
        replacement: r#"
#[cfg(verus_keep_ghost)] use vstd::view::View as _;
vstd::prelude::verus! {
pub trait __SpecCheckedIndex<Idx> {
    type Output: ?Sized;
    spec fn checked_req(&self, index: &Idx) -> bool;
    spec fn checked_rel(&self, index: &Idx, value: &Self::Output) -> bool;
    #[track_caller]
    fn __spec_checked_btree_index(&self, key: Idx) -> (value: &Self::Output)
        requires self.checked_req(&key),
        ensures self.checked_rel(&key, value);
}
impl<K, Q: ?Sized, V, A> __SpecCheckedIndex<&Q> for std::collections::BTreeMap<K, V, A>
    where K: core::borrow::Borrow<Q> + Ord, Q: Ord, A: std::alloc::Allocator + Clone,
{
    type Output = V;
    open spec fn checked_req(&self, key: &&Q) -> bool {
        &&& vstd::laws_cmp::obeys_cmp::<K>()
        &&& vstd::std_specs::btree::borrowed_key_ordering_matches::<K, Q>()
        &&& vstd::std_specs::btree::contains_borrowed_key(self@, *key)
    }
    open spec fn checked_rel(&self, key: &&Q, value: &V) -> bool {
        vstd::std_specs::btree::maps_borrowed_key_to_value(self@, *key, *value)
    }
    #[inline(always)]
    #[track_caller]
    fn __spec_checked_btree_index(&self, key: &Q) -> (value: &V) {
        self.get(key).expect("no entry found for key")
    }
}
}
"#
        .into(),
        origin: "resolved standard BTreeMap Index implementation".into(),
    });
    Ok(Some(Copy {
        file,
        edits,
        evidence: json!({"copies":evidence,
        "status":"candidate_not_verified","new_trusted_declarations":0,
        "scope":"Generated body candidate; verification and complete client correspondence remain required."}),
    }))
}

pub fn index_correspondence(
    before: &[ir::Record],
    after: &[ir::Record],
    cwd: &Path,
    candidate: &Path,
) -> Result<Value> {
    let proposal = index_propose(before, cwd)?.ok_or("missing original index proposal")?;
    let expected = proposal.evidence["copies"]
        .as_array()
        .ok_or("missing index copies")?
        .len();
    let canonical = |records: &[ir::Record], generated: bool| -> Result<Vec<ir::Record>> {
        let calls = records
            .iter()
            .filter(|r| {
                r["kind"] == "call"
                    && if generated {
                        r["target"].ends_with("__SpecCheckedIndex::__spec_checked_btree_index")
                    } else {
                        r["target"] == "std::ops::Index::index"
                    }
            })
            .collect::<Vec<_>>();
        if calls.len() != expected {
            return Err("index call count differs from recorded copies".into());
        }
        let mut result = records.to_vec();
        for call in calls {
            let instances = records
                .iter()
                .filter(|r| r["kind"] == "instance" && r["block"] == call["block"])
                .collect::<Vec<_>>();
            let [instance] = instances.as_slice() else {
                return Err("index instance is ambiguous".into());
            };
            if generated {
                let (file, signature, body, _) = pinned_body(&instance["definition_span"], cwd)?;
                if file.canonicalize()? != candidate.canonicalize()?
                    || signature != "fn__spec_checked_btree_index(&self,key:&Q)->(value:&V)"
                    || body != "{self.get(key).expect(\"noentryfoundforkey\")}"
                {
                    return Err("generated index implementation differs from copied body".into());
                }
                let source = fs::read_to_string(&file)?;
                let (_, coordinates) = ir::span(&instance["definition_span"], cwd)?;
                let (_, end) = ir::offsets(&source, coordinates)?;
                let start = end
                    + source[end..]
                        .find('{')
                        .ok_or("missing copied method body")?;
                if !source[start..].starts_with(
                    "{\n        self.get(key).expect(\"no entry found for key\")\n    }",
                ) {
                    return Err("generated index body literal changed".into());
                }
            }
            let shapes = records
                .iter()
                .filter(|r| r["kind"] == "call_shape" && r["block"] == call["block"])
                .collect::<Vec<_>>();
            let [shape] = shapes.as_slice() else {
                return Err("index call shape is ambiguous".into());
            };
            for r in &mut result {
                if r.get("block") == Some(&call["block"]) {
                    if r["kind"] == "call" {
                        r.insert("target".into(), "checked_std_btree_index".into());
                    }
                    if r["kind"] == "term" {
                        r.insert("operation".into(), shape["operation"].clone());
                    }
                }
            }
        }
        Ok(result)
    };
    let left = canonical(before, false)?;
    let right = canonical(after, true)?;
    Ok(
        json!({"matches":ir::opaque_signature(&left)? == ir::opaque_signature(&right)? && ir::compare(&left, &right, None)?,
        "copies":expected,"scope":"Pinned standard Index body copied as a trait method; only call identities are mapped. Arguments, type arguments, MIR statements, branches and borrow events use the unchanged comparator. Requires separate candidate verification and source integrity checks."}),
    )
}

// These copies keep their original bodies. Their guarantees must be verified.
pub fn pointer_propose(
    records: &[ir::Record],
    cwd: &Path,
    independent_resource_control: bool,
) -> Result<Option<Copy>> {
    const ADDR: &str = "core::ptr::NonNull::<T>::addr";
    let local_name = regex::Regex::new(r"^[A-Za-z_]\w*$")?;
    let calls = records
        .iter()
        .filter(|r| r["kind"] == "call" && matches!(r["target"].as_str(), ADDR | "core::mem::drop"))
        .collect::<Vec<_>>();
    if !calls.iter().any(|c| c["target"] == ADDR) {
        return Ok(None);
    }
    let decl = records
        .iter()
        .find(|r| r["kind"] == "decl")
        .ok_or("pointer copy needs source declaration")?;
    let (file, c) = ir::span(&decl["span"], cwd)?;
    let source = fs::read_to_string(&file)?;
    let (insertion, _) = ir::offsets(&source, c)?;
    if source.contains("__spec_checked_pointer_") || source.contains("checked_eq_failure") {
        return Err("pointer helper collision".into());
    }
    let range = |s: &str| -> Result<(usize, usize)> {
        let (p, c) = ir::span(s, cwd)?;
        if p != file {
            return Err("pointer copy spans source files".into());
        }
        ir::offsets(&source, c)
    };
    let mut edits = Vec::new();
    let mut evidence = Vec::new();
    let mut helpers = String::new();
    let mut emitted = std::collections::BTreeSet::new();
    for call in calls {
        let addr = call["target"] == ADDR;
        let (library, signature, body, sha) = pinned_body(&call["definition_span"], cwd)?;
        let expected = if addr {
            "pubfnaddr(self)->NonZero<usize>"
        } else {
            "pubconstfndrop<T>(_x:T)whereT:[const]Destruct,"
        };
        if signature != expected
            || body
                != if addr {
                    "{unsafe{NonZero::new_unchecked(self.as_ptr().addr())}}"
                } else {
                    "{}"
                }
        {
            return Err("pointer standard-library source changed".into());
        }
        let term = records
            .iter()
            .find(|r| r["kind"] == "term" && r["block"] == call["block"])
            .ok_or("missing std call location")?;
        let args = records
            .iter()
            .filter(|r| r["kind"] == "source_argument" && r["call_span"] == term["span"])
            .collect::<Vec<_>>();
        let [arg] = args.as_slice() else {
            return Err("pointer std operation needs one source argument".into());
        };
        let (a, b) = range(&term["span"])?;
        let (s, e) = range(&arg["span"])?;
        if !(a <= s && s < e && e <= b) {
            return Err("pointer std argument outside call".into());
        }
        let helper = if addr {
            "__spec_checked_pointer_addr"
        } else {
            "__spec_checked_pointer_drop"
        };
        edits.push(ir::Edit {
            start: a,
            end: b,
            replacement: format!("{helper}({})", &source[s..e]),
            origin: term["span"].clone(),
        });
        if emitted.insert(helper) {
            helpers.push_str(if addr {"\n#[vstd::prelude::verus_verify]\n#[verus_spec(ret => ensures ret.get() == ptr.view_ptr_mut().addr(),)]\nfn __spec_checked_pointer_addr<T>(ptr: core::ptr::NonNull<T>) -> core::num::NonZeroUsize {\nproof! { broadcast use vstd::raw_ptr::group_raw_ptr_axioms; }\nunsafe { core::num::NonZeroUsize::new_unchecked(ptr.as_ptr().addr()) }\n}\n"}else{"\n#[vstd::prelude::verus_verify]\nfn __spec_checked_pointer_drop<T>(_x: T) {}\n"});
        }
        evidence.push(json!({"target":call["target"],"definition":call["definition_span"],"library":library,"source_sha256":sha,"body":body,"helper":helper}));
    }
    let mut assertions = BTreeMap::<String, Vec<&ir::Record>>::new();
    for r in records.iter().filter(|r| {
        r["kind"] == "assert_operand"
            && matches!(r["macro"].as_str(), "core::assert_eq" | "std::assert_eq")
    }) {
        assertions
            .entry(r["assertion_span"].clone())
            .or_default()
            .push(r);
    }
    let inner = edits.clone();
    let macro_paths = records
        .iter()
        .filter_map(|r| r.get("span"))
        .filter_map(|s| ir::span(s, cwd).ok())
        .map(|(p, _)| p)
        .filter(|p| p.ends_with("core/src/macros/mod.rs"))
        .collect::<std::collections::BTreeSet<_>>();
    let macro_paths = macro_paths.into_iter().collect::<Vec<_>>();
    let [macro_path] = macro_paths.as_slice() else {
        return Err("one pinned core macro source required".into());
    };
    let macro_sha = hash(&fs::read(macro_path)?)?;
    if crate::assertion_rule::stamp()?["macro_sha256"] != macro_sha {
        return Err("pointer assertion macro source changed".into());
    }
    for (span, mut args) in assertions {
        args.sort_by_key(|r| &r["index"]);
        if args.len() != 2
            || args[0]["index"] != "0"
            || args[1]["index"] != "1"
            || args[0]["type"] != args[1]["type"]
        {
            return Err("pointer assertion structure unsupported".into());
        }
        let (a, b) = range(&span)?;
        let mut operands = Vec::new();
        let mut positions = Vec::new();
        for arg in &args {
            let (s, e) = range(&arg["span"])?;
            positions.push((s, e));
            let mut text = source[s..e].to_owned();
            let mut nested = inner
                .iter()
                .filter(|edit| edit.start >= s && edit.end <= e)
                .collect::<Vec<_>>();
            nested.sort_by_key(|e| e.start);
            for edit in nested.into_iter().rev() {
                text.replace_range(edit.start - s..edit.end - s, &edit.replacement);
            }
            operands.push(text);
        }
        let [(s, e), (u, v)] = positions[..] else {
            unreachable!()
        };
        let compact = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        if !(a < s && s < e && e < u && u < v && v < b)
            || !["assert_eq!(", "core::assert_eq!(", "std::assert_eq!("]
                .contains(&compact(&source[a..s]).as_str())
            || source[e..u].trim() != ","
            || ![")", ",)"].contains(&compact(&source[v..b]).as_str())
        {
            return Err("custom pointer assertion syntax unsupported".into());
        }
        let equality = if args[0]["type"] == "usize" {
            crate::assertion_rule::validate(records, &span)?;
            "*left_val == *right_val".to_owned()
        } else if args[0]["type"].starts_with("core::ptr::NonNull<") {
            // The primitive assertion rule deliberately excludes pointer equality.
            // Bound this separate copy to two Drop-free local-name operands.
            for arg in &args {
                let (s, e) = range(&arg["span"])?;
                let identifier = &source[s..e];
                if !local_name.is_match(identifier)
                    || records
                        .iter()
                        .filter(|r| {
                            r["kind"] == "binding"
                                && r["name"] == identifier
                                && r["type"] == arg["type"]
                        })
                        .count()
                        != 1
                    || !records.iter().any(|r| {
                        r["kind"] == "assert_domain"
                            && r["assertion_span"] == span
                            && r["index"] == arg["index"]
                            && r["drop_free"] == "true"
                            && r["direct_macro"] == "true"
                            && r["word_bits"] == "64"
                    })
                {
                    return Err(
                        "pointer equality copy requires direct Drop-free local operands".into(),
                    );
                }
            }
            let instances = records
                .iter()
                .filter(|r| {
                    r["kind"] == "instance"
                        && r["target"] == "<core::ptr::NonNull<T> as core::cmp::PartialEq>::eq"
                })
                .collect::<Vec<_>>();
            let [instance] = instances.as_slice() else {
                return Err("resolved NonNull equality instance required".into());
            };
            let (library, signature, body, sha) = pinned_body(&instance["definition_span"], cwd)?;
            if signature != "fneq(&self,other:&Self)->bool"
                || body != "{self.as_ptr()==other.as_ptr()}"
            {
                return Err("NonNull equality source changed".into());
            }
            if emitted.insert("__spec_checked_pointer_eq") {
                helpers.push_str("\n#[vstd::prelude::verus_verify]\n#[verus_spec(ret => ensures ret == (a.view_ptr_mut().addr() == b.view_ptr_mut().addr()),)]\nfn __spec_checked_pointer_eq<T>(a: &core::ptr::NonNull<T>, b: &core::ptr::NonNull<T>) -> bool { a.as_ptr() == b.as_ptr() }\n");
                evidence.push(json!({"target":instance["target"],"definition":instance["definition_span"],"library":library,"source_sha256":sha,"body":body,"helper":"__spec_checked_pointer_eq","return_guarantees":["ret == (a.view_ptr_mut().addr() == b.view_ptr_mut().addr())"]}));
            }
            "__spec_checked_pointer_eq(left_val, right_val)".into()
        } else {
            return Err("pointer observation equality type unsupported".into());
        };
        edits.retain(|edit| !(edit.start >= a && edit.end <= b));
        let replacement = if independent_resource_control {
            format!(
                "match (&({}), &({})) {{ (left_val, right_val) => {{ let _ = ({equality}); }} }}",
                operands[0], operands[1]
            )
        } else {
            crate::assertion_rule::instantiate(&operands[0], &operands[1])?.replacen(
                "*left_val == *right_val",
                &equality,
                1,
            )
        };
        edits.push(ir::Edit {
            start: a,
            end: b,
            replacement,
            origin: span,
        });
    }
    if !independent_resource_control {
        helpers.push_str(&format!(
            "\nvstd::prelude::verus! {{\n{}\n}}\n",
            include_str!("../../../experiments/test-spec/assertion_failure.rs")
        ));
    }
    if independent_resource_control {
        for r in records.iter().filter(|r| {
            r["kind"] == "assert_operand"
                && matches!(r["macro"].as_str(), "core::assert" | "std::assert")
        }) {
            if r["index"] != "0" || r["type"] != "bool" {
                return Err("resource control requires bool assertions".into());
            }
            let (a, b) = range(&r["assertion_span"])?;
            let (s, e) = range(&r["span"])?;
            if !(a < s && s < e && e < b) || source[e..b].trim() != ")" {
                return Err("custom bool assertion unsupported in resource control".into());
            }
            edits.push(ir::Edit {
                start: a,
                end: b,
                replacement: format!("let _ = ({})", &source[s..e]),
                origin: r["assertion_span"].clone(),
            });
        }
    }
    edits.push(ir::Edit {
        start: insertion,
        end: insertion,
        replacement: helpers,
        origin: decl["span"].clone(),
    });
    Ok(Some(Copy {
        file,
        edits,
        evidence: json!({"copies":evidence,"failure_helper":if independent_resource_control{None}else{Some("checked_eq_failure")},"macro_sha256":macro_sha,"independent_resource_control":independent_resource_control,"scope":"Runtime sized NonNull operations and generic drop; copied helpers require Verus checks. Equality observations preserve operand evaluation and abstract assertion failure; panic payload correspondence and const contexts are excluded. The independent resource control evaluates observations without asserting or assuming them."}),
    }))
}

pub(crate) fn pinned_body(span: &str, cwd: &Path) -> Result<(PathBuf, String, String, String)> {
    let (file, c) = ir::span(span, cwd)?;
    let source = fs::read_to_string(&file)?;
    let (a, b) = ir::offsets(&source, c)?;
    let start = source[b..].find('{').ok_or("std body missing")? + b;
    let mut depth = 0;
    let mut end = None;
    for (i, ch) in source[start..].char_indices() {
        if ch == '{' {
            depth += 1
        }
        if ch == '}' {
            depth -= 1;
            if depth == 0 {
                end = Some(start + i + 1);
                break;
            }
        }
    }
    let end = end.ok_or("std body unclosed")?;
    let compact = |s: &str| {
        s.lines()
            .map(|l| l.split_once("//").map_or(l, |(s, _)| s))
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
    };
    Ok((
        file,
        compact(&source[a..start]),
        compact(&source[start..end]),
        hash(source.as_bytes())?,
    ))
}

pub fn propose(records: &[ir::Record], cwd: &Path) -> Result<Option<Copy>> {
    let methods = records
        .iter()
        .filter(|r| r["kind"] == "source_method" && r["target"] == TARGET)
        .collect::<Vec<_>>();
    if methods.is_empty() {
        return Ok(None);
    }
    let mut edits = Vec::new();
    let mut evidence = Vec::new();
    let mut source_file = None;
    for method in methods {
        let args = records
            .iter()
            .filter(|r| r["kind"] == "source_argument" && r["call_span"] == method["span"])
            .map(|r| Ok((r["index"].parse::<usize>()?, r)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        if args.len() != 3
            || args[&0]["type"] != "core::option::Option<usize>"
                && args[&0]["type"] != "std::option::Option<usize>"
            || args[&1]["type"] != "usize"
        {
            return Err("std body copy currently requires Option<usize> and usize default".into());
        }
        let identity = records
            .iter()
            .filter(|r| r["kind"] == "identity_closure" && r["span"] == args[&2]["span"])
            .collect::<Vec<_>>();
        if !matches!(identity.as_slice(), [r] if r["binding_mode"]=="BindingMode(No, Not)" && r["type"].ends_with(", ()])"))
        {
            return Err("std body copy requires a noncapturing by-value identity callback".into());
        }
        let calls = records
            .iter()
            .filter(|r| r["kind"] == "call" && r["target"] == TARGET)
            .collect::<Vec<_>>();
        let [call] = calls.as_slice() else {
            return Err("ambiguous std body source call".into());
        };
        let (library, coords) = ir::span(&call["definition_span"], cwd)?;
        let library_source = fs::read_to_string(&library)?;
        let (definition, signature_end) = ir::offsets(&library_source, coords)?;
        let signature = &library_source[definition..signature_end];
        if !signature.starts_with("pub const fn map_or<U, F>(self, default: U, f: F) -> U") {
            return Err("std map_or signature changed".into());
        }
        let start = signature_end
            + library_source[signature_end..]
                .find('{')
                .ok_or("missing std body")?;
        let mut depth = 0;
        let mut end = None;
        for (i, c) in library_source[start..].char_indices() {
            if c == '{' {
                depth += 1;
            }
            if c == '}' {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + i + 1);
                    break;
                }
            }
        }
        let body = &library_source[start..end.ok_or("unclosed std body")?];
        if body
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            != "{matchself{Some(t)=>f(t),None=>default,}}"
        {
            return Err("std map_or body changed".into());
        }
        let (file, coords) = ir::span(&method["span"], cwd)?;
        if source_file.as_ref().is_some_and(|p| p != &file) {
            return Err("std copies span source files".into());
        }
        source_file = Some(file.clone());
        let source = fs::read_to_string(&file)?;
        if source.contains(HELPER) {
            return Err("reserved std helper already present".into());
        }
        let (a, b) = ir::offsets(&source, coords)?;
        let mut arguments = Vec::new();
        for index in 0..3 {
            let (p, c) = ir::span(&args[&index]["span"], cwd)?;
            let (s, e) = ir::offsets(&source, c)?;
            if p != file || s < a || e > b {
                return Err("std argument lies outside call".into());
            }
            arguments.push(format!("({})", &source[s..e]));
        }
        edits.push(ir::Edit {
            start: a,
            end: b,
            replacement: format!("{HELPER}({})", arguments.join(", ")),
            origin: method["span"].clone(),
        });
        let copied = body.replacen("self", "option", 1);
        let helper = format!(
            "\n#[cfg(verus_keep_ghost)] use vstd::prelude::FnWithRequiresEnsures as _;\nvstd::prelude::verus! {{\nfn {HELPER}<T,U,F:FnOnce(T)->U>(option:Option<T>,default:U,f:F)->U\nrequires option matches Some(value) ==> f.requires((value,)),\n{copied}\n}}\n"
        );
        edits.push(ir::Edit {
            start: source.len(),
            end: source.len(),
            replacement: helper.clone(),
            origin: call["definition_span"].clone(),
        });
        evidence.push(json!({"method":method,"arguments":args,"identity_callback":identity[0],"library":library,"library_sha256":hash(library_source.as_bytes())?,"source_file":file,"source_body":body,"copied_body":copied,"helper_source":helper,"helper":HELPER,"return_guarantees":[],"scope":"Runtime Option<usize> with noncapturing identity callback; actual body copy, no assumed postcondition"}));
    }
    Ok(Some(Copy {
        file: source_file.unwrap(),
        edits,
        evidence: json!(evidence),
    }))
}

pub fn validate(copies: &BTreeMap<String, Value>, package: &Path, snapshot: &Path) -> Result<()> {
    for entry in copies
        .values()
        .flat_map(|v| v.as_array().into_iter().flatten())
    {
        let file = Path::new(
            entry["source_file"]
                .as_str()
                .ok_or("missing std copy source")?,
        );
        let source = fs::read_to_string(snapshot.join(file.strip_prefix(package)?))?;
        let helper = entry["helper_source"]
            .as_str()
            .ok_or("missing copied helper source")?;
        if source.matches(helper).count() != 1 {
            return Err("copied std implementation changed or missing".into());
        }
        let library = fs::read(entry["library"].as_str().ok_or("missing std source")?)?;
        if hash(&library)? != entry["library_sha256"] {
            return Err("std source changed".into());
        }
    }
    Ok(())
}

pub fn correspondence_input(records: &[ir::Record], generated: bool) -> Result<Vec<ir::Record>> {
    let mut result = records.to_vec();
    let blocks = records
        .iter()
        .filter(|r| {
            r["kind"] == "call"
                && if generated {
                    r["target"].ends_with(&format!("::{HELPER}"))
                } else {
                    r["target"] == TARGET
                }
        })
        .map(|r| r["block"].clone())
        .collect::<Vec<_>>();
    for block in blocks {
        let shapes = records
            .iter()
            .filter(|r| r["kind"] == "call_shape" && r["block"] == block)
            .collect::<Vec<_>>();
        let [shape] = shapes.as_slice() else {
            return Err("missing unique compiler call shape".into());
        };
        for r in &mut result {
            if r.get("block") == Some(&block) {
                if r["kind"] == "call" {
                    r.insert("target".into(), "checked_std_option_map_or".into());
                }
                if r["kind"] == "term" {
                    r.insert("operation".into(), shape["operation"].clone());
                }
            }
        }
    }
    Ok(result)
}

pub fn pointer_correspondence_input(
    records: &[ir::Record],
    generated: bool,
) -> Result<Vec<ir::Record>> {
    let mut result = records.to_vec();
    for call in records.iter().filter(|r| r["kind"] == "call") {
        let target = &call["target"];
        let kind = if if generated {
            target.ends_with("::__spec_checked_pointer_addr")
        } else {
            target == "core::ptr::NonNull::<T>::addr"
        } {
            "addr"
        } else if if generated {
            target.ends_with("::__spec_checked_pointer_drop")
        } else {
            target == "core::mem::drop"
        } {
            "drop"
        } else if if generated {
            target.ends_with("::__spec_checked_pointer_eq")
        } else {
            target == "core::cmp::PartialEq::eq"
                && records.iter().any(|r| {
                    r["kind"] == "instance"
                        && r["block"] == call["block"]
                        && r["target"] == "<core::ptr::NonNull<T> as core::cmp::PartialEq>::eq"
                })
        } {
            "eq"
        } else {
            continue;
        };
        let shapes = records
            .iter()
            .filter(|r| r["kind"] == "call_shape" && r["block"] == call["block"])
            .collect::<Vec<_>>();
        let [shape] = shapes.as_slice() else {
            return Err("pointer copy call shape missing or ambiguous".into());
        };
        let generic = if generated && kind == "eq" {
            let inner = call["generic_args"]
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .ok_or("pointer copy generic arguments missing")?;
            format!("[core::ptr::NonNull<{inner}>, core::ptr::NonNull<{inner}>]")
        } else {
            call["generic_args"].clone()
        };
        for r in result
            .iter_mut()
            .filter(|r| r.get("block") == Some(&call["block"]))
        {
            if r["kind"] == "call" {
                r.insert("target".into(), format!("checked_pointer_{kind}"));
                r.insert("generic_args".into(), generic.clone());
            }
            if r["kind"] == "term" {
                r.insert("operation".into(), shape["operation"].clone());
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires AUTO_INDEX_RUN from an actual source-function candidate run"]
    fn actual_index_candidate_preserves_original_and_rejects_ambiguity() {
        let run = PathBuf::from(std::env::var("AUTO_INDEX_RUN").unwrap());
        let report: Value =
            serde_json::from_slice(&fs::read(run.join("report.json")).unwrap()).unwrap();
        let result = &report["cases"][1]["result"]["check"];
        assert_eq!(result["verification"]["summary"], json!([0, 1]));
        assert_eq!(
            result["index_proposal"]["verification"]["summary"],
            json!([2, 0])
        );
        assert_eq!(result["index_proposal"]["new_trusted_declarations"], 0);
        assert_eq!(
            result["index_proposal"]["status"],
            "candidate_verified_with_bounded_correspondence"
        );
        assert_eq!(
            result["index_proposal"]["runtime_comparison"]["matches"],
            false
        );
        let capture: Value =
            serde_json::from_slice(&fs::read(run.join("case-1/runtime.json")).unwrap()).unwrap();
        let records: Vec<ir::Record> = serde_json::from_value(capture["records"].clone()).unwrap();
        let copy = index_propose(&records, Path::new("/")).unwrap().unwrap();
        let original = fs::read(&copy.file).unwrap();
        assert_eq!(
            result["index_proposal"]["runtime_comparison"]["body_substitution"]["matches"],
            true
        );
        let generated: Value =
            serde_json::from_slice(&fs::read(run.join("case-1/index-check/runtime.json")).unwrap())
                .unwrap();
        let after: Vec<ir::Record> = serde_json::from_value(generated["records"].clone()).unwrap();
        let instance = after
            .iter()
            .find(|r| r["kind"] == "instance" && r["target"].contains("__SpecCheckedIndex"))
            .unwrap();
        let (candidate, _) = ir::span(&instance["definition_span"], Path::new("/")).unwrap();
        assert_eq!(
            index_correspondence(&records, &after, Path::new("/"), &candidate).unwrap()["matches"],
            true
        );
        let mut changed = after.clone();
        changed
            .iter_mut()
            .find(|r| r["kind"] == "loan")
            .unwrap()
            .insert("borrowed_place".into(), "_999".into());
        assert_eq!(
            index_correspondence(&records, &changed, Path::new("/"), &candidate).unwrap()["matches"],
            false
        );
        let mut omitted = after.clone();
        let position = omitted
            .iter()
            .position(|r| r["kind"] == "loan_issued")
            .unwrap();
        omitted.remove(position);
        assert_eq!(
            index_correspondence(&records, &omitted, Path::new("/"), &candidate).unwrap()["matches"],
            false
        );
        let control =
            std::env::temp_dir().join(format!("index-body-control-{}", std::process::id()));
        fs::create_dir(&control).unwrap();
        let changed_body = control.join("candidate.rs");
        fs::write(
            &changed_body,
            fs::read_to_string(&candidate)
                .unwrap()
                .replace("no entry found for key", "no entry found for kei"),
        )
        .unwrap();
        let mut relocated = after.clone();
        for r in relocated
            .iter_mut()
            .filter(|r| r["kind"] == "instance" && r["target"].contains("__SpecCheckedIndex"))
        {
            r.insert(
                "definition_span".into(),
                r["definition_span"]
                    .replace(candidate.to_str().unwrap(), changed_body.to_str().unwrap()),
            );
        }
        assert!(index_correspondence(&records, &relocated, Path::new("/"), &changed_body).is_err());
        fs::remove_dir_all(control).unwrap();
        let mut missing = records.clone();
        missing.retain(|r| r["kind"] != "instance");
        assert!(index_propose(&missing, Path::new("/")).is_err());
        let index = records
            .iter()
            .find(|r| r["kind"] == "source_index")
            .unwrap();
        let operand = records
            .iter()
            .find(|r| r["kind"] == "source_argument" && r["call_span"] == index["span"])
            .unwrap();
        let mut ambiguous = records.clone();
        ambiguous.push(operand.clone());
        assert!(index_propose(&ambiguous, Path::new("/")).is_err());
        assert_eq!(fs::read(copy.file).unwrap(), original);
    }
    #[test]
    #[ignore = "requires actual pointer capture in STD_POINTER_RECORDS and STD_POINTER_COMMAND"]
    fn actual_pointer_copies_require_native_source_and_instance() {
        let command: Value = serde_json::from_slice(
            &fs::read(std::env::var("STD_POINTER_COMMAND").unwrap()).unwrap(),
        )
        .unwrap();
        let cwd = Path::new(command["cwd"].as_str().unwrap());
        let raw = fs::read_to_string(std::env::var("STD_POINTER_RECORDS").unwrap()).unwrap();
        let bodies = crate::client::borrow_records(&raw).unwrap();
        let inputs = bodies.values().collect::<Vec<_>>();
        let [records] = inputs.as_slice() else {
            panic!("one source caller required")
        };
        let full = pointer_propose(records, cwd, false).unwrap().unwrap();
        let control = pointer_propose(records, cwd, true).unwrap().unwrap();
        assert_eq!(full.evidence["copies"].as_array().unwrap().len(), 3);
        assert_eq!(full.evidence["independent_resource_control"], false);
        assert_eq!(control.evidence["independent_resource_control"], true);
        assert!(
            full.edits
                .iter()
                .any(|e| e.replacement.contains("checked_eq_failure"))
        );
        assert!(
            !control
                .edits
                .iter()
                .any(|e| e.replacement.contains("assert!"))
        );
        let absent = records
            .iter()
            .filter(|r| r["kind"] != "instance")
            .cloned()
            .collect::<Vec<_>>();
        assert!(pointer_propose(&absent, cwd, false).is_err());
        let addr = records
            .iter()
            .find(|r| r["kind"] == "call" && r["target"] == "core::ptr::NonNull::<T>::addr")
            .unwrap();
        let mut wrong = (*records).clone();
        for r in wrong.iter_mut().filter(|r| {
            r["kind"] == "instance"
                && r["target"] == "<core::ptr::NonNull<T> as core::cmp::PartialEq>::eq"
        }) {
            r.insert("definition_span".into(), addr["definition_span"].clone());
        }
        assert!(pointer_propose(&wrong, cwd, false).is_err());
    }
    use super::*;
    #[test]
    #[ignore = "requires STD_BODY_CASE and STD_BODY_CONTEXT from an actual copied-body run"]
    fn actual_source_and_copy_guards() {
        let case = PathBuf::from(std::env::var_os("STD_BODY_CASE").unwrap());
        let context: crate::capture::Invocation = serde_json::from_slice(
            &fs::read(std::env::var_os("STD_BODY_CONTEXT").unwrap()).unwrap(),
        )
        .unwrap();
        let selected = fs::read_to_string(case.join("selection.txt")).unwrap();
        let bodies = crate::client::borrow_records(
            &fs::read_to_string(case.join("original.stderr")).unwrap(),
        )
        .unwrap();
        let records = &bodies[&selected];
        let copy = propose(records, &context.cwd).unwrap().unwrap();
        let copies = BTreeMap::from([(selected, copy.evidence)]);
        let package = Path::new(&context.env["CARGO_MANIFEST_DIR"]);
        validate(&copies, package, &case.join("package")).unwrap();
        let call_span = &records
            .iter()
            .find(|r| r["kind"] == "source_method" && r["target"] == TARGET)
            .unwrap()["span"];
        for kind in ["source_argument", "identity_closure"] {
            let mut changed = records.clone();
            let record = changed
                .iter_mut()
                .find(|r| {
                    r["kind"] == kind
                        && (kind != "source_argument"
                            || r["index"] == "1" && r["call_span"] == *call_span)
                })
                .unwrap();
            record.insert(
                if kind == "source_argument" {
                    "type"
                } else {
                    "binding_mode"
                }
                .into(),
                "changed".into(),
            );
            assert!(propose(&changed, &context.cwd).is_err());
        }
        let out = std::env::temp_dir().join(format!("std-body-guard-{}", std::process::id()));
        fs::create_dir(&out).unwrap();
        let library = &copies.values().next().unwrap()[0]["library"];
        let library = library.as_str().unwrap();
        let modified = out.join("option.rs");
        let source = fs::read_to_string(library).unwrap();
        assert!(source.contains("Some(t) => f(t)"));
        fs::write(
            &modified,
            source.replace("Some(t) => f(t)", "Some(t) => default"),
        )
        .unwrap();
        let mut changed = records.clone();
        for r in &mut changed {
            if r["kind"] == "call" && r["target"] == TARGET {
                r.insert(
                    "definition_span".into(),
                    r["definition_span"].replace(library, modified.to_str().unwrap()),
                );
            }
        }
        assert!(propose(&changed, &context.cwd).is_err());
        let snapshot = out.join("package");
        crate::runner::copy_package(&case.join("package"), &snapshot).unwrap();
        let path = snapshot.join(copy.file.strip_prefix(package).unwrap());
        let source = fs::read_to_string(&path).unwrap();
        fs::write(
            &path,
            source.replace("Some(t) => f(t)", "Some(t) => default"),
        )
        .unwrap();
        assert!(validate(&copies, package, &snapshot).is_err());
        fs::remove_dir_all(out).unwrap();
    }
}
