# M7c needs-recipe 评估（66 条，2026-11-12）

输入：`examples/ensures-coverage-candidates/candidates.json` 中
`constructibility = needs-recipe` 的 **66 条**。方法：逐条按冻结源
（rev 43da079）核对类型约束、构造器可见性、trait 实现与 tracked/ghost
参数可铸造性；对 6 条写出配方并实际生成 manifest（3 条编译+验证成功，
3 条在编译期/可达性上失败）。所有结论以源码与运行日志为准。

## 1. 分级汇总

| 分级 | 条数 | 含义 |
|---|---:|---|
| A-promoted | **3** | 配方可写、client 编译并通过 divergence 流程（判定见 §2） |
| A-attempted | **3** | 已写配方尝试，冻结面结构性阻断（编译/可达性证据见 §2） |
| C-infeasible | **60** | 按当前冻结面不可补：缺具体 trait 实现 / 无公开构造器 / tracked owner 不可铸造 / external_body / 模块不可达 |
| 合计 | **66** | — |

> 结论：M7b 前置报告把 66 条标为 needs-recipe 时假设"配方在冻结面存在
> 或已有同族先例"；M7c 的逐条核对表明，**60 条在 43da079 冻结面上没有
> 可写的构造配方**（关键是缺 `AnyUFrameMeta`/`Repr<MetaSlotSmall>` 具体
> 实现与不可铸造的 tracked owner），只有 3 条被真正提升为可执行义务。
> 这是对前置报告的重要修正，不是"配方写不出来"的托词。

## 2. 实际尝试（6 条）

### 2.1 成功提升（3）

| id | 函数 | 配方要点 | 义务 | 运行结果 | 产物 |
|---|---|---|---|---|---|
| C-103 | `AsVirtPtr::as_virt_ptr` | 类型 `&[u8]`；`use crate::mm::io::AsVirtPtr` | `out.vaddr == buf.as_ptr() as usize` | ContractGapCandidate，`c103_...rs:18`，expect_match ✓ | `/tmp/vostd-divergence-m7c-promoted-run2`（与 C-228 同批：vstd 1908/0，target 1497/2，baseline clean）；manifest 归档 `divergence-manifest-m7c-promoted1.json` |
| C-228 | `Jiffies::elapsed` | 无接收者；两次 elapsed 取值 | `second_v >= first_v`（单调性；候选草稿的原子值读回不可表达，见 §3） | ContractGapCandidate，`c228_...rs:18`，expect_match ✓ | 同上 |
| C-127 | `PageTablePageMeta::<C>::new` | `new(3)`；`use crate::mm::page_table::PageTablePageMeta` + `KernelPtConfig` | `out.level == 3` | ContractGapCandidate，`c127_...rs:16`，expect_match ✓ | `/tmp/vostd-divergence-m7c-promoted2-run1`（vstd 1908/0，target 1497/1，baseline clean）；manifest 归档 `divergence-manifest-m7c-promoted2.json` |

三条的 recipe 都在合成器 schema 内完成（无工具改动）；C-127 用
value-readback 的记录形态表达构造器后状态（候选 family 为 value-readback，
不强行改族）。C-228 的义务与候选草稿不同：`ELAPSED` 是 `pub(crate)` 且
`timer::jiffies` 模块私有，普通 client 无法从 crate 根读静态原子；改用
"两次 elapsed 单调不减"的文档义务。该差异在结果中如实登记，未改写候选。

### 2.2 尝试失败（3）

| id | 函数 | 配方要点 | 失败证据 | 结论 |
|---|---|---|---|---|
| C-006 | `Drhd::register_base_addr` | 16 字节 ACPI header + `Drhd::from_bytes` | `E0433: cannot find x86 in arch`（c006_...rs:10）；`arch/x86/kernel/acpi` 为 `pub(super)`（`arch/x86/kernel/mod.rs:2`），crate 根探针不可达 | C：模块可见性阻断，非契约结论 |
| C-053 | `LinkedList::new` | `LinkedList::<MetaSlotStorage>::new()` | `E0277: MetaSlotStorage: Repr<MetaSlotSmall> is not satisfied`（c053_...rs:15）；唯一实现是 `Repr<MetaSlotStorage>`（meta_owners.rs:108） | C：无具体 `Repr<MetaSlotSmall>` 实现，`LinkedList<M>` 无实例化类型 |
| C-056 | `<Link<M> as Deref>::deref` | `Link::<MetaSlotStorage>::new(MetaSlotStorage::Untyped)` | 同 `E0277`（c056_...rs:15）；`Link<M>` 的 `Repr<MetaSlotStorage>` impl 以 `M: Repr<MetaSlotSmall>` 为前置（linked_list_owners.rs:45） | C：同上 |

失败运行的判定都是 Inconclusive（phase-1 编译失败），按纪律不计缺陷；
编译日志在 `/tmp/vostd-divergence-m7c-infeasible-run1/precheck-build.log`
与 `/tmp/vostd-divergence-m7c-infeasible2-run1/precheck-build.log`。
配方归档：成功路径 `examples/vostd-divergence/recipes-m7c-promoted.json`
（含 C-103/C-228/C-127），失败路径
`examples/vostd-divergence/recipes-m7c-infeasible.json`（C-053/C-056）；
对应 manifest 分别归档为 `divergence-manifest-m7c-{promoted1,promoted2,
infeasible1,infeasible2}.json`，可由 `synthesize --select <id>` 逐字节
复现（本次已核对 sha256 一致）。

## 3. C 级的结构性原因（60 条）

| reason（计数） | 证据 |
|---|---|
| DMA / VmIo 族不可构造（39） | `AnyUFrameMeta` 无具体实现：`ostd/src/mm/frame/untyped.rs:60-80` 的 `impl_untyped_frame_meta_for!` 整体注释停用；`DmaCoherent::map` 需 `Segment<M>`（dma_coherent.rs:125）；`DmaStream` 无公开构造器；`VmIo`/`VmIoOnce` 仅由 DMA 实现（dma_stream.rs:1054、dma_coherent.rs:547） |
| 链表族无 `Repr<MetaSlotSmall>`（6，不含已尝试的 C-053/C-056） | 全仓只有 `Repr<MetaSlotStorage>`（meta_owners.rs:108）与 `Link<M>` 的条件 impl（linked_list_owners.rs:45）；无任何具体 `M` 满足 `AnyFrameMeta + Repr<MetaSlotSmall>` |
| VmSpace / 页表 cursor 的 tracked owner（11） | `VmSpace::{default,new}` 由 `with Tracked(&mut MetaRegionOwners), Tracked(&mut Guards)` 提供（vm_space.rs:155-180/207）；`MetaRegionOwners::inv` 要求全槽 map（meta_region_owners.rs:41-60），空 map 不可满足，冻结面无铸造函数 |
| external_body / 语义不可证（1） | `PageTable::new_kernel_page_table` 为 `#[verifier::external_body]` 且 body `unimplemented!()`（page_table/mod.rs:917-923） |
| 无 spec 身份关系（1） | `Waiter::task` 返回 `Arc<Task>`，冻结面无 Arc 身份/字段关系的 spec 谓词（wait.rs:412） |
| trait 声明无可用具体接收者（2） | `HasDaddr::daddr`、`HasPaddr::paddr`：具体 impl 为 DMA 族（不可构造）或 `&T` blanket；`IoMem::acquire` 依赖全局 allocator |
| **C 级小计** | **60**（39 + 6 + 11 + 1 + 1 + 2） |
| 已尝试失败（C-006 模块可见性；C-053/C-056 `Repr<MetaSlotSmall>`） | 3 |
| 已提升（C-103 / C-228 / C-127） | 3 |
| **合计** | **66** |

## 4. 逐条分级表

| id | 函数 | 族 | 分级 | 原因 / 证据 |
|---|---|---|---|---|
| C-006 | `Drhd::register_base_addr` | value-readback | A-attempted | 尝试失败：见 §2 的具体编译/可达性证据 |
| C-030 | `DmaCoherent<M>::daddr` | value-readback | C | 冻结面无 AnyUFrameMeta 具体实现（untyped.rs:60-80 宏注释停用）；DmaCoherent::map 需 Segment<M> |
| C-031 | `DmaCoherent<M>::paddr` | value-readback | C | 冻结面无 AnyUFrameMeta 具体实现（untyped.rs:60-80 宏注释停用）；DmaCoherent::map 需 Segment<M> |
| C-032 | `DmaCoherent<M>::nframes` | value-readback | C | 冻结面无 AnyUFrameMeta 具体实现（untyped.rs:60-80 宏注释停用）；DmaCoherent::map 需 Segment<M> |
| C-033 | `DmaCoherent<M>::nbytes` | value-readback | C | 冻结面无 AnyUFrameMeta 具体实现（untyped.rs:60-80 宏注释停用）；DmaCoherent::map 需 Segment<M> |
| C-034 | `<DmaCoherent<M> as HasDaddr>::daddr` | value-readback | C | 冻结面无 AnyUFrameMeta 具体实现（untyped.rs:60-80 宏注释停用）；DmaCoherent::map 需 Segment<M> |
| C-035 | `<DmaCoherent<M> as HasPaddr>::paddr` | value-readback | C | 冻结面无 AnyUFrameMeta 具体实现（untyped.rs:60-80 宏注释停用）；DmaCoherent::map 需 Segment<M> |
| C-036 | `<DmaCoherent<M> as VmIoOnce>::read_once` | other | C | 冻结面无 AnyUFrameMeta 具体实现（untyped.rs:60-80 宏注释停用）；DmaCoherent::map 需 Segment<M> |
| C-037 | `<DmaCoherent<M> as VmIoOnce>::write_once` | other | C | 冻结面无 AnyUFrameMeta 具体实现（untyped.rs:60-80 宏注释停用）；DmaCoherent::map 需 Segment<M> |
| C-038 | `DmaStreamSlice<Dma, M>::stream` | value-readback | C | DmaStreamSlice 需 Dma: AsRef<DmaStream>；DmaStream 无公开构造器 |
| C-039 | `DmaStreamSlice<Dma, M>::sync` | other | C | DmaStreamSlice 需 Dma: AsRef<DmaStream>；DmaStream 无公开构造器 |
| C-040 | `DmaStreamSlice<Dma, M>::daddr` | value-readback | C | DmaStreamSlice 需 Dma: AsRef<DmaStream>；DmaStream 无公开构造器 |
| C-041 | `DmaStream<M>::daddr` | value-readback | C | DmaStream 无公开构造器（inner 为 pub RwArc，但无构造路径） |
| C-042 | `DmaStream<M>::direction` | value-readback | C | DmaStream 无公开构造器（inner 为 pub RwArc，但无构造路径） |
| C-043 | `DmaStream<M>::sync` | value-readback | C | DmaStream 无公开构造器（inner 为 pub RwArc，但无构造路径） |
| C-044 | `DmaStream<M>::paddr` | value-readback | C | DmaStream 无公开构造器（inner 为 pub RwArc，但无构造路径） |
| C-045 | `DmaStream<M>::nframes` | value-readback | C | DmaStream 无公开构造器（inner 为 pub RwArc，但无构造路径） |
| C-046 | `DmaStream<M>::nbytes` | value-readback | C | DmaStream 无公开构造器（inner 为 pub RwArc，但无构造路径） |
| C-047 | `HasDaddr::daddr` | value-readback | C | trait 声明；具体 impl 仅 DMA 族（不可构造）与 &T blanket |
| C-053 | `LinkedList<M>::new` | value-readback | A-attempted | 尝试失败：见 §2 的具体编译/可达性证据 |
| C-054 | `<LinkedList<M> as Default>::default` | value-readback | C | 冻结面无 Repr<MetaSlotSmall> 具体实现（仅 Repr<MetaSlotStorage>，meta_owners.rs:108） |
| C-055 | `CursorMut<'a, M>::as_list` | guard-identity | C | 冻结面无 Repr<MetaSlotSmall> 具体实现（仅 Repr<MetaSlotStorage>，meta_owners.rs:108） |
| C-056 | `<Link<M> as Deref>::deref` | value-readback | A-attempted | 尝试失败：见 §2 的具体编译/可达性证据 |
| C-057 | `<Link<M> as DerefMut>::deref_mut` | value-readback | C | 冻结面无 Repr<MetaSlotSmall> 具体实现（仅 Repr<MetaSlotStorage>，meta_owners.rs:108） |
| C-058 | `<Link<M> as DerefMut>::deref_mut` | final-relation | C | 冻结面无 Repr<MetaSlotSmall> 具体实现（仅 Repr<MetaSlotStorage>，meta_owners.rs:108） |
| C-059 | `Link<M>::new` | value-readback | C | 冻结面无 Repr<MetaSlotSmall> 具体实现（仅 Repr<MetaSlotStorage>，meta_owners.rs:108） |
| C-061 | `<Link<M> as AnyFrameMeta>::is_untyped` | is_none-demand | C | 冻结面无 Repr<MetaSlotSmall> 具体实现（仅 Repr<MetaSlotStorage>，meta_owners.rs:108） |
| C-069 | `Segment<M>::from_unused` | final-relation | C | Segment<M> 需 AnyUFrameMeta；ghost repr_perm 不可铸造 |
| C-073 | `UniqueFrame<M>::try_from_shared` | value-readback | C | UniqueFrame 需 Frame<M> + MetaRegionOwners（不可铸造，见 C-140 族） |
| C-074 | `UntypedMem::reader` | value-readback | C | UntypedMem 具体 impl 仅 Segment<M>（AnyUFrameMeta，无具体 impl） |
| C-075 | `UntypedMem::writer` | value-readback | C | UntypedMem 具体 impl 仅 Segment<M>（AnyUFrameMeta，无具体 impl） |
| C-077 | `VmIo::read_bytes` | other | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-078 | `VmIo::read_bytes` | final-relation | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-079 | `VmIo::read_bytes` | final-relation | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-080 | `VmIo::read_val` | value-readback | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-081 | `VmIo::read_val` | final-relation | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-082 | `VmIo::read_slice` | other | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-083 | `VmIo::read_slice` | final-relation | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-084 | `VmIo::read_slice` | final-relation | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-085 | `VmIo::write_bytes` | other | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-086 | `VmIo::write_bytes` | final-relation | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-087 | `VmIo::write_val` | other | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-088 | `VmIo::write_val` | final-relation | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-089 | `VmIo::write_slice` | other | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-090 | `VmIo::write_slice` | final-relation | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-091 | `VmIo::write_vals` | value-readback | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-092 | `VmIo::write_vals` | final-relation | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-093 | `VmIoOnce::read_once` | value-readback | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-094 | `VmIoOnce::write_once` | other | C | VmIo/VmIoOnce 仅由 DmaStream/DmaCoherent 实现；DMA 族不可构造（且需 ghost owner） |
| C-103 | `AsVirtPtr::as_virt_ptr` | value-readback | A-promoted | 已提升：recipe 编译并运行（见 §2） |
| C-113 | `HasPaddr::paddr` | value-readback | C | HasPaddr 具体 impl 为 IoMem（需全局 IO allocator）与 DMA 族（不可构造） |
| C-118 | `CursorMut<'rcu, C>::protect_next` | value-readback | C | CursorMut 来自页表 owner/PageTable 构造链，需 tracked owner |
| C-120 | `PageTable<KernelPtConfig>::new_kernel_page_table` | value-readback | C | external_body 且 body 为 unimplemented!()；无 ensure 语义可证（pub(crate) 可达但不诚实） |
| C-127 | `PageTablePageMeta<C>::new` | value-readback | A-promoted | 已提升：recipe 编译并运行（见 §2） |
| C-140 | `VmSpace::default` | value-readback | C | VmSpace::{default,new,cursor,...} 需 Tracked(&mut MetaRegionOwners)/Guards；冻结面无铸造入口（inv 要求全槽 map） |
| C-141 | `VmSpace::default` | final-relation | C | VmSpace::{default,new,cursor,...} 需 Tracked(&mut MetaRegionOwners)/Guards；冻结面无铸造入口（inv 要求全槽 map） |
| C-142 | `VmSpace::default` | final-relation | C | VmSpace::{default,new,cursor,...} 需 Tracked(&mut MetaRegionOwners)/Guards；冻结面无铸造入口（inv 要求全槽 map） |
| C-143 | `VmSpace::new` | value-readback | C | VmSpace::{default,new,cursor,...} 需 Tracked(&mut MetaRegionOwners)/Guards；冻结面无铸造入口（inv 要求全槽 map） |
| C-144 | `VmSpace::new` | final-relation | C | VmSpace::{default,new,cursor,...} 需 Tracked(&mut MetaRegionOwners)/Guards；冻结面无铸造入口（inv 要求全槽 map） |
| C-145 | `VmSpace::cursor` | final-relation | C | VmSpace::{default,new,cursor,...} 需 Tracked(&mut MetaRegionOwners)/Guards；冻结面无铸造入口（inv 要求全槽 map） |
| C-146 | `VmSpace::cursor` | final-relation | C | VmSpace::{default,new,cursor,...} 需 Tracked(&mut MetaRegionOwners)/Guards；冻结面无铸造入口（inv 要求全槽 map） |
| C-147 | `VmSpace::cursor_mut` | final-relation | C | VmSpace::{default,new,cursor,...} 需 Tracked(&mut MetaRegionOwners)/Guards；冻结面无铸造入口（inv 要求全槽 map） |
| C-148 | `VmSpace::cursor_mut` | final-relation | C | VmSpace::{default,new,cursor,...} 需 Tracked(&mut MetaRegionOwners)/Guards；冻结面无铸造入口（inv 要求全槽 map） |
| C-150 | `CursorMut<'a>::protect_next` | value-readback | C | CursorMut 需 VmSpace 页表 owner（同 C-140 族） |
| C-219 | `Waiter::task` | guard-identity | C | Waiter::task 返回 Arc<Task>；冻结面无 Arc 身份 spec 关系可断言 |
| C-228 | `Jiffies::elapsed` | value-readback | A-promoted | 已提升：recipe 编译并运行（见 §2） |

## 5. 边界

- A-promoted 的三条是**契约缺口候选**（证明论），不是运行时反例；C-103/
  C-228/C-127 的义务形态与候选草稿的差异已在 §2 与运行产物中记录。
- C 级的"不可补"是**对 43da079 冻结面**的结论；若上游补上具体
  `AnyUFrameMeta`/`Repr<MetaSlotSmall>` 实现或 owner 铸造入口，配方可
  重新评估，届时需新的运行证据，不能沿用本表。
- 本表不改动 `candidates.json`；分级是 M7c 的新增评估，属于项目侧输入
  的补充证据，不是对 M7b 预注册的改写。
