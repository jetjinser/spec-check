# 判据双面 spike 归档（mutex-lock-contract 锚点，2026-11-11）

锚点：Mutex::lock 的 guard 引用身份义务（研究档案
`mutex-lock-contract/`：原契约 11 pass / client fail，候选契约 12 pass /
全量 1496/0）。预测先于运行写定于 `docs/M5-test-contract-divergence.md`
§6；v1 实测与 §6a 迭代记录见下。两面都中 ⇔ 判据闸门通过。

## v1（同一义务文本两面）：Side B 预测被证伪

义务文本（两面同）：`let lock_ref = MutexGuard::get_lock(&guard);
assert(lock_ref == &mutex);`

| 侧 | 源态 | 实测 | 预测 | 一致? |
|----|------|------|------|-------|
| A | `/tmp/vostd-43da079-ktest-chan` 原样（冻结原契约） | ContractGapCandidate（span :17，基线净 1497/1，vstd 1908/0） | ContractGapCandidate | ✓ |
| B | ktest-chan 副本 + client-injection.patch + spec-change.patch | ContractGapCandidate | ObligationSupported | **✗** |

根因（B 面日志逐字证据）：候选态本身干净——研究 client 所在的
sync::mutex 12 verified / 0 errors（研究结论复现），失败只有探针文本。
`rel_mutex` 两面皆 `pub closed spec fn`，client 侧只能作不透明谓词匹配
ensures 实例；`lock_ref == &mutex` 需展开 rel_mutex/get_lock 体，推不出。
教训：**同一语法文本 ≠ 同一义务的可表达性**；closed 谓词义务必须以
ensures 实例匹配形式转写。预测不改写，迭代记录进设计笔记 §6a。

## v2（同一语义义务、各态契约惯用形）：闸门通过

| 侧 | 义务文本 | 源态 | 实测 | 预测 | 一致? |
|----|----------|------|------|------|-------|
| A | `assert(guard.rel_mutex(mutex));`（by-value 形） | 冻结原契约 | **ContractGapCandidate**（span :16，基线净 1497 verified / 1 errors，vstd 1908/0） | ContractGapCandidate | ✓ |
| B | `assert(guard.rel_mutex(&mutex));`（= 研究 client 原文） | 候选契约 | **ObligationSupported**（目标 1499 verified / 0 errors，含研究 client 所在模块 12/0） | ObligationSupported | ✓ |

**两面皆中 → 判据验证通过**，进入首站运行。修复迭代（判据无关的工具/
manifest 问题，判定预测未因此改动）：

1. spike manifest 缺 `extra_cfgs: ["ktest_capture_gap"]` → 目标 cfg 门
   预编译失败（IrqLine 等无关模块错误），按 vacuity 归档 manifest 补齐。
2. 探针导入路径：`crate::sync::mutex::…` 私有 → 改用 crate 再导出
   `crate::sync::{Mutex, MutexGuard}`。
3. spec 上下文不能调用 exec fn（`assert(get_lock(…) == …)` 编译错）→
   exec 先绑定再断言（v1 文本）。
4. **工具 bug（probe_infra::run_captured）**：日志 append 模式导致 out
   目录复用时跨运行证据污染（v1 的失败 span 残留进 v2 日志、判定被旧
   事件污染）。修复：截断模式（一次运行独占一个日志文件）；vacuity 同
   享该修复。修复后所有 spike/run 均用全新 out 目录。

## 产物

- v2 A 面：`/tmp/vostd-divergence-spike-a`（manifest
  `divergence-spike-a.json`）
- v2 B 面：`/tmp/vostd-divergence-spike-b-run`（manifest
  `divergence-spike-b.json`；源态副本 `/tmp/vostd-divergence-spike-b` =
  ktest-chan + 两 patch，运行注入后已恢复，patch 态保留）
- v1 记录：设计笔记 §6a；v1 B 面日志曾存于
  `/tmp/vostd-divergence-spike-b-run`（修复后 v2 重跑覆盖）。
