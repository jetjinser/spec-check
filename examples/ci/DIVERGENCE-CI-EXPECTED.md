# CI divergence 预注册（运行前写定；2026-11-13）

对象：CI divergence 条目在 vostd `experiment/high-level-spec-checker` tip
`39235be76` 上的判定，使用
`examples/ci/divergence-manifest.template.json`（运行时把 `__VOSTD_DIR__`
替换为 CI checkout）。本文件在首次运行前写定，运行后不改写；判定只以
`analyses-divergence` 实测为准，`expect` 不匹配如实报告
（`expect_match=false`）。

## 1. 目标与检查口径

- 目标分支 tip：`39235be7600e3449f0bee9bfea1a0cf0370e8679`
  （`git -C ~/vie/work/AES/vostd rev-parse experiment/high-level-spec-checker`）。
- 分支基点：`7e53bd122`；冻结 rev `43da079` **不是** tip 的祖先（另有
  10+ 个 branch 提交）。因此逐条核对的是 tip 源码，而不是假设冻结面可用。
- 条目只取自 `43da079..39235be76` 中**逐字节未变**的文件：
  `git diff --quiet 43da079..39235be76 -- ostd/src/sync ostd/src/util`
  退出码 0，改动文件数 0。这保证义务的接口、契约与冻结运行（M5/M7c）
  面对的是同一份源码；分支其余改动（`mm/page_table` 等）不参与本批条目。
- 探针 gate 使用自定义 cfg `spec_check_probes`，而不是 `ktest`：
  `ktest` 会启用分支内的 `#[cfg(ktest)] mod test`（其中包含已被注释掉的
  API，见 `rwarc.rs` 的 `lockless_get`），使整个 ostd 编译失败并把所有条目
  降为 Inconclusive。自定义 cfg 在目标树中无同名分支
  （`git grep spec_check_probes 39235be76` 为空），只激活注入模块。
- `extra_cfgs` 为空；target `x86_64-unknown-none`；注入目录
  `ostd/src/spec_check_probes`；两阶段运行（`cargo-verus check
  --no-verify` → `cargo-verus build`），与现有 divergence 分析一致。

## 2. 选定条目与预注册预测

| # | entry | 接口 / 家族 | 义务 | 预期 | 预注册依据 |
|---|-------|-------------|------|------|------------|
| 1 | ci_mutex_lock_guard_identity | `Mutex::lock` | guard 属于产生它的锁（by-value） | ContractGapCandidate | `lock` 无 `#[verus_spec]`/ensures；M5 冻结运行同义务判 gap，源码逐字节未变 |
| 2 | ci_mutex_try_lock_rel_supported | `Mutex::try_lock` | `Some(g) ⇒ g.rel_mutex(*self)` 字面实例 | ObligationSupported | tip 的 `try_lock` ensures 恰为该实例，无推理间隙 |
| 3 | ci_mutex_try_lock_is_none_demand | `Mutex::lock` + `try_lock` | 持 guard 再 try_lock 得 `None` | ContractGapCandidate | 契约只给 `Some` 侧的 `rel_mutex`，无“已持有时返回 None”的 ensures；tip 内同名运行时回归测试（`test_mutex_try_lock_does_not_unlock`）只证明运行时事实 |
| 4 | ci_rwarc_get_cloned_value | `RwArc::get_cloned` | 返回构造值 | ContractGapCandidate | `get_cloned` 无 ensures；M7c C-178 同义务判 gap，源码逐字节未变 |
| 5 | ci_either_left_value_readback | `Either::left` | exec 绑定返回值 == `Some(5)` | ContractGapCandidate | `#[verus_verify(dual_spec)]` 只提供 spec twin，exec 契约无 `returns`；M7c C-232 同判 |
| 6 | ci_either_right_value_readback | `Either::right` | exec 绑定返回值 == `Some(5)` | ContractGapCandidate | 同上（C-233） |

预测汇总：**ContractGapCandidate 5 / ObligationSupported 1 /
Inconclusive 0 / Anomaly 0**；`expect_match` 预期 6/6；基线（探针模块外）
预期干净；vstd 0 errors。实际以运行报告为准。

## 3. 运行配置与预算

- 本地干跑用分支 tip 的独立副本
  `/tmp/vostd-ci-39235be76`（`git worktree add --detach`），不改
  `~/vie/work/AES/vostd` 主工作树；运行后以 `git status --porcelain` 核对
  副本无残留（注入目录/根文件已由工具恢复）。
- 生成 manifest：`DIVERGENCE_MANIFEST_TEMPLATE` 的模板替换；
  实际参与运行的 JSON 落在 `CHECK_OUT/divergence-manifest.json`。
- 命令（与 CI 入口一致的分析阶段）：

```
timeout 900 nix develop ~/vie/work/AES/vostd/nix -c bash -c '
  cd ~/vie/work/AES/spec-check &&
  ./target/debug/cargo-spec-check analyses-divergence \
    --manifest /tmp/spec-check-ci-divergence/divergence-manifest.json \
    --cargo-verus ~/vie/work/AES/vostd/tools/verus/source/target-verus/release/cargo-verus \
    --z3 ~/vie/work/AES/vostd/tools/verus/source/z3 \
    --vstd ~/vie/work/AES/vostd/tools/verus/source/vstd \
    --out /tmp/spec-check-ci-divergence/out'
```

- 预算：单次 900s（与既有 divergence 运行一致）；vstd 与 ostd 全量
  verified build 的耗时以实测为准。

## 4. 已知边界（预注册即承认）

1. **分支与冻结 rev 的关系**：tip 基于 `7e53bd122`，与 `43da079` 无祖先
   关系；两者的 `ostd/src/sync`、`ostd/src/util` 相同，但 `mm`、`page_table`
   等面存在大量差异。本批只对同步/工具族负责，不能外推到分支的其他模块。
2. **RcuOption 不可用**：tip 的 `ostd/src/sync/mod.rs` 把 `RcuOption` 等
   导出整体注释掉（冻结 rev 仅在 `cfg(ktest)` 下导出），因此 M5/M7c 的
   RCU 条目在本分支不可表达，本批不覆盖，也不以 `ktest` gate 硬启用。
3. **未覆盖的已知缺口族**：VmReader/VmWriter（`ostd/src/mm/io.rs` 在分支
   有改动）、WaitQueue::wake_all（源码同冻结面，但本批为控制规模未选）、
   SpinLock、RwLock/RwMutex 值读回等。它们不是本批结论。
4. **ContractGapCandidate 的语义**：是“原契约不足以让 verifier 推出 client
   义务”的证明论结论，不是运行时反例，也不主张实现错误。
5. **Inconclusive 不计缺陷**：若 stage 1 有编译错误或目标验证未运行，条目
   按 Inconclusive 报告，不算 gap，也不会被本文件的预测“修正”。
6. **自定义 cfg 的 lint**：vostd 的 `unexpected_cfgs` 只设 warn；新增
   `spec_check_probes` 可能产生 warning，不影响判定。
