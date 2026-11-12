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

## 扩展轮运行（run2，2026-11-12；预注册 `DIVERGENCE-EXPECTED-EXT.md`）

首轮后按 M5 边界清单扩展建模：RwMutex / RwLock / SpinLock / RcuOption 族
6 条新义务（#6–#11，manifest 项目侧建模；建模方式与转写边界见预注册文
件），11 条目单次注入构建。

```
timeout 900 nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  'cd ~/vie/work/AES/spec-check && \
   ./target/debug/cargo-spec-check analyses-divergence \
     --manifest examples/vostd-divergence/divergence-manifest.json \
     --cargo-verus .../vostd/tools/verus/source/target-verus/release/cargo-verus \
     --z3 .../vostd/tools/verus/source/z3 \
     --vstd .../vostd/tools/verus/source/vstd \
     --out /tmp/vostd-divergence-run2'
```

- 机器判定：vstd 1908/0；目标 **1500 verified / 8 errors**——8 个 error
  恰为 8 个预期 gap 义务的失败，span 全部落在各自 obligation fn 内；
  **baseline clean**。判定分布 **ContractGapCandidate 8 /
  ObligationSupported 3**（11 条目，`expect_match` 11/11 = true）。
- 判据闸门：编写期预检 v1–v4 迭代（E0614 → exec-in-assert mode 错 ×2 →
  let 绑定修复；v4 编译全通且判定 11/11 与预注册一致，详见预注册文件
  「编写期预检迭代」节；v4 产物 `/tmp/vostd-divergence-ext-authoring4`）。
- 运行后恢复核对：mutex.rs `eea4b78a…`、lib.rs `f1413d24…` 与运行前一致，
  `spec_check_probes/` 移除，replay 副本在 `run2/injections/`。

### 扩展轮逐条目判定表（预测 vs 实际）

| # | entry | 预期 | 实际 | 一致? | 失败归因 span |
|---|-------|------|------|-------|----------------|
| 1 | mutex_lock_guard_identity | ContractGapCandidate | ContractGapCandidate | ✓ | `mutex_lock_guard_identity.rs:16` |
| 2 | mutex_try_lock_rel_supported | ObligationSupported | ObligationSupported | ✓ | — |
| 3 | mutex_try_lock_rel_cross_negative | ContractGapCandidate | ContractGapCandidate | ✓ | `mutex_try_lock_rel_cross_negative.rs:18` |
| 4 | mutex_try_lock_is_none_demand | ContractGapCandidate | ContractGapCandidate | ✓ | `mutex_try_lock_is_none_demand.rs:17` |
| 5 | rwarc_get_cloned_value | ContractGapCandidate | ContractGapCandidate | ✓ | `rwarc_get_cloned_value.rs:16` |
| 6 | rwmutex_try_read_view_value | ContractGapCandidate | ContractGapCandidate | ✓ | `rwmutex_try_read_view_value.rs:16` |
| 7 | rwlock_try_read_view_value | ContractGapCandidate | ContractGapCandidate | ✓ | `rwlock_try_read_view_value.rs:16` |
| 8 | spinlock_lock_value_readback | ObligationSupported | ObligationSupported | ✓ | —（契约链组合证明过） |
| 9 | spinlock_lock_constant_cross_negative | ContractGapCandidate | ContractGapCandidate | ✓ | `spinlock_lock_constant_cross_negative.rs:29` |
| 10 | spinlock_lock_constant_trivial_cross_control | ObligationSupported | ObligationSupported | ✓ | —（`()==()` 平凡可证） |
| 11 | rcu_option_read_get_value | ContractGapCandidate | ContractGapCandidate | ✓ | `rcu_option_read_get_value.rs:17` |

### 扩展轮的语义读数

- **#9 vs #10 同文本异判定**：义务 `g.constant() == other.constant()` 在
  区分性不变式（`Constant=u64`，5≠7）下 gap、在平凡不变式（`Constant=()`)
  下 supported——契约对 guard↔锁关联的表达力依赖不变式参数，机器演示了
  研究 `rel_lock` 候选（引用身份）所要补的确切缺口；#8 同时给出有证明
  深度的 supported 面（`new`+`lock` ensures 与 open 模型不变式的组合，
  非 ensures 字面实例）。
- **#6/#7/#11 gap 的口径**：均为「文档义务（值读回 / 非空）在无 ensures
  的获取契约下不可证」——与首轮 #5 同级（obligation_fail 级证据），
  不升级 candidate_verified，不虚构 supported 面。
- 身份谓词（`rel_lock`/owner 字段）冻结面不可表达边界维持（M5 预注册
  边界 (a)）；`Rcu`/`RcuDrop` 未从 `crate::sync` 导出（边界比首轮记录
  更收窄，`RcuOption` 为唯一可达 RCU 族类型）。

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
- 扩展轮注册运行：`/tmp/vostd-divergence-run2`（同结构）；编写期预检
  `/tmp/vostd-divergence-ext-authoring{,2,3,4}`（v4 为编译/判据闸门轮）。
- spike：`/tmp/vostd-divergence-spike-a`、`/tmp/vostd-divergence-spike-b-run`
  （判据双面）；spike v1（同文本预测被证伪的记录）见设计笔记 §6a 与
  `spike/SPIKE.md`，产物 `/tmp/vostd-divergence-spike-b`（v1 目录名，
  已被 v2 覆盖运行）。
- manifest：`examples/vostd-divergence/divergence-manifest.json`（扩展轮
  起含 11 条目）；spike manifests
  `examples/vostd-divergence/spike/divergence-spike-{a,b}.json`。

## check 流程双分析端到端（2026-11-12，/tmp/spec-check-e2e）

动机：CI 入口 `examples/ci/run-spec-check.sh` 跑的是 `cargo-spec-check check`——
divergence 接入 check 后，一条命令（本地==CI）才覆盖双分析线。check 配置
声明 `vacuity`（`examples/vostd-guard/manifest.json`，M1 10 条目）+
`divergence`（本节 11 条目 manifest），capture 复用 `/tmp/ktest-chan-capture`
（`status=captured`、`changed_inputs=[]`，fresh → 未重跑捕获），两分析按声明
顺序在同一注入机制下依次运行（`probe_infra` 共享；各自注入后恢复原字节）。

```
./target/debug/cargo-spec-check check --project /tmp/spec-check-e2e/project.json \
  --cargo-verus .../target-verus/release/cargo-verus --z3 .../z3 --vstd .../vstd \
  --out /tmp/spec-check-e2e/check
```

结果：`== capture: reused fresh`；vacuity 10 条目判定与 M1 归档一致
（CandidateInconclusive 6 / ConfirmedLive 3 / ConfirmedVacuous 1）；divergence
11 条目判定与扩展轮一致（ContractGapCandidate 8 / ObligationSupported 3）；
合并 `verdict totals: CandidateInconclusive=6, ConfirmedLive=3,
ConfirmedVacuous=1, ContractGapCandidate=8, ObligationSupported=3`；
**退出码 0**（发现不置红，仅管道失败非零——exit-code 契约）。check 报告
`/tmp/spec-check-e2e/check/report.json`（analyses[] 逐分析 report + repro）。

边界：本轮的 capture 复用既有 fresh 捕获以隔离编排验证；真实夜班首跑
仍会执行 `project-capture`（ostd-nightly 剖面，成本见 CI-WIRING.md 决策④）。
