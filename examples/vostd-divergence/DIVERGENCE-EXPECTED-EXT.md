# DIVERGENCE 扩展轮预注册（注册运行前写定，运行后不改写；2026-11-12）

对象：vostd 首站义务对照**扩展轮**（`analyses-divergence`，11 条目 = 首轮 5
条 + 新建模 6 条）。首轮预注册见 `DIVERGENCE-EXPECTED.md`（其 5 条预测原样
带入本表，不改写）；本轮新增 **#6–#11**（RwMutex / RwLock / SpinLock /
RcuOption 族，M5 边界清单覆盖）。判据闸门：首轮 spike（`spike/SPIKE.md`）
+ 本轮 manifest 编写期预检（见下）。注册运行目录：`/tmp/vostd-divergence-run2`。

## 新建模义务的建模方式（manifest 项目侧输入，工具零项目知识）

冻结面事实（逐条与 `/tmp/vostd-43da079-ktest-chan` 契约原文核对）：

- `RwMutex::new` / `RwLock::new` 无契约（const fn，无 verus_spec）；
  `try_read` 为裸 `#[verus_spec]`（验证但无 ensures）；`RwMutexReadGuard` /
  `RwLockReadGuard` 的 `value` 为 `pub closed`、`view` 为 `pub open`，字段
  模块私有，无公开身份访问器。
- `SpinLock::new` **有真契约**：`ensures res.constant() == resource_constant`
  （requires `I::inv(resource_constant, val, resource)`）；`lock()` ensures
  `ret.constant() == self.constant()` ∧ `I::inv(ret.constant(), ret.value(),
  ret.resource())`；`SpinLockGuard::constant/value` 为 `pub closed`、`view`
  为 `pub open`。`Constant` 由泛型不变式参数 `I` 决定。
- `RcuOption`（`crate::sync` 唯一可达的 RCU 族类型，`#[cfg(ktest)]` 导出；
  `Rcu`/`RcuDrop`/`RcuReadGuard` 未重导出）：`new`/`read`/`get`/`is_none`
  均无 ensures；`RcuOptionReadGuard` 无 Drop trait、`is_none` 读内层
  `obj_ptr`。

建模方式：

1. **RwMutex / RwLock（#6/#7）**：义务 = 构造值经 `try_read` guard 读回
   （`g.view() == 5`）。研究形态锚点：acquisition-batch 的 try_* client
   结构（构造 → try_* 获取 → 断言 → 持有）+ owner-field-return-flows 的
   值义务断言形态（首轮 #5 同型）。身份谓词（`rel_lock`/`inner` 字段）在
   冻结面不可表达（字段模块私有、无公开访问器——M5 边界 (a) 维持），
   值读回是该族在冻结面可表达的**最强文档义务**（guard 允许读保护数据）。
   预期 gap：`new`/`try_read` 无 ensures，`view→value` 链与构造值无桥。
2. **SpinLock（#8/#9/#10）**：身份谓词不可表达，但契约的 `Constant` 泛型
   参数是冻结面**自带**的身份见证管道——manifest 侧以项目模型不变式
   `ProbeValueInv`（`Constant=u64`，`inv(c, v, ()) ⇔ v == c`，client 内
   定义、open 可展开）实例化，不改冻结源：
   - #8 值读回链：`new` ensures（constant==5）→ `lock` ensures（guard
     constant==锁 constant ∧ I::inv）→ 模型不变式展开 → `view()==5`。
     **有真实证明深度的 supported**（契约链组合，非 ensures 字面实例）。
   - #9 跨锁常数对照：`g.constant() == lock2.constant()`（两锁常数 5/7）。
     契约把 guard 常数钉在其产出锁的常数上 → 不可证 → gap。这是
     `rel_lock`（引用身份）在冻结面的**可表达最强代理**：阴性对照证明
     support 判定非平凡（首轮 #3 同型）。
   - #10 平凡不变式对照：同一义务文本、`TrivialResourceInvariant`
     （`Constant=()`）→ `()==()` 平凡可证 → supported。与 #9 同文本异
     判定，机器演示"契约的身份表达力依赖不变式参数"（`rel_lock` 必要性
     的机器见证；M1 `mutex_guard_new_perm_cell_shape` 同源弱点）。
3. **RcuOption（#11）**：义务 = 构造 `Some` 后 `read` guard 非 None
   （`!is_none()`，文档义务直陈）。研究形态锚点：owner-field-nested-rcu
   的 RCU client 结构（构造 → read → 断言 → 持有）。owner 谓词
   （`guard.0.rcu == &lock.0`）字段私有不可表达；`Rcu`/`RcuDrop` 未从
   `crate::sync` 导出（比 M5 边界记录更收窄，如实登记）。预期 gap：
   `new`/`read` 无 ensures，`is_none` 结果无桥。

## 转写边界（编写期实测，预注册即承认）

- **exec 调用不得落在 `assert` 实参内**（verus `assert` 实参按 spec 上下文
  求值）：`is_none`/`deref_target` 等 exec fn 须先 `let` 绑定（首轮 #4
  条目同型先例）。#11 据此转写。
- `BoxRef::deref`（Deref impl）在冻结源中注释停用 → `*r` 编译错
  （E0614）；`deref_target` exec 调用在 assert 内被 verus 拒 → #11 采用
  `is_none` 形态，值读回形态如实放弃（非沉默降级）。
- 义务 fail ≠ 运行时反例；契约缺口是证明论结论；Inconclusive 不计缺陷。

## 预注册预测

| # | entry | 族 / 接口 | 义务（转写来源） | 预期 | 预注册依据 |
|---|-------|-----------|------------------|------|------------|
| 1 | mutex_lock_guard_identity | Mutex::lock | guard 身份（首轮原样） | ContractGapCandidate | 首轮实测 gap（带入不改写） |
| 2 | mutex_try_lock_rel_supported | Mutex::try_lock | ensures 字面实例（首轮原样） | ObligationSupported | 首轮实测 supported |
| 3 | mutex_try_lock_rel_cross_negative | Mutex::try_lock | 异锁阴性对照（首轮原样） | ContractGapCandidate | 首轮实测 gap |
| 4 | mutex_try_lock_is_none_demand | Mutex | 持 guard 再 try 得 None（首轮原样） | ContractGapCandidate | 首轮实测 gap |
| 5 | rwarc_get_cloned_value | RwArc | 构造值读回（首轮原样） | ContractGapCandidate | 首轮实测 gap |
| 6 | rwmutex_try_read_view_value | RwMutex::try_read | 构造值经 guard 读回 `view()==5`（acquisition-batch try_read client 结构 + 值义务形态） | ContractGapCandidate | `new`/`try_read` 无 ensures；`view→value` 与构造值无桥 |
| 7 | rwlock_try_read_view_value | RwLock::try_read | 同上（RwLock 族） | ContractGapCandidate | 同 #6（rwlock.rs 契约原文同构） |
| 8 | spinlock_lock_value_readback | SpinLock::lock | 值读回链 `view()==5`（ProbeValueInv 实例化；spinlock-return-contract client 结构） | ObligationSupported | new+lock ensures 与 open 模型不变式组合可证 |
| 9 | spinlock_lock_constant_cross_negative | SpinLock::lock | 跨锁常数 `g.constant()==lock2.constant()`（5/7） | ContractGapCandidate | 契约钉 guard 常数于产出锁；5≠7 不可证 |
| 10 | spinlock_lock_constant_trivial_cross_control | SpinLock::lock | 同文本、`Constant=()` | ObligationSupported | `()==()` 平凡可证（身份表达力缺口的机器见证） |
| 11 | rcu_option_read_get_value | RcuOption::read | 构造 `Some` → guard 非 `is_none()`（nested-rcu client 结构 + 文档非空义务） | ContractGapCandidate | `new`/`read`/`is_none` 无 ensures |

预测汇总：**ContractGapCandidate 8 / ObligationSupported 3 / Inconclusive
0 / Anomaly 0**；基线净（目标工作区除探针义务失败外无错误事件）；vstd
0 errors。运行判定以 manifest 实测为准；`expect` 不匹配如实报告
（expect_match=false），不改写本文件。

## 判据闸门：manifest 编写期预检迭代（先于注册运行，如实记录）

注册运行前同一 manifest 的编译验证迭代（产物
/tmp/vostd-divergence-ext-authoring{,2,3,4}）：

- v1：#11 `*r` 对 `BoxRef` 解引用 E0614（冻结源 Deref impl 注释停用）。
- v2：#11 改 `deref_target()` → verus mode 错（exec 调用落在 `assert`
  spec 上下文）。
- v3：#11 改 `assert(!guard.is_none())` → 同 mode 错（根因确认：assert
  实参为 ghost 块；exec fn 须 let 绑定后断言）。
- v4：#11 改 `let __sq_is_none_0 = guard.is_none(); assert(!__sq_is_none_0)`
  → **11 条目全部预检通过并完成验证构建**；v4 判定 11/11 与上表一致
  （gap 8 / supported 3，expect_match 11/11）。v4 仅作判据与编译闸门，
  **注册判定以 run2 为准**。

## 运行配置

- 冻结源：`/tmp/vostd-43da079-ktest-chan`（rev 43da0797bbfa + 既有 ktest
  通道适配；注入后运行、运行后恢复原字节）。编写期各轮运行后已核对恢复
  （mutex.rs `eea4b78a…`、lib.rs `f1413d24…`，probes 目录移除）。
- 预算：900s 单次注册运行；产物 `/tmp/vostd-divergence-run2`。
