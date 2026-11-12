# M7c 义务 client 合成器首站预注册（运行前写定，运行后不改写；2026-11-12）

对象：vostd 首站（rev 43da079，`/tmp/vostd-43da079-ktest-chan`）由
`cargo-spec-check synthesize` 机械合成并由 `analyses-divergence` 执行的
**22 条义务 client**。判定只以 divergence 实跑为准；本文件是运行前假设，
不参与判定，也不在运行后修改。

## 1. 冻结输入

| 输入 | 路径 | sha256 |
|---|---|---|
| 候选（235 条） | `examples/ensures-coverage-candidates/candidates.json` | （M7b 前置归档，git 固定） |
| 项目侧配方 | `examples/vostd-divergence/recipes-m7c.json` | `2d6a72c7281cccd934f9617bb75fbbc107fae86f956c7dcc2dc1918cf487f19a` |
| 合成 manifest（22 条） | `examples/vostd-divergence/divergence-manifest-m7c.json` | `99ce4d2e5b9e79c0d43f3be5a2c049f9311f27f41b419ebabd85a7d9d74bc1e4` |
| 合成报告 | `examples/vostd-divergence/divergence-manifest-m7c.report.json` | `561c43fbc9bf6d81a671b7b7369debeb6d7c422e1523fa428639c518b8b2a4a8` |

合成统计（机械）：235 候选中 **22 条合成**，213 条跳过——`no-template-family`
61、`blocked` 69、`needs-recipe` 57、`no-interface-recipe` 26（合计 213）。
跳过项不进入 manifest（规则见 `docs/M7c-obligation-synthesizer.md` §6）。

## 2. 选条与分组

### 2.1 第一梯队（Top 10，constructible 且 registry 不重叠）

C-096/C-097/C-098（VmReader）、C-100/C-101/C-102/C-104（VmWriter）、
C-216（WaitQueue）、C-232/C-233（Either）。

### 2.2 第二梯队（12 条，与 registry 重叠，仅作对照，不重复记缺陷）

| 候选 | registry | 用途 |
|---|---|---|
| C-162 Mutex::new | partial F-002 | 锁族值读回对照 |
| C-167 RcuOption::new_none | partial F-017 | RCU 非空对照 |
| C-169 RcuOption::read | exact F-017 | M5 #11 同义务复现 |
| C-170 RcuOptionReadGuard::is_none | exact F-017 | 同上（demand 形态） |
| C-177 RwArc::clone_ro | partial F-006 | 克隆后读回 |
| C-178 RwArc::get_cloned | exact F-014 | M5 #5 同义务复现 |
| C-180/C-181 RwLock::{new,read} | partial F-004 | 锁族值读回 |
| C-184 RwLock::try_read | exact F-015 | M5 #7 同义务复现 |
| C-195/C-196 RwMutex::{new,read} | partial F-005 | 锁族值读回 |
| C-199 RwMutex::try_read | exact F-015 | M5 #6 同义务复现 |

## 3. 模板转写口径（预注册前承认的形态差异）

1. **`final(owned_local)` → 接收者后状态**：候选草稿的
   `final(r)`/`final(w)` 作用于自有局部，Verus `final` 要求 `&mut` 位置，
   不可编译。final-relation 骨架把义务降为"调用后同一局部当前值"的断言，
   语义仍是接收者后状态。涉及 C-097/C-098/C-101/C-102。
2. **exec 调用不在 `assert` 实参内**：value-readback/demand 骨架先
   `let out = <call>;`（必要时 `let flag = out.is_none();`），再断言。
   涉及 C-232/C-233（候选草稿把 `left()` 直接写在 assert 里）、
   C-167/C-169/C-170（`is_none()` 先绑定）。
3. **`Either` 的 dual_spec 口径**：`left`/`right` 带
   `#[verus_verify(dual_spec)]`，spec 位置可用同体 spec twin，但 exec
   契约没有 `returns`；本client 断言的是 exec 返回值，预期 gap。若运行
   显示 supported，说明 `when_used_as_spec` 泄漏进了该断言，需在归档中
   如实改判（预注册不改）。
4. **C-104 预注册覆盖**：候选草稿 `final(buf)`（`buf` 为自有数组）不可
   转写；义务改为构造器文档保证的 range 后状态
   `recv.avail_spec() == 8`，依据 `from` ensures
   （`r.wf(owner)` + `owner.range.end - owner.range.start == slice.len()`）
   可推，故 `expect: supported`。**这是本批唯一的 `recipe-override`**；
   原候选预注册为 `gap`，覆盖依据已写入配方 `expect_basis` 与合成报告。

## 4. 逐条预注册预测

期望值映射：`gap → ContractGapCandidate`、`supported → ObligationSupported`；
`Inconclusive`/`Anomaly` 不作为预注册值（运行后如实报告）。

| # | entry / 函数 | 族 | 义务（合成后） | 预期 | 依据（运行前） |
|---|---|---|---|---|---|
| 1 | c097_vmreader_limit / VmReader::limit | final-relation | `recv.limit(4); assert(recv.remain_spec() <= 4)` | ContractGapCandidate | `limit` ensures 只约束返回的 `r.*`（`r.remain_spec() <= max`），不约束接收者后状态；丢弃返回引用后无桥（仓库 `skip_in_place` 文档亦承认该 reborrow quirk） |
| 2 | c098_vmreader_skip / VmReader::skip | final-relation | 快照 cursor 后 `skip(4)`，断言后状态 `cursor.vaddr == before+4` | ContractGapCandidate | 同 #1；ensures 在返回引用 `r.cursor`，非接收者 |
| 3 | c101_vmwriter_limit / VmWriter::limit | final-relation | `recv.limit(4); assert(recv.avail_spec() <= 4)` | ContractGapCandidate | 同 #1（VmWriter 对称契约） |
| 4 | c102_vmwriter_skip / VmWriter::skip | final-relation | 快照 + `skip(4)` + 后状态断言 | ContractGapCandidate | 同 #2（VmWriter 对称契约） |
| 5 | c104_vmwriter_from / VmWriter::from | final-relation | `recv.avail_spec() == 8` | ObligationSupported | 覆盖项：from ensures `wf(owner)` ∧ owner range 长度 = slice 长度（8）⇒ avail_spec=8；契约字面可推 |
| 6 | c096_vmreader_has_remain / VmReader::has_remain | is_none-demand | `let out = recv.has_remain(); assert(out)` | ContractGapCandidate | `has_remain` 只有 requires `self.inv()`，无 ensures/returns；from 提供 inv 但无返回值关系 |
| 7 | c100_vmwriter_has_avail / VmWriter::has_avail | is_none-demand | `let out = recv.has_avail(); assert(out)` | ContractGapCandidate | 同上（VmWriter 对称） |
| 8 | c232_either_left / Either::left | value-readback | `let out = recv.left(); assert(out == Some(5))` | ContractGapCandidate | dual_spec 无 `returns`，exec 调用点无模型事实 |
| 9 | c233_either_right / Either::right | value-readback | `let out = recv.right(); assert(out == Some(5))` | ContractGapCandidate | 同 #8 |
| 10 | c216_waitqueue_wake_all / WaitQueue::wake_all | value-readback | `let out = recv.wake_all(); assert(out == 0)` | ContractGapCandidate | `wake_all` 无 `#[verus_spec]`/ensures；空队列返回 0 是文档/运行时事实，契约不表达 |
| 11 | c162_mutex_new / Mutex::new | value-readback | `lock().view() == 5`（持 guard） | ContractGapCandidate | `Mutex::new`/`lock` 无 ensures；`view` 为 closed spec，无值链（partial F-002 对照） |
| 12 | c180_rwlock_new / RwLock::new | value-readback | `read().view() == 5` | ContractGapCandidate | `new` const 无契约、`read` 无 ensures（partial F-004） |
| 13 | c181_rwlock_read / RwLock::read | value-readback | `read().view() == 5` | ContractGapCandidate | 同 #12 |
| 14 | c184_rwlock_try_read / RwLock::try_read | value-readback | `if let Some(out)=try_read(){ assert(out.view()==5) }` | ContractGapCandidate | `try_read` 裸 `#[verus_spec]` 无 ensures（exact F-015，M5 #7 复现） |
| 15 | c195_rwmutex_new / RwMutex::new | value-readback | `read().view() == 5` | ContractGapCandidate | 同 #12（partial F-005） |
| 16 | c196_rwmutex_read / RwMutex::read | value-readback | `read().view() == 5` | ContractGapCandidate | 同 #15 |
| 17 | c199_rwmutex_try_read / RwMutex::try_read | value-readback | if-let + `view()==5` | ContractGapCandidate | 同 #14（exact F-015，M5 #6 复现） |
| 18 | c177_rwarc_clone_ro / RwArc::clone_ro | value-readback | clone_ro → RoArc::read → `view()==5` | ContractGapCandidate | `clone_ro`/`read` 无 ensures；克隆值链不可证（partial F-006） |
| 19 | c178_rwarc_get_cloned / RwArc::get_cloned | value-readback | `get_cloned() == 5` | ContractGapCandidate | exact F-014；M5 #5 同义务复现 |
| 20 | c167_rcuoption_new_none / RcuOption::new_none | value-readback | read 后 `is_none()` 为真 | ContractGapCandidate | `new_none`/`read`/`is_none` 均无 ensures（partial F-017） |
| 21 | c169_rcuoption_read / RcuOption::read | guard-identity | read 后 `!is_none()` | ContractGapCandidate | exact F-017；M5 #11 同义务复现 |
| 22 | c170_rcuoptionreadguard_is_none / RcuOptionReadGuard::is_none | is_none-demand | read 后 `!is_none()` | ContractGapCandidate | 同 #21（demand 形态） |

**预注册分布**：ContractGapCandidate 21 / ObligationSupported 1（#5）。

## 5. 已知运行风险（不改变预测）

- phase-1（`cargo-verus check --no-verify`）若因合成代码形态失败，全部
  条目会按 divergence 规则落 `Inconclusive`（compile error）；届时在
  RUN 文档逐条归因，不把编译失败当 gap。
- `recv.limit(4);` 丢弃 `&mut Self` 返回值、`RwArc::clone_ro` 的临时
  RoArc 生命周期、`try_read` 的 if-let 形态是 three 个最可能触发编译
  差异的点；编写期预检结果见 §6。
- 义务 fail ≠ 运行时反例；`expect_match` 只作机器可查假设的命中记录。

## 6. 编写期预检（合成 manifest 编译冒烟；运行前）

两轮编写期预检，均在**注册运行之前**完成；§4 预测与 §1 哈希随之固定，
下述迭代只修 client 的可表达性/ghost plumbing，不改判定预测。

### 6.1 预检 v1（未绑定 `VmIoOwner`，`/tmp/vostd-divergence-m7c-precheck`）

- 22/22 编译通过，判定 22 × ContractGapCandidate；vstd 1908/0，目标
  1497/22，baseline clean。
- **发现**：`VmReader::from`/`VmWriter::from` 的 spec 用
  `with -> owner: Tracked<VmIoOwner>` 返回 ghost owner（`proof_with!`）；
  调用方不绑定该 tracked 输出时，Verus 在 `from` 调用点报
  `precondition not satisfied`（`io.rs:1949` 的 `with`），后续
  `limit/skip/has_*` 的 `old(self).inv()` 前置也连带失败。v1 的
  VmReader/VmWriter 失败因此混入了 client 不可表达因素，
  **不是干净的 ensures 缺口证据**。
- C-104 在 v1 中为 ContractGapCandidate（预注册 supported 未命中），
  根因同上：`from` 的 ensures 依赖未绑定的 ghost owner。

### 6.2 预检 v2（补 ghost owner 绑定，`/tmp/vostd-divergence-m7c-precheck2`）

- 修改：类型 recipe 增加通用字段 `bind_attr`，VmReader/VmWriter 的
  setup 增加 `proof_decl! { let tracked mut owner: ...VmIoOwner; }`，
  receiver 绑定上 `#[verus_spec(with => Tracked(owner))]`（与仓库内
  `io.rs:342-348` 的调用范式一致）。
- 结果：22/22 编译通过；**vstd 1908/0，目标 1498 verified / 21 errors**，
  baseline clean；21 个 error 全部是义务 fn 内 `assertion failed`
  （无 precondition/postcondition 事件）——VmReader/VmWriter 族证据
  变干净。
- 判定：21 × ContractGapCandidate + 1 × ObligationSupported（C-104），
  `expect_match` 22/22。C-104 在正确绑定 owner 后确有契约支持：
  `from` ensures `r.wf(owner)` ∧ owner range 长度 = slice 长度 ⇒
  `avail_spec == 8`。
- 注册运行将使用 v2 manifest（本文件 §1 哈希），证据目录
  `/tmp/vostd-divergence-m7c-run1`。
