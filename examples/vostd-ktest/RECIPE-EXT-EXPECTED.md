# 配方库扩充预注册（运行前写定，运行后不改写；2026-11-12）

对象：vostd 冻结源（rev `43da079` + ktest 通道）中的 **11 条** M7b 非
blocked 缺口候选（9 条 `range_difference_*` + 2 条 CpuSet 类），由
`cargo-spec-check synthesize` 生成批量 manifest，再由
`analyses-divergence` 在独立副本 `/tmp/vostd-43da079-ktest-chan-b` 上
执行。判定只以 divergence 实跑为准；本文件是运行前假设，不参与判定，
运行后不修改。

## 1. 冻结输入

| 输入 | 路径 | sha256 |
|---|---|---|
| 提取候选（153 条记录） | `examples/vostd-ktest/extracted-candidates.json` | `6b99754cd8c602365564de10ea674cc5c3d21b84d38995d8331ecb8731b5219a` |
| 扩展配方 | `examples/vostd-ktest/recipes-m7b-ext.json` | `03ace85a7c34d64b55a1a8b707061912f6831031c0070016bd9d6def5bf652d2` |
| 合成 manifest（11 条） | `examples/vostd-ktest/divergence-manifest-m7b-ext.json` | `7e7b1fb1f6b5f8c5b702a4e15cab51911814a2cf35190b8856ceb4c64c216102` |
| 合成报告 | `examples/vostd-ktest/divergence-manifest-m7b-ext.report.json` | `efe3a2a8f8dfa554c80bc90821afa01b04eb92b349881da84c7c59b8c7f602d7` |

独立副本：`/tmp/vostd-43da079-ktest-chan-b`（由
`cp -a /tmp/vostd-43da079-ktest-chan ...` 复制；440 个文件 sha256 与冻结源
逐一一致，复制后校验通过）。所有 manifest 的
`target_workspace.manifest_path` 指向该副本；运行后核对恢复。

## 2. 选条与骨架扩展

- 新增 `iterator-sequence`：`let mut it = call;` + `steps` 消费 +
  `assert`（`{it}` 可用）。
- 新增 `state-invariant`：`bridge` 变更 + 可选 `call` 查询输出 +
  `assert`（`{recv}`/`{out}` 可用）。
- 新增 `candidate_interfaces`（按候选 id）用于区分 9 条同函数
  `range_difference`。
- 新增 `obligation_family` + `family_basis`（骨架选择与审计）。

设计见 `docs/M7c-recipes-extension.md`。本轮不新增 CLI 参数。

## 3. 未运行的 9 条（页表/create_page_table/page_properties）

以下候选**不进入本轮 manifest**；阻断证据见 §5。它们是需求已提取、
但当前冻结 proof 面无法写成可验证 client 的项，按纪律不虚构占位义务。

| 候选 | 函数 | 阻断要点 |
|---|---|---|
| K070 | `PageTable::cursor_mut` | `cursor_mut` 需要 `Tracked(PageTableOwner)`/`Ghost(root_guard)`/`Tracked(MetaRegionOwners)`/`Tracked(Guards)`；`MetaRegionOwners.inv` 要求全槽 map，冻结面无铸造入口 |
| K071 | `PageTable::cursor_mut` | 同上；且 `0 < va.end` 前置使空区间 `0..0` 的 Err 义务不可写 |
| K072 | `page_walk` | setup helper `create_user_pt_mapped_at` 私有；`FrameAllocOptions::alloc_frame_with` + cursor tracked 参数 |
| K073 | `page_walk` | 同上；`page_walk` 本身是 `#[cfg(ktest)]` 的物理页表 walk |
| K078 | `query` | setup helper `setup_page_table_with_two_frames` 私有 + cursor tracked 参数 |
| K080 | `find_next` | 同上 |
| K068 | `start_paddr` | `create_user_page_table` 需 `Tracked(kernel_owner)`；`MOCK_KERNEL_PT` 是测试内静态；`new_kernel_page_table` 为 `external_body` + `unimplemented!()` |
| K069 | `entry` | `entry`/`is_node` 需要 `Tracked(NodeOwner/EntryOwner/MetaRegionOwners)`；`new_kernel_page_table` 无 ensures |
| K075 | `disable_preempt` | 断言在私有 helper `check_map_with_property`；`cursor_mut` tracked 参数；闭包内 `from_bits(...).unwrap()` 需量化模型 |

## 4. 逐条预测

期望映射：`supported → ObligationSupported`，`gap → ContractGapCandidate`。

| # | entry | 函数 / 测试 | 骨架 | 合成义务（摘要） | 预期 | 运行前依据 |
|---|---|---|---|---|---|---|
| 1 | `c145_range_difference` | `range_difference` / `range_difference_contained` | iterator-sequence | `0..10 - 3..7` 依次 `Some(0..3)`, `Some(7..10)`, `None` | **gap** | ensures 只在 `obeys_prophetic_iter_laws() && will_return_none()` 前件下给 `remaining()`；前件未被建立 |
| 2 | `c146_range_difference` | `range_difference` / `range_difference_all_same` | iterator-sequence | `0..10 - 0..10` 首次 `next() == None` | **gap** | 同上；`will_return_none()` 未建立 |
| 3 | `c147_range_difference` | `range_difference` / `range_difference_left_same` | iterator-sequence | `0..10 - 0..5` → `Some(5..10)`, `None` | **gap** | 同上 |
| 4 | `c148_range_difference` | `range_difference` / `range_difference_right_same` | iterator-sequence | `0..10 - 5..10` → `Some(0..5)`, `None` | **gap** | 同上 |
| 5 | `c149_range_difference` | `range_difference` / `range_difference_b_empty` | iterator-sequence | `0..10 - 0..0` → `Some(0..10)`, `None` | **gap** | 同上 |
| 6 | `c150_range_difference` | `range_difference` / `range_difference_a_empty` | iterator-sequence | `0..0 - 0..10` → `None` | **gap** | 同上 |
| 7 | `c151_range_difference` | `range_difference` / `range_difference_all_empty` | iterator-sequence | `0..0 - 0..0` → `None` | **gap** | 同上 |
| 8 | `c152_range_difference` | `range_difference` / `range_difference_left_intersected` | iterator-sequence | `5..10 - 0..6` → `Some(6..10)`, `None` | **gap** | 同上 |
| 9 | `c153_range_difference` | `range_difference` / `range_difference_right_intersected` | iterator-sequence | `5..10 - 6..12` → `Some(5..6)`, `None` | **gap** | 同上 |
| 10 | `c012_cpu_set_test_test_empty_cpu_set_iter_is_empty` | `CpuSet::new_empty` / `test_empty_cpu_set_iter_is_empty` | final-relation | `let recv = CpuSet::new_empty(); assert(recv@.len() == 0);` | **supported** | `new_empty` ensures `ret@ == Set::empty() && ret.inv()`；View 是 `Set<int>` |
| 11 | `c014_atomiccpuset_contains` | `AtomicCpuSet::contains` / `test_atomic_cpu_set_multiple_sizes` | state-invariant | 空 `AtomicCpuSet`，`add(CpuId::bsp())` 后 `contains(bsp)` | **gap** | `contains` 仅有 `cpu_id/64 >= num_parts() ==> ret == false`；`add` 无后置条件；只覆盖 CPU0 循环体实例 |

**预注册分布**：ObligationSupported 1 / ContractGapCandidate 10。

## 5. 阻断证据（源码行号）

- `ostd/src/mm/page_table/mod.rs:1287-1290`：`PageTable::empty` 为
  `#[verifier::external_body]`；
- `.../mod.rs:1293-1380`：`empty_with_owner` 的 tracked 输出与
  `MetaRegionOwners` 前置；
- `.../mod.rs:917-923`：`new_kernel_page_table` 为 `external_body`，
  body 是 `unimplemented!()`；
- `.../mod.rs:1425-1435`：`cursor_mut` 的
  `with Tracked(owner), Ghost(root_guard), Tracked(regions), Tracked(guards)`
  与 `self.relates_owner(...)`、`0 < va.end` 前置；
- `ostd/specs/mm/frame/meta_region_owners.rs:32-60`：
  `MetaRegionOwners.inv` 要求 `forall i, 0 <= i < max_meta_slots() ==>
  slot_owners.contains_key(i)`（全槽 map）；
- `ostd/src/mm/page_table/node/mod.rs:445-452`：`entry` 需要
  `Tracked(child_owner/owner/regions)`；
- `ostd/src/mm/page_table/node/entry.rs:123-134`：`is_node` 需要
  `Tracked(EntryOwner/NodeOwner)`；
- `ostd/src/mm/page_table/test.rs:20`、`:282`、`:414`：
  `create_user_pt_mapped_at`、`check_map_with_property`、
  `setup_page_table_with_two_frames` 均为测试模块内私有 helper。

## 6. 已知运行风险（不改变预测）

1. `range_difference` 的 `requires`（`obeys_cmp::<usize>()`、
   `finite_range_matches_ord::<usize>()`、`cloned` 全称）若不能在探针内
   由 vstd 默认 broadcast 证明，phase-2 的首个失败会落在调用前置，而非
   `assert`；判定仍是 gap（错误在 obligation fn 内），但 RUN 会逐条
   记录实际 span 与归因。
2. `assert(r0 == Some(0usize..3usize))` 依赖 `Range<usize>` 的 spec
   相等；若 Verus 不接受该形态而 phase-1 失败，整批会 Inconclusive，
   RUN 将如实记录并改为比较 `start`/`end` 字段（新运行，不改预注册）。
3. `CpuSet::new_empty` / `AtomicCpuSet::new` 的可达性基于 crate 根
   注入（`crate::cpu` 为 `pub mod`）；若 `#[verus_verify]` 生成的双重
   spec 造成名称歧义，phase-1 会全批 Inconclusive。
4. 运行前 baseline 预期：vstd 依赖 0 errors；目标 crate 探针模块外
   0 errors。baseline 不干净时分类器判 Anomaly，按实测记录。

## 7. 运行命令（预注册）

```
timeout 900 nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  'cd ~/vie/work/AES/spec-check && \
   ./target/debug/cargo-spec-check analyses-divergence \
     --manifest examples/vostd-ktest/divergence-manifest-m7b-ext.json \
     --cargo-verus /home/jinser/vie/work/AES/vostd/tools/verus/source/target-verus/release/cargo-verus \
     --z3 /home/jinser/vie/work/AES/vostd/tools/verus/source/z3 \
     --vstd /home/jinser/vie/work/AES/vostd/tools/verus/source/vstd \
     --out /tmp/vostd-divergence-m7b-ext-run1'
```

## 8. 证据边界

- 本轮提升的可运行数预期为 **11**（9 iterator + 2 CpuSet），不是 20；
  9 条页表类在冻结面上不可表达，计数与原因见 §3/§5。
- K012 只覆盖 `CpuSet::new_empty` 的集合空性，不覆盖原测试的
  `iter().collect::<Vec<_>>()` 适配器；K014 只覆盖 CPU0 的
  add→contains 循环体实例，不覆盖 10 种容量与 `%3` 模式。
- K145–K153 的义务与测试 helper 的序列检查同形，但**不是**对
  `range_difference` 实现的运行时反例：`ContractGapCandidate` 是证明论
  结论（契约未暴露迭代器义务）。
- 未运行的 9 条不是"无需求"；源码证据在 §5，若上游补 owner 铸造入口
  或公开 helper，需新运行证据，不能沿用本表。

## 9. 编写期预检（注册运行前）

### 9.1 预检 v1（`/tmp/vostd-divergence-m7b-ext-run1`）

- 使用首版配方（K014 用 `CpuId::bsp()`）。结果 **11/11 Inconclusive**；
  phase-1 `cargo-verus check --no-verify` 报
  `error: cannot use function ostd::cpu::CpuId::bsp which is ignored
  because it is either declared outside the verus! macro or it is marked
  as external`（`c014_...rs:16:14`）。
- 根因：冻结树里 `CpuId::bsp` 没有 `#[verus_verify]` 标注（`cpu/mod.rs:34`），
  Verus 不从探针暴露它；不是契约缺口。
- 修正（**注册运行前**）：K014 改用带 `#[verus_verify]`/`#[verus_spec]`
  的 `CpuId::try_from(0usize).unwrap()`，并在类型 preamble 加
  `broadcast use crate::cpu::axiom_cpu_count_bounds;` 证明
  `cpu_count() > 0`。§1 哈希已更新为修正后的输入；§4 预测不变。
- 该预检不计入缺陷/判定；判定只认下面的注册运行。
