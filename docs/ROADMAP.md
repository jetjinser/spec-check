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
  `examples/contract-vacuity-spike/`、`examples/vostd-guard/`）；
  vostd 清单 7 项恒假契约的全量探针运行已完成（M1，见
  `examples/vostd-guard/FULL-RUN.md`）。
- 缺陷台账在研究档案 `experiments/spec-quality-two-day/findings/registry.json`
  （14 条机械发现，9 个去重关系）。

## 主线条目（按优先级）

### M1. contract-vacuity 里程碑③：vostd 契约清单全量运行 —— 完成（2026-11-11）

- [x] 定位清单数据（2026-11-11）：
      `research/experiments/spec-quality-two-day/guard-return-inventory.csv`
      （60 行；`entry_precondition_vacuous=true` 共 7 行，全部
      `external_body=true`：cursor/locking::try_traverse_and_lock_subtree_root、
      node/entry::alloc_if_none、node/entry::split_if_mapped_huge、
      node/entry::alloc_absent_child、node::lock、node::make_guard_unchecked、
      sync::mutex MutexGuard::new）。
- [x] 提取 7 项 stub 契约为 manifest 条目（2026-11-11）：按既定边界做形状
      再现（tracked 参数 → 不透明 usize id；不可转写的 ghost 子句 → 具名
      uninterp spec fn；标量比较/算术机械保留），逐子句对照归档
      `examples/vostd-guard/EXTRACTION.md`。原计划的 extract_manifest.py
      未做：7 项契约分散在 5 个文件的属性块里、结构不一，手工提取加逐条
      核对更可靠；脚本化留待契约量增大（M5 一起考虑）。
- [x] 全量运行 `analyses-vacuity`（2026-11-11）：10 条目（7 契约 + 2 阴性
      + 1 阳性对照），manifest 首轮通过 cargo-verus 编译与触发推断。
      结果：7 项中 0 × ConfirmedVacuous、1 × ConfirmedLive（MutexGuard::new
      纯等式形状）、6 × CandidateInconclusive；对照全部按预期。10 条目中
      6 个与预注册一致；#1–#4 不一致，根因是预注册对 uninterp 谓词
      assert 语义的假设被运行证伪（详见 FULL-RUN.md，预期未改写）。
- [x] 归档 + registry 更新（2026-11-11）：`examples/vostd-guard/FULL-RUN.md`
      （含运行中发现并修复的分类器缺口：live 探针体内 witness `assertion
      failed` 曾被静默当作活性成功）；findings/registry.json V-002 升级为
      逐项探针判定（research 仓库 commit 紧随本 commit）。
- 已知边界（更新）：形状探针无法机器确认 7 项的恒假性——恒假性不在标量
  层（#1–#4 机械子句可满足、#5/#6 无标量约束、#7 等式可满足，均无
  ConfirmedVacuous），只能来自被抽象掉的 tracked 资源与不变式结构。
  `MutexGuard::new` 与 audit 判读的层级分歧保留在 V-001
  `rule_classified_probe_disputed`。恒假性的机器确认需 M2 tracked 建模。

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
