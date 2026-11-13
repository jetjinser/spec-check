# CI divergence 干跑归档（2026-11-13）

预注册：`DIVERGENCE-CI-EXPECTED.md`（运行前写定，本文件不改写它）。
本文件只记录实测，失败/未覆盖边界与预注册并存。

## 1. 对象与命令

- 目标：vostd `experiment/high-level-spec-checker` tip
  `39235be7600e3449f0bee9bfea1a0cf0370e8679`，独立 worktree
  `/tmp/vostd-ci-39235be76`（detached；主工作树未注入）。
- 清单：`examples/ci/divergence-manifest.template.json`，运行时展开为
  `/tmp/spec-check-ci-full/divergence-manifest.json`
  （sha256 `e35ae423…`；模板 `c004ec62…`）。
- 全入口干跑（check-smoke capture/vacuity + divergence；与 CI 同一脚本）：

```
VOSTD_DIR=/tmp/vostd-ci-39235be76 \
SPEC_CHECK_DIR=~/vie/work/AES/spec-check \
CHECK_OUT=/tmp/spec-check-ci-full \
CAPTURE_CONFIG=~/vie/work/AES/spec-check/examples/check-smoke/capture.json \
CAPTURE_MANIFEST=~/vie/work/AES/spec-check/examples/check-smoke/crate/Cargo.toml \
ANALYSIS_MANIFEST=~/vie/work/AES/spec-check/examples/check-smoke/manifest.json \
DIVERGENCE_MANIFEST_TEMPLATE=~/vie/work/AES/spec-check/examples/ci/divergence-manifest.template.json \
  ~/vie/work/AES/spec-check/examples/ci/run-spec-check.sh
```

worktree 不含 `tools/verus` 子模块内容，干跑期间用符号链接共享主检出
SDK（`ln -s ~/vie/work/AES/vostd/tools/verus /tmp/vostd-ci-39235be76/tools/verus`），
结束后删除。这是本地 worktree 的环境布置，不是分析输入。

## 2. 实际判定（与前文预注册逐条对照）

| # | entry | 预注册 | 实际 | expect_match | 失败 span |
|---|-------|--------|------|--------------|-----------|
| 1 | ci_mutex_lock_guard_identity | gap | ContractGapCandidate | ✓ | `ci_mutex_lock_guard_identity.rs:16` |
| 2 | ci_mutex_try_lock_rel_supported | supported | ObligationSupported | ✓ | — |
| 3 | ci_mutex_try_lock_is_none_demand | gap | ContractGapCandidate | ✓ | `ci_mutex_try_lock_is_none_demand.rs:17` |
| 4 | ci_rwarc_get_cloned_value | gap | ContractGapCandidate | ✓ | `ci_rwarc_get_cloned_value.rs:16` |
| 5 | ci_either_left_value_readback | gap | ContractGapCandidate | ✓ | `ci_either_left_value_readback.rs:16` |
| 6 | ci_either_right_value_readback | gap | ContractGapCandidate | ✓ | `ci_either_right_value_readback.rs:16` |

汇总：**ContractGapCandidate 5 / ObligationSupported 1 / Inconclusive 0 /
Anomaly 0；expect_match 6/6**。

## 3. 基线与日志读数

- vstd：**1908 verified / 0 errors**。
- 目标 ostd：**1470 verified / 5 errors**；5 个 error 全部为
  `assertion failed`，且全部落在 6 个探针模块的 obligation fn 内
  （见上表 span）；`baseline_clean = true`。
- 探针模块外目标源码零错误事件；无 `precondition not satisfied` /
  `postcondition not satisfied`。
- `spec_check_probes` 产生 `unexpected_cfgs` warning（ostd 的 check-cfg
  只声明 `ktest`），不影响判定。
- 目标报告：`/tmp/spec-check-ci-full/check/report.json`
  （sha256 `4b1fc365…`）；divergence replay 副本在
  `check/analyses/divergence/injections/`。

## 4. 耗时

- 全入口（含 nix develop、cargo build、smoke capture、vacuity、
  divergence）：`real 4m00.573s`（`user 15m51.563s`）。
- divergence 单阶段（直接运行 `analyses-divergence`，不含 nix 启动）：
  约 `2m39s`（out 目录创建到 target-build 结束）；预算 900s 未触发。
- 两者均在本机实测；CI hosted runner 冷启动会更慢，成本说明见
  `docs/CI-WIRING.md`。

## 5. 运行后恢复

- `git -C /tmp/vostd-ci-39235be76 status --porcelain` 为空；
  `--ignored` 也为空（SDK 符号链接已删除）；`ostd/src/spec_check_probes/`
  不存在；worktree HEAD 仍为 `39235be76`。
- 第一次尝试因 worktree 缺 `tools/verus/source/vstd` 在 precheck 阶段
  以 `cargo metadata` 失败，6 条均为 Inconclusive（`expect_match=false`）。
  这属于环境缺失，不是条目判定；补齐 SDK 链接后按上表重跑。该失败不写入
  预注册，也不计为契约缺口。

## 6. 边界（与预注册一致）

1. tip 基于 `7e53bd122`，与冻结 rev `43da079` 无祖先关系；本批 6 条的
   `ostd/src/sync`、`ostd/src/util` 在两 rev 间逐字节相同，结论不外推到
   分支的 `mm`、`page_table` 等改动。
2. tip 的 `ostd/src/sync/mod.rs` 不导出 `RcuOption`（冻结 rev 仅在
   `cfg(ktest)` 下导出），RCU 条目不可表达，未纳入；也未用 `ktest` gate
   硬启用。
3. 未覆盖：VmReader/VmWriter、WaitQueue、SpinLock、RwLock/RwMutex 的
   值读回等；不是本批结论。
4. `ContractGapCandidate` 是证明论结论，不是运行时反例；Inconclusive
   不计缺陷。
