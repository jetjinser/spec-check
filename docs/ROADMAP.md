# 主线路线图（mainline roadmap）

本文件是工具仓库的主线状态记录。**进度必须写在这里**，不依赖任何会话记忆；
每次完成或推进一个条目，更新本文件并提交。

## 当前状态（2026-11-11）

- 引擎已从研究仓库独立（spec-check-core，47 模块，93 测试/0 失败）。
- `project-capture` 可用；ktest-cfg 捕获通道已打通（vostd 上 1,496 验证 /
  0 错误，27 个 ktest 函数进入 VIR；详见研究档案
  `experiments/spec-quality-two-day/ktest-capture-channel/`）。
- 第一条分析 `contract-vacuity` 已实现并接入 CLI：
  `analyses-vacuity --manifest --cargo-verus --z3 --vstd --out`。
  判据经 spike 与 vostd 形状运行验证（见
  `examples/contract-vacuity-spike/`、`examples/vostd-guard/`）。
- 缺陷台账在研究档案 `experiments/spec-quality-two-day/findings/registry.json`
  （14 条机械发现，9 个去重关系）。

## 主线条目（按优先级）

### M1. contract-vacuity 里程碑③：vostd 契约清单全量运行 —— 待做

- [ ] 定位 60 接口 guard 清单 JSON（含"7 项入口前置恒假"分类）：约在研究
      档案 guard_owner 扫描输出（`checker-evidence/spec-quality/` 或
      `experiments/test-spec/`；扫描逻辑在 `guard_owner.rs`）。
- [ ] 写 `examples/vostd-guard/extract_manifest.py`：从清单 + 冻结源码
      `/tmp/vostd-43da079-ktest-chan` 提取每个接口的 `#[verus_spec]`
      requires 文本 → manifest 条目。跳过 tracked 参数契约并记录跳过原因
      （tracked 不可表达，见 M2）；无 tracked 参数的保留 requires 原文。
- [ ] 全量运行 `analyses-vacuity`，对照预注册预期（7 项恒假应报
      ConfirmedVacuous 或 Candidate；其余 ConfirmedLive）。
- [ ] 归档运行结果 + 更新研究档案 findings/registry.json 的 V-002 条目
      （7 项从 rule_classified 升级为探针判定）。
- 已知边界：`MutexGuard::new` 形状探针与 audit 判读不一致（PointsTo 不可
  伪造性无法在普通参数建模）——保持如实归档，不在模型上迁就。

### M2. tracked 参数建模 —— 设计中

形状探针无法表达 `Tracked<PointsTo<T>>` 不可伪造性（MutexGuard::new 判定
不一致的根因）。候选方向：manifest 支持 `tracked_stubs`（以 uninterp spec
fn 建模权限语义）。先做设计笔记，不急着实现。

### M3. ktest 通道 14 项排除闭合 —— 待做

见研究档案 `ktest-capture-channel/README.md` 排除清单。逐项判定：可修复
（如 dma re-export 类已完成）vs 永久排除（已删机制）。已在通道内修复的
不再重复。

### M4. CI 接入（vostd 首站）—— 等 M1-M3

子命令已具备条件；接入 = vostd 仓库加一个调用 spec-check 二进制的 job。

### M5. 更多分析 —— 设计中

test-contract-divergence（把研究管道的义务对照固化成子命令）复用 vacuity
的探针生成器框架。

## 维护纪律

1. 主线进度只认本文件与 git 历史；会话记忆不作为状态来源。
2. 每个条目推进后：更新本文件勾选状态 + 提交。
3. 引擎模块重构必须带重放验证（研究档案证据可重放）。
