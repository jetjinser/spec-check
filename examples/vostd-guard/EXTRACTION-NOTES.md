# EXTRACTION-NOTES.md — 60 接口全量提取对照表（预注册，写于运行前）

生成：`extract_full_manifest.py`（本目录）；输入 `/home/jinser/vie/work/AES/research/experiments/spec-quality-two-day/guard-return-inventory.csv`（60 行）+ 冻结源 `/tmp/vostd-43da079-ktest-chan`（rev 43da0797bbfa）。**本文件在运行 analyses-vacuity 之前生成**，含每条目的预期判定（预注册）；运行结果只归档于 FULL-RUN-60.md，不改写本文件。

## 建模边界（沿用 M1/EXTRACTION.md 已验证纪律）

- tracked 参数（`Tracked<...>`）与非标量参数抽象为不透明 usize stand-in（`<name>_id`）；
- requires 中对标量/常数的比较与算术机械转写为具名 `open spec fn`（触发推断；verus spec 算术为数学语义）；
- ghost 结构子句（字段/方法访问、`old`/`final`、`is`、量词等）机械不可表达 → 具名 `uninterp spec fn`（保名字与参数形状，丢定义体），逐条记录抽象损失；被引用的标量参数/提升常数作为 uninterp fn 参数保留（M1 先例：`in_count_window(parent_level, idx)`）；
- 无 requires 子句的契约（ensures-only / 裸 `#[verus_spec]`）：探针 requires 编码为字面 `true`——入口前置按源文本平凡可满足；
- 见证策略：含机械子句的条目做 0..=7/bool 回溯搜索求见证（标量层可满足性的纸面论证；M1 修正后的策略：机器判定以探针运行为准）；
- 锚点：7 项已知恒假 stub（14 行 CSV，stub 与 _VERUS_VERIFIED 孪生共享 span）复用 M1 `manifest.json` 条目原文（锚点一致性对照）；构造探针与 quantifier 形状对照复用 `tracked/manifest.json` 条目原文；M1 三对照（2 阴性 1 阳性）原样保留；
- manifest 中 `covers_csv_rows`/`source_span`/`abstraction_notes` 为溯源元数据（runner serde 忽略未知字段）。

## 预注册判定规则（E 规则）

| 规则 | 条目形状 | 预期判定 |
|---|---|---|
| E1 | requires 为空合取 | ConfirmedLive |
| E2 | requires 全为 uninterp | CandidateInconclusive |
| E3 | 机械 + uninterp 混合 | CandidateInconclusive（M1 根因：witness assert 不可证） |
| E4 | 全机械且见证可满足 | ConfirmedLive |
| E5 | 全机械但搜索无见证 | CandidateInconclusive |
| E6 | 机械子句联立矛盾（恒假在标量层） | ConfirmedVacuous（预期不出现；出现即新证据，须复核） |

意外结局预案：任一条目 Anomaly → 分类器/触发推断缺陷排查（RESULT.md/spike 先例）；E1 条目非 Live → 探针生成器退化；阳性对照不符 → 全部结果作废。

## 提取覆盖统计

- 清单 60 行 = 锚点复用 14 行（7 个去重契约）+ 自动提取 15 行（15 个契约）+ extraction_skipped 31 行；
- manifest 共 28 条目 = 7 锚点 + 15 自动 + 3 M1 对照 + 3 构造/形状对照（tracked 复用）。

## 逐契约转写对照表

### row00_read_inner（CSV 行 0；ostd/src/mm/dma/dma_coherent.rs:352）
接口：ostd::mm::dma::dma_coherent::impl&%5::read_inner
预期判定：**ConfirmedLive** — E1: requires 为空合取，exists 见证平凡成立；forall 否定不可证
```json
{
 "params": "self_id: usize",
 "requires": "true",
 "witness": "0usize"
}
```
（requires 子句：无）
- contract has no requires clauses (empty conjunction); probe requires encoded as literal true — entry precondition trivially satisfiable by source text

### row01_read_inner（CSV 行 1；ostd/src/mm/dma/dma_stream.rs:715）
接口：ostd::mm::dma::dma_stream::impl&%7::read_inner
预期判定：**ConfirmedLive** — E1: requires 为空合取，exists 见证平凡成立；forall 否定不可证
```json
{
 "params": "self_id: usize",
 "requires": "true",
 "witness": "0usize"
}
```
（requires 子句：无）
- contract has no requires clauses (empty conjunction); probe requires encoded as literal true — entry precondition trivially satisfiable by source text

### try_traverse_and_lock_subtree_root_shape（CSV 行 2, 3；ostd/src/mm/page_table/cursor/locking.rs:384）
接口：ostd::mm::page_table::cursor::locking::_VERUS_VERIFIED_try_traverse_and_lock_subtree_root；ostd::mm::page_table::cursor::locking::try_traverse_and_lock_subtree_root
预期判定：**CandidateInconclusive** — M1 全量运行（FULL-RUN.md）已归档的机器判定，作为锚点一致性对照
转写：锚点复用 M1 `manifest.json` 条目原文（逐子句对照见 M1 EXTRACTION.md 对应章节），不重复转写。

### row04_path_slot_as_mut（CSV 行 4；ostd/src/mm/page_table/cursor/mod.rs:134）
接口：ostd::mm::page_table::cursor::path_slot_as_mut
预期判定：**CandidateInconclusive** — E3: 机械+uninterp 混合（M1 根因）： witness assert 无法穿过 uninterp 合取项，空洞探针亦不可证 → 双败
```json
{
 "params": "path_id: usize, idx: usize, nr_levels: usize",
 "requires": "c4_k0_lt_idx_nr_levels(idx, nr_levels) && c4_k1_path_idx_as(path_id, idx)",
 "witness": "(0usize, 0usize, 1usize)"
}
```
| # | 原子句（原文） | 转写 | 类型 |
|---|---|---|---|
| 0 | `idx < NR_LEVELS` | `c4_k0_lt_idx_nr_levels` := `idx < nr_levels` | 机械 |
| 1 | `old(path)[idx as int] is Some` | `c4_k1_path_idx_as(path_id, idx)` uninterp | uninterp |
- param path: &'a mut [Option<PageTableGuard<'rcu, C>>; NR_LEVELS] abstracted to path_id (non-scalar stand-in)
- config consts lifted to parameters: NR_LEVELS -> nr_levels
- requires clause 0 mechanical -> c4_k0_lt_idx_nr_levels := idx < nr_levels
- requires clause 1 untranscribable over plain params -> uninterp c4_k1_path_idx_as(path_id, idx); body abstracted

### alloc_if_none_shape（CSV 行 5, 7；ostd/src/mm/page_table/node/entry.rs:616）
接口：ostd::mm::page_table::node::entry::impl&%2::_VERUS_VERIFIED_alloc_if_none；ostd::mm::page_table::node::entry::impl&%2::alloc_if_none
预期判定：**CandidateInconclusive** — M1 全量运行（FULL-RUN.md）已归档的机器判定，作为锚点一致性对照
转写：锚点复用 M1 `manifest.json` 条目原文（逐子句对照见 M1 EXTRACTION.md 对应章节），不重复转写。

### split_if_mapped_huge_shape（CSV 行 6, 8；ostd/src/mm/page_table/node/entry.rs:968）
接口：ostd::mm::page_table::node::entry::impl&%2::_VERUS_VERIFIED_split_if_mapped_huge；ostd::mm::page_table::node::entry::impl&%2::split_if_mapped_huge
预期判定：**CandidateInconclusive** — M1 全量运行（FULL-RUN.md）已归档的机器判定，作为锚点一致性对照
转写：锚点复用 M1 `manifest.json` 条目原文（逐子句对照见 M1 EXTRACTION.md 对应章节），不重复转写。

### alloc_absent_child_shape（CSV 行 9, 10；ostd/src/mm/page_table/node/entry.rs:2141）
接口：ostd::mm::page_table::node::entry::impl&%3::_VERUS_VERIFIED_alloc_absent_child；ostd::mm::page_table::node::entry::impl&%3::alloc_absent_child
预期判定：**CandidateInconclusive** — M1 全量运行（FULL-RUN.md）已归档的机器判定，作为锚点一致性对照
转写：锚点复用 M1 `manifest.json` 条目原文（逐子句对照见 M1 EXTRACTION.md 对应章节），不重复转写。

### node_lock_shape（CSV 行 11, 13；ostd/src/mm/page_table/node/mod.rs:375）
接口：ostd::mm::page_table::node::impl&%2::_VERUS_VERIFIED_lock；ostd::mm::page_table::node::impl&%2::lock
预期判定：**CandidateInconclusive** — M1 全量运行（FULL-RUN.md）已归档的机器判定，作为锚点一致性对照
转写：锚点复用 M1 `manifest.json` 条目原文（逐子句对照见 M1 EXTRACTION.md 对应章节），不重复转写。

### make_guard_unchecked_shape（CSV 行 12, 14；ostd/src/mm/page_table/node/mod.rs:401）
接口：ostd::mm::page_table::node::impl&%2::_VERUS_VERIFIED_make_guard_unchecked；ostd::mm::page_table::node::impl&%2::make_guard_unchecked
预期判定：**CandidateInconclusive** — M1 全量运行（FULL-RUN.md）已归档的机器判定，作为锚点一致性对照
转写：锚点复用 M1 `manifest.json` 条目原文（逐子句对照见 M1 EXTRACTION.md 对应章节），不重复转写。

### row15_vm_space_cursor_embedded（CSV 行 15；ostd/src/../specs/mm/embedding/cursor.rs:90）
接口：ostd::specs::mm::embedding::cursor::vm_space_cursor_embedded
预期判定：**CandidateInconclusive** — E2: requires 全为 uninterp 谓词：活性无法见证、空洞无法证否定 → 双败
```json
{
 "params": "vm_space_id: usize, regions_id: usize",
 "requires": "c15_k0_vm_space_inv(vm_space_id) && c15_k1_regions_inv(regions_id)"
}
```
| # | 原子句（原文） | 转写 | 类型 |
|---|---|---|---|
| 0 | `vm_space.inv()` | `c15_k0_vm_space_inv(vm_space_id)` uninterp | uninterp |
| 1 | `old(regions).inv()` | `c15_k1_regions_inv(regions_id)` uninterp | uninterp |
- param vm_space: &VmSpaceOwner abstracted to vm_space_id (non-scalar stand-in)
- param regions: &mut MetaRegionOwners abstracted to regions_id (non-scalar stand-in)
- param va: Range<Vaddr> abstracted to va_id (non-scalar stand-in)
- requires clause 0 untranscribable over plain params -> uninterp c15_k0_vm_space_inv(vm_space_id); body abstracted
- requires clause 1 untranscribable over plain params -> uninterp c15_k1_regions_inv(regions_id); body abstracted

### row16_vm_space_cursor_mut_embedded（CSV 行 16；ostd/src/../specs/mm/embedding/cursor.rs:131）
接口：ostd::specs::mm::embedding::cursor::vm_space_cursor_mut_embedded
预期判定：**CandidateInconclusive** — E2: requires 全为 uninterp 谓词：活性无法见证、空洞无法证否定 → 双败
```json
{
 "params": "vm_space_id: usize, regions_id: usize",
 "requires": "c16_k0_vm_space_inv(vm_space_id) && c16_k1_regions_inv(regions_id)"
}
```
| # | 原子句（原文） | 转写 | 类型 |
|---|---|---|---|
| 0 | `vm_space.inv()` | `c16_k0_vm_space_inv(vm_space_id)` uninterp | uninterp |
| 1 | `old(regions).inv()` | `c16_k1_regions_inv(regions_id)` uninterp | uninterp |
- param vm_space: &VmSpaceOwner abstracted to vm_space_id (non-scalar stand-in)
- param regions: &mut MetaRegionOwners abstracted to regions_id (non-scalar stand-in)
- param va: Range<Vaddr> abstracted to va_id (non-scalar stand-in)
- requires clause 0 untranscribable over plain params -> uninterp c16_k0_vm_space_inv(vm_space_id); body abstracted
- requires clause 1 untranscribable over plain params -> uninterp c16_k1_regions_inv(regions_id); body abstracted

### row17_tracked_restore（CSV 行 17；ostd/src/../specs/mm/page_table/cursor/owners.rs:193）
接口：ostd::specs::mm::page_table::cursor::owners::impl&%0::tracked_restore
预期判定：**CandidateInconclusive** — E2: requires 全为 uninterp 谓词：活性无法见证、空洞无法证否定 → 双败
```json
{
 "params": "self_id: usize",
 "requires": "c17_k0_self_idx_children(self_id)"
}
```
| # | 原子句（原文） | 转写 | 类型 |
|---|---|---|---|
| 0 | `old(self).idx < old(self).children.len()` | `c17_k0_self_idx_children(self_id)` uninterp | uninterp |
- param child: Self abstracted to child_id (non-scalar stand-in)
- requires clause 0 untranscribable over plain params -> uninterp c17_k0_self_idx_children(self_id); body abstracted

### row25_try_lock（CSV 行 25；ostd/src/sync/mutex.rs:82）
接口：ostd::sync::mutex::impl&%3::try_lock
预期判定：**ConfirmedLive** — E1: requires 为空合取，exists 见证平凡成立；forall 否定不可证
```json
{
 "params": "self_id: usize",
 "requires": "true",
 "witness": "0usize"
}
```
（requires 子句：无）
- contract has no requires clauses (empty conjunction); probe requires encoded as literal true — entry precondition trivially satisfiable by source text

### mutex_guard_new_shape（CSV 行 26, 27；ostd/src/sync/mutex.rs:225）
接口：ostd::sync::mutex::impl&%7::_VERUS_VERIFIED_new；ostd::sync::mutex::impl&%7::new
预期判定：**ConfirmedLive** — M1 全量运行（FULL-RUN.md）已归档的机器判定，作为锚点一致性对照
转写：锚点复用 M1 `manifest.json` 条目原文（逐子句对照见 M1 EXTRACTION.md 对应章节），不重复转写。

### row29_read（CSV 行 29；ostd/src/sync/rcu/mod.rs:420）
接口：ostd::sync::rcu::impl&%6::read
预期判定：**ConfirmedLive** — E1: requires 为空合取，exists 见证平凡成立；forall 否定不可证
```json
{
 "params": "self_id: usize",
 "requires": "true",
 "witness": "0usize"
}
```
（requires 子句：无）
- contract has no requires clauses (empty conjunction); probe requires encoded as literal true — entry precondition trivially satisfiable by source text

### row37_try_read（CSV 行 37；ostd/src/sync/rwlock.rs:449）
接口：ostd::sync::rwlock::impl&%4::try_read
预期判定：**ConfirmedLive** — E1: requires 为空合取，exists 见证平凡成立；forall 否定不可证
```json
{
 "params": "self_id: usize",
 "requires": "true",
 "witness": "0usize"
}
```
（requires 子句：无）
- contract has no requires clauses (empty conjunction); probe requires encoded as literal true — entry precondition trivially satisfiable by source text

### row39_try_write（CSV 行 39；ostd/src/sync/rwlock.rs:507）
接口：ostd::sync::rwlock::impl&%4::try_write
预期判定：**ConfirmedLive** — E1: requires 为空合取，exists 见证平凡成立；forall 否定不可证
```json
{
 "params": "self_id: usize",
 "requires": "true",
 "witness": "0usize"
}
```
（requires 子句：无）
- contract has no requires clauses (empty conjunction); probe requires encoded as literal true — entry precondition trivially satisfiable by source text

### row44_try_upgrade（CSV 行 44；ostd/src/sync/rwmutex.rs:906）
接口：ostd::sync::rwmutex::impl&%21::try_upgrade
预期判定：**ConfirmedLive** — E1: requires 为空合取，exists 见证平凡成立；forall 否定不可证
```json
{
 "params": "self_id: usize",
 "requires": "true",
 "witness": "0usize"
}
```
（requires 子句：无）
- contract has no requires clauses (empty conjunction); probe requires encoded as literal true — entry precondition trivially satisfiable by source text

### row47_try_read（CSV 行 47；ostd/src/sync/rwmutex.rs:361）
接口：ostd::sync::rwmutex::impl&%4::try_read
预期判定：**ConfirmedLive** — E1: requires 为空合取，exists 见证平凡成立；forall 否定不可证
```json
{
 "params": "self_id: usize",
 "requires": "true",
 "witness": "0usize"
}
```
（requires 子句：无）
- contract has no requires clauses (empty conjunction); probe requires encoded as literal true — entry precondition trivially satisfiable by source text

### row52_lock（CSV 行 52；ostd/src/sync/spin.rs:270）
接口：ostd::sync::spin::impl&%5::lock
预期判定：**ConfirmedLive** — E1: requires 为空合取，exists 见证平凡成立；forall 否定不可证
```json
{
 "params": "self_id: usize",
 "requires": "true",
 "witness": "0usize"
}
```
（requires 子句：无）
- contract has no requires clauses (empty conjunction); probe requires encoded as literal true — entry precondition trivially satisfiable by source text

### row53_try_lock（CSV 行 53；ostd/src/sync/spin.rs:308）
接口：ostd::sync::spin::impl&%5::try_lock
预期判定：**ConfirmedLive** — E1: requires 为空合取，exists 见证平凡成立；forall 否定不可证
```json
{
 "params": "self_id: usize",
 "requires": "true",
 "witness": "0usize"
}
```
（requires 子句：无）
- contract has no requires clauses (empty conjunction); probe requires encoded as literal true — entry precondition trivially satisfiable by source text

### row59_get_freelist_guard（CSV 行 59；ostd/src/util/range_alloc.rs:602）
接口：ostd::util::range_alloc::impl&%4::get_freelist_guard
预期判定：**ConfirmedLive** — E1: requires 为空合取，exists 见证平凡成立；forall 否定不可证
```json
{
 "params": "self_id: usize",
 "requires": "true",
 "witness": "0usize"
}
```
（requires 子句：无）
- contract has no requires clauses (empty conjunction); probe requires encoded as literal true — entry precondition trivially satisfiable by source text

## extraction_skipped（无契约可提取，如实记录）

原因（通用）：冻结源对应 span 处无 `#[verus_spec]` 属性、签名内亦无 requires 子句——接口无显式契约条款，入口前置为空合取，不存在可探针的恒假候选；按纪律不硬造条目。

| CSV 行 | 接口 | span |
|---|---|---|
| 18 | `ostd::sync::guard::impl&%0::guard` | ostd/src/sync/guard.rs:55 |
| 19 | `ostd::sync::guard::impl&%0::read_guard` | ostd/src/sync/guard.rs:58 |
| 20 | `ostd::sync::guard::impl&%1::guard` | ostd/src/sync/guard.rs:82 |
| 21 | `ostd::sync::guard::impl&%1::read_guard` | ostd/src/sync/guard.rs:86 |
| 22 | `ostd::sync::guard::impl&%2::guard` | ostd/src/sync/guard.rs:115 |
| 23 | `ostd::sync::guard::impl&%2::read_guard` | ostd/src/sync/guard.rs:119 |
| 24 | `ostd::sync::mutex::impl&%3::lock` | ostd/src/sync/mutex.rs:73 |
| 28 | `ostd::sync::rcu::impl&%10::read` | ostd/src/sync/rcu/mod.rs:833 |
| 30 | `ostd::sync::rcu::impl&%8::read` | ostd/src/sync/rcu/mod.rs:768 |
| 31 | `ostd::sync::rwarc::impl&%3::read` | ostd/src/sync/rwarc.rs:70 |
| 32 | `ostd::sync::rwarc::impl&%3::write` | ostd/src/sync/rwarc.rs:75 |
| 33 | `ostd::sync::rwarc::impl&%7::read` | ostd/src/sync/rwarc.rs:182 |
| 34 | `ostd::sync::rwlock::impl&%21::try_upgrade` | ostd/src/sync/rwlock.rs:984 |
| 35 | `ostd::sync::rwlock::impl&%21::upgrade` | ostd/src/sync/rwlock.rs:954 |
| 36 | `ostd::sync::rwlock::impl&%4::read` | ostd/src/sync/rwlock.rs:397 |
| 38 | `ostd::sync::rwlock::impl&%4::try_upread` | ostd/src/sync/rwlock.rs:563 |
| 40 | `ostd::sync::rwlock::impl&%4::upread` | ostd/src/sync/rwlock.rs:435 |
| 41 | `ostd::sync::rwlock::impl&%4::write` | ostd/src/sync/rwlock.rs:414 |
| 42 | `ostd::sync::rwmutex::impl&%18::downgrade` | ostd/src/sync/rwmutex.rs:692 |
| 43 | `ostd::sync::rwmutex::impl&%18::try_downgrade` | ostd/src/sync/rwmutex.rs:704 |
| 45 | `ostd::sync::rwmutex::impl&%21::upgrade` | ostd/src/sync/rwmutex.rs:876 |
| 46 | `ostd::sync::rwmutex::impl&%4::read` | ostd/src/sync/rwmutex.rs:327 |
| 48 | `ostd::sync::rwmutex::impl&%4::try_upread` | ostd/src/sync/rwmutex.rs:464 |
| 49 | `ostd::sync::rwmutex::impl&%4::try_write` | ostd/src/sync/rwmutex.rs:415 |
| 50 | `ostd::sync::rwmutex::impl&%4::upread` | ostd/src/sync/rwmutex.rs:353 |
| 51 | `ostd::sync::rwmutex::impl&%4::write` | ostd/src/sync/rwmutex.rs:338 |
| 54 | `ostd::task::preempt::guard::disable_preempt` | ostd/src/task/preempt/guard.rs:135 |
| 55 | `ostd::task::preempt::guard::impl&%1::transfer_to` | ostd/src/task/preempt/guard.rs:121 |
| 56 | `ostd::trap::irq::disable_local` | ostd/src/trap/irq.rs:220 |
| 57 | `ostd::trap::irq::impl&%2::new` | ostd/src/trap/irq.rs:242 |
| 58 | `ostd::trap::irq::impl&%3::transfer_to` | ostd/src/trap/irq.rs:253 |

## 汇总：预期判定分布（预注册，28 条目）

CandidateInconclusive=10, ConfirmedLive=16, ConfirmedVacuous=2（运行后以 FULL-RUN-60.md 实测为准）

