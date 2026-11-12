# 主线路线图（mainline roadmap）

本文件是工具仓库的主线状态记录。**进度必须写在这里**，不依赖任何会话记忆；
每次完成或推进一个条目，更新本文件并提交。

## 当前状态（2026-11-12）

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

- M7c 义务 client 合成器已实现：`cargo-spec-check synthesize --candidates
  FILE --recipes FILE --out MANIFEST [--select ...]`，把候选 findings +
  项目侧类型配方渲染为 divergence manifest（4 个通用族骨架 + recipe
  schema + 预注册继承 + 跳过报告）；vostd 首站 22 条（Top10 + 12 条
  registry 对照）运行判定 21 ContractGapCandidate / 1
  ObligationSupported，expect_match 22/22，baseline clean；needs-recipe
  66 条逐条分级（3 条提升、3 条尝试失败、60 条冻结面不可补）。见
  `docs/M7c-obligation-synthesizer.md`、`examples/vostd-divergence/
  DIVERGENCE-RUN-M7C.md`、`NEEDS-RECIPE-M7C.md`。
- M7a 规格普查已实现：`cargo-spec-check census --vir --crate --out`
  从 VIR 枚举 Exec/Proof 函数的返回值、`#[verus_spec]` ghost 返回值与
  `&mut` 输出，判定有效后置条件（含 trait declaration 继承、排除
  `default_ensures`）是否提及，输出 `census.json`；纯分析、不注入、不验证。
  设计 `docs/M7a-spec-census.md`，校准 `examples/vostd-census/`
  （43da079 vs 同事 artifact：交集 1210 函数，输出数一致率 99.34%、
  后置来源 99.42%、finding 集合 98.51%）。
- M7b ktest 义务提取已实现：`cargo-spec-check extract-ktest
  --source-root --out [--config] [--stats]` 从 `#[ktest]` 测试提取断言/
  成功需求/调用序列，输出与 `synthesize --candidates` 兼容的裸数组；
  43da079 冻结源复现原型 **153 测试 / 139 候选 / 22 非 blocked
  （8 direct + 10 helper + 4 quantified）/ 117 blocked / 14 excluded**
  的全部评级与类别计数，断言 446/459 一致；类型变体推断把 292 条
  `Some or Ok` 降到 191 条未知，闭包/控制流/解构模式显式标注。首站
  只运行 2 条（K007 `MemoryRegionArray::push` supported、K131
  `Mutex::try_lock` gap），expect_match 2/2、baseline clean；117 blocked
  = 105 capture-gap + 8 断连 + 4 环境。设计
  `docs/M7b-ktest-obligation-extraction.md`，归档
  `examples/vostd-ktest/`。
- M7e requires 全函数筛查已实现：`cargo-spec-check screen-requires
  --census FILE --out DIR [--tier] [--filter] [--limit] [--batch]`
  从 census 分层（A 全机械 / B 含 uninterp / C blocked）、生成 quantifier
  manifest、复用 `vacuity::run`。vostd 1468 函数 → 774 有 requires
  （A 9 / B 89 / C 676；694 无 requires）；98 条候选分三批运行，
  **9 ConfirmedLive / 89 CandidateInconclusive / 0 ConfirmedVacuous /
  0 Anomaly，98/98 与预注册一致**；60 接口清单映射为 42 无 requires +
  18 Tier C，与本轮候选零交集。见 `docs/M7e-requires-screening.md`、
  `examples/vostd-requires/`。

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

### M5. test-contract-divergence（义务对照分析）—— 完成（2026-11-11）

研究管道的义务对照固化为第二条分析子命令
`analyses-divergence --manifest --cargo-verus --z3 --vstd --out`，与
vacuity 互补（vacuity 问 requires 侧是否空洞，divergence 问 ensures 侧
是否足以支撑使用方义务）。设计笔记 `docs/M5-test-contract-divergence.md`
（预测先于实现与运行写定；§6a 为 spike 迭代记录）。

- [x] 插件实现：`crates/spec-check-analyses/src/divergence.rs`
      （divergence manifest：target_workspace 同构 + obligation client
      条目 + 机器可查 `expect` 预注册字段；obligation fn 注入模块生成；
      两阶段 runner 复用 vacuity 回路；判定 ObligationSupported /
      ContractGapCandidate / Inconclusive / Anomaly，基线对照 = 同一
      构建日志中探针模块外零错误事件）。公共基础设施提取为
      `probe_infra.rs`（TargetWorkspace、注入安装/恢复、cargo-verus
      命令、日志 span 模型），vacuity 公开 API 路径不变；顺带修复
      run_captured 日志 append 模式的跨运行证据污染 bug。CLI 子命令
      接入 cargo-spec-check。工具单测 8/8（原 5 + 新 3），workspace
      全测通过。
- [x] spike（判据闸门）：mutex-lock-contract 锚点双面——同一语义义务
      各态惯用形，A 面（冻结原契约）ContractGapCandidate / B 面
      （候选契约 spec-change.patch 应用态）ObligationSupported，两面
      皆中。v1 同文本预测的 B 面被运行证伪（closed 谓词只能按 ensures
      实例匹配），迭代记录见设计笔记 §6a 与
      `examples/vostd-divergence/spike/SPIKE.md`。
- [x] vostd 首站运行：5 条目 / 3 家族（Mutex 获取族 ×4 + RwArc 包装族
      ×1），全部机械转写自研究管道已归档义务。判定
      **ContractGapCandidate 4 / ObligationSupported 1**，5/5 与预注册
      一致；目标 1498 verified / 4 errors（恰为 4 个预期失败义务，
      span 逐条归因 obligation fn），baseline clean，运行后工作区恢复
      原字节。归档 `examples/vostd-divergence/{DIVERGENCE-EXPECTED.md,
      DIVERGENCE-RUN.md, divergence-manifest.json, spike/}`；产物
      `/tmp/vostd-divergence-run1`。
- [x] registry：F-002 补机器探针证据；新增 F-013（Mutex::try_lock
      is_none 需求）/F-014（RwArc::get_cloned 文档值义务），均
      obligation_fail 级（无候选不虚构升级）；research 仓 commit 紧随。
- [x] 扩展轮：check 流程接入 + 义务族扩展（2026-11-12）：
      (1) check CONFIG 双分析声明——`check_flow` registry 增
      `divergence` adapter（与 vacuity 同构，manifest 各自声明
      `target_workspace`，注入语义共用 `probe_infra`；CONFIG 中按声明
      顺序依次运行，orchestration 不变）；`ANALYSIS-CONTRACT.md` 新增
      §活工作区注入语义；`examples/ci/run-spec-check.sh` 增
      `DIVERGENCE_MANIFEST` 可选第二分析。check 单测 5/5（含双声明
      解析）。
      (2) 首站条目 5→11（+6）：RwMutex/RwLock try_read 值读回（#6/#7）、
      SpinLock constant 关联（#8/#9/#10，`ProbeValueInv` 项目侧模型
      不变式）、RcuOption::read 值义务（#11）。扩展轮运行 11/11 与
      预注册一致：**ContractGapCandidate 8 / ObligationSupported 3**，
      目标 1500v/8e（8 error 恰为 8 个预期失败义务），baseline clean。
      语义读数：#9 vs #10 同文本异判定演示了契约对 guard↔锁关联的
      表达力依赖泛型不变式参数；#8 是有证明深度的 supported 面。
      归档 `DIVERGENCE-RUN.md`、`DIVERGENCE-EXPECTED-EXT.md`
      （预注册），产物 `/tmp/vostd-divergence-run2`。
      (3) check 双分析端到端（`/tmp/spec-check-e2e`）：capture fresh
      复用 → vacuity 10 条目 + divergence 11 条目单次 check，verdict
      合并、退出码 0（发现不置红）。CI 夜班入口自此覆盖双分析线。
- 已知边界：RwLock/RwMutex/SpinLock guard **身份**谓词与 RCU owner 族
  的 owner 关系属研究侧建模态（冻结面无 rel_lock/owner 谓词、guard
  字段模块私有），不可机械转写；本轮以各该族冻结面可表达的最强文档
  义务（值读回/constant 关联）覆盖，身份谓词维持不覆盖。义务 fail 是
  证明论缺口候选，非运行时反例。

### M6. tracked 契约构造探针批量生成器 —— 完成（2026-11-11）

M2 手工单点构造探针路线的批量化：60 接口 quantifier 全量运行中 10 个
CandidateInconclusive 条目（6 个恒假 stub 锚点 + row04/15/16/17）全部
改走源级构造探针。工具代码零改动（manifest v1.1 原样承载）；全部产物
在项目侧：`examples/vostd-guard/extract_construction_manifest.py`
（从冻结源 `#[verus_spec]` 契约机械转写：契约面直调/方法形 stub 自由
fn stub 三种面形态自动选型 + 铸造策略表 + 叶证据 run 生成）与
`examples/vostd-guard/construction/`（两 manifest + 预注册 EXPECTED +
归档 RUN）。

- [x] spike（node_lock 单点，成败闸门）：铸造策略表实证——可铸叶
      （PCell/PCellMU PointsTo、Count::alloc、Guards、OwnerSubtree、
      MetaRegionOwners 字面量）与阻断叶（simple_pptr/array_ptr
      PointsTo 的 std 门控铸造 API + 私有字段；FrameRef/PageTableGuard
      值；传递阻断 NodeOwner/CursorOwner 等）。判据可迁移性分层：
      铸造链完整 → 拒绝型 ConfirmedVacuous；链缺失 → 通道缺席
      （编译级锚点），不虚构机器判定。
- [x] 全量运行（预注册 14 条目 verifying run + 8 条目叶证据 run）：
      verifying run 1508 verified / 3 errors，判定 ConfirmedVacuous 3 /
      ConfirmedLive 11，14/14 与预注册一致，伪造拒绝 span 逐字归因
      （row15/16 落在真实 axiom `old(regions).inv()`，cursor.rs:97/138）；
      叶证据 run 13 错误全部落在铸造调用点（5×E0599 `PPtr::new` +
      5×E0599 `ArrayPtr::new` + 3×E0624 `borrow_paddr` private，8 模块
      无杂音）。10 条目中 2 个升级为机器 ConfirmedVacuous（row15/16），
      8 个记录"holder 面可调用性机器确认 + 铸造通道缺席编译锚点"的
      部分升级。见 `construction/CONSTRUCTION-RUN.md`；registry 更新在
      research 仓库（紧随本 commit）。
- 已知边界：调用面模型 = 注入点可见 pub 面 + no_std 铸造通道（模型
  相对）；通道穷尽是文档化论证；alloc_absent stub 的
  `relate_guard(*old(self).node)` 为 rustc 可见形式偏差（语义保持，
  RUN 文档 §证据边界 4）。

### M7a. 自研 VIR 规格普查（spec census）—— 完成（2026-11-12）

自有漏斗顶端：对全函数枚举输出，判断有效后置条件是否提及，列出无后置
条件/external_body/requires 清单，作为 M7b 需求提取、M7c 模板合成、M7d
台账自动化的地基。事实层，不产出缺陷判定。

- [x] 设计：`docs/M7a-spec-census.md`。语义逐条复刻同事
      ensures-coverage：输出 = 返回值 + `#[verus_spec]` 注入的 ghost
      返回值分量 + 每个 `&mut` 参数；提及 = ensures 中的名字或 `returns`，
      `&mut` 仅认 `final(x)`；trait impl 合并 declaration 的 regular
      ensures/returns；`default_ensures` 不算；类型不变式仅作注记。
- [x] 实现：`spec-check-core::census` + `cargo-spec-check census` /
      `spec-check-core census`。`::_VERUS_VERIFIED_` 规范化为声明路径，
      以验证体记录为准；单位实际返回值不产生输出；`with` 参数中的
      `Tracked<&mut _>`/`Ghost<&mut _>` 计为输出。纯 VIR、零项目专有
      知识。7 个语义单测；全仓 100 测试通过。
- [x] VIR 导出：43da079（`vostd-pr830`），Verus
      0.2026.10.08.d128dda / Rust 1.98.1；
      `cargo dv focus --targets ostd -- --no-verify --log vir`。
- [x] 校准：`examples/vostd-census/`。交集 1210 函数：输出数一致率
      99.34%、后置来源 99.42%、finding 集合 98.51%；21 个受影响函数
      全部追溯到 b6db6df↔43da079 的签名/契约修订；无 D-MENTION/
      D-NONE 规则差异。196 个 artifact 源码解析器缺口里 188 个由 VIR
      普查覆盖（8 个因修订缺失）。汇总数字受修订影响：1468 vs 1502
      函数、有后置 52 vs 42 findings、无后置 171 vs 193。
- 已知边界：精确修订 b6db6df 的 VIR 需要 Rust 1.99.1 + Verus
  0.2026.11.11，本地无该组合；校准按路径交集进行。`default_ensures_only`
  注解会传播到继承该 default 的 impl（artifact 只在声明行标注），不影响
  finding。6 个 `decl_unavailable` 均为自带后置条件的
  `FallibleVm{Read,Write}` impl。


### M7b. ktest 断言 → 调用者义务提取 —— 完成（2026-11-12）

- [x] 设计：`docs/M7b-ktest-obligation-extraction.md`——提取架构
      （mask/字节偏移、测试发现、三层门控、断言/需求/调用序列、函数
      推导、helper 归因）、与 synthesize 的接口、可回放性分级
      （direct/needs-model/blocked）、断言→模板族映射、排除规则、降级
      策略、预注册边界。
- [x] 实现：`spec-check-core::ktest` + CLI `extract-ktest`；无新 crate
      （regex + serde）。项目侧配置 `examples/vostd-ktest/extract-config.json`
      （私有观察/环境依赖/门控 cfg/方法变体表/研究锚点/函数覆盖）。
      单测 8 条：mask 偏移、断言分类、变体推断、闭包/循环作用域、
      端到端确定性、helper 内联、capture-gap/断连、解构模式。
- [x] 提取：冻结源 153 测试 → 139 候选；`by_rating`/`by_category` 与
      原型逐项一致；断言 446/459 一致；需求 311/302 vs 原型 310/301
      （+1 = 元组解构 `let (va, Some(item))`，原型漏检）。117 blocked
      = 105 capture-gap + 8 断连 + 4 环境；22 非 blocked 但只有 2 条
      现有骨架可直接合成，其余 20 条缺 iterator/量化/ghost/集合模型。
- [x] 首站：K007 `MemoryRegionArray::push`（value-readback）与 K131
      `Mutex::try_lock`（is_none-demand）合成并注入；vstd 1908/0、
      target 1498/1、baseline clean；ObligationSupported 1 /
      ContractGapCandidate 1，**expect_match 2/2**。预检 v1 的
      `E0596`（`let recv` 非 mut）修正为合成器通用 `receiver_mut`
      recipe 字段，预注册 §6 记录、预测未改。
- [x] 归档：`examples/vostd-ktest/KTEST-RUN-M7B.md`、
      `KTEST-EXPECTED-M7B.md`、`extracted-candidates.json`、
      `recipes-m7b.json`、`divergence-manifest-m7b.json`、`extract-stats.json`；
      运行前后 216 个源文件 sha256 逐字节一致。

### M7c. 义务 client 合成器（obligation synthesizer）—— 完成（2026-11-12）

把手写义务 client 变成"模板族骨架 + 项目侧 recipe"的机械合成，并完成
vostd 首站运行归档。

- [x] 设计：`docs/M7c-obligation-synthesizer.md`——输入
      `candidates.json` + `recipes.json` → 输出 divergence manifest；
      4 个通用族骨架（value-readback / is_none-demand / guard-identity /
      final-relation，`other` 不合成）、`{recv}`/`{out}`/`{handle}`
      占位符与族校验、recipe schema（类型构造/导入/setup、接口
      call/bridge/snapshot/probes/assert/hold、可选 `bind_attr`）、
      预注册继承（candidate `prereg_expected`，recipe 覆盖须带
      `expect_basis`）、不可合成降级（skip + 报告；不把占位义务塞进
      manifest）、与 census 的关系。工具代码零项目专有知识。
- [x] 实现：`spec-check-analyses::synthesize` +
      `cargo-spec-check synthesize --candidates --recipes --out
      [--select] [--report]`。合成报告逐候选记录
      synthesized/skip reason/expect_source/渲染后义务；确定性与
      placeholder/族/重名/select 校验。12 个合成器单测；全仓测试通过。
- [x] 首站配方与合成：`examples/vostd-divergence/recipes-m7c.json`
      覆盖 VmReader/VmWriter/Either/WaitQueue + 锁族/RcuOption 对照；
      235 候选 → 22 条（213 skip：no-template-family 61、blocked 69、
      needs-recipe 57、no-interface-recipe 26）；预注册
      `DIVERGENCE-EXPECTED-M7C.md`（逐条依据 + 两轮编写期预检记录）。
- [x] 首站运行：`/tmp/vostd-divergence-m7c-run1`，vstd **1908/0**、
      target **1498/21**、baseline clean；判定 **21
      ContractGapCandidate / 1 ObligationSupported**，`expect_match`
      **22/22**；21 个 error 全为 obligation fn 内 `assertion failed`
      （无 precondition/postcondition 事件）。C-104
      `VmWriter::from` 的 `avail_spec()==slice.len()` 为 supported 面
      （需 tracked `VmIoOwner` 绑定）；VmReader/VmWriter 的
      limit/skip/has_* 为接收者后状态/返回值缺口。运行后 9 个文件
      sha256 逐字节恢复，探针目录移除。
- [x] 归档：`examples/vostd-divergence/DIVERGENCE-RUN-M7C.md`（判定表、
      与 M5/M7a 对照、证据边界、registry 处置）。
- [x] needs-recipe 评估：`examples/vostd-divergence/NEEDS-RECIPE-M7C.md`。
      66 条中 3 条成功提升（C-103 AsVirtPtr、C-228 Jiffies::elapsed、
      C-127 PageTablePageMeta::new，均 gap、expect_match ✓）、3 条尝试
      失败（E0433 模块可见性；E0277 `Repr<MetaSlotSmall>`）、60 条冻结面
      不可补（DMA/VmIo 缺 `AnyUFrameMeta` 实现、链表缺
      `Repr<MetaSlotSmall>`、VmSpace/页表 owner 不可铸造、external_body
      等）。修正了 M7b 前置"66 条 needs-recipe"的乐观口径。
- 已知边界：合成器不做义务发明（recipe 是项目侧输入）；skip 不进入
  manifest（避免污染整批构建）；`ContractGapCandidate` 是证明论结论；
  Either 的 dual_spec 口径依赖调用形态；VmReader/VmWriter 的干净证据
  以 ghost owner 绑定为前提（编写期 v1 未绑定时全部 client 不可表达）。

### M7b. ktest 断言 → 调用者义务提取 —— 完成（2026-11-12）

`extract-ktest --source-root --out [--config] [--stats]`：Rust 侧词法 mask +
结构匹配；相对原型消除闭包过近似、补控制流/解构/变体推断/函数推导。
冻结源口径与原型完全复现（153 测试/139 候选/117 blocked/14 排除）。
首站 2 条（push supported、try_lock gap），expect_match 2/2，baseline
clean，216 文件 sha256 恢复。重要修正："22 可注入" ≠ "22 可合成"——实际
仅 2 条跑通，其余卡 iterator/量化骨架、tracked owner、mock、闭包谓词。
commit 83a1541；registry 3a59fa67（F-013 独立复现证据）。

### M7c-扩展. 配方库扩充（iterator / 状态量化）—— 完成（2026-11-12）

新增通用族 `iterator-sequence`（steps 逐步消费 + 引用断言）与
`state-invariant`（bridge 变更 + 查询断言）；schema 扩展
`candidate_interfaces`（按候选覆盖 interfaces）、`steps`、
`obligation_family/family_basis`（骨架可不同于候选族但须审计依据）。
20 条缺口逐条评估：**11 条可表达并运行**（9 条 range_difference + CpuSet/
AtomicCpuSet），9 条阻断（页表 cursor tracked owners 不可铸造、私有
test helper、MOCK_KERNEL_PT、external_body unimplemented）。注册运行
1498v/10e、baseline clean、Supported 1 / Gap 10、expect_match 11/11；
关键读数：9 条 range 失败均落在最终 assert（前置已证），缺口是 ensures
把结论包在未建立的 iterator 前件下。独立副本 440 文件 sha256 恢复。
commit 9513b91。

### M7d. 台账自动化（ledger）—— 完成（2026-11-12）

`ledger --registry --report... [--candidates...] [--map] --out`：证据归一化
（verdict→kind 映射）、dedup（map→名称→dedup_key→规范化→语义 token）、
四类提案（append/new/noop/unmatched）；status_suggestion 恒 null、
requires_human=true；registry 只读并字节校验。首站对账 10 份 report /
91 条证据：matched 64 / ambiguous 4 / unmatched 23；append 1（V-001
construction_probe 合并 3 来源）、new 0、noop 63——F-013..F-017 与
F-018..F-024 全部匹配既有条目、零误建。registry 现 26 条（19 + M7c 新增
7）。commit 2b1644b。

### M7e. census 全函数 requires 筛查 —— 完成（2026-11-12）

把 vacuity 的 requires 侧筛查从"60 接口手写清单"推广为"基于 census 的全函数
筛查"。census 增加逐函数 `generics`/`params`/`requires`（机械 AST、span 文本、
特征标记）；`screen-requires --census FILE --out DIR [--tier] [--filter]
[--limit] [--batch K --batch-size N] [--cargo-verus --z3 --vstd]` 分层
（A 全机械 / B 含 uninterp / C blocked）、生成 quantifier manifest、复用
`vacuity::run` 并输出逐函数分类与 verdict/expect 对照。

- 分层：A 9 / B 89 / C 676（无 requires 694；函数总数 1468）。C 原因（按函数
  去重）：generic 505、ghost_param 168、reference_param 366。C 明确不是
  "无缺陷"。
- 首站：A 校准批 9 条 → **9 ConfirmedLive**（35.9s）；B1 45 条、B2 44 条 →
  **89 CandidateInconclusive**（38.1s / 37.5s）；合计 **98/98 与预注册一致**，
  0 ConfirmedVacuous / 0 Anomaly。三批均首次编译通过。
- 与 M1 60 接口对照：60 行映射为 42 `no_requires` + 18 Tier C，**0 个 A/B
  候选**——两套清单不相交；M1 的 7 项已知 vacuous stub 全在 Tier C，实证
  "Tier C ≠ 无缺陷"。
- 归档：`docs/M7e-requires-screening.md`、`examples/vostd-requires/`
  （screening.json、三批 manifest、REQUIRES-EXPECTED.md、REQUIRES-RUN.md、
  runs/{a,b1,b2}.json）。registry 无新增（无新 ConfirmedVacuous）。
- 边界：Tier C 676 未筛查；常量提升与有界见证；Tier B Inconclusive 不计缺陷；
  `screen-requires` 为离线步骤，未默认接入夜班。

### CI 入口更新（census 可选通道）—— 完成（2026-11-12）

`run-spec-check.sh` 新增 `SPEC_CHECK_CENSUS=1`（默认关）：`cargo dv focus
--log vir` 导出 → `census` → `CHECK_OUT/census/`；缺 dv/vir/目录均清晰非零
早退，census findings 不影响退出码。CI-WIRING.md 明确"夜班在线 =
capture + check(+可选 census)；离线 = extract-ktest / synthesize /
ledger"，不把离线合成器伪装成自动发现。验证：bash -n、既有 VIR 直跑 census
（1468 函数）、git diff --check。commit 8dead7c。

### M7e-扩展. Tier C 建模（泛型/引用/ghost）—— 完成（2026-11-12）

把 676 个不可筛查函数中的大部分降级为可生成探针：

- **泛型实例化**（项目侧 `--config` 的 `generic_instances`，exact/glob，不假设
  trait bound/布局）→ 505/505 个泛型函数实例化成功。
- **引用/ghost 保守抽象**：census 增 `param_uses/param_occurrences`；仅当引用
  参数的全部使用都是 receiver/field/deref root（且计数一致）才映射为不透明
  `{name}_id: usize` + 形状保持的 uninterp 子句；直接使用/整体传参保持 Tier C。
  映射 364 个引用参数/316 函数、211 个 ghost 参数/121 函数。抽象是 relaxation
  （真实模型 ⊆ 抽象模型）——抽象上的 `ConfirmedVacuous` 是真阳性，抽象上的
  Live 不当作真实 liveness。
- 分层 before→after：**A 9→12、B 89→633、C 676→129**，可运行候选 **98→645**
  （+547）；baseline 复跑与 M7e 逐函数 tier 0 差异。
- 运行 13 批 645 条（总 434s）：**ConfirmedLive 12 / CandidateInconclusive
  633 / Vacuous 0**，expect 645/645；registry 无新增。
- 与只读可行性分析的对照：分析预测 63 可升 A，保守实现实测仅新增 3 条 A——
  因为引用/ghost 的字段/方法访问不作语义解释（只作审计账本），多数降为
  uninterp B。差异与原因归档于 `examples/vostd-requires/analysis-tierc/` 与
  `TIERC-RUN.md`。
- 剩余 129 Tier C：ghost 直接使用 67、引用直接使用 41、二者 21（`plain_root`
  直接比较/整体传参，如 `perm.id() == self.cell_id()`）。
- 归档：`docs/M7e-tierc-modeling.md`、`examples/vostd-requires/
  {config-tierc.json,TIERC-EXPECTED.md,TIERC-RUN.md,screening-tierc.json,
  manifest-tierc.json,runs-tierc/,analysis-tierc/}`。commit 87657af + bd64101。

### M7e-投影. 显式投影语义映射 —— 完成（2026-11-12）

把剩余 Tier C 中可通过项目侧投影模型表达的 requires 升级为可判定探针：

- **schema**：`projection.models[]`（name/params/returns/body；受限标量表达式，
  `null`=uninterp）+ `projection.rules[]`（method/field/variant + model +
  relation: faithful/over-approx/under-approx）。翻译：`x.method(args)`→模型
  调用、`x.field`→field 模型、`x is Variant`→variant 模型；整体引用/ghost
  只能作模型实参；`==`/`!=` 允许 id 比较（底层相等的过近似）；`old` 透明；
  排序/算术、Ctor/Match/Block/未知节点保持未映射。
- **健全性**：只有 faithful/over-approx 模型上的 `ConfirmedVacuous` 可作候选
  证据；`ConfirmedLive` 一律只作模型相对可满足（不声称真实 liveness）。
- Tier：774 全库 **A 9→28 / B 89→695 / C 676→51**（C→A 16、C→B 609，
  剩余 C 51：未建模 call 26、ctor 15、Block 9、Match 6、unary/binary 等）。
- 运行 723 条 / 15 批：**ConfirmedLive 22 / ConfirmedVacuous 7 /
  CandidateInconclusive 694 / Anomaly 0**，与工具 expected 一致
  716/723；7 条不一致全是预注册 P5 的 E6 信号（VmIo）。
- **新发现 V-003**：`VmIo` 7 个 default fallback 方法的入口前置
  `!Self::obeys_vmio_read/write_requires()` 在 ostd crate 内已知 impl 上恒假
  （crate-scope 常量模型 faithful）→ 7/7 ConfirmedVacuous。边界已归档：
  公开 trait 的外部 impl 未建模，不是全局结论；default 不可达也可能是刻意
  安全网。
- M1 5 接口对照：`MutexGuard::new` C→A **ConfirmedLive**（复现 M1 形状判定，
  构造层 V-001/V-002 不变）；path_slot/vm_space×2/tracked_restore 保持 B
  Inconclusive。
- 归档：`docs/M7e-projection-mapping.md`、`examples/vostd-requires/
  {config-projections.json,PROJECTION-EXPECTED.md,PROJECTION-RUN.md}`。
  commits 64587a2；registry 129ebd20（V-003）。

### M7e-E6. 信号产品化 + 剩余 Tier C 表达式扩展 —— 完成（2026-11-13）

把 7 条 VmIo 的“工具 expected 与预注册/实际不一致”从归档手工对比升级为一等输出，
并把剩余 51 条 Tier C 中的 26 条按保守原则升级：

- **E6 信号**：`screen-requires --expectations FILE`（`spec-check.expectations/1`，
  entry → expected/basis/scope/registry）；同时比较预注册 expected 与工具内部
  expected；输出 `signals[]`（direction/severity/model_relation/scope/evidence/
  suggested_registry_action）与聚合统计。`vacuous-signal` 仅在 exact/faithful/
  over-approx 关系上为 candidate；under-approx/undeclared 降为 info；Live 不声称
  真实 liveness；工具不自动改 registry。新增 `signals` 子命令可对已有 run 重算。
- **723 基线复核**（pre-E6 二进制 + 归档 census，15 批）：Live 22 / Vacuous 7 /
  Inconclusive 694 / Anomaly 0；7 条信号全部 VmIo、expected_source=tool、
  preregistered_match=true、candidate、confirm-registry:V-003；其余 716 无信号。
  实际模型关系 6 faithful + 1 over-approx（`r0358_write_vals`），预注册的 7×faithful
  有 1 条偏差，已归档。
- **表达式扩展**：Ctor→结构 token、Match→标量 if-else、Block→let 序列、Clip cast、
  `/`/`%`、数值 tuple field token；量化 projected 子句改 named uninterp（避免 trigger
  推断失败）。扩展仅对 baseline C 启用，baseline A/B 探针文本保持不变。
- Tier：774 全库 **A 28 / B 721 / C 25**（C→B 26、C→A 0，候选 723→749）。
  仍 C 25：未建模 call（`relate_guard`/`node_matching`/`pcell_maybe_uninit::id`/
  `atomic::value`/`disjoint` 等）与 1 个 `==` 类型冲突，逐条原因见 `E6-RUN.md`。
- 扩展运行 749 条 / 15 批：**Live 22 / Vacuous 7 / Inconclusive 720 / Anomaly 0**；
  原 723 逐条 verdict 0 差异；26 个新条目全 Inconclusive；7 条信号精确复现，
  0 新 Vacuous、0 新信号。registry 未修改（V-003 仍 crate-scope，不升全局）。
- 归档：`docs/M7e-e6-signals.md`、`examples/vostd-requires/
  {E6-EXPECTED.md,E6-RUN.md,e6-expectations.json,e6-screening.json,e6-signals.json}`。
  实现：`signals.rs`、`census.rs`、`requires.rs`、`cargo-spec-check/src/main.rs`。
- 全测：`cargo test --offline` 全绿；core lib 152 passed / 72 ignored。

## 维护纪律

1. 主线进度只认本文件与 git 历史；会话记忆不作为状态来源。
2. 每个条目推进后：更新本文件勾选状态 + 提交。
3. 引擎模块重构必须带重放验证（研究档案证据可重放）。
