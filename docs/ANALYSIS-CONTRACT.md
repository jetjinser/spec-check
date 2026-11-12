# 分析接口契约（v1，2026-11-11 钉定）

本文件是 vacuity 分析的**接口钉定**。两个并行工作分支（`wt-a` check 入口、
`wt-b` construction 探针）都以本契约为准；合并时以本契约仲裁。变更须先改
本文件并单独提交。

## Manifest schema（v1.1，向后兼容 v1.0）

```jsonc
{
  // 可选：构造探针的目标工作区。缺省 = 纯 quantifier 探针（standalone crate）。
  "target_workspace": {
    "manifest_path": "/abs/path/to/target/Cargo.toml",
    "inject_dir": "ostd/src/spec-check-probes",   // 探针模块注入目录（相对 target crate 根）
    "gate_cfg": "ktest"                            // 注入模块的 cfg 门
  },
  "preamble": "...",          // 可选：standalone 探针的 verus 前置（spec fn 等）
  "contracts": [
    {
      "name": "...",
      "probe": "quantifier",    // 默认；standalone crate，crates 内 vstd 依赖
      "params": "s: u64",       // quantifier 专用
      "requires": "...",        // quantifier 专用；具名 spec fn 应用（触发）
      "witness": "0",           // quantifier 专用；可选

      "probe": "construction",  // 构造探针：注入 target workspace
      "model_preamble": "...",  // construction 专用：建模 verus 源（uninterp/
                                //   spec fn、结构体），插在 verus!{ 之后
      "params": "m: Mutex, perm_id: usize",  // 调用面模型参数（普通类型）
      "expect_fail_requires": "perm_id == m.cell_id()",  // 预期不可满足的调用前置
      "construct": "...",       // construction 专用：合法铸造 + 持有路径的 verus 源
      "witness": "..."          // construction 专用：可选（holder 面调用示范）
    }
  ]
}
```

## Runner 行为（`vacuity::run` 签名不变）

`run(manifest, cargo_verus, z3, vstd, out)` —— target_workspace 信息由
manifest 自带，签名不增参。

- 纯 quantifier manifest：现有行为（standalone crate + vstd 依赖）。
- 含 construction 条目：额外两阶段——(1) 向 target workspace 注入探针模块
  （`inject_dir/<name>.rs`，`model_preamble` + entry fn，`#[cfg(gate_cfg)]`，
  附 `--cfg gate_cfg` 的 rustc check 回路先验编译）；(2) cargo-verus 构建
  target workspace，解析日志。
- 日志归因：`postcondition not satisfied` 按 `--> ` 行号归因到最近前趋的
  `proof fn`；`precondition not satisfied` 按 span 归因到
  `expect_fail_requires` 所在 entry fn。

## 分类规则（按 probe 类型）

| 条目 | ConfirmedVacuous | ConfirmedLive | Candidate |
|---|---|---|---|
| quantifier | vacuous 探针 verified | live 探针 verified（含见证） | 两者皆败 |
| construction | 外部铸造调用在 `expect_fail_requires` 处被拒 | holder 面调用验证通过（`construct` 路径） | 两者皆败 |

- 两条探针皆 verified（construction）或皆失败但归因缺位 → `Anomaly`（人工检视）。
- `candidate_inconclusive` 不计缺陷；`Anomaly` 必须人工检视后重分类。

## `check` 入口流程（wt-a）

```
cargo-spec-check check --project CONFIG --cargo-verus P --z3 P --vstd P --out D
```

- CONFIG（项目侧 JSON）：`capture`（project-capture 参数 + 输出目录，含缓存
  判断）+ `analyses: [{ "name": "...", "manifest": "..." }, ...]`。注册表
  现有分析名：`vacuity`（quantifier + construction 一体）、`divergence`
  （义务对照）。多分析并存 = analyses 数组多条声明，逐条按声明顺序运行。
- 流程：capture 目录缺失/陈旧时运行 project-capture；逐个声明分析运行；
  终端输出人类可读摘要（逐契约判定 + 复现命令）；`--json` 输出机器格式。
- 退出码：仅管道自身失败（capture/工具异常）非零；发现本身写报告。
- **流程与分析解耦**：新增分析 = CONFIG 声明 + 对应适配器 + 注册表一行，
  check 流程代码不改。

## 活工作区注入语义（vacuity construction / divergence 共用）

两条分析线的 target-workspace 探针（vacuity 的 construction 条目、
divergence 的义务 client）共用同一注入机制（`probe_infra`），语义如下：

1. **注入点在分析 manifest 里声明，check CONFIG 不感知**。每份 manifest
   自带 `target_workspace`（manifest_path / inject_dir / gate_cfg /
   extra_cfgs / target）；check 的 capture 与各分析可以（也允许）指向
   不同的工作区路径——capture 是基线记录，注入分析需要的是 manifest
   声明的**活工作区**（字节冻结的工作副本）。
2. **注入 → 验证 → 恢复**。运行期向 `inject_dir` 写探针模块、向 crate 根
   追加一行 cfg 门声明；运行结束（含失败路径，guard drop）恢复原字节；
   replay 副本保留在分析输出目录。捕获步骤本身不改动目标工作区。
3. **顺序组合**。一次 check 内多条注入分析按声明顺序执行，每条返回前
   恢复工作区，后一条看到的是恢复后的同一字节态；探针产物互不串扰
   （各自的 precheck/build 目录、日志、report 独立）。
4. **同一构建内对照**。单条注入分析的基线（目标工作区除探针模块外的
   验证）与探针证据出自同一构建日志——义务失败/构造拒绝与目标自身契约
   验证互证，对应研究证据形态 "N pass / client fail"。
5. **发现不落盘、不置红**。注入是临时的；判定只进报告，退出码契约
   不变。
6. **恢复以 guard drop 为路径**（正常返回/Err 均恢复）；进程被硬杀
   （SIGKILL/超时强杀）时 guard 不运行，注入残留需按该分析 out 目录
   `injections/replay.json` 的 restore 说明手工恢复（`lib.rs.orig` 逐字
   节回写 + 删除创建的探针目录），已实测可行。

## 合并预期

- wt-b 合并：vacuity.rs 扩展 + examples/vostd-guard/tracked 验证记录。
- wt-a 合并：CLI check 分支 + 流程模块。
- 合并后端到端验证：construction 条目经 `check` 流程运行（wt-a 流程 +
  wt-b 能力，应零改动组合）。
