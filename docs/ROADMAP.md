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
- M2 tracked 建模已完成：构造探针判据（route c）机器确认 MutexGuard::new
  形状 ConfirmedVacuous 且阴性对照零误报；量词判据下真实 tracked 语义
  判 ConfirmedLive（固有可满足），V-001 分歧精确定性为"孤立语义 vs
  分布性可达性"。见 `docs/M2-tracked-modeling.md`、
  `examples/tracked-modeling-spike/`。

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

### M2. tracked 参数建模 —— 完成（2026-11-11）

**生成器接入已完成**（M1/M2 分支合并后，commit fcfa3e3）：manifest v1.1
（target_workspace 注入 + construction 条目）实现于 vacuity.rs，3/3 验证
一致；`check` 流程端到端组合验证通过（wt-a 流程 + wt-b 能力零改动组合，
报告 `examples/vostd-guard/tracked/integration-report.json`）。

设计笔记 `docs/M2-tracked-modeling.md` + spike
`examples/tracked-modeling-spike/`（预测先于运行写定，结果逐项回填
RESULT.md）。三路线结论：

- (a) uninterp `forgeable` 建模铸造集：**不可行**（结构性：uninterp 谓词
  只能加模型，空洞方向不可证；公理化铸造集为空即假设结论；M1 条目
  #5/#6 的 CandidateInconclusive 是同一机制的机器佐证）。
- (b) 真实 tracked 类型 + 平面量词探针：**不能产生 ConfirmedVacuous**，
  但给出 V-001 分歧的精确刻画——`perm.id() == mutex.cell_id()` 在孤立
  vstd 语义下固有可满足（空洞 forall 被 `PCell::new` 配对反例否证，
  铸造见证活性探针 verified → ConfirmedLive），audit 恒假是分布性判断。
- (c) 源级构造探针（spike 主体）：**可行**。恒假 = 唯一可写伪造通道
  （新铸 PCell 配对）被 verifier 拒绝且失败归因于被预测的 requires
  （c1 → ConfirmedVacuous，构造判据）+ 伪造原语 `Tracked::assume_new`
  被 verus 编译级阻断（s1 归档）；活性对照 holder 面调用（c2）与外部
  可构造的 read 形状（c3）均 verified → 阴性对照零误报。不含任何直接
  假设结论的公理；不可伪造性复用 verus 项纪律，非注入公理。
- 生成器实现建议（`vacuity.rs` 五条：construction 条目类型、tracked
  params 发射、precondition 失败归因、assume_specification 信任边界、
  detail 证据形态标注）在 RESULT.md 末节，M3+ 实施时取用。
- registry V-001 的 machine_status 更新属 research 仓库，本次未动。

### M3. ktest 通道 14 项排除闭合 —— ✅ 完成（research commit d32e67f7）

4 项 closed-fixed（into_non_overlapping external_body 恢复、va_range、
kvirt_area len/query、protect_range 闭包桥接）/ 3 项 permanent（机制已删）/
7 项 fixable-expensive（工作量已评估）。重跑 1 轮：1,497 verified /
0 错误，ktest owner 27→28。处置表与残余风险见
`research/.../ktest-capture-channel/README.md`。

### M4. CI 接入（vostd 首站）—— 完成（2026-11-11；配置与脚本交付，未触发 CI 运行）

调研结论：vostd 已有 GitHub Actions（GitHub-hosted `ubuntu-24.04`/
`macos-14` runner，无 self-hosted；`setup-verus` 复合 action 负责 apt +
rustup + `cargo dv bootstrap` SDK 与缓存，bootstrap 路径正是 check 需要的
`tools/verus/source/{target-verus/release/cargo-verus,z3,vstd}`；CI 内不用
nix，flake 仅本地）。`ci.yml` 已有 spec-quality `--policy report` 先例
（发现写报告不算失败），与本工具设计原则一致。spec-check 仓库当前无 git
remote。详见 `docs/CI-WIRING.md`。

- [x] 入口脚本 `examples/ci/run-spec-check.sh`（2026-11-11）：本地与 CI
      同一条命令——nix devshell 进入（`SPEC_CHECK_NO_NIX=1` 可关）+ 构建
      + `cargo-spec-check check` + artifact 目录整理（`CHECK_OUT/` 下
      project.json、capture 剖面、capture/、check/、run.log）。
- [x] workflow `vostd:.github/workflows/spec-check.yml`（2026-11-11）：
      schedule 每夜（cron 40 2 * * *）+ workflow_dispatch（repo/ref 可
      覆盖）；checkout vostd + spec-check → setup-verus → nix-installer
      → 入口脚本 → summary（verdict 总数）→ 上传 `spec-check-report`
      artifact（7 天）。发现不置红，退出码契约同 check。
- [x] 文档：`docs/CI-WIRING.md`（现状、数据流、workflow 全文、本地冒烟
      命令、上线前人工决定清单）；本节。
- [x] 冒烟验证（2026-11-11，check-smoke 玩具 crate 输入换入）：脚本全
      流程跑通，fresh capture → 4 契约判定（2 ConfirmedVacuous +
      2 ConfirmedLive，与 wt-a E2E 一致），artifact 目录完整，
      vostd 工作区无新增改动。
- 上线前待人工决定（CI-WIRING.md 清单）：① spec-check remote 未定，
  workflow 的 `OWNER/spec-check` 占位符必须填定；② spec-check rev 的
  pin 策略（当前默认 main）；③ cron 时刻；④ capture 剖面成本（默认
  ostd 全量验证 ≈ ci.yml verify 量级，夜班冷 runner 首轮叠加 SDK
  bootstrap，job timeout 300min）；⑤ ktest cfgs 剖面是否换入；
  ⑥ artifact 保留期；⑦ nix-installer-action@v23 升级策略；
  ⑧ 私有仓库 checkout 权限（视 ①）。

### M5. 更多分析 —— 设计中

test-contract-divergence（把研究管道的义务对照固化成子命令）复用 vacuity
的探针生成器框架。

## 维护纪律

1. 主线进度只认本文件与 git 历史；会话记忆不作为状态来源。
2. 每个条目推进后：更新本文件勾选状态 + 提交。
3. 引擎模块重构必须带重放验证（研究档案证据可重放）。
