# M7d 台账自动化首站运行（vostd 研究台账对账）

生成时间：2026-11-12。工具：`cargo-spec-check ledger`（spec-check master，
基于 commit `c25ea69` 的开发工作区）。**只读 registry，只产出提案**。

## 1. 命令

```
cargo build --offline -p cargo-spec-check

./target/debug/cargo-spec-check ledger \
  --registry /home/jinser/vie/work/AES/research/experiments/spec-quality-two-day/findings/registry.json \
  --report \
    /tmp/vostd-divergence-run1/report.json \
    /tmp/vostd-divergence-run2/report.json \
    /tmp/vostd-divergence-m7b-run1/report.json \
    /tmp/vostd-divergence-m7c-run1/report.json \
    /tmp/vostd-divergence-m7c-promoted-run1/report.json \
    /tmp/vostd-divergence-m7c-promoted-run2/report.json \
    /tmp/vostd-divergence-m7c-promoted2-run1/report.json \
    /tmp/vac-construction-main6/report.json \
    /tmp/vac-vostd-full60-run1/report.json \
    examples/vostd-guard/tracked/integration-report.json \
  --candidates \
    examples/vostd-ktest/divergence-manifest-m7b.report.json \
    examples/vostd-ktest/extracted-candidates.json \
    examples/vostd-divergence/divergence-manifest-m7c.report.json \
    examples/vostd-divergence/divergence-manifest-m7c-promoted1.report.json \
    examples/vostd-divergence/divergence-manifest-m7c-promoted2.report.json \
    examples/ensures-coverage-candidates/candidates.json \
  --out examples/ledger
```

未使用 `--map`：自动匹配（entry 名锚点 / dedup_key / 函数接口 / 语义 token）
已覆盖全部可行动证据；歧义项均为对照/非可行动判定，映射不会改变 action。

## 2. 数据来源

| 来源 | 类型 | 条目 | 可行动 |
|---|---|---:|---:|
| `/tmp/vostd-divergence-run1/report.json`（M5 首轮） | divergence | 5 | 4 |
| `/tmp/vostd-divergence-run2/report.json`（M5 扩展轮） | divergence | 11 | 8 |
| `/tmp/vostd-divergence-m7b-run1/report.json`（M7b ktest） | divergence | 2 | 1 |
| `/tmp/vostd-divergence-m7c-run1/report.json`（M7c 合成器） | divergence | 22 | 21 |
| `/tmp/vostd-divergence-m7c-promoted-run1/report.json`（needs-recipe 失败轮） | divergence | 3 | 0 |
| `/tmp/vostd-divergence-m7c-promoted-run2/report.json`（needs-recipe 成功轮） | divergence | 2 | 2 |
| `/tmp/vostd-divergence-m7c-promoted2-run1/report.json` | divergence | 1 | 1 |
| `/tmp/vac-construction-main6/report.json`（构造探针批量） | vacuity | 14 | 3 |
| `/tmp/vac-vostd-full60-run1/report.json`（60 接口全量） | vacuity | 28 | 2 |
| `examples/vostd-guard/tracked/integration-report.json`（check 流程嵌套） | vacuity | 3 | 1 |

候选索引（只补全 `function`/`dedup_key`，不产生证据）：
`divergence-manifest-m7b.report.json`、`extracted-candidates.json`、
`divergence-manifest-m7c.report.json`、`divergence-manifest-m7c-promoted{1,2}.report.json`、
`examples/ensures-coverage-candidates/candidates.json`。

**registry 实际规模：26 条**（F-001..F-024 中实际存在者 + V-001/V-002；
本任务背景里写的 19 条与文件不符，以运行读取的 26 条为准）。registry
31,658 字节，运行前后字节一致（`registry.unchanged_after_run = true`），
sha256 `5f9de690a38d91aa97b6bbe3ae7859d23ce3c92fa88d3f2e90f1bf6f392a34a9`。

## 3. 顶层结果（以 ledger-report.json summary 为准）

| 指标 | 数量 |
|---|---:|
| 证据记录（report verdicts） | 91 |
| 可行动（gap / ConfirmedVacuous） | 43 |
| 匹配到既有条目 | 64 |
| 歧义（最高分并列，未自动挂接） | 4 |
| 未匹配 | 23 |
| action `append_evidence` | 1（合并 3 个来源） |
| action `new_entry` | 0 |
| action `noop` | 63 |
| action `unmatched` | 27 |

- 91 条记录的 `status_suggestion` 全部为 `null`；`requires_human` 仅
  append/new_entry 为 true。
- 1 条 append 提案（registry id `V-001`）合并了 3 个来源：
  `/tmp/vac-construction-main6`、`/tmp/vac-vostd-full60-run1`、
  `examples/vostd-guard/tracked/integration-report.json#vacuity`。这是
  `mutex_guard_new_construction`（`construction_probe`），V-001 既有证据
  未逐字引用这些运行/entry，因此按维护规则提议追加证据，不变更状态。
- 0 条 new_entry：本批所有可行动 gap/vacuous 证据都能匹配到既有条目，
  **没有产生重复条目**。

## 4. 已知重叠核对

| registry | 语义 | 匹配记录数 | 结果 |
|---|---|---:|---|
| F-013 | Mutex::try_lock is_none demand | 3 | M5 run1、M5 run2、M7b K131 全部匹配 F-013；registry 已逐字记录 → noop，无新条目 |
| F-014 | RwArc get_cloned / clone_ro 值读回 | 4 | M5 run1/run2 + M7c C-177/C-178 → noop |
| F-015 | Mutex/RwLock/RwMutex 值读回 | 9 | M5 run2 + M7c C-162/C-180/C-181/C-184/C-195/C-196/C-199 → noop |
| F-016 | SpinLock constant 关系 | 3 | 跨锁 gap、平凡 control、supported 三个探针均匹配 F-016 → noop |
| F-017 | RcuOption 值读回 | 4 | M5 #11 + M7c C-167/C-169/C-170 → noop |
| F-018 | VmReader/VmWriter limit/skip 接收者后状态 | 4 | C-097/C-098/C-101/C-102 → noop |
| F-019 | has_remain/has_avail 布尔需求 | 2 | C-096/C-100 → noop |
| F-020 | Either left/right exec 返回值 | 2 | C-232/C-233 → noop |
| F-021 | WaitQueue wake_all 空队列 | 1 | C-216 → noop |
| F-022 | AsVirtPtr 指针读回 | 2 | promoted run1（Inconclusive）/run2（gap）→ noop |
| F-023 | Jiffies elapsed 单调性 | 2 | promoted run1/run2 → noop |
| F-024 | PageTablePageMeta::new level 读回 | 1 | C-127 → noop |

说明：noop 的判定是"该条目的既有 evidence ref/detail 已引用同一 entry
名，或已引用同一运行目录"。因此**已人工记入 registry 的来源不会被重复
追加**；只有未被记录的新来源才进入 append。这与"同一语义只计一条、
证据追加不虚构升级"一致。

## 5. 歧义与未匹配实例

### 5.1 歧义（4 条，均未自动挂接）

| 来源 | entry | verdict | 候选（分数） |
|---|---|---|---|
| run1 / run2 | `mutex_try_lock_rel_supported` | ObligationSupported | F-013(600), F-015(600) |
| run1 / run2 | `mutex_try_lock_rel_cross_negative` | ContractGapCandidate | F-013(600), F-015(600) |

两条都是 mutex try_lock 关系族的**对照/支持证据**（非新缺口）；entry 名
不含接收者类型，语义 token 同时命中 F-013 与 F-015。按规则进入人工队列，
action 记为 `unmatched`（reason `ambiguous-match`），不 append、不新建。
若需消歧，可在项目侧 `--map` 中加
`{"entry":"mutex_try_lock_rel_*","registry_id":"F-002"}`（F-002 的
guard-owner 关系是这两条对照的直接锚点）。

### 5.2 未匹配分类

- **对照条目（3 条）**：`arith_bounds_live_control`、
  `arith_oob_vacuous_control`、`mutex_new_any_value_control`。即使
  `arith_oob_vacuous_control` 是 ConfirmedVacuous 也不产生新条目。
- **非可行动判定且 registry 未引用（20 条）**：`ObligationSupported`
  （`c007_memoryregionarray_push`、`c104_vmwriter_from`）、`Inconclusive`
  （`c006_drhd_register_base_addr`）、`ConfirmedLive` / `CandidateInconclusive`
  的 `rowNN_*` 与 holder-face 条目等。它们不是缺口证据，按规则只报
  unmatched，不写成 finding。
- **函数名/抽象名漂移**：`row00_read_inner`、`row29_read`、
  `row25_try_lock`、`row37_try_read` 等只有方法名/行号抽象，缺少
  registry 侧可对齐的接收者或 entry 锚点；工具如实报 unmatched，而不是
  猜测挂到某个锁族条目。宏合并名（如 `arc_*` / `lock_*` 抽象）同理。

## 6. 覆盖看板读法

`DASHBOARD.md` 的 registry coverage 表中 12 条 finding（F-001、F-003..
F-012）本轮 matched=0，是因为本次输入没有覆盖这些家族的报告，**不代表**
它们没有既有证据（这些条目 evidence 非空）。逐函数覆盖率按候选
`function`（无候选时按 entry 语义键）聚合；`append` 与 `noop` 分列，
便于人工判断哪些条目需要应用补丁。

## 7. 边界与失败项（如实）

- registry 实际 26 条，不是任务背景所述 19 条。
- 本批输入含 `/tmp` 下的运行产物；RUN.md 记录路径，不复制大文件。缺失
  这些目录时无法逐字节复现同一报告，但同一输入集合的输出是确定的
  （单测 `output_is_deterministic`）。
- 自动匹配没有产生 new_entry；new_entry 路径由单测覆盖
  （`unmatched_gap_becomes_new_entry_proposal`），本批没有需要新建的
  finding。
- 4 条歧义未自动解决；没有引入项目专有匹配规则（`--map` 留空）。
- `mutex_guard_new_construction` 的 append 提案合并三个来源；这是本轮
  唯一未在 V-001 evidence 中逐字出现的构造探针来源集合。其
  `match_score=790`，V-002/F-015 以 600 分排在候选中，未影响唯一最高分。
- 未修改 ROADMAP；未修改 registry.json；未触碰
  `synthesize.rs`/`examples/vostd-ktest/**`/`examples/ci/**`。

## 8. 产物

- `ledger-report.json`：91 条记录的匹配状态、action、候选、提案。
- `registry-patch.json`：同上（按 action 排序）+ registry 元信息；
  `requires_human: true`，全部 `status_suggestion: null`。
- `DASHBOARD.md`：输入、四类计数、verdict/kind、registry 覆盖、逐函数
  覆盖、未匹配/歧义清单。
- `RUN.md`：本文件。
