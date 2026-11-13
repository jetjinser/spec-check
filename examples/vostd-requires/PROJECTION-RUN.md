# M7e-projection — 运行归档

状态：完成。本文档记录实际运行；预注册见 `PROJECTION-EXPECTED.md`，设计与健全性边界见
`docs/M7e-projection-mapping.md`。所有数字来自运行产物，未手工改写。

## 1. 输入、环境与批次

- 仓库：`~/vie/work/AES/spec-check`，M7e-projection 工作树（本轮 commit 见交付说明）。
- VIR：`/tmp/m7a-vir-export2/crate.vir`；冻结源：`/tmp/vostd-43da079-ktest-chan`。
- census：`/tmp/m7e-proj-census/census.json`（重新生成，附带 `requires[].expr`）。
- 配置：`examples/vostd-requires/config-projections.json`（244 个模型 / 244 条规则）。
- 探针环境：`nix develop ~/vie/work/AES/vostd/nix`；cargo-verus/z3/vstd 均取自
  `/home/jinser/vie/work/AES/vostd/tools/verus/source`；target `x86_64-unknown-none`。
- 批次：15 批 × 最多 50 条，`--batch K --batch-size 50`，每批独立 900s 预算；
  全部批次完成，无超时，最终 `Anomaly=0`。

## 2. Tier before → after

| 范围 | baseline A/B/C | projection A/B/C | 变化 |
|---|---|---|---|
| 全库（774 有 requires） | 9 / 89 / 676 | **28 / 695 / 51** | C→A 16，C→B 609，仍 C 51；A 净 +19 |
| 剩余 129（M7e-ext 后仍 C） | 0 / 0 / 129 | **6 / 72 / 51** | C→A 6，C→B 72，仍 C 51 |
| 候选探针（A/B） | - | 723 | 实际运行 723 条（50×14 + 23） |

注：baseline 是默认配置（无泛型实例化、无引用/ghost 抽象）；projection 配置同时启用泛型 `* -> u64`、
`reference/ghost=opaque` 和显式投影模型。全库 A 净 +19 包含 9 条 baseline A 的稳定保留、3 条 M7e-ext
泛型 A 的保留、以及 16 条 baseline C→A。

## 3. 运行判定总览

| verdict | 数量 | 说明 |
|---|---|---|
| ConfirmedLive | 22 | 全部是 A 且有 bounded witness；**模型相对可满足，不是真实 liveness** |
| ConfirmedVacuous | 7 | 全部是 VmIo default fallback；crate-scope 常量模型，模型相对候选证据（见 §6） |
| CandidateInconclusive | 694 | 主要是 B 的 opaque/quantified 投影；验证器不完备，不计缺陷 |
| Anomaly | 0 | - |

`expect_match`：**716/723** 与工具 `expected` 一致。7 条不一致全部是 VmIo：
工具按保守规则记 `CandidateInconclusive`（Tier A/B 无 witness），预注册 P5 预测 `ConfirmedVacuous`；
实际 7/7 `ConfirmedVacuous`。这不是异常，正是预注册的 E6 信号。特别地 `r0358_write_vals` 的 Tier 是 B
（其余子句是 opaque），但首子句同样是 `!obeys_vmio_write_requires()`，因此合取恒假；逐条表里它的
“工具 expected”是 Inconclusive，预注册判定按 B 规则也写成 Inconclusive——这是预注册粒度过粗，实际方向
与 P5 机制一致，作为更强的 E6 候选记录，不追溯改写预注册。

## 4. 剩余 129 的逐条判定（仅 A/B，51 个 C 不生成探针）

| entry | function | tier | 模型 relation | expected | verdict | match |
|---|---|---|---|---|---|---|
| `r0190_daddr_inner` | `ostd::mm::dma::dma_coherent::impl&%4::daddr_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive | True |
| `r0192_nbytes_inner` | `ostd::mm::dma::dma_coherent::impl&%4::nbytes_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive | True |
| `r0193_nframes_inner` | `ostd::mm::dma::dma_coherent::impl&%4::nframes_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive | True |
| `r0194_paddr_inner` | `ostd::mm::dma::dma_coherent::impl&%4::paddr_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive | True |
| `r0220_daddr_inner` | `ostd::mm::dma::dma_stream::impl&%3::daddr_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive | True |
| `r0221_direction_inner` | `ostd::mm::dma::dma_stream::impl&%3::direction_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive | True |
| `r0223_nbytes_inner` | `ostd::mm::dma::dma_stream::impl&%3::nbytes_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive | True |
| `r0224_nframes_inner` | `ostd::mm::dma::dma_stream::impl&%3::nframes_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive | True |
| `r0225_paddr_inner` | `ostd::mm::dma::dma_stream::impl&%3::paddr_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive | True |
| `r0246_from_raw` | `ostd::mm::frame::frame_ref::NonNullPtr::from_raw` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive | True |
| `r0250_borrow_paddr` | `ostd::mm::frame::frame_ref::impl&%0::borrow_paddr` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive | True |
| `r0264_borrow_with_permission` | `ostd::mm::frame::impl&%7::borrow_with_permission` | B | faithful:1, over-approx:7 | CandidateInconclusive | CandidateInconclusive | True |
| `r0267_reference_count` | `ostd::mm::frame::impl&%7::reference_count` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive | True |
| `r0269_start_paddr` | `ostd::mm::frame::impl&%7::start_paddr` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive | True |
| `r0270_slot` | `ostd::mm::frame::impl&%8::slot` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive | True |
| `r0271_from_raw` | `ostd::mm::frame::impl&%9::from_raw` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive | True |
| `r0277_cursor_back_mut` | `ostd::mm::frame::linked_list::impl&%2::cursor_back_mut` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive | True |
| `r0278_cursor_front_mut` | `ostd::mm::frame::linked_list::impl&%2::cursor_front_mut` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive | True |
| `r0280_is_empty` | `ostd::mm::frame::linked_list::impl&%2::is_empty` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive | True |
| `r0282_pop_back` | `ostd::mm::frame::linked_list::impl&%2::pop_back` | B | over-approx:6 | CandidateInconclusive | CandidateInconclusive | True |
| `r0283_pop_front` | `ostd::mm::frame::linked_list::impl&%2::pop_front` | B | over-approx:6 | CandidateInconclusive | CandidateInconclusive | True |
| `r0286_size` | `ostd::mm::frame::linked_list::impl&%2::size` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive | True |
| `r0290_move_next` | `ostd::mm::frame::linked_list::impl&%3::move_next` | B | over-approx:5 | CandidateInconclusive | CandidateInconclusive | True |
| `r0291_move_prev` | `ostd::mm::frame::linked_list::impl&%3::move_prev` | B | over-approx:5 | CandidateInconclusive | CandidateInconclusive | True |
| `r0306_addr_of` | `ostd::mm::frame::meta::impl&%1::addr_of` | A | faithful:1 | ConfirmedLive | ConfirmedLive | True |
| `r0309_frame_paddr` | `ostd::mm::frame::meta::impl&%1::frame_paddr` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive | True |
| `r0328_from_unused` | `ostd::mm::frame::unique::impl&%2::from_unused` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive | True |
| `r0329_meta` | `ostd::mm::frame::unique::impl&%2::meta` | B | over-approx:6 | CandidateInconclusive | CandidateInconclusive | True |
| `r0331_repurpose` | `ostd::mm::frame::unique::impl&%2::repurpose` | B | over-approx:9 | CandidateInconclusive | CandidateInconclusive | True |
| `r0335_from_raw` | `ostd::mm::frame::unique::impl&%4::from_raw` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive | True |
| `r0338_slot` | `ostd::mm::frame::unique::impl&%4::slot` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive | True |
| `r0339_start_paddr` | `ostd::mm::frame::unique::impl&%4::start_paddr` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive | True |
| `r0350_read` | `ostd::mm::io::VmIo::read` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive | True |
| `r0354_write` | `ostd::mm::io::VmIo::write` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive | True |
| `r0426_get_kernel_page_table` | `ostd::mm::kspace::kvirt_area::get_kernel_page_table` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive | True |
| `r0549_lemma_perm_well_formed_with_region_preserved` | `ostd::mm::page_table::PageTableConfig::lemma_perm_well_formed_with_region_preserved` | B | over-approx:6 | CandidateInconclusive | CandidateInconclusive | True |
| `r0564_clone` | `ostd::mm::page_table::RCClone::clone` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive | True |
| `r0568_cur_entry` | `ostd::mm::page_table::cursor::impl&%3::cur_entry` | B | over-approx:10 | CandidateInconclusive | CandidateInconclusive | True |
| `r0569_cur_va_range` | `ostd::mm::page_table::cursor::impl&%3::cur_va_range` | B | over-approx:6 | CandidateInconclusive | CandidateInconclusive | True |
| `r0576_push_level` | `ostd::mm::page_table::cursor::impl&%3::push_level` | B | over-approx:17 | CandidateInconclusive | CandidateInconclusive | True |
| `r0581_map` | `ostd::mm::page_table::cursor::impl&%4::map` | B | over-approx:11 | CandidateInconclusive | CandidateInconclusive | True |
| `r0588_replace_cur_entry` | `ostd::mm::page_table::cursor::impl&%4::replace_cur_entry` | B | over-approx:16 | CandidateInconclusive | CandidateInconclusive | True |
| `r0594_dfs_mark_stray_and_unlock` | `ostd::mm::page_table::cursor::locking::dfs_mark_stray_and_unlock` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive | True |
| `r0614_create_user_page_table` | `ostd::mm::page_table::impl&%9::create_user_page_table` | B | over-approx:11 | CandidateInconclusive | CandidateInconclusive | True |
| `r0618_load_pte` | `ostd::mm::page_table::load_pte` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive | True |
| `r0619_from_pte` | `ostd::mm::page_table::node::child::impl&%1::from_pte` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive | True |
| `r0622_from_pte` | `ostd::mm::page_table::node::child::impl&%3::from_pte` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive | True |
| `r0627_is_none` | `ostd::mm::page_table::node::entry::impl&%2::is_none` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive | True |
| `r0629_protect` | `ostd::mm::page_table::node::entry::impl&%2::protect` | B | over-approx:10 | CandidateInconclusive | CandidateInconclusive | True |
| `r0634_protect_child` | `ostd::mm::page_table::node::entry::impl&%3::protect_child` | B | over-approx:12 | CandidateInconclusive | CandidateInconclusive | True |
| `r0648_read_pte` | `ostd::mm::page_table::node::impl&%3::read_pte` | B | over-approx:5 | CandidateInconclusive | CandidateInconclusive | True |
| `r0650_write_pte` | `ostd::mm::page_table::node::impl&%3::write_pte` | B | over-approx:5 | CandidateInconclusive | CandidateInconclusive | True |
| `r0693_map` | `ostd::mm::vm_space::impl&%2::map` | B | over-approx:10 | CandidateInconclusive | CandidateInconclusive | True |
| `r0840_step_as_list` | `ostd::specs::mm::embedding::list_store::impl&%1::step_as_list` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive | True |
| `r0841_step_contains` | `ostd::specs::mm::embedding::list_store::impl&%1::step_contains` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive | True |
| `r0842_step_current_meta` | `ostd::specs::mm::embedding::list_store::impl&%1::step_current_meta` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive | True |
| `r0850_step_is_empty` | `ostd::specs::mm::embedding::list_store::impl&%1::step_is_empty` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive | True |
| `r0859_step_size` | `ostd::specs::mm::embedding::list_store::impl&%1::step_size` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive | True |
| `r0925_borrow_meta` | `ostd::specs::mm::frame::meta_owners::borrow_meta` | B | over-approx:5 | CandidateInconclusive | CandidateInconclusive | True |
| `r0926_borrow_meta_mut` | `ostd::specs::mm::frame::meta_owners::borrow_meta_mut` | B | over-approx:5 | CandidateInconclusive | CandidateInconclusive | True |
| `r1259_lemma_agree` | `ostd::specs::task::cpu_local::impl&%5::lemma_agree` | A | faithful:2 | ConfirmedLive | ConfirmedLive | True |
| `r1260_lemma_distinct` | `ostd::specs::task::cpu_local::impl&%5::lemma_distinct` | A | faithful:2 | ConfirmedLive | ConfirmedLive | True |
| `r1261_lemma_same_cpu_has_distinct_auth` | `ostd::specs::task::cpu_local::impl&%5::lemma_same_cpu_has_distinct_auth` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive | True |
| `r1277_release_lock` | `ostd::sync::mutex::impl&%3::release_lock` | A | faithful:2 | ConfirmedLive | ConfirmedLive | True |
| `r1279_unlock` | `ostd::sync::mutex::impl&%3::unlock` | A | faithful:2 | ConfirmedLive | ConfirmedLive | True |
| `r1280_new` | `ostd::sync::mutex::impl&%7::new` | A | faithful:2 | ConfirmedLive | ConfirmedLive | True |
| `r1302_update` | `ostd::sync::rcu::impl&%6::update` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive | True |
| `r1317_from_raw` | `ostd::sync::rcu::non_null::NonNullPtr::from_raw` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive | True |
| `r1320_borrow_perm_as_ref_perm` | `ostd::sync::rcu::non_null::NonNullPtrRef::borrow_perm_as_ref_perm` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive | True |
| `r1321_borrow_ref_perm` | `ostd::sync::rcu::non_null::NonNullPtrRef::borrow_ref_perm` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive | True |
| `r1323_raw_as_ref` | `ostd::sync::rcu::non_null::NonNullPtrRef::raw_as_ref` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive | True |
| `r1405_new` | `ostd::sync::spin::impl&%3::new` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive | True |
| `r1408_release_lock` | `ostd::sync::spin::impl&%5::release_lock` | B | over-approx:5 | CandidateInconclusive | CandidateInconclusive | True |
| `r1432_get` | `ostd::task::scheduler::info::impl&%1::get` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive | True |
| `r1433_set_anyway` | `ostd::task::scheduler::info::impl&%1::set_anyway` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive | True |
| `r1434_set_if_is_none` | `ostd::task::scheduler::info::impl&%1::set_if_is_none` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive | True |
| `r1435_set_to_none` | `ostd::task::scheduler::info::impl&%1::set_to_none` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive | True |
| `r1456_free` | `ostd::util::range_alloc::impl&%4::free` | B | faithful:2, over-approx:4 | CandidateInconclusive | CandidateInconclusive | True |

小结：6 A 全部 `ConfirmedLive`（mutex 三接口、`meta::addr_of`、cpu_local `lemma_agree`/`lemma_distinct`）；
72 B 全部 `CandidateInconclusive`；无 `ConfirmedVacuous`。
A 条目的 `ConfirmedLive` 都是投影抽象上的可满足性；例如 `MutexGuard::new` 的等式形状可满足，
**不**推出真实构造路径可达，构造层真空洞仍属 registry V-001。

## 5. M1 5 接口对照

| M1 接口 | entry | 本轮 tier | M1 归档 verdict | 本轮 verdict | 一致性 |
|---|---|---|---|---|---|
| `MutexGuard::new` | `r1280_new` | A | mutex_guard_new_shape / ConfirmedLive | ConfirmedLive | 一致 |
| `cursor::path_slot_as_mut` | `r0599_path_slot_as_mut` | B | CandidateInconclusive | CandidateInconclusive | 一致 |
| `vm_space_cursor_embedded` | `r0747_vm_space_cursor_embedded` | B | CandidateInconclusive | CandidateInconclusive | 一致 |
| `vm_space_cursor_mut_embedded` | `r0748_vm_space_cursor_mut_embedded` | B | CandidateInconclusive | CandidateInconclusive | 一致 |
| `cursor::owners::tracked_restore` | `r1015_tracked_restore` | B | CandidateInconclusive | CandidateInconclusive | 一致 |

`MutexGuard::new`：`perm.id() == mutex.cell_id()` 在两侧 faithful identity 投影下化为 `perm_id == mutex_id`，
witness `(0, 0)`，Verus 返回 **ConfirmedLive**。这与 M1 的 `mutex_guard_new_shape` 形状结论一致：
标量层没有新真空洞；M1 的真空洞在构造层（V-001），本轮没有推翻也没有升级它。

其余 4 个 M1 接口：`path_slot_as_mut` 的 `old(path)[idx] is Some` 用 opaque variant 投影，
`vm_space*` 的 `inv()` 用 opaque predicate，`tracked_restore` 的 `idx < children.len()` 用 opaque field/len；
因此保持 Tier B / `CandidateInconclusive`，与 M1 归档一致（验证器无法穿过 opaque 投影）。

## 6. VmIo：本轮唯一的 7 个 ConfirmedVacuous

| entry | function | tier | tool expected | actual | match |
|---|---|---|---|---|---|
| `r0350_read` | `ostd::mm::io::VmIo::read` | B | CandidateInconclusive | CandidateInconclusive | True |
| `r0351_read_bytes` | `ostd::mm::io::VmIo::read_bytes` | A | CandidateInconclusive | ConfirmedVacuous | False |
| `r0352_read_slice` | `ostd::mm::io::VmIo::read_slice` | A | CandidateInconclusive | ConfirmedVacuous | False |
| `r0353_read_val` | `ostd::mm::io::VmIo::read_val` | A | CandidateInconclusive | ConfirmedVacuous | False |
| `r0354_write` | `ostd::mm::io::VmIo::write` | B | CandidateInconclusive | CandidateInconclusive | True |
| `r0355_write_bytes` | `ostd::mm::io::VmIo::write_bytes` | A | CandidateInconclusive | ConfirmedVacuous | False |
| `r0356_write_slice` | `ostd::mm::io::VmIo::write_slice` | A | CandidateInconclusive | ConfirmedVacuous | False |
| `r0357_write_val` | `ostd::mm::io::VmIo::write_val` | A | CandidateInconclusive | ConfirmedVacuous | False |
| `r0358_write_vals` | `ostd::mm::io::VmIo::write_vals` | B | CandidateInconclusive | ConfirmedVacuous | False |

证据链：

- 7 个函数的首个（或唯一）requires 子句是 `!Self::obeys_vmio_read_requires()` 或
  `!Self::obeys_vmio_write_requires()`；模型 `proj_..._obeys_vmio_*` 的 body 为 `true`，relation `faithful`。
- 常量模型使 `!true` 恒假，因此整个入口前置不可满足；Verus 直接确认空洞探针（`vacuous_probe_failed=false`）。
- **边界**：该 faithful 是 **crate-scope**。`VmIo` 是公开 trait，外部 impl 可以返回 `false`；本模型只覆盖
  ostd crate 内已知 impl。因此这是 crate 内 default 方法的入口真空洞候选，不是“所有 VmIo 实现”的全局结论。
- 7 条中 6 条 Tier A（唯一子句就是 `!obeys`），`write_vals` 是 Tier B（还有 opaque 子句），但首子句同样恒假。

## 7. 全库 22 条 ConfirmedLive（模型相对）

| entry | function | witness |
|---|---|---|
| `r0106_lemma_encode_prop_flags_matches` | `ostd::arch::mm::lemma_encode_prop_flags_matches` | `0usize` |
| `r0127_new` | `ostd::boot::memory_region::impl&%19::new` | `(0usize, 0usize, 0usize)` |
| `r0166_lemma_u64_distinct_bits_differ` | `ostd::cpu::set::lemma_u64_distinct_bits_differ` | `(0usize, 1usize)` |
| `r0167_lemma_u64_nonzero_has_set_bit` | `ostd::cpu::set::lemma_u64_nonzero_has_set_bit` | `1usize` |
| `r0169_lemma_u64_unit_bit_projection` | `ostd::cpu::set::lemma_u64_unit_bit_projection` | `0usize` |
| `r0185_check_and_insert_dma_mapping` | `ostd::mm::dma::check_and_insert_dma_mapping` | `(0, 0, 0, 0)` |
| `r0306_addr_of` | `ostd::mm::frame::meta::impl&%1::addr_of` | `(0, 0)` |
| `r0438_paddr_to_vaddr` | `ostd::mm::kspace::paddr_to_vaddr` | `(1, 0, 0)` |
| `r0560_new_pt` | `ostd::mm::page_table::PageTableEntryTrait::new_pt` | `(0usize, 1usize)` |
| `r0732_lemma_paddr_to_vaddr_properties` | `ostd::specs::arch::x86::lemma_paddr_to_vaddr_properties` | `(0usize, 0usize, 1usize)` |
| `r0733_lemma_vaddr_to_paddr_properties` | `ostd::specs::arch::x86::lemma_vaddr_to_paddr_properties` | `(0usize, 0usize, 1usize)` |
| `r1059_lemma_split_sub_page_big_j` | `ostd::specs::mm::page_table::cursor::page_size_lemmas::lemma_split_sub_page_big_j` | `(2usize, 2usize, 1usize, 2usize)` |
| `r1152_lemma_count_present_upto_bound` | `ostd::specs::mm::page_table::node::owners::lemma_count_present_upto_bound` | `0usize` |
| `r1153_lemma_count_present_upto_monotone` | `ostd::specs::mm::page_table::node::owners::lemma_count_present_upto_monotone` | `(0usize, 0usize)` |
| `r1196_lemma_vaddr_make_eq_page_size` | `ostd::specs::mm::page_table::owners::lemma_vaddr_make_eq_page_size` | `(0usize, 1usize, 0usize, 1usize)` |
| `r1231_from_vaddr` | `ostd::specs::mm::virt_mem::impl&%5::from_vaddr` | `(0, 0, 0)` |
| `r1259_lemma_agree` | `ostd::specs::task::cpu_local::impl&%5::lemma_agree` | `(0, 0)` |
| `r1260_lemma_distinct` | `ostd::specs::task::cpu_local::impl&%5::lemma_distinct` | `(0, 0)` |
| `r1262_tracked_update` | `ostd::specs::task::cpu_local::impl&%5::tracked_update` | `(0, 0)` |
| `r1277_release_lock` | `ostd::sync::mutex::impl&%3::release_lock` | `(0, 0)` |
| `r1279_unlock` | `ostd::sync::mutex::impl&%3::unlock` | `(0, 0)` |
| `r1280_new` | `ostd::sync::mutex::impl&%7::new` | `(0, 0)` |

这些 Live 只说明投影约束的标量核心可满足。对含 opaque 投影的 B 条目，Verus 未给出 Live；
对 A 条目，Live 也不能替代真实实现路径/构造可达性。

## 8. 仍未映射的 51 个剩余 C

这些函数**没有生成探针**，不是“已筛查无缺陷”。按静态 `projection_unmapped` 归类（可重叠）：

| 类别 | 出现次数 | 说明 |
|---|---|---|
| 未建模/冲突 call | 26 | 保持 C；不猜 |
| constructor | 15 | 保持 C；不猜 |
| 未识别 VIR 节点: Block | 9 | 保持 C；不猜 |
| 未识别 VIR 节点: Match | 6 | 保持 C；不猜 |
| 未识别 VIR 节点: unary | 6 | 保持 C；不猜 |
| 未识别 VIR 节点: binary | 4 | 保持 C；不猜 |
| 无 datatype 的 field | 3 | 保持 C；不猜 |
| 其他: `==` expects operands of the same type | 1 | 保持 C；不猜 |

重点类别：

- **多类型 call 冲突**：`relate_guard`、`node_matching`、`atomic::id/value`、`pcell_maybe_uninit::id` 在
  同一 callee 上出现 bool/usize 两种参数类型，单条 opaque 模型无法同时覆盖；工具选择不猜，相关函数保持 C。
- **constructor / Match / Block**：`Some(...)`/`Tracked(...)`/tuple、`match` 表达式、成块 requires，
  当前投影 AST 不解释；这些是下一轮可扩展点，不是本轮缺陷。
- **无 datatype 的 field**：VIR 中 `Place Field` 缺少 datatype（如 tuple index），无法按类型匹配 field 规则。

## 9. 证据边界（必须随结论一起引用）

1. **模型相对性**：所有投影判定都是项目侧模型上的判定。`faithful`/`over-approx` 声明由项目侧给出；
   工具只做语法/类型/arity 校验。错误的关系声明会使对应的 ConfirmedVacuous 失效。
2. **Live 不是 liveness**：22 条 ConfirmedLive 不代表真实调用可达或实现正确；它们只说明抽象可满足。
3. **Opaque 模型**：`body:null` 的 method/field/variant 模型是任意函数，按 over-approx 计入空洞方向；
   Verus 不能穿过它们证明 Live，所以相关条目是 Tier B/Inconclusive。
4. **identity 模型**：仅用于 id 值投影（`PointsTo::id`/`cell_id` 等）和 `addr_of` 的 `PointsTo::value`；
   工具拒绝同一 base id 上两个不同 identity 模型，避免把不同投影强制相等。
5. **crate-scope**：VmIo 的 7 条 Vacuous 只覆盖 ostd crate 内 impl；公开 trait 的外部 impl 未建模。
6. **`old`**：在入口按 `old(e)=e` 处理；对 id 抽象是精确的，不声明 post-state 变化。
7. **未映射**：§8 的 51 个 C 仍是未筛查区域；不得把 Tier 重算当作全库结论。

## 10. 产物与复现

- 分类报告：`/tmp/proj-gen-final/screening.json`（28 A / 695 B / 51 C；723 候选）。
- 探针运行：`/tmp/m7e-proj-runs/batch-0..14/`（每批 `manifest.json`、`screening.json`、`run/report.json`、`run/verdicts.json`）。
- 复现命令（单批）：

```sh
nix develop ~/vie/work/AES/vostd/nix -c bash -c '\
  cd ~/vie/work/AES/spec-check && ./target/release/cargo-spec-check screen-requires \
    --census /tmp/m7e-proj-census/census.json \
    --out /tmp/m7e-proj-runs/batch-K \
    --config examples/vostd-requires/config-projections.json \
    --batch K --batch-size 50 \
    --cargo-verus .../target-verus/release/cargo-verus \
    --z3 .../z3 --vstd .../vstd'
```

registry：本轮新增 V-003（VmIo default fallback，crate-scope 模型相对），详见
`research/experiments/spec-quality-two-day/findings/registry.json`。没有虚构升级：M1 的构造层 V-001 不变，
51 个未映射 C 不登记为缺陷。

## 11. 后续裁定（2026-11-13）：V-003 按设计关闭

§6 的 7 条 `ConfirmedVacuous` 已完成人工复核：`!Self::obeys_vmio_{read,write}_requires()`
恒假是刻意设计，不是缺陷。trait 的两个 requires hook 默认 `false`（`ostd/src/mm/io.rs:1175-1181`），
fallback 只在 impl 未 opt-in 自定义 requires 时可用；opt-in 的 impl 按文档必须覆写
（`ostd/src/mm/io.rs:1307-1308,1357-1358,1419-1420`）。crate 内 3 个 active verified impl
（`DmaCoherent`/`DmaStream`/`DmaStreamSlice`）全部 opt-in（`obeys = true`）且未覆写，因此
§6 的 crate-scope 恒假结果准确且属按设计关闭。历史证据（`c404572bb` 将 `external_body`
默认方法改为带 `!obeys` 前置并证明方法体）支持同一结论。

registry V-003 的 `human_review` 已记为 `intentional-safety-net` / `closed-by-design`，
`machine_status` 不升级。完整证据链与重开条件见
`research/experiments/spec-quality-two-day/vmio-default-vacuity/README.md`。
