# Tier C 676 函数建模可行性分析与优先级（只读）

生成时间：2026-11-12（UTC）
输入：`spec-check/examples/vostd-requires/screening.json`、`/tmp/m7e-census/census.json`、
冻结源 `/tmp/vostd-43da079-ktest-chan`（43da079）、M7e 设计、M1/M2/M6 归档、registry（28 条）。
性质：**只读分析**。未写 spec-check/research 仓库、未提交、未构建、未运行探针。

## 1. 方法

1. 以 screening 的 676 条 `tier=C` 为全集，按 path 与 `/tmp/m7e-census/census.json` join；
   后者含 M7e 新增的 `generics/params/requires`（`class`、`features`、`calls`、`vars`、`consts`、span/text）。
2. 把 `reasons` 解析为 G（generic）/R（reference）/T（ghost）三轴，得到 7 种组合。
3. 对每条 requires 子句取多标签形态：`call`、`field_access`、`quantifier`、`ctor`、`const`、
   `mechanical`；函数级取主形态（quantifier > ctor > call > field_access > const > mechanical）。
4. 按“能否在不改变语义方向的前提下把 requires 变成探针可表达的前置”给出配方：
   - `A-*`：可得到真判定（含 model-relative 的构造探针）；
   - `B-uninterp`：至少一条标量子句可机械转写，但仍有资源/不变式子句 → 按 M1 纪律转
     `uninterp`，预期 `CandidateInconclusive`；
   - `D-deferred`：全部子句是资源/不变式/深结构谓词，没有标量或标量投影内容；id 抽象只能
     得到零信息 B，构造链多被阻断，建议暂缓。
5. 所有 A 判定均回读冻结源确认行号与 requires 原文；文本缺失的 47 个函数标为 `unknown`
   依据（其中 A 项已手工回读源码补证）。

> 说明：本报告的 A/B/C 是**建模后的工程分层**，不是新的机器判定。A 的“真判定”包含
> shape/model-relative 结果；凡 model-relative 项均在 `recipes.json` 的 `risk` 字段标注。

## 2. 基线与统计

- 函数总数 1468；有 requires 774；Tier A 9 / B 89 / C 676。
- Tier C 子句 2266：mechanical 74 / unsupported 2192；113 条子句无源码文本（4.99%）。
- 函数级形态（多标签）：`call` 664、`field_access` 362、`quantifier` 84、`ctor` 44、
  `const` 140、`mechanical` 63。
- 主形态：`call` 550、`quantifier` 84、`ctor` 30、`field_access` 9、`mechanical` 2、`const` 1。
- 子句级形态（多标签）：`call` 1984、`field_access` 874、`quantifier` 130、`ctor` 60、
  `const` 235。
- 仅 260/676 个 Tier C 函数在 requires 中引用了标量参数；416 个没有引用任何标量参数。

## 3. 阻塞组合分布

| 组合 | 函数数 | 占比 | 说明 |
|---|---:|---:|---|
| G | 252 | 37.3% | 只有泛型；多数 requires 为 `segment.inv()` / `this@.inv()` 或类型常量算术 |
| R | 143 | 21.2% | 只有引用；含大量 `&mut self` owner/cursor 状态 |
| G+R | 113 | 16.7% | 泛型 + 引用，典型 page table/dma/frame |
| G+R+T | 94 | 13.9% | 三者齐全，page-table owner 家族为主 |
| G+T | 46 | 6.8% | frame/dma 的 tracked 资源 + 泛型 |
| R+T | 16 | 2.4% | `MetaSlot`/`VmIo` 的引用 + tracked |
| T | 12 | 1.8% | 纯 ghost，含 `memcpy`、`inc_frame_ref_count` 等 |
| **合计** | **676** | 100% | |

ASCII 分布：

```
G      ███████████████████████████ 252
R      ███████████████ 143
G+R    ████████████ 113
G+R+T  ██████████ 94
G+T    █████ 46
R+T    ██ 16
T      █ 12
```

高频泛型名：`C` 230、`M` 129、`C+A` 33、`Self%` 22、`T` 15、`A` 13、`E` 10、
`Fallibility` 9、`V` 8、`L` 5。
高频引用名：`self` 239、`regions` 41、`s` 33、`owner` 18、`this` 9、`bytes` 8、
`entry` 8、`writer`/`reader` 各 7。
高频 ghost 名：`regions` 87、`owner` 82、`guards` 31、`perm` 24、`parent_owner` 11、
`root_guard` 9、`metadata_perm` 6、`mem` 6。

模块集中度：page-table specs/owner 系（`cursor::owners` 47、`page_table::owners` 37、
`node::entry_owners` 16、`cursor::*` 合计 >100）、`specs::mm::embedding*` 73、
`mm::io` 36、`mm::dma::*` 33、`mm::frame*` >60。

## 4. 配方分类

### 4.1 G 泛型实例化

| 子类 | 判定 | 证据/说明 |
|---|---|---|
| G1 可实例化 | C→`UserPtConfig`/`KernelPtConfig`；`Self%`→具体 impl；`E`/`T`/`Fallibility`→标量或具体类型 | `ostd/src/mm/page_table/mod.rs:1668` `new_pt` 的 `Self=arch::x86::mm::PageTableEntry`；`page_table/mod.rs:859/876` 的 `C::NR_LEVELS()` |
| G2 需类型选择 | `M`（`AnyFrameMeta`/`AnyUFrameMeta`）、`A`（guard）、`P`（指针）、`V/L`（用户值）、`T/I` 等 | `mm/frame/untyped.rs:36` 的 `AnyUFrameMeta` 多个实现；选择影响 associated type 与常量 |
| G3 不可实例化/关联投影 | `Projection :trait_typ_args ...`、无具体 impl 的 trait decl | 少数 trait_decl（19 个 D 类）在 requires 中直接用 `Self::`/关联类型，未选定 impl 前无法求值 |

关键结论：**generic 是最可机械消除的阻塞**。只要选定一个具体类型，绝大多数 requires 的
`C::NR_LEVELS()`、`C::BASE_PAGE_SIZE()`、`MAX_PADDR` 都变成常量；但 `M`/`A` 的选择会改变
语义（不同 page-table config 的层级/页大小不同），必须记录实例。

### 4.2 R 引用抽象

| 子类 | 判定 | 证据/示例 |
|---|---|---|
| R1 纯标量投影 | A：字段/id/range 投影为标量参数 | `util/range_alloc.rs:214` `self@.start <= allocate_range.start < ...`；`page_table/mod.rs:838` `r.end > 0`；`cursor/owners.rs:193` `old(self).idx < old(self).children.len()` |
| R2 纯不变量/方法调用 | B：`self.inv()` / `wf()` / `relate_*` 转 `uninterp` | `cpu/set.rs:168` `self.inv()`；`mm/frame/linked_list.rs:366` `wf_region/relate_region` |
| R3 混合 | B：机械子句 + 不透明合取项（M1 E3 根因） | `path_slot_as_mut` 的 `idx<NR_LEVELS` + `old(path)[idx] is Some`（后者可进一步用 Seq 模型升 A） |
| R4 不可映射（A 不可达） | B-only：`&mut` 状态机、alias/生命周期、深结构 old(self) 不变式 | `cursor/mod.rs:1773 push_level`、`1677 pop_level`；`frame/mod.rs:273` 含 `Tracked<Option<&'a MetadataPerm>>`；`locking.rs:100 lock_range` |

R4 的准确含义：**仍可做 id/uninterp 的 B 探针**，但前置中的 `&mut` 别名/生命周期/后状态
语义在探针参数中不可表达，所以不能升级到 A；这类不是“不可生成”，而是“生成了也没有证据”。

### 4.3 T ghost/tracked 抽象

依据 M2/M6 铸造策略表（`examples/vostd-guard/construction/CONSTRUCTION-RUN.md`）：

| 子类 | 判定 | 类型/证据 |
|---|---|---|
| T1 可 id 化 | A/B：`Ghost<usize>`、`Ghost<CellId>`、纯 `id()`/`value()` 等值 | `mutex.rs:225` `perm.id()==mutex.cell_id()`；`cpu_local.rs:195` `self.id()==auth.id()` |
| T2 可构造 | A-model：`Guards`、`pcell::PointsTo`、`Count`、`OwnerSubtree` | M6：`Guards` 结构字面量、`PCell::new`、`Count::alloc`、`OwnerSubtree::tracked_new_val` |
| T2p 部分可构造 | B/C：`MetaRegionOwners` 可构造但 `inv()` 不可满足 | M6：空 map 字面量通过构造，`old(regions).inv()` 被真实 axiom 拒绝 |
| T3 构造链阻断 | B-only：`simple_pptr/array_ptr PointsTo`、`FrameRef/PageTableGuard`、`NodeOwner/CursorOwner/EntryOwner/MetadataPerm/...` | M6 叶证据：`PPtr::new`/`ArrayPtr::new` E0599（std 门控），`borrow_paddr` E0624（private） |
| T4 未知 | unknown：`VmIoOwner`/`MemView`/`TlbModel` 等未列入 M6 表 | 需单独 spike；本报告不假定可构造 |

在 676 个函数中，T3 owner-family 有 125 个 ghost 参数实例（跨函数）；T2p
`MetaRegionOwners` 90 个实例；T2 `Guards` 49 个实例。数量上 **T3/T2p 是 ghost 侧主成本**。

### 4.4 组合类

| 组合 | 主要配方 | 可升 A 的关键 | 典型 |
|---|---|---|---|
| G+R | G1 + R1/R2 | 引用字段可标量化 + C/M 可实例化 | `page_table::is_valid_range`、`dma::*@.inv()` |
| G+T | G1/G2 + T2/T3 | tracked 资源可构造且 consumes 的 inv 可满足 | `frame::unique::tracked_borrow_repr_perm`（tag） |
| R+T | R1 + T1/T3 | id 等值/字段投影 | `meta::addr_of`（T3 阻断，仅 shape） |
| G+R+T | G1 + R1 + T2/T3 | 只有少数能同时跨过三轴 | `mutex::new`（A-shape）、`alloc_if_none`（D） |

## 5. 逐类可行性与 after 分布

### 5.1 A（可得到真判定）— 63 个（9.3%）

| 配方组 | 数量 | 预期 | 说明 |
|---|---:|---|---|
| A-projection-typeconst | 20 | Live | C/Self 实例化后只剩 level/va/页大小标量；含 8 个 `page_size_lemmas`、8 个 `page_table::lemma_*` |
| A-inv-model | 9 | Live | `CpuSet::add/add_all/...` 的 `self.inv()` 用具体 bitset 模型 |
| A-projection-scalar | 8 | Live | `range_alloc`、`tracked_restore`、`do_inc_index`、`dfs_get_idx_range` 等 |
| A-type-level | 7 | **可能 Vacuous** | `VmIo` 的 7 个 fallback 默认方法：crate 内所有 impl `obeys_*=true` → `!obeys` 为假 |
| A-projection-tag | 5 | Live | `entry_owners` 的 `kind is Node/Frame`、`repr_perm is Some` |
| A-projection-id | 3 | Live | `mutex::new/release_lock/unlock` 的 `perm.id()==cell_id` |
| A-construction-model | 2 | **Vacuous（模型相对）** | `vm_space_cursor_embedded/_mut`（V-002，M6 已证） |
| A-mechanical | 2 | Live | `0 <= n`、`0 <= n1 <= n2` |
| A-shape-property | 2 | Live | `rcu::update/compare_exchange` 的 `is_nullable()/Option is Some` |
| A-type-axiom | 2 | Live（或真阳性） | `range_difference` / `lemma_range_difference_set` 的类型公理 + forall |
| A-shape-id | 1 | Live（shape） | `meta::addr_of` `self == perm.value()` |
| A-shape-array | 1 | Live（模型） | `path_slot_as_mut` 的 `old(path)[idx] is Some` |
| A-shape-construction | 1 | Live/Vacuous 双证据 | `MutexGuard::new`（V-001 分歧） |
| **合计** | **63** | | 其中 confidence: high 32 / medium 30 / low 1；cost 1 的 48 个，cost 2 的 15 个 |

### 5.2 B（可建模但预期 Inconclusive）— 162 个（24.0%）

这些函数至少有一条可机械转写的标量子句，但仍含 `inv/wf/relate_owner/metaregion_sound/
spec_index/dom/ghost-tree` 等不透明合取项。按 M1 E3 根因，双探针会双败，预期
`CandidateInconclusive`。它们能**从 C 移出**（获得“已筛查但模型不完备”的负面证据），
但不产生真判定。B 中 116 个有 ≥4 条子句，主要是 page-table owner/leader 家族。

### 5.3 C-deferred（建议暂缓）— 451 个（66.7%）

全部子句都是资源/不变式/深结构谓词，没有机械子句或标量投影：

- 312/451 个函数**完全没有引用任何标量参数**；
- 182 个含 `&mut` 引用（其中 52 个同时含 ghost）；
- 128 个含 ghost；121 个 ghost 类型属于 owner/metadata/PointsTo 家族；
- 248 个 proof、203 个 exec；19 个 trait_decl；18 个有 ≥8 条子句。

> 技术上，451 个都能用“id/uninterp”机械降级成 B 探针；但那是**零信息 B**（所有探针双败），
> 且增加 manifest/运行成本。因此这里把它们标为 C-deferred 而不是“不可建模”。

### 5.4 after 分布（两种口径）

| 口径 | A | B | C | 说明 |
|---|---:|---:|---:|---|
| 技术可达（全部用 id/uninterp） | 63 | 613 | 0 | 没有任何函数在形式上不可生成探针；B 中 451 个为零信息 |
| **推荐近期** | **63** | **162** | **451** | 建议只做有标量内容的 225 个；451 个暂缓 |

ASCII：

```
技术可达   A ███ 63 | B ███████████████████████████████████████ 613 | C 0
推荐近期   A ███ 63 | B ██████████ 162 | C ███████████████████████████ 451
```

## 6. Top 20（按收益/成本排序）

完整机器可读表见 `priority.csv`；`benefit`：3=可能真阳性/解决 registry 争议，2=registry/M1
证据升级，1=非真空洞确认；`cost`：1=单子句/现成类型，2=需类型实例化或标量/结构模型，
3=需深资源/构造链（本 Top 20 无 cost 3）。

| # | 函数 | 阻塞 | 形态 | 配方组 | 终层 | 成本 | 收益 | registry | 理由 |
|---:|---|---|---|---|---|---:|---:|---|---|
| 1 | `ostd::mm::io::VmIo::read_bytes` | G | call | A-type-level | A | 1 | 3 | io 家族 | crate 内 3 个 impl 均 `obeys_vmio_read_requires()=true`，`!obeys` 为假 → 可能 crate-scope 真空洞 |
| 2 | `VmIo::write_bytes` | G | call | A-type-level | A | 1 | 3 | io 家族 | 同上；同簇 7 个 fallback 方法（read_val/read_slice/write_val/write_slice/write_vals） |
| 3 | `ostd::sync::mutex::impl&%7::new` | G+R+T | call | A-shape-construction | A | 1 | 3 | V-001 | shape Live vs construction Vacuous，正是 V-001 争议 |
| 4 | `specs::mm::embedding::cursor::vm_space_cursor_embedded` | R | call | A-construction-model | A | 2 | 3 | V-002 | M6 已机器 ConfirmedVacuous；复现并统一配方 |
| 5 | `...vm_space_cursor_mut_embedded` | R | call | A-construction-model | A | 2 | 3 | V-002 | 同上 |
| 6 | `ostd::util::ops::range_difference` | G | quantifier | A-type-axiom | A | 2 | 3 | F-025 | 实例化 T=usize 后求值 `obeys_cmp`/`finite_range_matches_ord`/`cloned`；若失败即真阳性 |
| 7 | `ostd::util::ops::lemma_range_difference_set` | G | call | A-type-axiom | A | 2 | 2 | F-025 | 同 F-025 的前置伴生引理 |
| 8 | `ostd::util::range_alloc::impl&%4::alloc_specific` | R | call | A-projection-scalar | A | 1 | 2 | F-009 | `self@.start/end` 与 `allocate_range.start/end` 标量投影 |
| 9 | `ostd::util::range_alloc::impl&%4::free` | R+T | call | A-projection-scalar | A | 1 | 2 | F-009 | range 投影 + `allocated.id()==self.id()` |
| 10 | `ostd::mm::page_table::cursor::path_slot_as_mut` | G+R | call | A-shape-array | A | 2 | 2 | M1 row04 | `idx<NR_LEVELS` + `old(path)[idx] is Some`；Seq<Option<id>> 模型可把 Inconclusive 升 Live |
| 11 | `specs::...::owners::impl&%0::tracked_restore` | G+R | call | A-projection-scalar | A | 1 | 2 | M1 row17 | `old(self).idx < old(self).children.len()` |
| 12 | `ostd::sync::mutex::impl&%3::release_lock` | G+R+T | call | A-projection-id | A | 1 | 2 | F-002 邻接 | `perm.id()==self.cell_id()` |
| 13 | `ostd::sync::mutex::impl&%3::unlock` | G+R+T | call | A-projection-id | A | 1 | 2 | F-002 邻接 | 同上 |
| 14 | `ostd::cpu::set::impl&%6::add` | R | call | A-inv-model | A | 2 | 2 | F-026 邻接 | `self.inv()` 的 bitset/CPU 数约束可具体展开 |
| 15 | `ostd::cpu::set::impl&%6::contains` | R | call | A-inv-model | A | 2 | 2 | F-026 邻接 | 同簇 9 个方法共享 inv 模型 |
| 16 | `ostd::sync::rcu::impl&%6::update` | G+R | call | A-shape-property | A | 1 | 2 | RCU 家族 | `is_nullable() || new_ptr is Some` 可投影 |
| 17 | `ostd::mm::page_table::pte_index` | G | call | A-projection-typeconst | A | 1 | 1 | — | `1<=level<=C::NR_LEVELS()`，C=UserPtConfig |
| 18 | `ostd::mm::page_size` | G | call | A-projection-typeconst | A | 1 | 1 | — | 8 个 `page_size_lemmas` 的代表 |
| 19 | `specs::...::owners::impl&%2::lemma_view_mappings_contains` | G | field | A-projection-scalar | A | 1 | 1 | — | `1<=self.level<=NR_LEVELS` |
| 20 | `specs::...::entry_owners::impl&%1::tracked_borrow_node` | G+R | field | A-projection-tag | A | 1 | 1 | — | 4 个 enum-tag 函数的代表 |

## 7. 与 registry / M1 对齐

### 7.1 M1 的 18 行 requires 接口

`REQUIRES-RUN.md:230` 指出 18 行 M1 接口全部在 Tier C。按 census 规范化后是 **11 个 canonical
路径 + 7 个 `_VERUS_VERIFIED` 孪生路径缺失**（wrapper 未进入 census 规范化记录）：

| canonical 函数 | CSV 行 | 本轮配方 | 预计 | M1 quantifier 结果 |
|---|---|---|---|---|
| `mutex::impl&%7::new` | 26/27 | A-shape-construction | A（shape Live / construct Vacuous） | ConfirmedLive（V-001） |
| `cursor::path_slot_as_mut` | 4 | A-shape-array | A | CandidateInconclusive |
| `specs::...::vm_space_cursor_embedded` | 15 | A-construction-model | A（Vacuous） | CandidateInconclusive |
| `...vm_space_cursor_mut_embedded` | 16 | A-construction-model | A（Vacuous） | CandidateInconclusive |
| `cursor::owners::impl&%0::tracked_restore` | 17 | A-projection-scalar | A | CandidateInconclusive |
| `locking::try_traverse_and_lock_subtree_root` | 2/3 | B（continuation map） | B | CandidateInconclusive |
| `entry::impl&%2::alloc_if_none` | 5/7 | D（owner 家族） | C-deferred | CandidateInconclusive |
| `entry::impl&%2::split_if_mapped_huge` | 6/8 | D | C-deferred | CandidateInconclusive |
| `entry::impl&%3::alloc_absent_child` | 9/10 | B（含 1 条机械子句） | B | CandidateInconclusive |
| `node::impl&%2::lock` | 11/13 | D | C-deferred | CandidateInconclusive |
| `node::impl&%2::make_guard_unchecked` | 12/14 | D | C-deferred | CandidateInconclusive |

即 5/11 canonical（6/18 CSV 行）可升 A，6/11（12/18 行）仍为 B 或 C-deferred
（2 个 B：try_traverse、alloc_absent_child；4 个 D：alloc_if_none、split_if_mapped_huge、
lock、make_guard_unchecked）。M1 的 7 项 `entry_precondition_vacuous=true` stub 中，
`MutexGuard::new` 已有机器 Vacuous（V-001）；其余 6 项（try_traverse、alloc 三联、
lock/make_guard）仍是 B/C-deferred。M6 的 10 个 construction 条目中，vm_space 两 axiom 获得
机器 Vacuous（V-002），其余 8 个是铸造通道阻断的叶证据；tracked_restore 的 requires 本身
可 A，但 Fab 面仍受 T3 阻断。

### 7.2 registry 条目

| registry | 与 Tier C 的关系 | 本轮优先集 |
|---|---|---|
| V-001 `MutexGuard::new` | 直接命中 `mutex::impl&%7::new` | **Top 3**；复现 shape/construct 分歧 |
| V-002 7 项 guard 入口 | 直接命中 vm_space 两 axiom；其余相关 canonical 为 B/D（见 7.1） | **Top 4–5**；vm_space 已机器 Vacuous |
| F-009 range allocator | 直接命中 `range_alloc::alloc_specific/free` | **Top 8–9**；A 投影 |
| F-025 range_difference 迭代器序列 | 直接命中 `util::ops::range_difference` / `lemma_range_difference_set` | **Top 6–7**；类型公理可能暴露真阳性 |
| F-026 AtomicCpuSet | **无直接路径**：`AtomicCpuSet` 不在 Tier C；`CpuSet` 9 个方法是同族邻接 | Top 14–15；仅主题邻接，不要宣称复现 F-026 |
| F-002 Mutex lock/try_lock | `lock/try_lock` 是 `no_requires`，不在 Tier C；`unlock/release_lock` 是身份 requires 邻接 | Top 12–13；不是 F-002 的复现 |
| F-018/F-019 VmReader/VmWriter | 无直接路径；Tier C 的 `mm::io` 是 `VmIo` trait/default 方法，不是 `VmReader/VmWriter` 接口 | VmIo fallback 归 io 家族，勿混同 |
| F-024 PageTablePageMeta::new | 无 Tier C 路径（无 requires） | 不建模 |
| F-017 RcuOption / F-020 Either / F-021 WaitQueue / F-022 AsVirtPtr / F-023 Jiffies | 均无直接 Tier C 路径 | 不建模 |
| F-001/F-003..F-008/F-010..F-016 | 主要落在 ensures/obligation 面；Tier C requires 无直接路径 | 不建模 |

**最可能产出新证据/真阳性的优先集**：
1. `VmIo` 7 个 fallback 方法（crate-scope 真空洞候选，未入 registry）；
2. V-001（MutexGuard::new，已有争议）；
3. V-002（vm_space 两 axiom，已有机器 Vacuous，复现价值）；
4. F-025（range_difference 类型公理；若实例化后公理不成立即真阳性）；
5. F-009（range_alloc 两个入口，廉价 A 证据）。

## 8. 明确“不值得建模”的类别

以下类别的共同点：**A 不可达；B 可达但预期全为 CandidateInconclusive（零信息），且成本高 /
模型风险大**。建议近期不做；不是“永久不可能”。

| 类别 | 规模（约） | 证据/示例 | 建议 |
|---|---:|---|---|
| `&mut` 状态机 / alias / 生命周期 | 182 个 D 含 `&mut`；全库 191 个 mut+inv-only | `cursor/mod.rs:1773 push_level`（13 条多行 requires，含 `Tracked(regions)`/owner）、`cursor/mod.rs:1677 pop_level`、`linked_list.rs:366 push_back`（`wf_region/relate_region/*old(regions)`）；`frame/mod.rs:273` 含 `&'a MetadataPerm` | 仅 B；不做 |
| owner/metadata tracked 资源、构造链阻断 | 121 个 D 的 ghost 属 owner 家族 | M6 表：`PPtr::new`/`ArrayPtr::new` E0599（std 门控），`borrow_paddr` E0624（private）；`split_if_mapped_huge`（entry.rs:968，12 子句）、`alloc_if_none`（616，8 子句）、`node::impl&%2::lock/make_guard_unchecked`（node/mod.rs:375/401） | 仅 B；不做 construction 探针 |
| 深结构 ghost tree/map/quantifier | 18 个 D 有 ≥8 子句；全库 84 个 quantifier | `locking.rs:100 lock_range`（`relates_owner`/`relate_guard`/`forall i<NR_ENTRIES`）、`continuations[..].all_some()`（try_traverse） | 仅 B；不做 |
| 关联类型投影 / 未选 impl 的 trait decl | 19 个 D 是 trait_decl；31 个 Tier C trait_decl 中多数 | `frame_ref::NonNullPtr::from_raw`（`Self::ptr_perm_match` + `perm@.inv()`）、`page_table/impl&%9/create_user_page_table` 的类型投影 | 先选 impl；否则 unknown |
| 纯类型级 proof lemma、无标量投影 | G-only D 128 个 | 例如 `specs::mm::page_table::node::owners` 的 owner 引理、`specs::mm::embedding::list_store` 的 `map spec_index/contains_key` | 仅 B；不做 |
| 原始指针地址/位运算 | 少数（如 `either::remove_bits`，`addr & bits`） | `sync/rcu/non_null/either.rs:488` | A-near-miss：需 bitwise 生成器扩展；否则 B |

### 8.1 “仅生命周期/&mut 状态更新”的不可映射说明

探针的 `requires` 是**入口前置**，只能描述参数进入函数时的状态。对于：

- `&mut self` / `&mut owner` 的 `old(self).field` 与 requires 中的 mut-ref 不变式：id 抽象后
  只能得到 `uninterp(id)`，丢失 alias/借用分裂/生命周期约束；
- 含 `Tracked<&'a ...>`（如 `frame/mod.rs:273`）：`'a` 是 Rust 生命周期，探针 manifest 没有
  生命周期参数，只能把引用抽象成 id；
- 深结构 old(self)（`*old(regions) == store.regions`、`wf_region(*old(owner), *old(regions))`）：
  id 等值不足以表达结构相等，转 bool 后只能 B。

这些不应被写成“已建模/已筛查”。它们最多是 B（Inconclusive），不能产生 V-001/V-002 级别的
真判定。

## 9. 风险

1. **错误映射风险（最高）**：id 等值（`perm.id()==cell_id`、`self==perm.value()`）是 shape，
   不保留“权限不可伪造/分布可达性”。V-001 已展示 shape Live 与 construction Vacuous 可以
   同时成立；任何 A-shape 结果必须标 `model-relative`。
2. **类型实例选择**：`C` 选 `UserPtConfig` vs `KernelPtConfig` 会改变 `NR_LEVELS/BASE_PAGE_SIZE`；
   `M` 选 `MetaSlotStorage` vs 其它 metadata 会改变 `ReprPerm/Owner` 关联类型。A 结果只对所选
   实例成立。`recipes.json` 的 A 记录均标注实例建议。
3. **VmIo 类型级真空洞是 crate-scope**：crate 内 3 个 impl 均为 `true`，但 trait 是 `pub`，
   外部 crate 可实现 `obeys=false`。因此不能写成“trait 全局恒假”，只能写“本 crate 无可调用
   实例/默认方法 dead”。
4. **见证域限制**：Tier A 的有限 witness 域（0..8/15/16/31/32/63/64/255/256/4096）可能错过
   大地址/大常量；本报告 A 的 witness 建议按具体常量（MAX_PADDR、NR_LEVELS）构造，不依赖
   小域搜索。
5. **常量提升**：M7e 把 `ConstVar` 提升为自由参数；对真空洞是保守近似（可能漏报，不会误报）。
   本报告的 A 投影若把 `PAGE_SIZE` 等常量固定，需确认与 VIR 常量一致。
6. **缺失文本**：47 个函数至少一条 requires 无源码文本（113 条子句）；这些行的 A/B 判断标
   `unknown`，A 项已回读源码，其余按 span/结构推断。
7. **不计缺陷**：B 的 `CandidateInconclusive` 不是缺陷；C-deferred 不是“无缺陷”。本报告
   不新增 registry 条目，只给建模优先级。

## 10. 产出

- `/tmp/tierc-analysis/REPORT.md`：本文件。
- `/tmp/tierc-analysis/recipes.json`：676 条逐函数配方（blockers、requires 形态、G/R/T 子配方、
  预期终层、cost/benefit、risk、registry、源码行号），以及 15 个配方组的汇总。
- `/tmp/tierc-analysis/priority.csv`：Top 20 排序清单（函数、阻塞、形态、配方、终层、成本/收益、
  registry、证据）。
- 中间产物（只读复算用）：`tierc_rows.json`、`tierc_classified.json`、`A_paths.json`、
  `analyze.py`、`classify.py`、`build_outputs.py`。

## 11. 结论（一句话）

Tier C 的 676 个函数里，**63 个（9.3%）有明确配方可升 A**，**162 个（24%）可降为 B
（预期 Inconclusive）**，**451 个（66.7%）是纯资源/不变式/&mut 状态，建议暂缓**；
最值得先做的是 `VmIo` fallback 的 crate-scope 真空洞候选、V-001/V-002 复现、
F-025 类型公理和 F-009 range allocator 的标量投影；`&mut` 状态机、owner/metadata 构造链
（M6 已证阻断）和深 ghost tree 量化是明确的“不值得建模”类别。
