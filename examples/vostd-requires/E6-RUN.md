# M7e-E6 — 运行归档（E6 信号产品化 + 剩余 Tier C 表达式扩展）

状态：完成。设计与预注册见 `docs/M7e-e6-signals.md`、`E6-EXPECTED.md`；本文档只报告
实际结果，不回溯改写预注册。commit 见 `git log -1`（本文件与实现同一提交）。

## 1. 输入与命令

- VIR：`/tmp/m7a-vir-export2/crate.vir`（vostd 43da079）。
- 冻结源：`/tmp/vostd-43da079-ktest-chan`（运行后未改）。
- 投影配置：`examples/vostd-requires/config-projections.json`（250 models / 250 rules；
  相对 M7e-projection 追加 6 个 over-approx 模型/规则，见 §6）。
- 预注册：`examples/vostd-requires/e6-expectations.json`（723 条；7 条 VmIo 预测
  `ConfirmedVacuous`，其余等于归档工具 expected）。
- 重生成 census + 筛查：

```sh
cargo-spec-check census --vir /tmp/m7a-vir-export2/crate.vir --crate ostd \
  --source-root /tmp/vostd-43da079-ktest-chan --out /tmp/m7e-e6-census4
cargo-spec-check screen-requires --census /tmp/m7e-e6-census4/census.json \
  --out OUT --config examples/vostd-requires/config-projections.json \
  --expectations examples/vostd-requires/e6-expectations.json \
  --batch K --batch-size 50 --cargo-verus ... --z3 ... --vstd ...
```

- 723 基线复核使用 pre-E6 二进制（git HEAD `ee06213` 的临时 worktree），以避免表达式
  扩展改变历史条目；最终扩展运行使用本提交的新二进制。两者都用新的 `signals` 子命令/
  集成输出计算 E6 信号。
- `cargo test --offline`：**全绿**（core lib 152 passed / 72 ignored；cargo-spec-check
  102 passed / 72 ignored；0 failed）。

## 2. E6 信号产品化（实现）

- `screen-requires --expectations FILE`：严格校验
  `spec-check.expectations/1`；每条形如
  `entry -> {expected, basis, scope?, registry?}`。未知 schema、非法 verdict、空 basis
  都是加载错误。
- 信号判定：比较 **预注册 expected** 与 **工具内部 expected** 对 actual 的两种 divergence；
  同日只发一条，冲突时 `expected_source=pre-registered`，并保留 `tool_expected/tool_match`。
- 方向：`vacuous-signal`（expected 非 Vacuous → actual Vacuous）、
  `expected-falsified`（expected Vacuous → actual 非 Vacuous）、`anomaly`、
  `verdict-divergence`。
- 模型关系门控：`exact`/`faithful`/`over-approx` 的 Vacuous 才是 `candidate`；
  `under-approx`/`undeclared` 降级为 `info`；`expected-falsified`/`verdict-divergence`
  一律 `info`；`Live` 从不声称真实 liveness。
- 输出：`run/signals.json`、`screening.json` 的 `signals[]`/`signal_counts`、
  `run/verdicts.json` 的同名字段；`signals` 子命令支持对已有 run 重算。
- 新增/更新单测：signals 8 个（方向、tool/pre 双源、缺失 expectations、under-approx
  门控、structural over-approx、确定性、严格加载）；requires 7 个（Ctor、Block、
  Match、Clip/div/mod、tuple field、无 datatype 诊断、类型冲突）。

## 3. 723 条 M7e-projection 基线复核（pre-E6 census/二进制，15 批）

判定分布：**ConfirmedLive 22 / ConfirmedVacuous 7 / CandidateInconclusive 694 /
Anomaly 0**（与归档 723 完全一致）。信号：**7，全部 `vacuous-signal`，全部
`expected_source=tool`，`preregistered_match=true`，`severity=candidate`，
`suggested_registry_action=confirm-registry:V-003`，`scope=crate`**。

| entry | function | model_relation | expected | actual |
|---|---|---|---|---|
| `r0351_read_bytes` | `ostd::mm::io::VmIo::read_bytes` | faithful | CandidateInconclusive | ConfirmedVacuous |
| `r0352_read_slice` | `ostd::mm::io::VmIo::read_slice` | faithful | CandidateInconclusive | ConfirmedVacuous |
| `r0353_read_val` | `ostd::mm::io::VmIo::read_val` | faithful | CandidateInconclusive | ConfirmedVacuous |
| `r0355_write_bytes` | `ostd::mm::io::VmIo::write_bytes` | faithful | CandidateInconclusive | ConfirmedVacuous |
| `r0356_write_slice` | `ostd::mm::io::VmIo::write_slice` | faithful | CandidateInconclusive | ConfirmedVacuous |
| `r0357_write_val` | `ostd::mm::io::VmIo::write_val` | faithful | CandidateInconclusive | ConfirmedVacuous |
| `r0358_write_vals` | `ostd::mm::io::VmIo::write_vals` | over-approx | CandidateInconclusive | ConfirmedVacuous |

其余 **716/723 无信号**。预注册预测 7 × faithful；实际 **6 faithful + 1 over-approx**
（`r0358_write_vals` 的 Tier B 还使用了其他 over-approx 子句）。这是一条预注册偏差，
不改变 7 条均为 candidate、均确认 V-003 的结论。

## 4. Tier C 表达式扩展：before → after

| 指标 | before（M7e-projection） | after（本提交） |
|---|---|---|
| Tier A | 28 | **28** |
| Tier B | 695 | **721** |
| Tier C | 51 | **25** |
| 候选条目 | 723 | **749** |
| C → A | — | **0** |
| C → B | — | **26** |
| 仍 C | — | **25** |
| 新增 Vacuous | — | **0** |

扩展只对 **baseline M7e tier = C** 的函数启用（`Ctor`、`Match`、`Block`、`Clip` cast、
`/`/`%`、数值 tuple field token）。baseline A/B 条目保持原探针文本；这避免了首轮实现中
5 个已有 B 条目（`r0158/r0159/r0162/r0917/r0923`）因 `/`/`%` 变成可判定而改变判定。

### 4.1 升级的 26 个函数（全部 C → B）

| 类别 | 函数 |
|---|---|
| Block / tuple field | `meta::get_slot`、`io::memcpy`、`virt_mem::copy_nonoverlapping`、`virt_mem::copy_offset`、`virt_mem::read`、`virt_mem::read_offset`、`virt_mem::read_volatile`、`cursor::clone_item`、`PageTableConfig::lemma_raw_item_well_formed_preserved` |
| binary `/`/`%` | `segment::from_raw`、`segment::slice`、`cursor::owners::tracked_new_child` |
| Ctor | `PageTableConfig::{item_from_raw, lemma_huge_raw_item_untracked, lemma_item_from_raw_roundtrip, lemma_item_from_raw_well_formed, lemma_item_into_raw_roundtrip, lemma_raw_item_well_formed_split}`、`node::{level, lock, make_guard_unchecked}` |
| Match | `vm_space::{activate_reader, activate_writer, dispose_reader}`、`io::{tracked_read_view_unwrap, write_to_read}` |

26 个条目在扩展运行中全部 `CandidateInconclusive`（Tier B，无 witness），无信号。

### 4.2 仍为 Tier C 的 25 个函数（逐条原因）

| function | 原因 |
|---|---|
| `ostd::mm::frame::impl&%5::meta` | unmodeled call: `frame_specs::impl&%4::inv` |
| `ostd::mm::frame::meta::impl&%1::drop_meta_in_place` | unmodeled calls: `vstd::atomic::impl&%22::id`、`impl&%21::value`、`pcell_maybe_uninit::impl&%1::id`；`==` 类型冲突 |
| `ostd::mm::frame::unique::impl&%4::reset_as_unused` | unmodeled call: `vstd::atomic::impl&%21::value` |
| `ostd::mm::frame::unique::impl&%5::drop` | unmodeled call: `vstd::atomic::impl&%21::value` |
| `ostd::mm::frame::unique::impl&%6::from_unique` | unmodeled call: `vstd::atomic::impl&%21::value` |
| `ostd::mm::kspace::kvirt_area::impl&%6::map_frames` | unmodeled calls: `relate_guard`、`meta_to_frame` |
| `ostd::mm::kspace::kvirt_area::impl&%6::map_untracked_frames` | unmodeled call: `relate_guard` |
| `ostd::mm::page_table::cursor::impl&%3::new` | unmodeled call: `relate_guard` |
| `ostd::mm::page_table::cursor::impl&%4::new` | unmodeled call: `relate_guard` |
| `ostd::mm::page_table::cursor::locking::dfs_acquire_lock` | unmodeled call: `relate_guard` |
| `ostd::mm::page_table::cursor::locking::lock_range` | unmodeled call: `relate_guard` |
| `ostd::mm::page_table::impl&%10::cursor` | unmodeled call: `relate_guard` |
| `ostd::mm::page_table::impl&%10::cursor_mut` | unmodeled call: `relate_guard` |
| `ostd::mm::page_table::node::entry::impl&%2::is_node` | unmodeled call: `relate_guard` |
| `ostd::mm::page_table::node::entry::impl&%2::new_at` | unmodeled call: `relate_guard` |
| `ostd::mm::page_table::node::entry::impl&%2::to_ref` | unmodeled call: `node_matching` |
| `ostd::mm::page_table::node::impl&%3::entry` | unmodeled call: `relate_guard` |
| `ostd::mm::page_table::node::impl&%3::nr_children` | unmodeled call: `pcell_maybe_uninit::impl&%1::id` |
| `ostd::mm::page_table::node::impl&%3::nr_children_mut` | unmodeled call: `pcell_maybe_uninit::impl&%1::id` |
| `ostd::mm::page_table::node::impl&%3::stray_mut` | unmodeled call: `pcell_maybe_uninit::impl&%1::id` |
| `ostd::mm::vm_space::impl&%0::cursor` | unmodeled call: `relate_guard` |
| `ostd::mm::vm_space::impl&%0::cursor_mut` | unmodeled call: `relate_guard` |
| `ostd::mm::vm_space::vm_space_specs::impl&%1::dispose_writer` | unmodeled call: `vstd::set::impl&%0::disjoint` |
| `ostd::specs::mm::frame::meta_owners::write_metadata_into_storage` | unmodeled call: `pcell_maybe_uninit::impl&%1::id` |
| `ostd::specs::mm::page_table::cursor::owners::impl&%0::lemma_rel_children_from_node_matching` | unmodeled call: `node_matching` |

## 5. 扩展运行（749 条，15 批）

判定分布：**ConfirmedLive 22 / ConfirmedVacuous 7 / CandidateInconclusive 720 /
Anomaly 0**。原 723 条目的逐条 verdict 与 §3 基线 **0 差异**；新增 26 条全部
`CandidateInconclusive`。信号：**7**（就是 §3 的 VmIo），`candidates=7`、`info=0`、
`missing_expectations=26`、`unmatched=0`；**0 个新 Vacuous、0 个新信号、0 个 Anomaly**。

E6 输出（合并后）：

```json
{
  "counts": {
    "signals": 7,
    "candidates": 7,
    "info": 0,
    "by_direction": {"vacuous-signal": 7},
    "by_expected_source": {"tool": 7},
    "by_model_relation": {"faithful": 6, "over-approx": 1}
  },
  "expectations": {"provided": true, "entries": 723, "matched": 723,
                   "missing": 26, "unmatched": 0}
}
```

## 6. V-003 复核

7 条 VmIo `ConfirmedVacuous` 精确复现，与 registry `129ebd20` /
`vacuity:vmio-default-hooks` 一致；scope 保持 **crate-scope**，不升级全局。
建议动作是 `confirm-registry:V-003`（不自动写 registry）。`r0358_write_vals` 的
`model_relation=over-approx` 表示该信号的证据是松弛的（仍满足健全性要求），不是新的
faithful 声明。

## 7. 配置/实现补充（全部 over-approx，除非已有 faithful）

- `VmIoMemView::{ReadView, WriteView}` variant 模型（Match 标量化）。
- `VmSpaceOwner::{mem_view, mv_range}`、`VmIoOwner::range`、`MemView::mappings`
  field 模型（Match/range 子句）。
- 数值 tuple field 无 datatype：翻译为新鲜标量 token，`modeling.tuple_field_projection`
  记录 `relation=over-approx`；非数值字段保留 `field without datatype` 明确诊断。
- 量化 projected 子句改为 named `uninterp spec fn`（Tier B）；避免 Verus 对任意投影
  量词无法推断 trigger。首轮实现曾因此导致 batch 2/4/5 编译失败并被误判为 Anomaly；
  修复后最终运行 Anomaly=0。
- 表达式扩展由 baseline C 门控；不改写 baseline A/B 探针。

## 8. 预注册偏差（不追溯改写 `E6-EXPECTED.md`）

| 预注册 | 实际 |
|---|---|
| 7 条 VmIo 全部 faithful | 6 faithful + 1 over-approx（`r0358`） |
| C→B 23 / 仍 C 28 | C→B 26 / 仍 C 25 |
| 预测升级 `meta`、`dispose_writer` | 仍 C（`frame_specs::impl&%4::inv`、`vstd::set::impl&%0::disjoint`） |
| 预测 5 个 tuple-field 函数仍 C | 全部升级（通用数值 tuple field 投影） |
| `copy_nonoverlapping` 由 Block 升级 | 实际由 tuple-field 投影升级 |
| §4 中间筛查 33 A / 716 B | 被 §5 基线 C 门控取代：28 A / 721 B |

没有出现需要新增 registry 的新 Vacuous；registry 未修改。

## 9. 归档文件

- `docs/M7e-e6-signals.md`（设计 + 预注册契约）
- `examples/vostd-requires/E6-EXPECTED.md`（预注册，含两段 pre-probe addendum）
- `examples/vostd-requires/e6-expectations.json`（723 条预注册输入）
- `examples/vostd-requires/e6-screening.json`（最终 749 条筛查 + 合并信号）
- `examples/vostd-requires/e6-signals.json`（最终 7 条信号）
- `examples/vostd-requires/E6-RUN.md`（本文件）
- 实现：`crates/spec-check-core/src/signals.rs`、
  `crates/spec-check-core/src/{census.rs,requires.rs}`、
  `crates/cargo-spec-check/src/main.rs`（`--expectations`、`signals` 子命令）
- ROADMAP：新增 M7e-E6 小节。
