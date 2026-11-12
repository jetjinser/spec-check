# M7e-E6 — 预注册期望（写于 E6 实现、全量复核与 Tier C 扩展运行前）

状态：预注册。本文档在 E6 信号实现、723 条全量复核、Tier C 表达式扩展实现和扩展探针运行
**之前**写成；运行后只由 `E6-RUN.md` 报告实际结果，不改写本文档。设计与输出 schema 见
`docs/M7e-e6-signals.md`。

## 1. 输入与判定纪律

- census：`/tmp/m7a-vir-export2/crate.vir`（vostd 43da079），重新生成（含 `requires[].expr`）。
- 投影配置：`examples/vostd-requires/config-projections.json`；Tier C 扩展若需要新增
  variant 模型，只在同一配置中追加，并在下文中预先声明。
- expectations：`examples/vostd-requires/e6-expectations.json`，覆盖 M7e-projection 归档的
  全部 723 个 A/B 条目。
- 判定纪律：
  - `ConfirmedLive` 只是投影抽象可满足，不是真实 liveness；
  - 只有 `faithful`/`over-approx` 关系上的 `ConfirmedVacuous` 才是候选证据；
  - `CandidateInconclusive` 不计缺陷；
  - 工具不自动修改 registry。

## 2. E6 信号预注册（全量 723）

规则：723 个条目的预注册 verdict 等于 M7e-projection 归档中的**工具 expected**，唯一例外是
下列 7 个 VmIo 条目，预注册为 `ConfirmedVacuous`（P5）。因此：

| 项 | 预期 |
|---|---|
| 信号总数 | **7** |
| 信号方向 | 7 × `vacuous-signal` |
| 信号来源 | 7 × `expected_source = "tool"`（工具 expected 为 Inconclusive，实际 Vacuous） |
| 预注册匹配 | 7 × `preregistered_match = true` |
| 模型关系 | 7 × `faithful`（crate-scope 常量模型 `obeys_vmio_* = true`） |
| severity | 7 × `candidate` |
| registry 建议 | 7 × `confirm-registry:V-003`（不新增、不升级为全局） |
| 其余 716 条 | **0 信号** |

7 个 VmIo 条目：

| entry | function |
|---|---|
| `r0351_read_bytes` | `ostd::mm::io::VmIo::read_bytes` |
| `r0352_read_slice` | `ostd::mm::io::VmIo::read_slice` |
| `r0353_read_val` | `ostd::mm::io::VmIo::read_val` |
| `r0355_write_bytes` | `ostd::mm::io::VmIo::write_bytes` |
| `r0356_write_slice` | `ostd::mm::io::VmIo::write_slice` |
| `r0357_write_val` | `ostd::mm::io::VmIo::write_val` |
| `r0358_write_vals` | `ostd::mm::io::VmIo::write_vals` |

`r0350_read` 与 `r0354_write` 的归档实际判定是 `CandidateInconclusive`，因此不在 7 条内。

## 3. Tier C 表达式扩展预注册

扩展的通用节点：`Ctor`（结构/不透明 token）、`Match`（标量条件 if-else 链）、`Block`
（简单 `let` 序列 + 尾表达式）、`UnaryOp Clip`（`as` 转换，按 over-approx 身份处理）、
`/` 与 `%`（扩展整数域上的 over-approx）、无 datatype 的 tuple field 明确诊断、类型冲突明确诊断。
无法安全建模的保持 Tier C 并逐条记录原因。

### 3.1 方向性预测

扩展前：28 A / 695 B / 51 C（723 候选）。扩展后预测：

| 指标 | 预测 |
|---|---|
| C → A | **0**（新增条目都至少含一个 opaque/投影模型） |
| C → B | **23** |
| 仍 C | **28** |
| 新候选总数 | **746**（723 + 23） |
| 新 Vacuous | **0** |

### 3.2 预期升级（23）

| 类别 | 函数 |
|---|---|
| Block（4） | `ostd::mm::frame::impl&%5::meta`、`ostd::mm::frame::meta::get_slot`、`ostd::mm::io::memcpy`、`ostd::specs::mm::virt_mem::impl&%5::copy_nonoverlapping` |
| binary `/`/`%`（4） | `ostd::mm::frame::segment::impl&%1::from_raw`、`ostd::mm::frame::segment::impl&%3::slice`、`ostd::specs::mm::page_table::cursor::owners::impl&%0::tracked_new_child`、`ostd::specs::mm::virt_mem::impl&%5::read_volatile` |
| Ctor（9） | `ostd::mm::page_table::PageTableConfig::item_from_raw`、`...::lemma_huge_raw_item_untracked`、`...::lemma_item_from_raw_roundtrip`、`...::lemma_item_from_raw_well_formed`、`...::lemma_item_into_raw_roundtrip`、`...::lemma_raw_item_well_formed_split`、`ostd::mm::page_table::node::impl&%1::level`、`...::impl&%2::lock`、`...::impl&%2::make_guard_unchecked` |
| Match（6） | `ostd::mm::vm_space::vm_space_specs::impl&%1::activate_reader`、`...::activate_writer`、`...::dispose_reader`、`...::dispose_writer`、`ostd::specs::mm::io::impl&%3::tracked_read_view_unwrap`、`...::write_to_read` |

Match 的 4 个函数（`dispose_*`、`tracked_read_view_unwrap`、`write_to_read`）需要新增
`VmIoMemView::ReadView` / `VmIoMemView::WriteView` 两个 `over-approx` variant 规则；这是
项目侧模型补充，预先声明。`Some`/`None` 规则已存在。

### 3.3 预期仍 C（28）

| 原因 | 函数 |
|---|---|
| 未建模/类型冲突 call（21） | `drop_meta_in_place`、`reset_as_unused`、`drop`、`from_unique`、`map_frames`、`map_untracked_frames`、`cursor::impl&%3::new`、`cursor::impl&%4::new`、`dfs_acquire_lock`、`lock_range`、`page_table::impl&%10::cursor`、`...::cursor_mut`、`entry::impl&%2::is_node`、`...::new_at`、`...::to_ref`、`node::impl&%3::entry`、`nr_children`、`nr_children_mut`、`stray_mut`、`write_metadata_into_storage`、`lemma_rel_children_from_node_matching` |
| 无 datatype tuple field（5） | `lemma_raw_item_well_formed_preserved`、`cursor::impl&%3::clone_item`、`virt_mem::impl&%5::read`、`...::copy_offset`、`...::read_offset` |
| 另（2） | `vm_space::impl&%0::cursor`、`vm_space::impl&%0::cursor_mut`（`relate_guard` 未建模） |

注：上表“21”包含 `relate_guard` 未建模的 13 个函数；`page_table::impl&%10::cursor(_mut)` 等
同时有 `relate_guard` 阻塞。`drop_meta_in_place` 还有 `==` 类型冲突。精确计数以运行为准；
本表是方向性预注册，运行后不追溯改写。

### 3.4 扩展运行中的 E6 预测

- 23 个新条目没有逐条预注册；按 `expectations.missing` 计数，并与工具 expected 比较。
- 预测新条目全部为 `CandidateInconclusive`（Tier B），**0 个新 Vacuous**。
- 若出现新 Vacuous 且模型关系为 `faithful`/`over-approx`，先复现，再按维护规则登记 registry；
  不升级为全局，不虚构。
- 原 7 条 VmIo 信号必须仍精确复现；其余原条目不得新增信号。

## 4. Pre-probe addendum（扩展实现后、扩展探针运行前写入）

写入时点：表达式扩展已实现，census 已用新解析器重建，筛查（纯分类，无 Verus 判定）已完成；
**尚未运行任何扩展探针**。以上 §3 的方向性预测保持原样，不追溯改写。

实际筛查：**33 A / 716 B / 25 C，749 候选**；C→B **26**，C→A **0**，仍 C **25**。
与 §3.1 的 23/28 相比，多升级 3 个、少留 3 个 C。偏差原因：

- `Ctor` 9 个、binary 4 个全部按预期升级；
- Block 只升级 `get_slot`、`memcpy`；`meta` 被未建模的 `frame_specs::impl&%4::inv` 阻塞，
  `copy_nonoverlapping` 通过通用 tuple-field 投影升级；
- Match 5/6：`dispose_writer` 仍被未建模的 `vstd::set::impl&%0::disjoint` 阻塞；
- 通用 tuple-field 投影（无 datatype 的数值字段 → 新鲜标量 token，relation 记为 over-approx）
  使 5 个原预测仍 C 的函数升级：`lemma_raw_item_well_formed_preserved`、`clone_item`、`read`、
  `copy_offset`、`read_offset`。这是 §3.3 之外的工具级泛化，预先在此声明。

26 个新条目逐条预注册为 `CandidateInconclusive`（全部 Tier B；无 witness）：

| function | tier | expected |
|---|---|---|
| `ostd::mm::frame::meta::get_slot` | B | CandidateInconclusive |
| `ostd::mm::frame::segment::impl&%1::from_raw` | B | CandidateInconclusive |
| `ostd::mm::frame::segment::impl&%3::slice` | B | CandidateInconclusive |
| `ostd::mm::io::memcpy` | B | CandidateInconclusive |
| `ostd::mm::page_table::PageTableConfig::item_from_raw` | B | CandidateInconclusive |
| `ostd::mm::page_table::PageTableConfig::lemma_huge_raw_item_untracked` | B | CandidateInconclusive |
| `ostd::mm::page_table::PageTableConfig::lemma_item_from_raw_roundtrip` | B | CandidateInconclusive |
| `ostd::mm::page_table::PageTableConfig::lemma_item_from_raw_well_formed` | B | CandidateInconclusive |
| `ostd::mm::page_table::PageTableConfig::lemma_item_into_raw_roundtrip` | B | CandidateInconclusive |
| `ostd::mm::page_table::PageTableConfig::lemma_raw_item_well_formed_preserved` | B | CandidateInconclusive |
| `ostd::mm::page_table::PageTableConfig::lemma_raw_item_well_formed_split` | B | CandidateInconclusive |
| `ostd::mm::page_table::cursor::impl&%3::clone_item` | B | CandidateInconclusive |
| `ostd::mm::page_table::node::impl&%1::level` | B | CandidateInconclusive |
| `ostd::mm::page_table::node::impl&%2::lock` | B | CandidateInconclusive |
| `ostd::mm::page_table::node::impl&%2::make_guard_unchecked` | B | CandidateInconclusive |
| `ostd::mm::vm_space::vm_space_specs::impl&%1::activate_reader` | B | CandidateInconclusive |
| `ostd::mm::vm_space::vm_space_specs::impl&%1::activate_writer` | B | CandidateInconclusive |
| `ostd::mm::vm_space::vm_space_specs::impl&%1::dispose_reader` | B | CandidateInconclusive |
| `ostd::specs::mm::io::impl&%3::tracked_read_view_unwrap` | B | CandidateInconclusive |
| `ostd::specs::mm::io::impl&%3::write_to_read` | B | CandidateInconclusive |
| `ostd::specs::mm::page_table::cursor::owners::impl&%0::tracked_new_child` | B | CandidateInconclusive |
| `ostd::specs::mm::virt_mem::impl&%5::copy_nonoverlapping` | B | CandidateInconclusive |
| `ostd::specs::mm::virt_mem::impl&%5::copy_offset` | B | CandidateInconclusive |
| `ostd::specs::mm::virt_mem::impl&%5::read` | B | CandidateInconclusive |
| `ostd::specs::mm::virt_mem::impl&%5::read_offset` | B | CandidateInconclusive |
| `ostd::specs::mm::virt_mem::impl&%5::read_volatile` | B | CandidateInconclusive |

E6 预测不变：原 7 条 VmIo 信号精确复现；26 个新条目 `expectations.missing`，但
`actual = CandidateInconclusive = tool expected`，因此 **0 个新信号、0 个新 Vacuous**。

## 5. Second pre-probe addendum（最终扩展探针运行前写入）

在 §4 之后、最终扩展探针运行前，实现增加了**基线 C 门控**：表达式扩展（`Ctor`、`Match`、
`Block`、`Clip` cast、`/`/`%`、tuple-field token）只对 baseline M7e tier 为 C 的函数启用；
baseline A/B 条目保持原有探针文本。原因：首轮扩展筛查把 5 个已有 B 条目的 `/`/`%` 子句
变成可判定，实际判定从 `CandidateInconclusive` 变为 `ConfirmedLive`，与预注册冲突，且不属于
“剩余 51 Tier C”的范围。门控后这些已有条目恢复原判定；26 个 baseline C 升级不变。

最终筛查（纯分类，尚未运行最终探针）：**28 A / 721 B / 25 C，749 候选**；C→B **26**，
C→A **0**，仍 C **25**。§4 的 33 A / 716 B 是门控前的中间筛查，已被本节取代；§3/§4 的
26 个升级函数表仍有效。

最终探针预测：7 条 VmIo 信号精确复现；其余 742 个条目 0 信号；0 个新 Vacuous。
