# vostd 首站义务 client 合成运行归档（M7c，2026-11-12）

预注册：`DIVERGENCE-EXPECTED-M7C.md`（**运行前写定**，本文件不改写它；
其中 §6 记录两轮编写期预检，均在注册运行之前）。判定只以 divergence
实测为准。合成输入与输出的哈希见预注册 §1（运行前后一致）。

## 1. 运行命令

```
timeout 900 nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  'cd ~/vie/work/AES/spec-check && \
   ./target/debug/cargo-spec-check analyses-divergence \
     --manifest examples/vostd-divergence/divergence-manifest-m7c.json \
     --cargo-verus /home/jinser/vie/work/AES/vostd/tools/verus/source/target-verus/release/cargo-verus \
     --z3 /home/jinser/vie/work/AES/vostd/tools/verus/source/z3 \
     --vstd /home/jinser/vie/work/AES/vostd/tools/verus/source/vstd \
     --out /tmp/vostd-divergence-m7c-run1'
```

- 合成器：`cargo-spec-check synthesize --candidates ... --recipes
  examples/vostd-divergence/recipes-m7c.json --out
  examples/vostd-divergence/divergence-manifest-m7c.json`
  （235 候选 → 22 条义务；213 跳过：`no-template-family` 61、`blocked`
  69、`needs-recipe` 57、`no-interface-recipe` 26）。
- 冻结源 `/tmp/vostd-43da079-ktest-chan`（rev 43da079 + ktest 通道），
  注入 `ostd/src/spec_check_probes/`，`#[cfg(ktest)]` +
  `--cfg ktest_capture_gap`，target `x86_64-unknown-none`；两阶段
  （`cargo-verus check --no-verify` → `cargo-verus build`）。
- 单次注册运行在 900s 预算内完成，未拆分。

## 2. 机器判定汇总

- vstd 依赖：**1908 verified / 0 errors**；
- 目标 workspace（ostd）：**1498 verified / 21 errors**；
- **baseline clean = true**：21 个 error 全部落在 22 个探针模块的
  obligation fn 内；探针模块外（目标 crate 冻结源）零错误事件；
- 21 个 error 全部是 `assertion failed`（无 `precondition not
  satisfied`/`postcondition not satisfied` 事件；`grep -c 'assertion
  failed' = 42` 为日志行数，对应 21 个诊断）；
- 判定分布：**ContractGapCandidate 21 / ObligationSupported 1 /
  Inconclusive 0 / Anomaly 0**；
- **`expect_match` 22/22 = true**（预注册分布 21 gap + 1 supported，逐条
  命中）。

## 3. 逐条目判定表（预测 vs 实际 vs 一致）

| # | entry | 函数 | 族 | 预期 | 实际 | 一致 | 失败归因 span |
|---|---|---|---|---|---|---|---|
| 1 | c096_vmreader_has_remain | VmReader::has_remain | is_none-demand | ContractGapCandidate | ContractGapCandidate | ✓ | `c096_vmreader_has_remain.rs:19` |
| 2 | c097_vmreader_limit | VmReader::limit | final-relation | ContractGapCandidate | ContractGapCandidate | ✓ | `c097_vmreader_limit.rs:19` |
| 3 | c098_vmreader_skip | VmReader::skip | final-relation | ContractGapCandidate | ContractGapCandidate | ✓ | `c098_vmreader_skip.rs:20` |
| 4 | c100_vmwriter_has_avail | VmWriter::has_avail | is_none-demand | ContractGapCandidate | ContractGapCandidate | ✓ | `c100_vmwriter_has_avail.rs:19` |
| 5 | c101_vmwriter_limit | VmWriter::limit | final-relation | ContractGapCandidate | ContractGapCandidate | ✓ | `c101_vmwriter_limit.rs:19` |
| 6 | c102_vmwriter_skip | VmWriter::skip | final-relation | ContractGapCandidate | ContractGapCandidate | ✓ | `c102_vmwriter_skip.rs:20` |
| 7 | c104_vmwriter_from | VmWriter::from | final-relation | ObligationSupported | ObligationSupported | ✓ | —（义务验证过） |
| 8 | c162_mutex_new | Mutex::new | value-readback | ContractGapCandidate | ContractGapCandidate | ✓ | `c162_mutex_new.rs:16` |
| 9 | c167_rcuoption_new_none | RcuOption::new_none | value-readback | ContractGapCandidate | ContractGapCandidate | ✓ | `c167_rcuoption_new_none.rs:17` |
| 10 | c169_rcuoption_read | RcuOption::read | guard-identity | ContractGapCandidate | ContractGapCandidate | ✓ | `c169_rcuoption_read.rs:17` |
| 11 | c170_rcuoptionreadguard_is_none | RcuOptionReadGuard::is_none | is_none-demand | ContractGapCandidate | ContractGapCandidate | ✓ | `c170_rcuoptionreadguard_is_none.rs:17` |
| 12 | c177_rwarc_clone_ro | RwArc::clone_ro | value-readback | ContractGapCandidate | ContractGapCandidate | ✓ | `c177_rwarc_clone_ro.rs:17` |
| 13 | c178_rwarc_get_cloned | RwArc::get_cloned | value-readback | ContractGapCandidate | ContractGapCandidate | ✓ | `c178_rwarc_get_cloned.rs:16` |
| 14 | c180_rwlock_new | RwLock::new | value-readback | ContractGapCandidate | ContractGapCandidate | ✓ | `c180_rwlock_new.rs:16` |
| 15 | c181_rwlock_read | RwLock::read | value-readback | ContractGapCandidate | ContractGapCandidate | ✓ | `c181_rwlock_read.rs:16` |
| 16 | c184_rwlock_try_read | RwLock::try_read | value-readback | ContractGapCandidate | ContractGapCandidate | ✓ | `c184_rwlock_try_read.rs:16` |
| 17 | c195_rwmutex_new | RwMutex::new | value-readback | ContractGapCandidate | ContractGapCandidate | ✓ | `c195_rwmutex_new.rs:16` |
| 18 | c196_rwmutex_read | RwMutex::read | value-readback | ContractGapCandidate | ContractGapCandidate | ✓ | `c196_rwmutex_read.rs:16` |
| 19 | c199_rwmutex_try_read | RwMutex::try_read | value-readback | ContractGapCandidate | ContractGapCandidate | ✓ | `c199_rwmutex_try_read.rs:16` |
| 20 | c216_waitqueue_wake_all | WaitQueue::wake_all | value-readback | ContractGapCandidate | ContractGapCandidate | ✓ | `c216_waitqueue_wake_all.rs:16` |
| 21 | c232_either_left | Either::left | value-readback | ContractGapCandidate | ContractGapCandidate | ✓ | `c232_either_left.rs:16` |
| 22 | c233_either_right | Either::right | value-readback | ContractGapCandidate | ContractGapCandidate | ✓ | `c233_either_right.rs:16` |

不一致项：**无**。唯一的预注册覆盖（C-104，`gap → supported`）在
注册运行中命中；其编写期 v1 失败与 v2 修正见预注册 §6（不是运行后改写）。

## 4. 语义读数

### 4.1 VmReader / VmWriter（本批新增的主要缺口面）

- `limit`/`skip` 的 ensures 只约束**返回的 `&mut Self`**（`r.*`），不约束
  接收者后状态；client 丢弃返回引用后再读 `recv` 的后状态，21 个 gap 中
  的 #2/#3/#5/#6 即此形态。仓库自身在 `VmReader::skip_in_place` 的文档里
  承认 "`&mut self`-returning-`&mut Self` reborrows don't auto-propagate
  the return-value's ensures (`r.*`) to the post-state of `*self`
  (`final(self).*`)"，本运行是该文档陈述的机器见证：契约对**链式使用者**
  足够，对**丢弃返回值的普通调用者**不足。
- `has_remain`/`has_avail` 只有 `requires self.inv()`，无 ensures/returns；
  `from` 能给出 `inv`，但布尔的返回值与 `remain_spec()>0` 无桥。
- `VmWriter::from`（C-104）的契约**支持** `avail_spec() == slice.len()`：
  `from` ensures `r.wf(owner)` ∧ `owner.range.end - start == old(slice).len()`，
  `wf` 把 owner range 钉在 `cursor..end`。这是本批唯一的 supported 面，
  且是"非平凡 supported"（需要 tracked owner 绑定后组合 `wf`/range 事实，
  非 ensures 字面实例）。
- **ghost owner plumbing 边界**（重要）：`from` 的 `with -> owner:
  Tracked<VmIoOwner>` 是 `proof_with!` 输出；不绑定该 tracked 输出的
  client 在 Verus 下会在 `from` 调用点报 `precondition not satisfied`
  （io.rs:1949 的 `with`），并使后续 `inv` 前置连带失败。编写期 v1 因此
  全部 7 条 VmReader/VmWriter 条目失败；v2 在 recipe 里补
  `proof_decl!` + `#[verus_spec(with => Tracked(owner))]`（与 io.rs:
  342-348 的仓库内调用范式一致）后，证据才变干净。**这是本次合成器最
  重要的可表达性发现之一**：契约的文档后置条件对普通 exec 调用者不是
  自动可用的，需要项目侧 ghost 绑定配方。

### 4.2 Either（C-232/C-233）

`left`/`right` 带 `#[verus_verify(dual_spec)]`，其 spec twin 在 spec
位置可展开；但 exec 契约没有 `returns`（仓库维护指南明确要求
`#[verus_spec(returns f(args))]` 才有 exec 调用点模型事实）。client 先
`let out = e.left();` 再断言 exec 返回值，故 gap。若把调用写进 `assert`
实参，Verus 会用 `when_used_as_spec` 的 spec twin，判定会翻转——本批
有意采用 exec 绑定形态（更接近真实使用方），该差异已写入预注册 §3.3。

### 4.3 WaitQueue（C-216）

`wake_all` 是验证过的 exec fn 但无 `#[verus_spec]`/ensures；空队列返回 0
是文档/运行时事实，契约不表达 → gap。`wake_all` 的循环不变量与
`usize::MAX` 假设只服务函数体内证明，不构成调用者契约。

### 4.4 锁族对照（第二梯队）

- 与 M5 精确重叠的 4 条（C-178/F-014、C-184/F-015、C-199/F-015、
  C-169/C-170/F-017）**全部复现 gap**，与 M5 run2 的判定一致；本轮不
  重复记缺陷（registry 不加新条）。
- partial 重叠的 `new`/`read`/`clone_ro`（C-162/F-002、C-180/C-181/F-004、
  C-195/C-196/F-005、C-177/F-006）均为 gap：构造值经 guard `view()`
  读回在冻结面无桥。它们只补充 F-002/F-004/F-005/F-006 的接口面证据，
  不改变其语义关系，不升级 machine_status。

## 5. 与 M7a census / M5 的对照

- 22 条选中候选的 `frozen_census_status` **全部为
  `reproduced-in-43da079`**；行号以 43da079 冻结源为准（候选
  `artifact_location` 来自 artifact 修订，存在漂移，未用于转写）。
- 22 条均为 **有后置条件** 的函数（census 的 `with_postcondition` 行），
  不是"无后置条件"的最强缺口形态；其中 7 条 VmReader/VmWriter 是
  "有 ensures 但未覆盖该输出/后状态"（M7a 的 ensures-coverage 语义），
  15 条是"ensures 未提及返回值的值关系"。
- 与 M5 run2 的关系：M5 的 11 条（mutex/rwlock/rwmutex/spinlock/rcu）
  中与 registry 精确重叠的 4 条在本轮作为对照复现；本批新增的是
  VmReader/VmWriter/Either/WaitQueue 四族，M5 未覆盖。
- M5 的 #8/#10（SpinLock supported 面）在本批无对应条目；本批的
  supported 面是 C-104（构造器 range 后状态）。

## 6. 证据边界

1. **ContractGapCandidate 是证明论结论**：义务在原契约下不可证且基线
   干净；不是运行时反例，不证明实现错误，也不等于"契约应当改"。
2. **Inconclusive 不计缺陷**：本批 0 Inconclusive；合成阶段跳过的 213
   条不是判定，只是"没有 recipe / 无模板 / 被标 blocked"的未执行项
   （明细在 `divergence-manifest-m7c.report.json`）。
3. **ghost owner 绑定**：C-096–C-102、C-104 的判定以项目侧 recipe 补
   tracked owner 绑定为前提；不补时是 client 不可表达（v1 全部
   Inconclusive-in-spirit，但分类器按 obligation 区域事件记为 gap）。
   归档以 v2 为注册证据，v1 只作为编写期边界记录。
4. **final 转写**：`final(owned_local)` 不可编译，降为"调用后局部当前
   值"的后状态断言；语义等价但形式不同（预注册 §3.1）。
5. **Either dual_spec**：spec 位置可用 spec twin；本批断言 exec 返回值，
   故 gap。该口径不适用于"assert 内直接调用"的写法。
6. **模型边界**：`VmReader::from`/`VmWriter::from` 依赖
   `axiom_slice_in_kernel` 信任边界；client 的切片是内核地址的信任假设，
   不验证切片真实有效性。`RcuOption` 只在 `cfg(ktest)` 下可从
   `crate::sync` 导出（43da079 的收窄边界，M5 已记录）。
7. **运行后恢复**：9 个被触及/核对文件的 sha256 与运行前一致
   （mutex.rs `eea4b78a…`、lib.rs `f1413d24…`、io.rs `22f3dbc0…`、
   wait.rs `90d91367…`、either.rs `bca6b8ad…`、rwlock.rs `8606975a…`、
   rwmutex.rs `decb6a7e…`、rwarc.rs `3865d9b0…`、rcu/mod.rs
   `137f3686…`）；`ostd/src/spec_check_probes/` 已移除，lib.rs 无注入
   声明；replay 副本在 `run1/injections/`。
8. **单次运行**：22 条目同一次注入构建；vstd 1908/0、target 1498/21、
   baseline clean。运行产物：`/tmp/vostd-divergence-m7c-run1/`
   （`report.json`、`precheck-build.log`、`target-build.log`、
   `injections/`）。

## 7. registry 处置（research 仓 `findings/registry.json`）

按维护纪律（同一语义关系一条、`obligation_fail` 级不虚构升级）：

- **新增 7 条**：F-018（VmReader/VmWriter limit/skip 接收者后状态）、
  F-019（has_remain/has_avail 布尔返回）、F-020（Either left/right exec
  返回值）、F-021（WaitQueue::wake_all 空队列返回值）、F-022
  （AsVirtPtr::as_virt_ptr 指针读回）、F-023（Jiffies::elapsed 单调性）、
  F-024（PageTablePageMeta::new level 读回）。全部 `machine_status =
  obligation_fail`，证据指向本运行/提升运行的判定与 span。
- **扩展 3 条既有条目**：F-014（RwArc 值读回：新增 clone_ro→read 接口与
  C-177/C-178 证据）、F-015（锁族值读回：新增 Mutex/RwLock/RwMutex 的
  new/read 接口与 C-162/C-180/C-181/C-195/C-196 证据）、F-017
  （RcuOption：新增 C-167/C-169/C-170 证据）。
- 4 条与 M5 精确重叠（F-014/F-015/F-017）只补证据，不重复记缺陷、
  不升级 machine_status。
- C-104（supported）不是 gap，不新增条目。
- 本轮 research 仓只改 `experiments/spec-quality-two-day/findings/registry.json`。

## 8. needs-recipe 评估

66 条 needs-recipe 的逐条分级与实际尝试见
`examples/vostd-divergence/NEEDS-RECIPE-M7C.md`：**3 条成功提升**为
可执行义务（C-103 AsVirtPtr、C-228 Jiffies::elapsed、C-127
PageTablePageMeta::new，均 ContractGapCandidate、expect_match ✓），
**3 条尝试失败**（C-006 模块可见性 E0433；C-053/C-056
`Repr<MetaSlotSmall>` E0277），其余 **60 条**在 43da079 冻结面无可用
配方（DMA/VmIo 缺 `AnyUFrameMeta` 具体实现、链表缺
`Repr<MetaSlotSmall>`、VmSpace/页表 owner 不可铸造、external_body /
无 spec 身份等）。这是 M7b 前置"needs-recipe 66"的修正性结论。
