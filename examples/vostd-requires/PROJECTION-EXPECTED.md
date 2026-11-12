# M7e-projection — 预注册期望（写于探针运行前）

生成：由 `config-projections.json` + `/tmp/m7e-proj-census/census.json` 的筛查结果导出；
本文件在探针运行前生成，运行后不改写。规则与健全性边界见 `docs/M7e-projection-mapping.md`。

## 1. 输入与模型关系

- census：VIR `/tmp/m7a-vir-export2/crate.vir`（vostd 43da079），复跑 census 加入 `requires[].expr` 通用表达式树。
- config：`examples/vostd-requires/config-projections.json`；`reference.mode=opaque`、`ghost.mode=opaque`、泛型 `* -> u64`。
- 模型关系：
  - `faithful`：id 投影（`PointsTo::id`/`cell_id` 等 11 条）与 VmIo `obeys_*` 常量模型（2 条，**crate-scope**）。
  - `over-approx`：其余 method/field/variant 模型（opaque；无约束，仅用于空洞方向的过近似）。
  - 无 `under-approx` 模型。

**判定纪律**：任何 `ConfirmedLive` 都只是投影抽象可满足，不是真实 liveness；只有 `faithful`/`over-approx` 模型上的
`ConfirmedVacuous` 才能作为候选证据（VmIo 条目为 crate-scope 候选，外部 impl 未建模）。

## 2. 规则预注册

| id | 情况 | 预期 |
|---|---|---|
| P1 | 无 `projection` 段 | baseline 与 M7e-ext 一致：9 A / 89 B / 676 C |
| P2 | `perm.id() == mutex.cell_id()` 两侧 identity | Tier A，witness `(0,0)`，**ConfirmedLive**（形状/模型相对；复现 M1 `mutex_guard_new_shape`） |
| P3 | 未建模 call/field/variant | 保持 Tier C，`projection_unmapped` 记录原因 |
| P4 | `body:null` opaque 投影 | Tier B，`CandidateInconclusive`；Live 不是真实 liveness |
| P5 | 常量模型导致合取恒假（VmIo `obeys_* = true`，`!obeys`） | Tier A 无 witness；预测 **ConfirmedVacuous**（模型相对、crate-scope）；工具 `expected` 记 `CandidateInconclusive`，故该条 `expect_match=false` 是预注册的 E6 信号 |
| P6 | `under-approx` | 不接受为空洞证据（本配置未使用） |
| P7 | 关系/类型/arity/重复模型名/非法 body | config load 错误 |
| P8 | ctor / Match / Block / 未建模 VIR 节点 | 涉及引用/ghost 参数时保持 Tier C |
| P9 | 全部条目 | A 有 witness → ConfirmedLive；A 无 witness → CandidateInconclusive（工具）/ E6 预测见 P5；B → CandidateInconclusive |

## 3. 逐条预注册（剩余 129 的 A/B + M1/registry 关联项）

“模型”列是该条目实际使用的 relation 计数；“工具 expected”是 runner 的 `expect_match` 目标；
“预注册判定”是本文档的独立预测。

| entry | function | tier | 模型 | 工具 expected | 预注册判定 |
|---|---|---|---|---|---|
| `r0190_daddr_inner` | `ostd::mm::dma::dma_coherent::impl&%4::daddr_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r0192_nbytes_inner` | `ostd::mm::dma::dma_coherent::impl&%4::nbytes_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r0193_nframes_inner` | `ostd::mm::dma::dma_coherent::impl&%4::nframes_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r0194_paddr_inner` | `ostd::mm::dma::dma_coherent::impl&%4::paddr_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r0220_daddr_inner` | `ostd::mm::dma::dma_stream::impl&%3::daddr_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r0221_direction_inner` | `ostd::mm::dma::dma_stream::impl&%3::direction_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r0223_nbytes_inner` | `ostd::mm::dma::dma_stream::impl&%3::nbytes_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r0224_nframes_inner` | `ostd::mm::dma::dma_stream::impl&%3::nframes_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r0225_paddr_inner` | `ostd::mm::dma::dma_stream::impl&%3::paddr_inner` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r0246_from_raw` | `ostd::mm::frame::frame_ref::NonNullPtr::from_raw` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive |
| `r0250_borrow_paddr` | `ostd::mm::frame::frame_ref::impl&%0::borrow_paddr` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive |
| `r0264_borrow_with_permission` | `ostd::mm::frame::impl&%7::borrow_with_permission` | B | faithful:1, over-approx:7 | CandidateInconclusive | CandidateInconclusive |
| `r0267_reference_count` | `ostd::mm::frame::impl&%7::reference_count` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive |
| `r0269_start_paddr` | `ostd::mm::frame::impl&%7::start_paddr` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r0270_slot` | `ostd::mm::frame::impl&%8::slot` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r0271_from_raw` | `ostd::mm::frame::impl&%9::from_raw` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r0277_cursor_back_mut` | `ostd::mm::frame::linked_list::impl&%2::cursor_back_mut` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive |
| `r0278_cursor_front_mut` | `ostd::mm::frame::linked_list::impl&%2::cursor_front_mut` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive |
| `r0280_is_empty` | `ostd::mm::frame::linked_list::impl&%2::is_empty` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive |
| `r0282_pop_back` | `ostd::mm::frame::linked_list::impl&%2::pop_back` | B | over-approx:6 | CandidateInconclusive | CandidateInconclusive |
| `r0283_pop_front` | `ostd::mm::frame::linked_list::impl&%2::pop_front` | B | over-approx:6 | CandidateInconclusive | CandidateInconclusive |
| `r0286_size` | `ostd::mm::frame::linked_list::impl&%2::size` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive |
| `r0290_move_next` | `ostd::mm::frame::linked_list::impl&%3::move_next` | B | over-approx:5 | CandidateInconclusive | CandidateInconclusive |
| `r0291_move_prev` | `ostd::mm::frame::linked_list::impl&%3::move_prev` | B | over-approx:5 | CandidateInconclusive | CandidateInconclusive |
| `r0306_addr_of` | `ostd::mm::frame::meta::impl&%1::addr_of` | A | faithful:1 | ConfirmedLive | ConfirmedLive (model-relative) |
| `r0309_frame_paddr` | `ostd::mm::frame::meta::impl&%1::frame_paddr` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r0328_from_unused` | `ostd::mm::frame::unique::impl&%2::from_unused` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive |
| `r0329_meta` | `ostd::mm::frame::unique::impl&%2::meta` | B | over-approx:6 | CandidateInconclusive | CandidateInconclusive |
| `r0331_repurpose` | `ostd::mm::frame::unique::impl&%2::repurpose` | B | over-approx:9 | CandidateInconclusive | CandidateInconclusive |
| `r0335_from_raw` | `ostd::mm::frame::unique::impl&%4::from_raw` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive |
| `r0338_slot` | `ostd::mm::frame::unique::impl&%4::slot` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r0339_start_paddr` | `ostd::mm::frame::unique::impl&%4::start_paddr` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r0350_read` | `ostd::mm::io::VmIo::read` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive |
| `r0354_write` | `ostd::mm::io::VmIo::write` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive |
| `r0426_get_kernel_page_table` | `ostd::mm::kspace::kvirt_area::get_kernel_page_table` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive |
| `r0549_lemma_perm_well_formed_with_region_preserved` | `ostd::mm::page_table::PageTableConfig::lemma_perm_well_formed_with_region_preserved` | B | over-approx:6 | CandidateInconclusive | CandidateInconclusive |
| `r0564_clone` | `ostd::mm::page_table::RCClone::clone` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r0568_cur_entry` | `ostd::mm::page_table::cursor::impl&%3::cur_entry` | B | over-approx:10 | CandidateInconclusive | CandidateInconclusive |
| `r0569_cur_va_range` | `ostd::mm::page_table::cursor::impl&%3::cur_va_range` | B | over-approx:6 | CandidateInconclusive | CandidateInconclusive |
| `r0576_push_level` | `ostd::mm::page_table::cursor::impl&%3::push_level` | B | over-approx:17 | CandidateInconclusive | CandidateInconclusive |
| `r0581_map` | `ostd::mm::page_table::cursor::impl&%4::map` | B | over-approx:11 | CandidateInconclusive | CandidateInconclusive |
| `r0588_replace_cur_entry` | `ostd::mm::page_table::cursor::impl&%4::replace_cur_entry` | B | over-approx:16 | CandidateInconclusive | CandidateInconclusive |
| `r0594_dfs_mark_stray_and_unlock` | `ostd::mm::page_table::cursor::locking::dfs_mark_stray_and_unlock` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive |
| `r0614_create_user_page_table` | `ostd::mm::page_table::impl&%9::create_user_page_table` | B | over-approx:11 | CandidateInconclusive | CandidateInconclusive |
| `r0618_load_pte` | `ostd::mm::page_table::load_pte` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive |
| `r0619_from_pte` | `ostd::mm::page_table::node::child::impl&%1::from_pte` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive |
| `r0622_from_pte` | `ostd::mm::page_table::node::child::impl&%3::from_pte` | B | over-approx:4 | CandidateInconclusive | CandidateInconclusive |
| `r0627_is_none` | `ostd::mm::page_table::node::entry::impl&%2::is_none` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive |
| `r0629_protect` | `ostd::mm::page_table::node::entry::impl&%2::protect` | B | over-approx:10 | CandidateInconclusive | CandidateInconclusive |
| `r0634_protect_child` | `ostd::mm::page_table::node::entry::impl&%3::protect_child` | B | over-approx:12 | CandidateInconclusive | CandidateInconclusive |
| `r0648_read_pte` | `ostd::mm::page_table::node::impl&%3::read_pte` | B | over-approx:5 | CandidateInconclusive | CandidateInconclusive |
| `r0650_write_pte` | `ostd::mm::page_table::node::impl&%3::write_pte` | B | over-approx:5 | CandidateInconclusive | CandidateInconclusive |
| `r0693_map` | `ostd::mm::vm_space::impl&%2::map` | B | over-approx:10 | CandidateInconclusive | CandidateInconclusive |
| `r0840_step_as_list` | `ostd::specs::mm::embedding::list_store::impl&%1::step_as_list` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r0841_step_contains` | `ostd::specs::mm::embedding::list_store::impl&%1::step_contains` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r0842_step_current_meta` | `ostd::specs::mm::embedding::list_store::impl&%1::step_current_meta` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r0850_step_is_empty` | `ostd::specs::mm::embedding::list_store::impl&%1::step_is_empty` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r0859_step_size` | `ostd::specs::mm::embedding::list_store::impl&%1::step_size` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r0925_borrow_meta` | `ostd::specs::mm::frame::meta_owners::borrow_meta` | B | over-approx:5 | CandidateInconclusive | CandidateInconclusive |
| `r0926_borrow_meta_mut` | `ostd::specs::mm::frame::meta_owners::borrow_meta_mut` | B | over-approx:5 | CandidateInconclusive | CandidateInconclusive |
| `r1259_lemma_agree` | `ostd::specs::task::cpu_local::impl&%5::lemma_agree` | A | faithful:2 | ConfirmedLive | ConfirmedLive (model-relative) |
| `r1260_lemma_distinct` | `ostd::specs::task::cpu_local::impl&%5::lemma_distinct` | A | faithful:2 | ConfirmedLive | ConfirmedLive (model-relative) |
| `r1261_lemma_same_cpu_has_distinct_auth` | `ostd::specs::task::cpu_local::impl&%5::lemma_same_cpu_has_distinct_auth` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r1277_release_lock` | `ostd::sync::mutex::impl&%3::release_lock` | A | faithful:2 | ConfirmedLive | ConfirmedLive (model-relative) |
| `r1279_unlock` | `ostd::sync::mutex::impl&%3::unlock` | A | faithful:2 | ConfirmedLive | ConfirmedLive (model-relative) |
| `r1280_new` | `ostd::sync::mutex::impl&%7::new` | A | faithful:2 | ConfirmedLive | ConfirmedLive (model-relative) |
| `r1302_update` | `ostd::sync::rcu::impl&%6::update` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r1317_from_raw` | `ostd::sync::rcu::non_null::NonNullPtr::from_raw` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive |
| `r1320_borrow_perm_as_ref_perm` | `ostd::sync::rcu::non_null::NonNullPtrRef::borrow_perm_as_ref_perm` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r1321_borrow_ref_perm` | `ostd::sync::rcu::non_null::NonNullPtrRef::borrow_ref_perm` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r1323_raw_as_ref` | `ostd::sync::rcu::non_null::NonNullPtrRef::raw_as_ref` | B | over-approx:3 | CandidateInconclusive | CandidateInconclusive |
| `r1405_new` | `ostd::sync::spin::impl&%3::new` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r1408_release_lock` | `ostd::sync::spin::impl&%5::release_lock` | B | over-approx:5 | CandidateInconclusive | CandidateInconclusive |
| `r1432_get` | `ostd::task::scheduler::info::impl&%1::get` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r1433_set_anyway` | `ostd::task::scheduler::info::impl&%1::set_anyway` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r1434_set_if_is_none` | `ostd::task::scheduler::info::impl&%1::set_if_is_none` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r1435_set_to_none` | `ostd::task::scheduler::info::impl&%1::set_to_none` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r1456_free` | `ostd::util::range_alloc::impl&%4::free` | B | faithful:2, over-approx:4 | CandidateInconclusive | CandidateInconclusive |
| `r0599_path_slot_as_mut` | `ostd::mm::page_table::cursor::path_slot_as_mut` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r0747_vm_space_cursor_embedded` | `ostd::specs::mm::embedding::cursor::vm_space_cursor_embedded` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r0748_vm_space_cursor_mut_embedded` | `ostd::specs::mm::embedding::cursor::vm_space_cursor_mut_embedded` | B | over-approx:2 | CandidateInconclusive | CandidateInconclusive |
| `r1015_tracked_restore` | `ostd::specs::mm::page_table::cursor::owners::impl&%0::tracked_restore` | B | over-approx:1 | CandidateInconclusive | CandidateInconclusive |
| `r1250_tracked_restore` | `ostd::specs::task::cpu_core::impl&%4::tracked_restore` | B | none | CandidateInconclusive | CandidateInconclusive |
| `r0351_read_bytes` | `ostd::mm::io::VmIo::read_bytes` | A | none | CandidateInconclusive | ConfirmedVacuous (E6 candidate) |
| `r0352_read_slice` | `ostd::mm::io::VmIo::read_slice` | A | none | CandidateInconclusive | ConfirmedVacuous (E6 candidate) |
| `r0353_read_val` | `ostd::mm::io::VmIo::read_val` | A | none | CandidateInconclusive | ConfirmedVacuous (E6 candidate) |
| `r0355_write_bytes` | `ostd::mm::io::VmIo::write_bytes` | A | none | CandidateInconclusive | ConfirmedVacuous (E6 candidate) |
| `r0356_write_slice` | `ostd::mm::io::VmIo::write_slice` | A | none | CandidateInconclusive | ConfirmedVacuous (E6 candidate) |
| `r0357_write_val` | `ostd::mm::io::VmIo::write_val` | A | none | CandidateInconclusive | ConfirmedVacuous (E6 candidate) |
| `r0358_write_vals` | `ostd::mm::io::VmIo::write_vals` | B | none | CandidateInconclusive | CandidateInconclusive |

## 4. M1 5 接口的显式假设

| M1 接口 | 本轮 entry | 本轮 tier | M1 归档 | 预注册 |
|---|---|---|---|---|
| `MutexGuard::new` | `r1280_new` | A | `mutex_guard_new_shape` = ConfirmedLive | **复现 ConfirmedLive（shape）**；构造层真空洞仍属 V-001，不在本轮标量层 |
| `cursor::path_slot_as_mut` | `r0599_path_slot_as_mut` | B | CandidateInconclusive | CandidateInconclusive（`is Some` 以 opaque variant 投影，无法见证） |
| `vm_space_cursor_embedded` | `r0747_vm_space_cursor_embedded` | B | CandidateInconclusive | CandidateInconclusive |
| `vm_space_cursor_mut_embedded` | `r0748_vm_space_cursor_mut_embedded` | B | CandidateInconclusive | CandidateInconclusive |
| `cursor::owners::tracked_restore` | `r1015_tracked_restore` | B | CandidateInconclusive | CandidateInconclusive |

## 5. 运行前分布（预测，不是结论）

- baseline：9 A / 89 B / 676 C。
- 配置后：**28 A / 695 B / 51 C**（其中 baseline C → A 16、C → B 609、仍 C 51）。
- 剩余 129：**6 A / 72 B / 51 C**；51 个仍 C 的类别见下。
- 全量候选 723 条，按 50 条/批分 15 批，每批独立 900s 预算。

51 个仍 C 的主要类别（运行前静态分类）：
- 26 个含未建模/冲突 call（`relate_guard`、`node_matching`、`atomic::id/value`、`pcell_maybe_uninit::id` 的多类型用法）；
- 15 个含 constructor（`Some`/`Tracked`/tuple 等）；
- 9 个 requires 是 `Block`、6 个 `Match`、6 个未识别的 `unary`、4 个 `binary`、3 个无 datatype 的 field；
- 其余为 `is` 判别式或无投影细节。

这些类别不是“无缺陷”，是当前投影模型不可表达；不得写成已筛查结论。
