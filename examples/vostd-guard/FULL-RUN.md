# vostd 契约清单全量运行归档（M1 里程碑③，2026-11-11）

> 本文件是 M1 的 7 项恒假 stub 子集（10 条目）运行档案。全量化运行
> （60 行清单 + 锚点 + 构造对照，28 条目）见 `FULL-RUN-60.md`；
> 预注册与逐条转写对照见 `EXTRACTION-NOTES.md`。

对象：`guard-return-inventory.csv` 中 `entry_precondition_vacuous=true` 的全部
7 项（external_body stub）+ 3 个对照。冻结源
`/tmp/vostd-43da079-ktest-chan/ostd/src`（rev 43da0797bbfa）。
`FULL-RUN-EXPECTED.md` 为运行前预注册；manifest 与逐子句转写见
`manifest.json`、`EXTRACTION.md`。判定只以探针运行为准。

## 运行命令

```
timeout 600 nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  'cd ~/vie/work/AES/spec-check && \
   cargo build --offline && \
   ./target/debug/cargo-spec-check analyses-vacuity \
     --manifest examples/vostd-guard/manifest.json \
     --cargo-verus .../vostd/tools/verus/source/target-verus/release/cargo-verus \
     --z3 .../vostd/tools/verus/source/z3 \
     --vstd .../vostd/tools/verus/source/vstd \
     --out /tmp/vac-vostd-full-run3'
```

最终报告：`/tmp/vac-vostd-full-run3/report.json`；日志
`/tmp/vac-vostd-full-run3/probe-build.log`。vstd 依赖 2060 verified / 0
errors；探针 crate 14 verified / 16 errors（16 = 12 条 `failed this
postcondition` + 4 条 witness `assertion failed`，与下表归因逐一对应）。

## 逐契约判定 vs 预注册预期

| # | entry（冻结源出处） | 预期 | 实际 | 一致? |
|---|--------------------|------|------|-------|
| 1 | `try_traverse_and_lock_subtree_root_shape`（cursor/locking.rs:384） | ConfirmedLive | **CandidateInconclusive** | ✗ |
| 2 | `alloc_if_none_shape`（node/entry.rs:616，impl&%2） | ConfirmedLive | **CandidateInconclusive** | ✗ |
| 3 | `split_if_mapped_huge_shape`（node/entry.rs:968，impl&%2） | ConfirmedLive | **CandidateInconclusive** | ✗ |
| 4 | `alloc_absent_child_shape`（node/entry.rs:2141，impl&%3） | ConfirmedLive | **CandidateInconclusive** | ✗ |
| 5 | `node_lock_shape`（node/mod.rs:375，impl&%2） | CandidateInconclusive | CandidateInconclusive | ✓ |
| 6 | `make_guard_unchecked_shape`（node/mod.rs:401，impl&%2） | CandidateInconclusive | CandidateInconclusive | ✓ |
| 7 | `mutex_guard_new_shape`（sync/mutex.rs:225，impl&%7） | ConfirmedLive | ConfirmedLive | ✓ |
| 8 | `mutex_new_any_value_control`（阴性对照） | ConfirmedLive | ConfirmedLive | ✓ |
| 9 | `arith_bounds_live_control`（阴性对照） | ConfirmedLive | ConfirmedLive | ✓ |
| 10 | `arith_oob_vacuous_control`（阳性空洞对照） | ConfirmedVacuous | ConfirmedVacuous | ✓ |

6/10 一致；#1–#4 不一致（同一根因，见下）。无 Anomaly。所有 7 项契约条目
均为 shape reproduction；机器判定汇总：**7 项中 0 项 ConfirmedVacuous、
1 项 ConfirmedLive（#7，纯等式形状）、6 项 CandidateInconclusive**。

## #1–#4 不一致的根因（如实记录，不改预注册）

预注册的 ConfirmedLive 预期建立在一个事后被证伪的假设上："uninterp 谓词
上的见证 assert 平凡成功"。实际语义相反：`assert(f(args))` 对 uninterp
`f` **不可证**——没有公理强制 `f(args)` 为真（它的否定同样不可证）。#1–#4
的 requires 都含 uninterp 合取项（2/4/4/9 条），因此：

- 空洞探针 `forall … !pre` 失败（机械标量约束可满足，witness 取值即反例
  ——恒假性确实不在标量层）；
- 活性探针 `exists … pre` + 见证也失败（witness assert 无法穿过 uninterp
  合取项）；
- 双败 → CandidateInconclusive（验证器对不透明形状不完备，按判据不计缺陷）。

这落在预注册声明的可能空间内（"形状抽象后可能变为 ConfirmedLive 或
Candidate"），但逐条预期 #1–#4 写的是 ConfirmedLive，判为**不一致**，根因
如上。标量层可满足性（EXTRACTION.md 各 witness 取值）是纸面论证，不是机器
判定。对 audit 的含义：7 项的恒假性不在纯标量层（否则空洞探针会成功），
而在 uninterp 抽象掉的 ghost 资源/不变式层——与 audit 静态判读相容，但
形状探针无法机器确认，需 M2 的 tracked 建模。

## 运行期间发现并修复的工具缺陷（均在 spec-check 仓库内）

1. **分类器归因缺口（实质缺陷，已修复）**：首轮运行
   （`/tmp/vac-vostd-full/probe-build.log`）#1–#4 判 ConfirmedLive，但日志
   中 16 个错误 ≠ 12 条 postcondition 失败——多出的 4 条是 live 探针函数
   体内 witness assert 的 `error: assertion failed`。verus 对被毒化路径的
   ensures 平凡放行，不再报 postcondition 错误，而 `classify()` 只归因
   postcondition 标记 → 不可证的 witness 被静默当作活性成功。违反判据
   文档"活性探针无失败归因才判 ConfirmedLive"。修复：归因器同时识别
   caret 行上的 `assertion failed` 标记（vacuity.rs，附带清理了死代码块）。
   修复后 #1–#4 正确落 CandidateInconclusive。
2. 修复过程本身引入过一次回归（caret 行带 `|` 边栏前缀，
   `starts_with('^')` 永不匹配 → 中间一轮全部误报 Anomaly，
   `/tmp/vac-vostd-full-run2`），同日修复；该轮未作为结果归档，仅作调试
   记录。

Manifest 首轮即通过 cargo-verus 编译与触发推断（10 条目 40 个生成函数），
未消耗 manifest 修复迭代；工具修复共两轮迭代（上限 3 次之内）。

## 证据边界（重要）

- **7 项均为 shape reproduction, not literal contract**：tracked 参数
  （`Tracked<&mut CursorOwner/NodeOwner/OwnerSubtree/MetaRegionOwners/Guards>`
  、`Tracked<PointsTo<T>>`、`Tracked<&NodeOwner>`）抽象为不透明 usize id；
  ghost 结构谓词（invariants、inv、metaregion_sound*、match_pte、
  slots.contains_key、continuations 域/all_some 等）为 uninterp 声明，
  定义体未复现。逐子句对照见 EXTRACTION.md。
- **ConfirmedVacuous 为零**不构成对 audit"恒假"判读的否证：形状探针在
  标量层证明了 7 项的 requires 均非标量自相矛盾（#1–#4 机械子句联立可
  满足、#5/#6 无标量约束、#7 等式可满足），恒假性只可能来自被抽象掉的
  权限/不变式结构（调用面上下文相关），属于 M2（tracked 建模）的范围。
- **CandidateInconclusive 不计缺陷**（判据：验证器不完备）。
- #7（`MutexGuard::new`）与形状运行（git 7a2afd7）结论一致：形状层
  ConfirmedLive，与 audit 判读的层级分歧已记录于 findings registry V-001
  （`rule_classified_probe_disputed`），本次运行不改变该状态。
- 对照集全部按预期（2 阴性 ConfirmedLive、1 阳性 ConfirmedVacuous），
  探针框架与判据在本运行自洽。
- 判定判据沿用 spike（examples/contract-vacuity-spike/RESULT.md）；
  本轮补充：witness assert 失败计入活性探针失败（判据文档既有语义，
  实现缺口已闭合）。

## 历史运行产物

- 首轮（修复前分类器）：`/tmp/vac-vostd-full/`（report.json 中 #1–#4 的
  ConfirmedLive 为误判，证据见其 probe-build.log 的 4 条 assertion failed）
- 回归轮（仅调试）：`/tmp/vac-vostd-full-run2/`
- **最终轮：`/tmp/vac-vostd-full-run3/`（本文件所有数字来源）**
