# M7c 前置：ensures-coverage 235 条 findings 的义务候选与可合成性评估

- 日期：2026-11-12
- 输入（只读）：
  - `/tmp/jinser-bzyA6G/ensures-coverage/`（`head.results` 1502 函数、`all.md` 235 findings）
  - 冻结源 `/tmp/vostd-43da079-ktest-chan/ostd/src`（rev 43da079；行号证据均指此树）
  - `~/vie/work/AES/spec-check/examples/vostd-divergence/divergence-manifest.json` / `DIVERGENCE-RUN.md`
  - `~/vie/work/AES/research/experiments/spec-quality-two-day/findings/registry.json`
- 性质：**只读解析 + 源码核对，无构建、无注入、无仓库写入**。所有产出只在 `/tmp/m7c-candidates/`。
- 产出：`candidates.json`（235 条结构化记录）、`REPORT.md`（本文件）、`gen.py` / `_parsed3.json`（解析与生成脚本/中间数据）。

---

## 1. 结论摘要

| 项 | 数量 |
|---|---|
| findings 总数 | **235**（204 个函数；有后置条件 42、无后置条件 193） |
| 输出类别 | 返回值 174、`&mut` 参数状态 61、元组分量单列 0 |
| 执行/证明 | exec 232、proof 3 |
| 后置条件来源 | 自身 40、trait 声明 2、无 193 |
| `external_body` | 30 |
| 与 registry 重叠 | **56 条**（exact 8 + partial 48），涉及 51 个不同函数 |
| 43da079 可复现性 | **196 条**可在 peer M7a 的 43da079 census 找到对应函数（至少一个 finding）；**39 条仅存于 artifact 修订**（APIC 14 + Debug fmt 22 + AsAtomicModeGuard 3），且全部已归 `blocked` |
| 可合成性 | **constructible 48 / needs-recipe 66 / blocked 121** |
| 预注册期望（运行前假设） | **gap 93 / inconclusive 142 / supported 0** |
| Top 10 | 10/10 为 constructible 且与 registry 不重叠（见 §6） |

**一句话**：235 条中真正“构造即得、公开可观察”的只有 48 条（20%），且集中在我们尚未入账的 VmReader/VmWriter 与 Either/WaitQueue 小族；锁族的值读回候选大多与 F-004/F-005/F-006/F-014/F-015/F-017 部分或完全重叠。宏/派生/不透明 trait 生成的噪声占 40 条（17%）。

---

## 2. 方法

1. **以 `all.md` 的 “Selected functions” 表为 finding 索引**：1502 行，findings 计数之和恰为 235（与报告摘要一致）；每行给函数名、artifact 位置、exec/proof、`non-pub` 标记、该函数 finding 数。
2. **解析详细章节**：155 个 `###` 标题、175 个 bullet；三个合并 trait 章节（Clone 15、Default 3、Debug 11）按实现数展开后得到 235 条。每条记录保存 `artifact_location`、输出名、artifact 报告类型、`Type invariant only` 标记。
3. **与 `head.results` 连接**：按 `(file,line)` 匹配、同名回退；1502/1502 全部匹配。记录 `post own|decl`、`trait`、`external-body`、`outs`、`ret`/`mut` 类型、VIR 推断名。
4. **源码核对按函数名解析，不按 artifact 行号**（修订差异见 §8.1）：所有证据行号来自冻结源，并由 `rg` 定位构造器/可见性/访问器。
5. **义务模板**：复用 DIVERGENCE 四族语义；每条的草案是按族 + 接收者类型 + 方法名实例化的 Rust/Verus 伪代码，不做项目侧假设。
6. **可合成性判据**（三档）：
   - `constructible`：接收者/参数可由公开构造器、字面量或公开字段机械构造；`requires` 可满足；输出有公开访问器/可观察谓词。
   - `needs-recipe`：需要类型族配方（具体 `M: AnyFrameMeta+Repr<...>`、DMA/页表/帧元数据 owner、ghost map/proof 参数、字节布局），但配方在冻结面存在或已有同族先例。
   - `blocked`：私有字段/无公开构造器/输出无访问器/`pub(in crate::mm)` 不可达/类型未导出/external_body 无可证语义/仅 proof-ghost。
   - 可见性按注入探针位于 crate 根（`ostd/src/spec_check_probes/`）判定：`pub` 与 `pub(crate)` 可达，裸 `fn` 与 `pub(in crate::mm)` 不可达。
7. **期望预注册**：`gap` = 可构造且义务可表达、契约（含类型不变式）推不出；`inconclusive` = 客户端或断言不可机械构造/不可观察；`supported` 仅在契约已蕴含该义务时登记。**全部为运行前假设，未验证。**

---

## 3. 义务家族分布

族定义与既有工作一致：

- **value-readback**：构造/获取后的值（含 guard 的 `view()`、构造器状态、纯 getter）应读回为期望值。
- **guard-identity**：返回的 guard/句柄应与产生它的锁/owner 有身份关系（`rel_lock`/`rel_mutex`/`rel_rcu`/atomic-mode 同一性）。
- **is_none-demand**：布尔谓词/`Option` 存在性需求（`is_none`/`is_empty`/`has_remain`/`is_left` …）在给定构造下应取指定值。
- **final-relation**：调用后 `&mut` 参数（含 ghost `&mut`）的状态应满足 `final(...)` 关系。
- **other**：trait 派生样板（Clone/Debug/Default）、Result/错误语义、external_body、全局静态、proof-only ghost、不透明返回类型。

| 家族 | findings | constructible | needs-recipe | blocked | gap | inconclusive |
|---|---:|---:|---:|---:|---:|---:|
| value-readback | 97 | 37 | 36 | 24 | 72 | 25 |
| guard-identity | 20 | 1 | 2 | 17 | 1 | 19 |
| is_none-demand | 11 | 5 | 1 | 5 | 5 | 6 |
| final-relation | 46 | 5 | 18 | 23 | 15 | 31 |
| other | 61 | 0 | 9 | 52 | 0 | 61 |
| **合计** | **235** | **48** | **66** | **121** | **93** | **142** |

其他切片：

| 维度 | 分布 |
|---|---|
| 输出类别 | return 174（VR 96、GI 20、demand 11、other 47）；`&mut` 61（FR 46、other 15） |
| 模块 | mm 122、sync 69、arch 24、task 6、util 5、specs 3、cpu 2、timer 2、trap 2 |
| 可见性 | pub 208、private 27（artifact `non-pub` 标记） |
| `Type invariant only` | 25 条 bullet（artifact 摘要表推算 26，见 §8.3） |
| VIR 推断名（宏/派生） | 14 条 finding；artifact 共 196 个推断名函数 |
| 合并 trait 章节渲染 | 40 条（Clone 15、Default 3、Debug 22），全部 `other` |

---

## 4. 可合成性评估

### 4.1 三档与理由码

| 档 | 数量 | 理由码（条数） |
|---|---:|---|
| constructible | 48 | `public-ctor+observable` 48 |
| needs-recipe | 66 | `needs-type-family-recipe` 66 |
| blocked | 121 | `private-visibility` 40、`other-blocked` 44、`no-observable-state` 12、`external-body-no-spec` 11、`ghost-only` 7、`derived-no-spec-equality` 6、`not-exported` 1 |

### 4.2 constructible（48）的构成

- **锁/同步值读回（23）**：`Mutex::new`、`RwLock::{new,read,write,upread,try_*}`、`RwMutex::{new,read,write,upread,try_*}`、`RwArc::{read,write,clone_ro,get_cloned}`、`RoArc::read`、`RwLockUpgradeableGuard::upgrade`、`RwMutex{WriteGuard::downgrade,UpgradeableGuard::upgrade}`。
  证据：`ostd/src/sync/mutex.rs:53,202`；`rwlock.rs:344,397,414,435,449,507,563,954`（guard `view` open：`724,815,942`）；`rwmutex.rs:276,327,338,353,361,415,464,692,876`（`598,669,859`）；`rwarc.rs:59,70,75,111,174,182`。
- **VmReader/VmWriter（9）**：`from(&[u8])` / `from(&mut [u8])` 公开且无 requires（`mm/io.rs:1959,2016`），`cursor/has_remain/limit/skip/has_avail` 公开（`1580,1589,1606,1631,1714,1723,1740,1765`）。
- **RcuOption（4）**：`RcuOption` 在 `cfg(ktest)` 下从 `crate::sync` 重导出（`sync/mod.rs:25`），`Box<T>: NonNullPtr`（`rcu/non_null/mod.rs:239`），`new/new_none/read/is_none` 公开（`rcu/mod.rs:788,797,833,903`）。
- **Either（5）**：`pub enum` + `derive(Clone,PartialEq)`（`util/either.rs:11`），`left/right/is_left/is_right` 公开（`25,35,44,52`）。
- **其他（7）**：`AtomicCpuSet::new`（`cpu/set.rs:537`，requires `CpuSet.inv`，`new_empty` 保证 `inv`；`load` 公开）；`WaitQueue::{new,wake_one,wake_all}` 与 `Default`（`sync/wait.rs:97,146,189`；`is_empty` 私有，但 `wake_all` 可观察空队列）；`is_page_aligned`（`mm/mod.rs:323`，纯字面量，低价值）；`RangeAllocator::new`（ZST，义务空洞，低价值）。

> constructible 名单中 `RangeAllocator::new`、`is_page_aligned`、`Either::clone` 属低价值/空洞候选，Top 10 已降权。

### 4.3 needs-recipe（66）的主要类型族

| 类型族 | 代表 findings | 配方/证据 |
|---|---|---|
| DMA | `DmaCoherent::{daddr,paddr,nframes,nbytes}` + trait impls + `VmIoOnce`（8）；`DmaStream*`（9） | `DmaCoherent::map(segment,...)` 需 `segment.inv()`（`dma/dma_coherent.rs:125`）；`DmaStream.inner: RwArc<...>` 公开但无公开 `DmaStream::new`（`dma/dma_stream.rs:94,125`）；`DmaStreamSlice` 字段公开（`98-102`） |
| VmSpace/页表 | `VmSpace::{default,new,cursor,cursor_mut}`（10）、`CursorMut::protect_next`（2） | `vm_space.rs:163,207,306,376` 需 tracked page-table owner + ghost `regions/guards` |
| VmIo | `VmIo::{read_bytes,read_val,read_slice,write_*}`（14）、`VmIoOnce`（2） | `VmIo` 仅由 `DmaStream`/`DmaCoherent` 实现（`dma_stream.rs:1054`、`dma_coherent.rs:547`）；ghost `&mut P` owner |
| 帧/链表 | `LinkedList::new`、`Link::new`、`UniqueFrame::try_from_shared`、`Segment::from_unused`、`PageTablePageMeta::new`、`UntypedMem::{reader,writer}`、`Waiter::task`、`Drhd::register_base_addr` | `linked_list.rs:195,1696`（需 `M: AnyFrameMeta+Repr<MetaSlotSmall>`）；`segment.rs:278`（ghost repr_perm）；`unique.rs:613`；`node/mod.rs:631`；`untyped.rs:24`；`wait.rs:328,412`；`remapping.rs:44,349`（`from_bytes` 公开，需合法 header 字节） |
| 其他 | `AsVirtPtr::as_virt_ptr`、`PageTable::new_kernel_page_table`、`Jiffies::elapsed` | `io.rs` trait；`page_table/mod.rs:918`（`pub(crate)` 可达但 external_body）；`jiffies.rs:9,69`（`ELAPSED` 为 `pub(crate)`，原子 load 建模） |

### 4.4 blocked（121）按 reason code

| reason | 条数 | 典型/证据 |
|---|---:|---|
| private-visibility | 40 | `WaitQueue::is_empty`（`wait.rs:235` 裸 fn）、`XApic::read`（`xapic.rs:92`）、`OpsStack::*`（`tlb.rs:278`）、`RcuMonitor`（`rcu/monitor.rs:45,103`，`pub(super)` 且模块私有）、`PageTable::create_user_page_table`（`page_table/mod.rs:995`，`pub(in crate::mm)`）、`dfs_release_lock`（`locking.rs:632` unsafe 私有）、`KVirtArea` 无公开构造器；ACPI `Clone` derive（字段私有、无 `PartialEq`/`View`）也落此桶 |
| other-blocked | 44 | 通用不可达/无观察面：generic trait decl（`AnyFrameMeta`、`PageTableEntryTrait`、`AsVirtPtr`）、静态/全局、`RangeAllocator` 之外的 opaque 返回、`Rcu::read`（`sync/mod.rs:14` 未导出，note “same”）等 |
| no-observable-state | 12 | `TlbFlusher` 全族（`tlb.rs:31,48,69` 字段私有）、`FrameAllocOptions::{new,zeroed}`（`allocator.rs:20,35,46`，`zeroed` 无 getter）、`XApic/X2Apic::enable`、`SpinGuardian` 六个 impl、`GuardTransfer::transfer_to`（2）、`disable_local`、`KERNEL_PAGE_TABLE` |
| external-body-no-spec | 11 | `Dmar::new`、`dma_type`、`Frame::into_dyn`、`Waker::wake_up`（body 为 `unimplemented!()`）、`disable_preempt`、`DisabledLocalIrqGuard::new`、`unlock_range` 等 |
| ghost-only | 7 | `VmIoOwner::advance`（proof ghost return）、`VmSpaceOwner::{activate_reader,activate_writer}`（proof ghost `&mut self`）、`KVirtArea::{map_frames,map_untracked_frames}`、`Segment::from_unused` 的 `repr_perm`、`VmIo` 的 ghost owner |
| derived-no-spec-equality | 6 | `PagingConsts/CpuSet/VmReader/KernelPtConfig/TlbFlushOp/UserPtConfig` 的 `Clone`（无 spec 等式/类型渲染错误） |
| not-exported | 1 | `Rcu::new`（`sync/mod.rs:5,14`：`mod rcu;` 私有、`pub use` 被注释；`Rcu`/`RcuReadGuard`/`RcuDrop` 不可达） |
| **合计** | **121** | 与 §4.1 的 reason code 分布一致 |

---

## 5. 预注册期望（运行前假设，未验证）

| 期望 | 条数 | 含义/依据 |
|---|---:|---|
| gap | **93** | 客户端可机械构造/配方已知，义务可表达，而冻结契约（含 `type_inv`）推不出。例：`RwLock::try_read` 的 `g.view()==5`、`VmReader::limit` 的 `final(r).remain()<=4`、`Either::left()==Some(x)`。 |
| inconclusive | **142** | 客户端不可构造、无可观察状态、仅 ghost/proof、或 external_body 无可证语义。例：`TlbFlusher::issue_tlb_flush`（可构造但无访问器）、`SpinGuardian::guard`（无关系谓词）、`Rcu::read`（类型未导出）。 |
| supported | **0** | 选择器只列“后置条件未提及的输出”，因此不存在“契约已蕴含”的字面实例；本批无可作为 supported 控制面的条目（对照 M5 的 #2/#8 属非 finding 控制）。 |

按族：gap = VR 72 / GI 1 / demand 5 / FR 15 / other 0；inconclusive = VR 25 / GI 19 / demand 6 / FR 31 / other 61。

**重要口径**：`gap` 是证明论假设（在冻结契约下义务 client 预期失败），不是运行时反例；`inconclusive` 不计缺陷。所有判定需 M7c 注入运行后回填，`candidates.json` 中 `prereg_status` 固定为 `hypothesis-before-run (unverified)`。

---

## 6. Top 10 优先候选

排序原则：`constructible` 且 registry 不重叠优先，其次语义价值（可表达的 `final`/值关系、公开可观察、非空洞）。

| # | id | 函数 | 族 | 义务草案（伪代码） | 关键证据 | 预期 |
|---|---|---|---|---|---|---|
| 1 | C-097 | `VmReader::limit` | final-relation | `let mut r=VmReader::from(&bytes[..]); let _=r.limit(4); assert(final(r).remain() <= 4);` | `mm/io.rs:1606,2016`（`limit` 只 ensure `r.*`，无 `final(self)`） | gap |
| 2 | C-098 | `VmReader::skip` | final-relation | `let _=r.skip(4); assert(final(r).cursor.vaddr == old(r).cursor.vaddr + 4);` | `1631,2016`（需 `nbytes<=remain`，可满足） | gap |
| 3 | C-101 | `VmWriter::limit` | final-relation | `let mut w=VmWriter::from(&mut buf[..]); let _=w.limit(4); assert(final(w).remain_spec() <= 4);` | `1740,1959` | gap |
| 4 | C-102 | `VmWriter::skip` | final-relation | `let _=w.skip(4); assert(final(w).cursor.vaddr == old(w).cursor.vaddr + 4);` | `1765,1959` | gap |
| 5 | C-104 | `VmWriter::from` | final-relation | `let w=VmWriter::from(&mut buf[..]); assert(final(buf).len()==old(buf).len());` | `1959`（ensure owner/range，未提 `final(slice)`） | gap |
| 6 | C-096 | `VmReader::has_remain` | is_none-demand | `let r=VmReader::from(&bytes[..]); assert(r.has_remain());` | `1589,2016`（仅 requires `self.inv()`） | gap |
| 7 | C-100 | `VmWriter::has_avail` | is_none-demand | `let w=VmWriter::from(&mut buf[..]); assert(w.has_avail());` | `1723,1959` | gap |
| 8 | C-232 | `Either::left` | value-readback | `let e=Either::<u64,u64>::Left(5); assert(e.left()==Some(5));` | `util/either.rs:25` | gap |
| 9 | C-233 | `Either::right` | value-readback | `let e=Either::<u64,u64>::Right(5); assert(e.right()==Some(5));` | `either.rs:35` | gap |
| 10 | C-216 | `WaitQueue::wake_all` | value-readback | `let q=WaitQueue::new(); assert(q.wake_all()==0);` | `wait.rs:97,189` | gap |

备选（同为 constructible+none）：`VmReader::cursor`（C-095）、`VmWriter::cursor`（C-099）、`VmReader::has_remain` 之外的 `Either::is_left/is_right`（C-234/235）、`Either::clone`（C-231）、`WaitQueue::{new,wake_one}`（C-214/215）、`is_page_aligned`（C-114，空洞）、`RangeAllocator::new`（C-106，空洞）。

### 6.1 第二梯队：高价值但与 registry 重叠

| id | 函数 | 族 | 重叠 | 备注 |
|---|---|---|---|---|
| C-180 | `RwLock::new` | value-readback | partial F-004 | `new` 本身未被 F-004 覆盖；`read().view()==5` 与 F-015 同型 |
| C-181 | `RwLock::read` | value-readback | partial F-004 | 值读回是冻结面最强可表达义务（身份谓词不可表达） |
| C-195 | `RwMutex::new` | value-readback | partial F-005 | 同上 |
| C-196 | `RwMutex::read` | value-readback | partial F-005 | 同上 |
| C-162 | `Mutex::new` | value-readback | partial F-002 | `new(5); lock().view()==5`；`lock` 无 ensures |
| C-029 | `AtomicCpuSet::new` | value-readback | partial F-010 | `new(cs).load(Relaxed)@==cs@`；`load` 只 ensure `num_parts` |
| C-177 | `RwArc::clone_ro` | value-readback | partial F-006 | clone_ro 后 read 的 `view()==5` |
| C-167 | `RcuOption::new_none` | value-readback | partial F-017 | `new_none().read().is_none()` |
| C-184/C-199 | `RwLock/RwMutex::try_read` | value-readback | exact F-015 | 与既有 #6/#7 同义务，直接复用 |
| C-178 | `RwArc::get_cloned` | value-readback | exact F-014 | 与既有 #5 同义务 |
| C-169/C-170 | `RcuOption::read` / `is_none` | guard-identity / demand | exact F-017 | 与既有 #11 同义务 |

---

## 7. 与 registry 的 dedup 对照

| registry | 关系 | 本 artifact 对应 | 条数 | 类型 |
|---|---|---|---:|---|
| F-001 NodeOwner::inv | — | 无 | 0 | — |
| F-002 Mutex lock guard 关系 | 同类型/同族 | `Mutex::new`（C-162） | 1 | partial |
| F-003 SpinLock lock/try_lock | — | `SpinLockGuard` 仅 Debug；`SpinGuardian` 非同一关系 | 0 | — |
| F-004 RwLock try_* guard 关系 | 同族 | `RwLock::{new,read,write,upread,try_write,try_upread}`、`RwLockUpgradeableGuard::upgrade` | 7 | partial |
| F-005 RwMutex try_* guard 关系 | 同族 | `RwMutex::{new,read,write,upread,try_write,try_upread}`、`WriteGuard::downgrade`、`UpgradeableGuard::upgrade` | 8 | partial |
| F-006 RwArc read/write/RoArc read | **同关系** | `RwArc::{read,write}`、`RoArc::read`（3 exact）+ `clone_ro`（1 partial） | 4 | exact 3 / partial 1 |
| F-007 DmaCoherent read_inner guard | 同类型/邻接 | `DmaCoherent::{daddr,paddr,nframes,nbytes}`、HasDaddr/HasPaddr impls、VmIoOnce impls | 8 | partial |
| F-008 DmaStream read_inner guard | 同类型/邻接 | `DmaStream*::{daddr,direction,paddr,nframes,nbytes,sync,stream}` | 9 | partial |
| F-009 range allocator range_difference | 同类型 | `RangeAllocator::new` | 1 | partial |
| F-010 AtomicCpuSet load→iter | 同类型 | `AtomicCpuSet::new` | 1 | partial |
| F-011 AtomicCpuSet test out-of-premise | — | 无 | 0 | — |
| F-012 VmSpace 投影 query | 同族 | `VmSpace::{default,new,cursor,cursor_mut}`、`CursorMut<'a>::protect_next` | 10 | partial |
| F-013 Mutex try_lock is_none | — | 无（`try_lock` 有 ensures，不在 findings） | 0 | — |
| F-014 RwArc::get_cloned | **同关系** | `RwArc::get_cloned`（C-178） | 1 | exact |
| F-015 RwMutex/RwLock try_read 值读回 | **同关系** | `RwLock::try_read`（C-184）、`RwMutex::try_read`（C-199） | 2 | exact |
| F-016 SpinLock constant 关系 | — | 无（SpinLock 的 new/lock 有 ensures） | 0 | — |
| F-017 RcuOption read/get 值/非空 | **同关系** | `RcuOption::read`、`is_none`（2 exact）+ `new/new_none`（2 partial） | 4 | exact 2 / partial 2 |
| V-001 MutexGuard::new stub vacuity | — | 无 | 0 | — |
| V-002 guard 入口 7 项 vacuity | — | 无（`VmSpace::cursor` 只计 F-012） | 0 | — |

合计：**exact 8 条 / partial 48 条 = 56 条**，占 235 的 23.8%；涉及 **51 个不同函数**。exact 8 条可直接复用 M5 已归因义务（F-006/F-014/F-015/F-017），不建议重复投入；partial 48 条中真正未覆盖的“接口”是各锁族的 `new`/`read`/`write`/`upread`/`upgrade`/`downgrade`（F-004/F-005/F-006 只覆盖 try_* 或 guard-owner 关系）。

---

## 8. 方法与局限

### 8.1 artifact 与冻结源的修订差异（按名交集处理）

任务提示 artifact 可能来自 b6db6df，而冻结源为 43da079。实测：

- 204 个有 finding 的函数中，**11 个 artifact 行号超出冻结文件长度**；
- 仅 **131/204** 能在 artifact 行号 ±6 行内找到同名 `fn`；其余 81 个名字存在但行号漂移，中位 **14 行**、最大 **692 行**；
- **189/204** 函数名在冻结源中可按 `fn <name>` 找到；缺失 15 个均为派生/静态：7 个 ACPI `Clone` derive、`PagingConsts/CpuSet/TlbFlushOp/RcuDrop/Either` 的 `Clone`、3 个 `static`（`KVIRT_AREA_ALLOCATOR`、`KERNEL_PAGE_TABLE`、`ELAPSED`）。
- 因此：`candidates.json` 同时保留 `artifact_location` 与 `source_evidence`（冻结源行号）；所有构造性结论以源码名字解析为准，行号漂移不影响结论，但**逐条位置核对不可直接引用 artifact 行号**。

**与 peer M7a 43da079 census 的交叉核对**（`/tmp/m7a-census-43da079/census.json`，1468 函数 / 223 findings；peer 报告已按 VIR 路径交集 1210、finding 集合一致 1192/1210 = 98.51% 校准）：

- 按 `(file, method)` 连接（比 VIR 路径稳健；artifact 的 `impl&%N` 编号在修订间会重排，按路径只得到 130/235 精确命中）：**196/235** 条 finding 的函数在 43da079 census 中存在且至少有一个 finding；**39/235** 条在 43da079 中无对应函数。
- 这 39 条的构成：
  - **APIC 模块 14**：`ostd/src/arch/x86/kernel/mod.rs:3-8` 的 `pub(super) mod apic;` 整体在 `/* ... */` 内，apic 未编译；43da079 census 的 arch 只有 acpi/dmar、acpi/remapping、x86/mm。
  - **Debug `fmt` impl 22**：冻结源中这些 impl 均在 `/* ... */` 内（如 `sync/mutex.rs:280-286`、`sync/rwlock.rs:784-790`、`mm/frame/mod.rs:147-154`、`mm/frame/segment.rs:55-62`、`mm/frame/unique.rs:51-57`），未编译。
  - **`AsAtomicModeGuard` 3**：`task/atomic_mode.rs:62-75` 的 trait/impl 未标 `#[verus_verify]`，43da079 census 未收录。
- **这 39 条全部已是 `blocked`**（22 other + 12 value-readback + 3 guard-identity + 2 final-relation），因此 §4/§6 的 constructible/needs-recipe 结论不受修订差异影响；它们只影响“能在 43da079 上跑”的可执行范围，故一律为 `inconclusive`。
- 反向：census 另有约 **27 条 artifact 没有的 finding**（223 − 196；来自 43da079 新增/改约的函数，如 `frame_ref`、`linked_list` 的 `pop_*`/`cursor_*`、`Frame::from_unused` 等），本任务未纳入（不属于 artifact 的 235 条）。
- `candidates.json` 每条新增 `frozen_census_status`（`reproduced-in-43da079` / `absent-in-43da079`）、`frozen_census_path`、`revision_note`，便于 M7c 选择目标修订。

### 8.2 宏/派生与合并渲染（196 个推断名）

- `head.results` 有 **196** 个函数名来自 VIR 推断（源解析器未找到），本批 findings 只命中其中 **14** 个。
- `all.md` 对 Clone/Default/Debug 合并章节只渲染“第一个实现”的类型：40 条 finding 的 `output_type_artifact` 被错误复用（例如 15 个 `Clone` 全写 `Drhd`，`<LinkedList as Default>::default` 写 `FrameAllocOptions`）。`candidates.json` 用 `output_type_head_results`（来自 `head.results` 的 `ret`）纠正，并置 `artifact_grouped_type_artifact_bug=true`（40 条）。分析时以 head.results 类型为准。
- 这 40 条全部归入 `other`，不进入 Top 10；其中 Either::clone 是唯一可机械验证的例外（保留为 constructible）。

### 8.3 类型不变式（type invariant only）

- 25 条 bullet 带 “Type invariant only” 标注（摘要表按列相加为 26，artifact 内部有 1 条不一致；未强行对齐）。`type_inv` 只给“满足不变式”，不给“与构造值/旧状态的关系”，因此对 value-readback/final-relation 义务没有帮助；本报告仍按 `gap`（可构造）或 `inconclusive`（不可观察）处理，不把 `type_inv` 当作 supported。
- 典型：`AtomicCpuSet::new`（只 `num_parts`）、`Mutex::new`、`RwLock::new`、`RwMutex::new`、`RcuOption::new`。

### 8.4 `&mut` 与 ghost 参数

- 61 条 `&mut` findings 中，**23 条是 ghost `&mut`**（`&mut Guards`、`&mut MetaRegionOwners`、`&mut TlbModel`、`&mut P`、`&mut <M as Repr<..>>::ReprPerm`），客户端无法从普通 Rust 值构造；这些多为 `needs-recipe`（有 page-table/repr 配方）或 `blocked`（无公开构造器）。
- `final(self)` 类义务（VmReader/VmWriter `limit/skip`）是唯一“cfg 可构造 + 义务可观察”的 `final-relation` 面；其余 `final` 候选（FrameAllocOptions、TlbFlusher、XApic）状态字段私有，归 `blocked`。

### 8.5 external_body、proof-only、可见性

- 30 条 external_body：其规范是**被信任的**，不能把“无 ensures”直接等同 gap；本报告对 external_body 一律 `inconclusive`，除非同时存在公开访问器且语义可表达（例如 `Drhd::from_bytes` 不是 external）。
- 3 条 proof-only（`VmIoOwner::advance`、`VmSpaceOwner::{activate_reader,activate_writer}`）：无 exec 客户端，`blocked/inconclusive`。
- 可见性以注入探针在 crate 根为准：`pub(crate)` 可达（`X2Apic::new`、`PageTable::new_kernel_page_table`），`pub(in crate::mm)` 不可达（`create_user_page_table`）。`X2Apic::new` 虽可达，但返回类型只有私有 unit 字段、无 spec 观察面，仍归 `blocked/no-observable-state`。

### 8.6 其他

- 无运行时验证；`gap/supported/inconclusive` 是运行前假设，必须由 M7c 注入运行回填。`expect` 不匹配时如实报告，不改写预注册。
- `RangeAllocator::new`、`is_page_aligned`、`Either::clone` 等虽然 `constructible`，但义务空洞或样板化，Top 10 已降权；它们更适合作为“构造闸门”而非研究级 gap。
- `output_kind` 未出现“元组分量”独立 finding：选择器把所有非 `&mut` 输出作为整体返回值报告（`Result<...>`、`Option<...>` 不再拆分）；`outs N` 的额外输出对应 `&mut`/ghost 参数，已逐条记录。

---

## 9. 产出文件

| 文件 | 内容 |
|---|---|
| `/tmp/m7c-candidates/candidates.json` | 235 条记录：`function`、`vir_path`、`artifact_location`、`source_evidence`、`mode`、`visibility`、`artifact_section`、`post_source`、`trait_decl`、`external_body`、`frozen_symbol_found`、`artifact_inferred_name`、`output_kind/name/type_artifact/type_head_results`、`invariant_only`、`family`、`obligation_draft`、`constructibility(+reason/evidence/note)`、`prereg_expected`、`prereg_status`、`dedup_key`、`registry_overlap`、`frozen_census_status/path`、`revision_note` |
| `/tmp/m7c-candidates/REPORT.md` | 本文件（统计、Top 10、dedup 对照、局限） |
| `/tmp/m7c-candidates/_parsed3.json` | all.md + head.results 的连接结果（1502 行、155 章节） |
| `/tmp/m7c-candidates/gen.py` | 解析/分类/生成脚本（可复现，不写仓库） |

复现（只读）：`python3 /tmp/m7c-candidates/gen.py`。
