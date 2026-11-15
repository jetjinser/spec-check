//! Executable model of page table vs TLB for internal reasoning only.
//! NOT the kernel and NOT a proof; it makes the counterexample concrete by
//! keeping the page table (PT) and the cached TLB mappings separate.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mapping { va_start: u64, va_end: u64, pa: u64, page_size: u64 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op { All, Address(u64), Range(u64, u64) }

fn covers(op: Op, m: Mapping) -> bool {
    match op {
        Op::All => true,
        Op::Address(va) => m.va_start <= va && va < m.va_end,
        Op::Range(r0, r1) => m.va_start < r1 && r0 < m.va_end,
    }
}

fn apply_op(mappings: &[Mapping], op: Op) -> Vec<Mapping> {
    mappings.iter().copied().filter(|m| !covers(op, *m)).collect()
}

fn apply_all(mappings: &[Mapping], pending: &[Op]) -> Vec<Mapping> {
    pending.iter().fold(mappings.to_vec(), |ms, op| apply_op(&ms, *op))
}

fn dispatch(tlb: &mut Vec<Mapping>, pending: &mut Vec<Op>) {
    *tlb = apply_all(tlb, pending);
    pending.clear();
}

/// Minimal `map` shape. The split substitutes 4K entries in the PT; the TLB
/// only changes if an address flush is issued on the split path.
fn map_with_split(
    pt: &mut Vec<Mapping>,
    tlb: &mut Vec<Mapping>,
    pending: &mut Vec<Op>,
    va: u64,
    new: Mapping,
    split: bool,
    flush_on_split: bool,
) {
    if let Some(old) = pt.iter().copied().find(|m| m.va_start <= va && va < m.va_end) {
        pt.retain(|m| *m != old);
        let mut p = old.va_start;
        while p < old.va_end {
            pt.push(Mapping { va_start: p, va_end: p + 4096, pa: old.pa + (p - old.va_start), page_size: 4096 });
            p += 4096;
        }
    }
    pt.retain(|m| !(m.va_start <= va && va < m.va_end));
    pt.push(new);
    if split && flush_on_split {
        pending.push(Op::Address(va));
    }
    dispatch(tlb, pending);
}

fn unmap_with_ops(
    pt: &mut Vec<Mapping>,
    tlb: &mut Vec<Mapping>,
    pending: &mut Vec<Op>,
    start: u64,
    end: u64,
    dispatch_all: bool,
) {
    let removed: Vec<Mapping> = pt
        .iter()
        .copied()
        .filter(|m| start <= m.va_start && m.va_start < end)
        .collect();
    for m in &removed {
        pt.retain(|x| x != m);
        pending.push(Op::Address(m.va_start));
    }
    if dispatch_all {
        dispatch(tlb, pending);
    }
}

fn main() {
    let huge = Mapping { va_start: 0, va_end: 2 * 1024 * 1024, pa: 0, page_size: 2 * 1024 * 1024 };
    let new0 = Mapping { va_start: 0, va_end: 4096, pa: 0x1000, page_size: 4096 };

    // 1. split-then-remap without the split flush: the PT is split/replaced but
    // the stale huge mapping remains cached.
    let mut pt = vec![huge];
    let mut tlb = vec![huge];
    let mut pending = vec![];
    map_with_split(&mut pt, &mut tlb, &mut pending, 0, new0, true, false);
    assert!(pt.iter().all(|m| *m != huge), "PT must be split");
    assert!(tlb.iter().any(|m| *m == huge), "without flush the huge TLB entry survives");

    // 2. with the split-gated flush, the stale huge entry is gone.
    let mut pt = vec![huge];
    let mut tlb = vec![huge];
    let mut pending = vec![];
    map_with_split(&mut pt, &mut tlb, &mut pending, 0, new0, true, true);
    assert!(!tlb.iter().any(|m| *m == huge), "split flush must remove the stale entry");

    // 3. a plain new mapping takes no flush and is inserted in the PT.
    let mut pt = vec![];
    let mut tlb = vec![];
    let mut pending = vec![];
    map_with_split(&mut pt, &mut tlb, &mut pending, 0, new0, false, true);
    assert!(pending.is_empty());
    assert!(pt.contains(&new0));

    // 4. unmap per-item ops + dispatch_all revokes the cached entries.
    let a = Mapping { va_start: 0, va_end: 4096, pa: 0, page_size: 4096 };
    let b = Mapping { va_start: 4096, va_end: 8192, pa: 4096, page_size: 4096 };
    let mut pt = vec![a, b];
    let mut tlb = vec![a, b];
    let mut pending = vec![];
    unmap_with_ops(&mut pt, &mut tlb, &mut pending, 0, 8192, true);
    assert!(pt.is_empty() && tlb.is_empty(), "unmap must revoke both layers");

    // 5. W2 shape: removing the PT entries without dispatch leaves the TLB.
    let mut pt = vec![a, b];
    let mut tlb = vec![a, b];
    let mut pending = vec![];
    unmap_with_ops(&mut pt, &mut tlb, &mut pending, 0, 8192, false);
    assert!(pt.is_empty(), "PT is updated");
    assert_eq!(tlb.len(), 2, "without dispatch the TLB keeps both entries");

    // 6. Range flush removes every intersecting mapping, not just start's.
    let tlb = vec![a, b];
    assert!(apply_op(&tlb, Op::Range(0, 8192)).is_empty());

    println!("tlb model tests passed");
}
