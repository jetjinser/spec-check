# M7d 设计笔记：台账自动化（证据挂接 + 跨工具 dedup + 覆盖看板）

**写定于实现与运行之前**（2026-11-12）。本文件钉定 `cargo-spec-check ledger`
的输入、证据归一化、匹配/dedup 规则、patch 生成与安全边界。工具只生成
**提案**；registry 的维护决定（新增条目、状态转移）始终由人工/root 完成。

## 1. 目标与非目标

目标：把多条分析线的机器产物（vacuity/divergence report、M7b ktest
candidates、M7c synthesis report）与既有研究台账（registry.json）做
**只读对账**，产出三件可审计产物：

```
registry.json (只读) ──┐
report.json × N ───────┼─► ledger ─► ledger-report.json   逐条证据的去向
candidates.json × N ───┤            ├─► registry-patch.json 结构化提案
map.json (可选) ───────┘            └─► DASHBOARD.md        覆盖/未匹配看板
```

非目标：

- **不修改 registry.json**。工具只读台账；写出的 `registry-patch.json` 供
  人工或 root 应用。运行前后对 registry 字节做一致性检查，若变化则报错。
- **不升级 machine_status**。`obligation_fail → candidate_verified` 等状态
  转移必须人工；patch 中状态建议恒为 `null`，并在顶层标注
  `requires_human: true`。
- **不虚构证据**。只消费 report 中已有的判定；report 未给出的信息不推断。
  不因为候选被合成/被选中就声称它是缺口。
- **不做项目专有推断**。工具消费通用 report/registry schema；项目侧的
  接口别名、人工锚点只允许出现在 `--map` 输入文件中。Rust 源码与单测不得
  出现目标项目类型名。

## 2. CLI

```
cargo-spec-check ledger \
  --registry FILE \
  --report FILE... \
  [--candidates FILE...] \
  [--map FILE] \
  --out DIRECTORY
```

- `--report`：vacuity/divergence 判定报告。支持两种形态：
  - 顶层 `verdicts[]`（`name`/`verdict`/`detail`，可选 `expect`/
    `expect_match`/`baseline_clean`/`*_summary`）；
  - `check` 汇总报告中的嵌套 `analyses[].report`（递归解析，来源标签追加
    `#<analysis>`）。
- `--candidates`：候选清单（M7b `extracted-candidates.json` 数组或 M7c
  `synthesize` report 的 `entries[]`）。只用于把 entry 名补全为
  `function`/`dedup_key`/`receiver_type`，**不单独产生证据**。
- `--map`：项目侧可选映射配置，用于人工消歧（见 §6）。
- `--out`：输出目录，写入 `ledger-report.json`、`registry-patch.json`、
  `DASHBOARD.md`。

同一输入集合（路径排序后处理）产生逐字节相同的 JSON 与 Markdown。

## 3. 输入 schema

### 3.1 registry（只读）

消费字段（未知字段忽略并保留，不影响匹配）：

```
schema, findings[] {
  id, kind, subject, relation, dedup_key, interfaces[],
  discovered_by[], machine_status, evidence[] { kind, ref, detail },
  human_review
}
```

工具要求 `findings` 为数组；`id`/`dedup_key` 用于定位；`evidence` 用于
判断"已记录"。不要求 registry 含任何项目专有字段。

### 3.2 判定报告

```json
{
  "verdicts": [
    {"name": "...", "verdict": "...", "detail": "...",
     "expect": "gap|supported", "expect_match": true}
  ],
  "dependency_summary": "...", "target_summary": "...",
  "baseline_clean": true
}
```

判定集合按工具线区分（报告本身不带工具名）：

| 判定集合 | 工具线 |
|---|---|
| `ContractGapCandidate` / `ObligationSupported` / `Inconclusive` | divergence |
| `ConfirmedVacuous` / `ConfirmedLive` / `CandidateInconclusive` | vacuity |
| `Anomaly` | 两者共用，强制人工 |

### 3.3 candidates（仅补全信息）

- 数组：`{id, function, family, dedup_key?, ...}`；
- synthesize report：`{entries: [{candidate_id, entry_name, function,
  family, dedup_key?, receiver_type?}], ...}`。

entry 名中的数字前缀（如 `c096_`、`row15_`、`k007_`）按数字与
`candidate_id`（`C-096`/`K007`）补齐映射；不要求项目侧命名规则固定。

## 4. 证据归一化（verdict → kind）

| 工具线 / 判定 | 证据 kind | 可行动 | 新条目 kind / 状态（仅当未匹配） |
|---|---|---|---|
| divergence `ContractGapCandidate` | `obligation_fail` | 是 | `contract_gap` / `obligation_fail` |
| vacuity `ConfirmedVacuous`（detail 以 `construction:` 开头） | `construction_probe` | 是 | `vacuous_precondition` / `construction_probe_confirmed_vacuous` |
| vacuity `ConfirmedVacuous`（其他） | `probe_run` | 是 | `vacuous_precondition` / `candidate_unverified` |
| divergence `ObligationSupported` | `obligation_supported` | 否 | — |
| vacuity `ConfirmedLive` | `probe_run` | 否 | — |
| `CandidateInconclusive` / `Inconclusive` | `probe_inconclusive` | 否 | — |
| `Anomaly` | `anomaly` | 否（强制人工） | — |
| 未知判定 | `machine_evidence` | 否 | — |

附加规则：

- 名称含 `_control` / `cross_negative` / `negative_control` 的条目视为
  **对照**，不产生 append/new_entry（匹配到已记录证据时记 noop；否则记
  unmatched，原因 `control-entry`）。
- 非可行动判定若匹配到条目，记 noop（原因
  `verdict-not-actionable`）；未匹配记 unmatched。不得把 supported/live
  写成缺口证据。

## 5. 匹配与 dedup（按优先级）

1. **map 规则**（人工显式）：`source` glob + `entry` glob + 可选
   `report_format`，命中后直接指定 `registry_id`/`dedup_key`，并可覆盖
   `evidence_kind`。规则按文件顺序，首个命中生效；目标 id/dedup_key 不存在
   则整次运行失败（fail closed，避免拼写错误静默空跑）。
2. **entry 名锚点**：registry 任一文本字段（subject/relation/dedup_key/
   interfaces/evidence ref/detail）中出现与报告 entry 完全相同的标识符。
   这是跨工具 dedup 的最强锚点（研究档案通常逐字引用机器 entry 名）。
3. **dedup_key 精确**：候选/报告自带 `dedup_key` 与 `findings[].dedup_key`
   完全相同。
4. **函数/接口规范化**：把候选 `function`（或 entry 名，去掉
   `c096_`/`row15_`/`k007_` 前缀与 `_shape`/`_holder_face`/`_construction`
   等阶段后缀）拆成 token；registry 侧从 `interfaces[]`、`subject` 中的
   `Type::method` 链构造接口别名。接收者 token 与方法 token 全部命中时
   记函数级匹配。
5. **关系语义相似度 / 歧义队列**：仅 token 部分命中、或最高分不唯一时，
   归为 `ambiguous`，列出候选 id/dedup_key/分数，进入人工队列。**歧义不
   自动 append，也不新建条目。**

不匹配任何条目时：

- 可行动且非对照 → `new_entry` 提案；
- 否则 → `unmatched`（附原因，如 `no-match`、`non-actionable-verdict`、
  `control-entry`、`ambiguous-match`）。

"已记录"判定（matched 后）：

- 该条目的任一既有 evidence 的 ref/detail 文本包含报告 entry 名；或
- 既有 evidence 的 ref 与本次来源路径归一化后互相包含（同一运行目录）。
- 已记录 → `noop`（原因 `evidence-already-present`），不重复追加。

同一 `(registry_id, evidence_kind, 语义 entry)` 在多份报告中重复出现时，
合并为**一个** append 提案（`sources` 列出全部来源），其余记录记
`noop`（原因 `merged-into-proposal`）。同理合并同一语义的新条目提案。

## 6. map schema（项目侧）

```json
{
  "schema": "spec-check.ledger-map/1",
  "rules": [
    {
      "source": "*vostd-divergence-run1*",
      "entry": "mutex_try_lock_is_none_demand",
      "registry_id": "F-013"
    },
    {
      "entry": "c096_*",
      "dedup_key": "vmio-has-remain-has-avail-demand",
      "evidence_kind": "obligation_fail"
    }
  ]
}
```

glob 仅支持 `*`/`?`。所有 selector 同时满足才命中；省略的 selector 视为
不限制。map 只做消歧与别名，不产生证据。

## 7. patch 生成

`registry-patch.json`：

```json
{
  "schema": "spec-check.ledger-patch/1",
  "requires_human": true,
  "note": "status transitions and registry application are human-only",
  "registry": {"path": "...", "findings": 19, "bytes": 31658,
               "unchanged_after_run": true},
  "summary": {"append_evidence": 0, "new_entry": 0,
              "noop": 0, "unmatched": 0, "records": 0},
  "proposals": [
    {
      "action": "append_evidence",
      "registry_id": "F-013",
      "dedup_key": "mutex-try-lock-is-none-demand",
      "match": {"tier": "entry-anchor", "score": 1000},
      "evidence": {"kind": "obligation_fail", "entry": "...",
                   "verdict": "...", "sources": ["..."]},
      "proposed_evidence": {"kind": "obligation_fail",
                            "ref": "...", "detail": "..."},
      "status_suggestion": null,
      "requires_human": true,
      "reason": "..."
    }
  ]
}
```

四类 action：

- `append_evidence`：匹配成功且证据未记录；只追加 evidence。
- `new_entry`：可行动证据无任何匹配；给出保守的 `proposed_entry`
  （`id: null`，kind/status 按 §4 表，dedup_key 为
  `ledger:<slug>` 占位，需人工改定）。证据一并放入 `evidence[]`。
- `noop`：已记录、非可行动、对照、或已合并到其他提案。
- `unmatched`：无匹配且不可行动/歧义；带原因与候选。

`proposed_entry` 不写 `machine_status` 升级，不写 `candidate_verified`。

## 8. 覆盖看板（DASHBOARD.md）

- 输入报告清单（来源、格式、条目数、可行动数）；
- patch 四类计数；
- 报告证据按 verdict/kind 分布；
- registry 覆盖：每条 finding 的 kind/status/接口数/既有证据数/本轮匹配
  记录数/append/noop；
- 逐函数（或 entry）覆盖：记录数、匹配条目、append、unmatched；
- 未匹配/歧义证据清单（含原因与候选）。

所有计数由实际运行生成，不手填。

## 9. 安全边界

- registry 只读；运行前后重新读取并比较字节，`unchanged_after_run` 写入
  patch。若变化，`ledger` 报错（说明有并发写入），不产出可信 patch。
- 输出目录由调用方给出；工具只写三个约定文件，不写 registry 路径。
- 判定解释保持证明论口径：`ContractGapCandidate` 不是运行时反例，
  `Inconclusive` 不计缺陷。patch 的 `reason` 字段保留原始 detail。
- 未匹配、歧义、对照一律如实报告；不以"看起来像"补匹配。
