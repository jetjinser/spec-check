# FULL-RUN 预注册预期（写于运行前，2026-11-11）

Manifest（examples/vostd-guard/manifest.json，10 条目）固定后、运行
`analyses-vacuity` 之前写下。判定只以探针运行为准；若实际与预期不符，
如实记录于 FULL-RUN.md，不改本文件迁就结果。

背景判读：audit（research/experiments/spec-quality-two-day
external-body-owner-audit）对 7 项 stub 的静态判读为**入口前置在调用面恒假**。
但本 manifest 全部 7 项均为 shape reproduction（见 EXTRACTION.md）：tracked
线性资源抽象为不透明 id，ghost 结构谓词 uninterp，只有标量比较/算术被机械
保留。**audit 的恒假性大多来源于 ghost 资源不可伪造与深层不变式，形状抽象
预期把这些条目推向 ConfirmedLive（标量层可满足）或 CandidateInconclusive
（全 uninterp 不可判定）；两种结果都不构成对 audit 的否证，只是层级不同。**
逐条预期与理由如下。

| # | entry | 预期判定 | 依据 |
|---|-------|----------|------|
| 1 | `try_traverse_and_lock_subtree_root_shape` | **ConfirmedLive** | requires = 1 条机械等式（`cursor_level == nr_levels`）+ 2 条 uninterp。witness `(3,3,0)` 满足等式，uninterp 子句可平凡断言，空洞探针 forall 否定被该 witness 驳倒。audit 的恒假判读源于 continuation 树在 `NR_LEVELS-1` 层 all_some 的 ghost 结构，uninterp 抽象吃掉了它。 |
| 2 | `alloc_if_none_shape` | **ConfirmedLive** | 3 条机械标量约束（path_len==level、level<inc-1、level+parent==inc）联立可满足（witness `0,0,1,0,0,2,2,3,2`），其余 uninterp 断言平凡成功。audit 恒假性在 ghost 层（OwnerSubtree/NodeOwner 不变式与 in_count_window），不在标量层。 |
| 3 | `split_if_mapped_huge_shape` | **ConfirmedLive** | 12 条 requires 中 5 条机械（==、<、+==）+ 1 条半机械蕴含（witness 取假前件），联立可满足（witness 见 manifest）。同 #2 的层级论证。 |
| 4 | `alloc_absent_child_shape` | **ConfirmedLive** | 16 条 requires 中 7 条机械可联立满足（witness `0,0,16,1,1,1,true,2,2,2,0,3,3`），其余 uninterp。同 #2。 |
| 5 | `node_lock_shape` | **CandidateInconclusive** | requires 全为 uninterp（node_invariants、external_meta_wf、guards_unlocked），无机械标量内容，按 EXTRACTION.md 见证策略不给 witness：活性探针无法构造见证，空洞探针无法证 forall 否定（uninterp 谓词可取真）。双败 → 不可判定，不计缺陷。 |
| 6 | `make_guard_unchecked_shape` | **CandidateInconclusive** | 与 #5 同形状（requires 完全相同），同预期。 |
| 7 | `mutex_guard_new_shape` | **ConfirmedLive** | 与形状运行（git ae0f897 #1）完全同形：`perm_id == mutex_id` 抽象等式平凡可满足，witness `(0,0)`。形状运行已机器确认此结局；V-001 在 registry 保持 `rule_classified_probe_disputed` 不动。 |
| 8 | `mutex_new_any_value_control`（阴性对照） | **ConfirmedLive** | `Mutex::new` 阴性对照，值域谓词 `val_id < 8`，见证 0。 |
| 9 | `arith_bounds_live_control`（阴性对照） | **ConfirmedLive** | 算术前置 `i < len`，见证 `(0,1)`。 |
| 10 | `arith_oob_vacuous_control`（阳性对照） | **ConfirmedVacuous** | `i < len && i >= len` 矛盾前置，恒假；spike 判据再现。 |

## 意外结局的解读预案（预注册，不运行后改写）

- #1–#4、#7 若得 **ConfirmedVacuous**：说明对应 requires 在纯标量层即自相
  矛盾——超出 audit 静态判读（audit 未主张标量层矛盾），也超出本预注册的
  抽象分析。若发生：复核 witness 是否满足全部机械子句、检查机械转写是否有
  笔误（如 usize 无符号语义），复核后如实归档。
- #5–#6 若得 **ConfirmedLive**：只可能是探针生成器把 exists 退化（或分类器
  归因错误），需检查 probe-build.log 与生成的 lib.rs。
- 任一条目 **Anomaly**：逻辑不可能，指示分类器或触发推断缺陷，按
  RESULT/spike 先例排查（此前发生过 span 对齐 bug，已修复）。
- 对照 #8–#10 任一不符：探针框架或判据失效，全部 7 项结果作废重查。
