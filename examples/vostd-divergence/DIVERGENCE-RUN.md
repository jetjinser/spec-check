# vostd 首站义务对照运行归档（test-contract-divergence M5，2026-11-11）

预注册：`DIVERGENCE-EXPECTED.md`（**运行前写定**，本文件不改写它）。
判据闸门：`spike/SPIKE.md`（mutex-lock-contract 锚点双面验证，两面皆中）。
工具：`analyses-divergence`（spec-check master，本次引入）；判定只以
manifest 实测为准。

## 运行命令

```
timeout 900 nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  'cd ~/vie/work/AES/spec-check && \
   ./target/debug/cargo-spec-check analyses-divergence \
     --manifest examples/vostd-divergence/divergence-manifest.json \
     --cargo-verus .../vostd/tools/verus/source/target-verus/release/cargo-verus \
     --z3 .../vostd/tools/verus/source/z3 \
     --vstd .../vostd/tools/verus/source/vstd \
     --out /tmp/vostd-divergence-run1'
```

冻结源 `/tmp/vostd-43da079-ktest-chan`（rev 43da0797bbfa + 既有 ktest 通道
适配）：注入 `ostd/src/spec_check_probes/`（5 个义务 client 模块，
`#[cfg(ktest)]`，`--cfg ktest_capture_gap` 转发，target
`x86_64-unknown-none`），两阶段（precheck `cargo-verus check --no-verify` →
`cargo-verus build` 全量验证）。**运行后恢复原字节已核对**：mutex.rs/rwarc.rs
哈希与运行前一致（mutex.rs `eea4b78a…`），probes 目录已移除，lib.rs 无残留
声明；replay 副本在 `out/injections/`。单次运行 900s 预算内完成。

## 机器判定汇总

- vstd 依赖：1908 verified / 0 errors；
- 目标 workspace（ostd）：**1498 verified / 4 errors**——4 个 error 恰为
  4 个预期失败义务的 `assertion failed`，span 全部落在各自 obligation fn
  内（见下表）；除探针模块外目标工作区零错误事件 → **baseline clean**；
- 判定分布：**ContractGapCandidate 4 / ObligationSupported 1 /
  Inconclusive 0 / Anomaly 0**（5 条目，5/5 与预注册一致）。

## 逐条目判定表（预测 vs 实际）

| # | entry | 预期 | 实际 | 一致? | 失败归因 span |
|---|-------|------|------|-------|----------------|
| 1 | mutex_lock_guard_identity | ContractGapCandidate | ContractGapCandidate | ✓ | `mutex_lock_guard_identity.rs:16` |
| 2 | mutex_try_lock_rel_supported | ObligationSupported | ObligationSupported | ✓ | —（义务验证过） |
| 3 | mutex_try_lock_rel_cross_negative | ContractGapCandidate | ContractGapCandidate | ✓ | `mutex_try_lock_rel_cross_negative.rs:18` |
| 4 | mutex_try_lock_is_none_demand | ContractGapCandidate | ContractGapCandidate | ✓ | `mutex_try_lock_is_none_demand.rs:17` |
| 5 | rwarc_get_cloned_value | ContractGapCandidate | ContractGapCandidate | ✓ | `rwarc_get_cloned_value.rs:16` |

`expect_match` 5/5 = true（机器可查预注册逐条命中）。

## 与研究实验结论的一致性对照

| 条目 | 研究档案结论 | 本轮机器判定 | 一致性 |
|------|--------------|----------------|--------|
| #1 | mutex-lock-contract：原契约 11 pass / client 1 fail；候选 12 pass（spike B 面本轮复现 ObligationSupported，1499/0） | ContractGapCandidate（frozen 面） | ✓ 同一义务两面判定与研究两态结论一致 |
| #4 | mutex-try-lock-demand：HIR 生成 client `is_none()` 断言失败（11 pass / 1 error） | ContractGapCandidate（逐字转写 client） | ✓ 首次由 spec-check 工具线独立复现 |
| #5 | owner-field-return-flows：RwArc 文档 client `assert(value == 5)` 失败（sync::rwarc 8 pass / 1 error），无 adequate candidate | ContractGapCandidate（obligation_fail 级，不升级 candidate_verified） | ✓ |
| #2/#3 | （无对应实验——判据自身对照） | supported 控制 + 阴性对照双双按预期，support 判定非平凡 | ✓ |

registry 更新：F-002 补探针运行证据；新增 F-013（#4）、F-014（#5）为
obligation_fail 级（与 #5 研究口径一致：无候选不虚构升级）。research 仓
commit 紧随本 commit。

## 证据边界

1. **义务 fail ≠ 运行时反例**。ContractGapCandidate 是证明论结论：当前
   契约（ensures 面）不足以让验证器推出该使用方义务；不证明实现错误，
   不证明不存在任何可证义务的 proof。
2. **义务表达受冻结面限制**。#1 采用 by-value 惯用形（冻结 `rel_mutex`
   即值比较语义——正是研究 mutex-lock-contract 指出并修正的建模弱点）；
   RwLock/RwMutex/SpinLock guard 身份与 RCU owner 族谓词属研究侧建模态，
   冻结面不可机械表达，本站不覆盖。
3. **基线对照内建于同一构建**。义务失败与目标工作区自身契约验证在同一
   日志中互证（除探针模块外零错误事件），对应研究证据形态
   "N pass / client fail"。
4. **#2 supported 是"ensures 字面实例"级**：义务恰为 `try_lock` 原有
   ensures 的实例，证明无推理间隙；不代表更强义务（如 #1）被支持。
5. 判定模型相对性：义务文本为项目侧 manifest 输入（工具零项目知识）；
   注入位置 `spec_check_probes` 与 cfg 门继承 vacuity 框架的模型相对性
   口径（M6 CONSTRUCTION-RUN.md §证据边界同源）。

## 产物路径

- 首站运行：`/tmp/vostd-divergence-run1`（report.json / precheck-build.log /
  target-build.log / injections/）。
- spike：`/tmp/vostd-divergence-spike-a`、`/tmp/vostd-divergence-spike-b-run`
  （判据双面）；spike v1（同文本预测被证伪的记录）见设计笔记 §6a 与
  `spike/SPIKE.md`，产物 `/tmp/vostd-divergence-spike-b`（v1 目录名，
  已被 v2 覆盖运行）。
- manifest：`examples/vostd-divergence/divergence-manifest.json`；
  spike manifests `examples/vostd-divergence/spike/divergence-spike-{a,b}.json`。
