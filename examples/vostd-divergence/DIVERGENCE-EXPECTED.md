# DIVERGENCE 预注册（运行前写定，运行后不改写；2026-11-11）

对象：vostd 首站义务对照运行（`analyses-divergence`），5 条目 / 2 模块 3 家族
（Mutex 获取族 ×4、RwArc 包装族 ×1）。义务条目全部机械转写自研究管道已归档
义务，转写规则见 `docs/M5-test-contract-divergence.md` §7（义务落在冻结面可
表达形态上，语义与研究义务一致）。判据闸门已过：mutex-lock-contract 锚点
双面 spike（A 面 ContractGapCandidate / B 面 ObligationSupported，见
`spike/` 与设计笔记 §6a）。

判据边界（预注册即承认）：RwLock/RwMutex/SpinLock guard 身份义务与 RCU
owner 族的谓词（`rel_lock`、owner 关系）由研究侧建模补丁引入，冻结面不
存在、guard 字段模块私有，本站不覆盖（不硬造表达形态）；义务 fail ≠ 运行
时反例；契约缺口是证明论结论；`rwarc_get_cloned_value` 研究侧无 adequate
candidate，本站登记 obligation_fail 级证据，不虚构 supported 面。

## 预注册预测

| # | entry | 家族 / 接口 | 义务（转写来源） | 预期 | 预注册依据 |
|---|-------|-------------|------------------|------|------------|
| 1 | mutex_lock_guard_identity | Mutex::lock | guard 属于产生它的锁（by-value 惯用形；mutex-lock-contract client） | ContractGapCandidate | 研究 11 pass / client fail；spike A 面同判 |
| 2 | mutex_try_lock_rel_supported | Mutex::try_lock | `ret is Some ⇒ ret->0.rel_mutex(*self)` 字面实例 | ObligationSupported | 义务恰为原 ensures 实例，无推理间隙 |
| 3 | mutex_try_lock_rel_cross_negative | Mutex::try_lock | 异锁义务 `g.rel_mutex(other)`（阴性对照） | ContractGapCandidate | ensures 只给接收者实例；异锁无桥，support 判定须不可平凡成立 |
| 4 | mutex_try_lock_is_none_demand | Mutex::lock + try_lock | 持 guard 再 try_lock 得 None（mutex-try-lock-demand ktest 需求转写） | ContractGapCandidate | 研究 11 pass / 1 error（is_none 断言失败） |
| 5 | rwarc_get_cloned_value | RwArc::get_cloned | get_cloned 返回构造值（文档义务；owner-field-return-flows RwArc client） | ContractGapCandidate | 研究 sync::rwarc 8 pass / 1 error |

预测汇总：**ContractGapCandidate 4 / ObligationSupported 1 / Inconclusive
0 / Anomaly 0**；基线净（目标工作区除探针义务失败外无错误事件）；vstd
0 errors。运行判定以 manifest 实测为准；`expect` 不匹配如实报告
（expect_match=false），不改写本文件。

## 运行配置

- 冻结源：`/tmp/vostd-43da079-ktest-chan`（rev 43da0797bbfa + 既有 ktest
  通道适配；注入后运行、运行后恢复原字节）。
- 判据 spike 已闸：`spike/divergence-spike-{a,b}.json` 双面判定与
  设计笔记 §6/§6a 一致（A: ContractGapCandidate，B: ObligationSupported）。
- 预算：900s 单次运行；产物 `/tmp/vostd-divergence-run1`。
