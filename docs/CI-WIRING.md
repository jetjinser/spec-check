# CI 接入：vostd 首站（M4）

状态：配置与脚本已交付（2026-11-11；2026-11-12 增加可选 census 通道）。
**未实际触发 CI 运行**；上线前的人工决定项见文末清单。

## 设计原则

1. **本地与 CI 是同一条命令**：本地复现与 CI 运行共用
   `examples/ci/run-spec-check.sh`，workflow 只负责环境准备与 artifact
   上传，没有任何 CI 专属的 check 逻辑。
2. **命令完成即成功**：`cargo-spec-check check` 的退出码契约是只有管道
   失败（配置、capture、工具、未知分析）才非零；发现（vacuous/anomaly
   判定）写报告不算失败。workflow 不做二次失败判定。
3. **确定性**：与 CI 相关的唯一性质是可复现——capture 缓存以
   `changed_inputs` 纪律判定新鲜度，同一输入得到同一报告。
4. **夜班只做在线可自动化的部分**：capture、check（vacuity/divergence）
   与可选 census 可以在无人值守下产生事实/判定；`extract-ktest`、
   `synthesize`、`screen-requires`、`ledger` 是离线人机流程，不伪装成
   夜班自动发现。

## 运行边界：夜班在线 vs 离线人工

```
夜班在线（run-spec-check.sh，同一命令本地/CI）
  vostd workspace
    ├─ capture ──► check(vacuity) ───────────────┐
    ├─ check(divergence，设 DIVERGENCE_MANIFEST) ─┤──► CHECK_OUT/ ──► artifact
    └─ [SPEC_CHECK_CENSUS=1，默认关]              │
         cargo dv focus --log vir ──► crate.vir ──► census ──► CHECK_OUT/census/

离线人工（不进入夜班入口）
  source tree ──► extract-ktest ──► candidates.json ─┐
  recipes.json ──────────────────────────────────────┴─► synthesize ──► manifest
                                                        （人工评审后才交给 check）
  ledger（M7d 台账提案）──► 人工台账/提案（当前尚无 CLI，不写回仓库）
```

| 能力 | 位置 | 输入 | 产物 | 夜班 |
|------|------|------|------|------|
| capture | 在线 | vostd workspace + capture 剖面 | `capture/` | 是 |
| check（vacuity/divergence） | 在线 | capture + manifest | `check/report.json` | 是 |
| census | 在线（可选） | 导出的 VIR + crate 名 | `census/census.json` | 默认关，`SPEC_CHECK_CENSUS=1` 开 |
| extract-ktest | 离线 | `#[ktest]` 源码树 | candidates JSON | 否 |
| synthesize | 离线 | candidates + 项目侧 recipes | divergence manifest | 否 |
| screen-requires | 离线 | `census.json` | requires manifest + 逐函数分层 | 否 |
| ledger | 离线 | 人工/提案输入 | 台账提案 | 否 |

**census 不是 check 的在线分析**：它不经过 capture/check 管线，也不注入
探针或调用验证器；输入是先导出的 VIR（`cargo dv focus --targets ostd --
--no-verify --log vir --log-dir DIR`），纯语法扫描 Exec/Proof 函数并输出
“输出是否被有效后置条件提及”的事实（见 `docs/M7a-spec-census.md`）。
因此它需要额外的 VIR 导出成本（分钟级、VIR 约 100 MB），默认关闭。

**离线合成器不进入夜班**：`synthesize` 依赖项目侧 recipes 与人工挑选，
`extract-ktest` 的候选需要评审，`ledger` 仍是提案；夜班不会自动生成或
提交 manifest/台账。`screen-requires`（M7e）同样离线：它从 census 生成
requires 分层的 quantifier manifest，实际探针运行按批手工发起；夜班只在
`SPEC_CHECK_CENSUS=1` 时产出 census 事实（见 `docs/M7e-requires-screening.md`）。

## vostd CI 现状（2026-11-11 调研）

- vostd 有可用的 GitHub Actions：`.github/workflows/` 下有 `ci.yml`
  （push/PR/dispatch）、`ci-irc11.yml`、`ci-upstream-verus.yml`、
  `verus-update-test.yml`（repository_dispatch），外加本地复合 action
  `.github/actions/setup-verus` 与 dependabot（actions 周检）。
- runner：GitHub-hosted（`ubuntu-24.04`、`ubuntu-latest`、`macos-14`），
  无 self-hosted。
- **CI 内不使用 nix**：现有 workflow 走 apt + rustup；
  `nix/flake.nix` devshell 只用于本地（`.envrc`）。
- 已有同原则先例：`ci.yml` 的 spec-quality 步骤 `ci.py ... --policy
  report`——发现写报告、上传 artifact、不让构建失败。
- setup-verus bootstrap 出的 SDK 正是 check 需要的工具路径：
  `tools/verus/source/target-verus/release/cargo-verus`、
  `tools/verus/source/z3`、`tools/verus/source/vstd`。

结论：走 GitHub Actions 分支——vostd 仓库新增
`.github/workflows/spec-check.yml`（schedule 每夜 + workflow_dispatch）。

## 交付物

| 文件 | 仓库 | 作用 |
|------|------|------|
| `.github/workflows/spec-check.yml` | vostd | 每夜站点：checkout ×2 → setup-verus → nix → 入口脚本 → artifact |
| `examples/ci/run-spec-check.sh` | spec-check | 唯一入口命令（本地 = CI）：环境进入 + 构建 + check + 可选 census + artifact 目录整理 |
| 本文档 | spec-check | 现状、数据流、占位符与上线清单 |

## 入口脚本（run-spec-check.sh）

```
CHECK_OUT/
  project.json   生成的 check CONFIG（绝对路径，可复现）
  capture.json   capture 剖面（默认生成或 CAPTURE_CONFIG 覆盖）
  capture/       project-capture 输出（report.json、changed_inputs）
  check/         check 输出（report.json、analyses/<name>/…、探针日志）
  census/        census 输出（census.json）；仅 SPEC_CHECK_CENSUS=1
  census-vir/    VIR 导出输入（crate.vir，约 100 MB）；同一开关
  run.log        本次运行完整 stdout/stderr
```

环境变量（全部有默认值，除 `VOSTD_DIR`）：

| 变量 | 默认 | 说明 |
|------|------|------|
| `VOSTD_DIR` | 必填 | vostd 检出（需已 `cargo dv bootstrap` 出 SDK） |
| `SPEC_CHECK_DIR` | 脚本所在仓库根 | spec-check 检出 |
| `VERUS_SDK` | `$VOSTD_DIR/tools/verus/source` | cargo-verus/z3/vstd 所在目录 |
| `CHECK_OUT` | `target/spec-check-ci` | artifact 目录 |
| `CAPTURE_CONFIG` | 生成默认剖面 | capture 计划 JSON |
| `CAPTURE_MANIFEST` | `$VOSTD_DIR/Cargo.toml` | capture 目标 workspace |
| `ANALYSIS_MANIFEST` | `examples/vostd-guard/manifest.json` | vacuity 清单 |
| `ANALYSIS_NAME` | `vacuity` | 注册表分析名 |
| `DIVERGENCE_MANIFEST` | 空 | 可选第二分析（注册名 `divergence`） |
| `SPEC_CHECK_NO_NIX=1` | 关 | 跳过 nix develop，直接用当前环境 |
| `SPEC_CHECK_CARGO_FLAGS` | 空 | 额外 cargo build 参数（如 `--offline`） |
| `SPEC_CHECK_JSON=1` | 关 | check 传 `--json` |
| `SPEC_CHECK_CENSUS=1` | 关 | 可选 census：导出 VIR → `census --vir --crate --out` |
| `CENSUS_CRATE` | `ostd` | census `--crate` |
| `CENSUS_TARGET` | `$CENSUS_CRATE` | 导出 VIR 的 dv target |
| `CENSUS_VIR` | 空 | 已有 `crate.vir`；设置后跳过导出 |
| `CENSUS_VIR_DIR` | `$CHECK_OUT/census-vir` | VIR 导出目录（先清空）；指到 CHECK_OUT 外可减小 artifact |
| `CENSUS_SOURCE_ROOT` | 空 | 可选 census `--source-root`（相对源路径） |

默认 capture 剖面 = `ostd` 包、`x86_64-unknown-none`、无附加 cfgs
（即普通验证构建；ktest cfgs 通道可经 `CAPTURE_CONFIG` 换入）。
环境进入 = `nix develop $VOSTD_DIR/nix`（vostd 自带 devshell：rust
toolchain + z3 + openssl），与本地构建命令完全一致；脚本在构建 check
前会 `cargo fetch` 目标 workspace，因为 project-capture 用
`cargo metadata --offline` 解析目标。

census 开关的失败语义：显式开启后，缺少 VIR 导出能力（vostd 无 `dv`
子模块/`cargo dv` alias）或导出没有产生 `crate.vir` 是**管道错误**（非零，
清晰提示如何用 `CENSUS_VIR` 复用已有 VIR）；一旦 census 跑完，其
findings 只是事实，不改变退出码。census 在 check 之前运行，便于快速暴露
导出问题。

## workflow（vostd 侧全文）

占位符：`OWNER/spec-check`（见清单 #1）。其余为最终形态。

```yaml
name: Spec Check (nightly)

# Nightly spec-check station (spec-check roadmap M4, vostd first site).
# Runs the same entry script a local run uses —
# spec-check: examples/ci/run-spec-check.sh — inside the vostd nix
# devshell: build cargo-spec-check, capture the vostd contract surface,
# run the registered analyses, upload the reports.
#
# By contract (ANALYSIS-CONTRACT v1) findings never fail the run: the
# check command exits non-zero only on pipeline errors (config, capture,
# tool, unknown analysis).

on:
  schedule:
    # Nightly. Adjust the time when the station goes live.
    - cron: "40 2 * * *"
  workflow_dispatch:
    inputs:
      spec-check-repo:
        description: >-
          spec-check repository (owner/name). PLACEHOLDER until the
          spec-check repo has a remote — replace the default below.
        required: false
        default: "OWNER/spec-check"
      spec-check-ref:
        description: "spec-check ref to check out (branch, tag, or rev)"
        required: false
        default: "main"
      census:
        description: >-
          Also export VIR and run census. Opt-in: adds minutes of VIR
          export and roughly 100 MB to the artifact.
        required: false
        type: boolean
        default: false

permissions:
  contents: read

concurrency:
  group: spec-check-${{ github.event.schedule || github.ref }}
  cancel-in-progress: false

env:
  CARGO_TERM_COLOR: always
  # Scheduled runs have no `inputs`, so this falls back to "0" (census off).
  SPEC_CHECK_CENSUS: ${{ inputs.census && '1' || '0' }}

defaults:
  run:
    shell: bash

jobs:
  spec-check:
    runs-on: ubuntu-24.04
    timeout-minutes: 300
    env:
      VOSTD_DIR: ${{ github.workspace }}
      SPEC_CHECK_DIR: ${{ github.workspace }}/spec-check
      CHECK_OUT: ${{ github.workspace }}/spec-check-report
    steps:
      - name: Checkout repository
        uses: actions/checkout@v7
        with:
          ref: ${{ github.sha }}
          submodules: recursive

      - name: Set up toolchain and caches
        uses: ./.github/actions/setup-verus
        with:
          verus: 'true'
          install-verusfmt: 'false'

      - name: Checkout spec-check
        uses: actions/checkout@v7
        with:
          repository: ${{ inputs['spec-check-repo'] || 'OWNER/spec-check' }}
          ref: ${{ inputs['spec-check-ref'] || 'main' }}
          path: spec-check

      - name: Install Nix
        uses: DeterminateSystems/nix-installer-action@v23

      - name: Run spec-check
        run: |
          echo "vostd: $VOSTD_DIR"
          echo "spec-check: $SPEC_CHECK_DIR ($(git -C "$SPEC_CHECK_DIR" rev-parse HEAD))"
          echo "artifact dir: $CHECK_OUT"
          echo "census: ${SPEC_CHECK_CENSUS:-0} (1 = VIR export + census, opt-in)"
          "$SPEC_CHECK_DIR/examples/ci/run-spec-check.sh"

      - name: Publish summary
        if: always()
        run: |
          if [[ -f "$CHECK_OUT/check/report.json" ]]; then
            python3 - "$CHECK_OUT/check/report.json" <<'PY' >> "$GITHUB_STEP_SUMMARY"
          import json, sys

          report = json.load(open(sys.argv[1]))
          capture = report["capture"]
          summary = report["summary"]
          print("## Spec check (nightly)")
          print()
          print(f"- spec-check CONFIG: `{report['project_config']}`")
          stale = capture["recaptured_stale_reason"]
          capture_state = "reused fresh capture" if capture["reused"] else "ran project-capture"
          if stale:
              capture_state += f" (stale: {stale})"
          print(f"- capture: {capture_state}")
          print(f"- analyses: {summary['analyses']}")
          verdicts = summary["verdicts"]
          if verdicts:
              totals = ", ".join(f"{name}={count}" for name, count in verdicts.items())
              print(f"- verdict totals: {totals}")
          else:
              print("- verdict totals: none")
          print("- report artifact: `spec-check-report`")
          PY
          else
            {
              echo "## Spec check (nightly)"
              echo
              echo "- ❌ no report.json produced; see the run log"
            } >> "$GITHUB_STEP_SUMMARY"
          fi

      - name: Upload report artifact
        if: always()
        uses: actions/upload-artifact@v7
        with:
          name: spec-check-report
          path: ${{ env.CHECK_OUT }}/
          retention-days: 7
          if-no-files-found: warn
```

注意：summary 步骤的 python heredoc 内容与 `PY` 结束符必须保持在与
`if` 同一列（YAML 按 `run: |` 首行剥缩进后，`PY` 需在第 0 列）。

## 本地运行

每夜站点等价命令（本地）：

```
VOSTD_DIR=~/vie/work/AES/vostd ~/vie/work/AES/spec-check/examples/ci/run-spec-check.sh
```

快速冒烟（用 check-smoke 的玩具 crate，几分钟）：

```
VOSTD_DIR=~/vie/work/AES/vostd \
SPEC_CHECK_DIR=~/vie/work/AES/spec-check \
CAPTURE_CONFIG=~/vie/work/AES/spec-check/examples/check-smoke/capture.json \
CAPTURE_MANIFEST=~/vie/work/AES/spec-check/examples/check-smoke/crate/Cargo.toml \
ANALYSIS_MANIFEST=~/vie/work/AES/spec-check/examples/check-smoke/manifest.json \
CHECK_OUT=/tmp/spec-check-ci-smoke \
SPEC_CHECK_CARGO_FLAGS=--offline \
  ~/vie/work/AES/spec-check/examples/ci/run-spec-check.sh
```

（2026-11-11 已按此命令冒烟通过：capture fresh → 4 契约判定，报告与
`check/report.json` 落盘，vostd 工作区无字节改动。）

可选 census（默认关；VIR 导出分钟级，VIR 约 100 MB）：

```
VOSTD_DIR=~/vie/work/AES/vostd \
SPEC_CHECK_CENSUS=1 \
CHECK_OUT=/tmp/spec-check-ci-census \
  ~/vie/work/AES/spec-check/examples/ci/run-spec-check.sh
```

手动两步等价（便于先导出、再复用）：

```
# 1) 导出 VIR（vostd；用新目录避免 cargo 因缓存不写文件）
(cd ~/vie/work/AES/vostd && nix develop ./nix -c bash -c '
  export VERUS_USE_RUSTUP=0 \
         VERUS_Z3_PATH=$PWD/tools/verus/source/z3 \
         RUSTC_BOOTSTRAP=1
  cargo dv focus --targets ostd -- --no-verify --log vir --log-dir /tmp/spec-check-vir
')

# 2) census（spec-check；纯分析，无验证）
~/vie/work/AES/spec-check/target/debug/cargo-spec-check census \
  --vir /tmp/spec-check-vir/crate.vir \
  --crate ostd \
  --out /tmp/spec-check-census

# 3) 复用已有 VIR 走入口脚本（跳过导出）
VOSTD_DIR=~/vie/work/AES/vostd \
SPEC_CHECK_CENSUS=1 \
CENSUS_VIR=/tmp/spec-check-vir/crate.vir \
CHECK_OUT=/tmp/spec-check-ci-census \
  ~/vie/work/AES/spec-check/examples/ci/run-spec-check.sh
```

census 输出在 `CHECK_OUT/census/census.json`；入口脚本默认把 VIR 导出到
`CHECK_OUT/census-vir/crate.vir`，可用 `CENSUS_VIR_DIR` 指到 artifact 外
以控制上传体积。

离线流程（人工，不属于夜班入口；产物评审后才作为 manifest 输入）：

```
cargo-spec-check extract-ktest --source-root <vostd-src> --out candidates.json
cargo-spec-check synthesize --candidates candidates.json --recipes recipes.json \
  --out divergence-manifest.json
# ledger：M7d 台账提案，当前尚无 CLI；保持人工/离线，不写回仓库
```

## 上线前人工决定清单

1. **spec-check 的 git remote 不存在**（本仓库当前无 remote）。
   workflow 的 `spec-check-repo` input 默认值是占位符
   `OWNER/spec-check`：仓库推送后必须改成真实 owner/name，否则 checkout
   步骤失败。这是唯一的硬性占位符。
2. **spec-check rev 的 pin 策略**：当前默认 `main`。建议定调：每夜站点
   跟 main（跟踪分析演进）或 pin tag/rev（结果可复现优先）+ 定期手动
   升 pin；也可交给 dependabot 对 workflow 内 action 的周检一并覆盖。
   workflow_dispatch 已提供 repo/ref 覆盖入口，策略改变不需要改文件。
3. **cron 时刻**：默认 `40 2 * * *`（UTC）。仓库时区/避开其他夜班任务
   由目标仓库定。
4. **capture 剖面与成本**：默认 `ostd` 无附加 cfgs = 整个 ostd 的
   cargo-verus 验证构建，量级同 ci.yml 的 verify leg（verify job
   timeout 240 分钟）；夜班冷 runner 首轮还要叠加 verus SDK bootstrap
   （setup-verus 缓存未命中时）。job timeout 定 300 分钟，接近 hosted
   runner 单 job 6 小时上限，如溢出需换 capture 剖面或加缓存预热。
5. **census 是否进入夜班、以什么频率**：默认关闭。开启后每轮增加一次
   VIR 导出（分钟级）与约 100 MB 的 `crate.vir`，census 本身是纯 VIR
   分析（秒级到分钟级）。可选：每夜开、每周一次、仅 workflow_dispatch
   开（当前草案用 `census` boolean input，见 workflow 的
   `SPEC_CHECK_CENSUS`），或完全离线跑。若只关心 census 报告而不需要
   留 VIR，设 `CENSUS_VIR_DIR` 到 `CHECK_OUT` 外，或在 artifact 上传前
   删除 `census-vir/`。
6. **ktest cfgs 通道是否作为夜班剖面**：M1/M3 的捕获通道带
   `ktest, ktest_capture_gap` cfgs（research 档案
   `ktest-capture-channel/capture-config.json`）；若要在夜班覆盖 ktest
   面，用 `CAPTURE_CONFIG` 换入该剖面（注意其对 vostd 源的 cfg 依赖已
   在 M3 闭合，但 workflow 不含 ktest-channel.patch，当前主线 vostd
   是否需要补丁未经 CI 验证）。
7. **artifact 保留期与体积**：默认 7 天（与 ci.yml 一致）。常规
   `capture/` + `check/` 之外，census 开启时还有 `census/`（小）与
   `census-vir/`（约 100 MB）；需决定是否缩短 census 相关保留期、上传
   前剔除 VIR，或把 VIR 放 CI 临时盘仅保留报告。
8. **离线 recipes / candidates 的版本管理**：`extract-ktest` 的
   candidates 与 `synthesize` 的项目侧 recipes 由人工维护；建议以
   commit pin 住输入，生成的 manifest 经评审后随 spec-check 提交，夜班
   只消费已评审的 manifest，不自动重跑合成器。`ledger` 仍是 M7d 提案，
   没有 CLI，不纳入夜班。
9. **nix-installer-action 版本**：已 pin `@v23`（2026-11 时最新
   major）；仓库 dependabot 的 github-actions 周检会自动提版本 PR，
   接受与否由目标仓库定。
10. **权限与密钥**：workflow 仅 `contents: read`，不需要任何 secret；
    spec-check 若是私有仓库，需要 checkout 权限（PAT 或同 org 内默认
    GITHUB_TOKEN 的可见范围）——取决于 #1 的落点。
