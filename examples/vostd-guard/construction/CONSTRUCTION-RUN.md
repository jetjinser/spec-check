# tracked 契约构造探针批量生成器 — 全量归档（2026-11-11）

对象：60 接口 quantifier 全量运行（FULL-RUN-60.md）中 10 个
CandidateInconclusive 条目。生成器：`extract_construction_manifest.py`
（从冻结源 `/tmp/vostd-43da079-ktest-chan`（rev 43da0797bbfa，ktest 通道
工作副本）的 `#[verus_spec]` 契约逐字机械转写）。预注册：
`CONSTRUCTION-EXPECTED.md`（生成于 fableaf6 / main6 两个归档运行之前，
运行后未改写；重生成 diff 为空）。

## 探针架构（静态判读 → 运行验证）

每契约两个面：

- **holder 面**（`face_requires=true`）：契约 requires 作为 entry fn 前置，
  construct 以面参数调用契约面。契约面可见者**直接调用真实契约**
  （node_lock / make_guard_unchecked 调真实方法；vm_space 两 axiom 调真实
  `pub axiom fn`；tracked_restore 调真实 `pub proof fn`——零转写间隙），
  契约面不可见者（MutexGuard::new 式可见性墙）调逐字转写的
  external_body 契约面 stub（try_traverse / path_slot 私有 fn → 自由 fn
  stub；alloc 三联 `pub(in crate::mm)` → **方法形 stub**：在
  `impl Entry<'_, '_, UserPtConfig>` 上以 `#[verus_spec]` 属性携带逐字
  契约（接收者 `self` 保留），与冻结源契约面同形）。
- **伪造面**（fabrication）：按铸造策略表在 construct 内铸造 tracked
  参数后调用契约面。铸造策略表（no_std target、根级注入点的实测结论，
  见下）显示 10 条目中仅 row15/16（vm_space 两 axiom）具备完整铸造链，
  其余 8 条目的铸造链在特定叶类型结构性阻断 → 叶证据 run（见下）。

### 铸造策略表（spike + 叶运行实证）

| tracked 类型 | no_std 铸造 | 途径 |
|---|---|---|
| vstd `pcell::PointsTo` / `pcell_maybe_uninit::PointsTo` | ✓ | `PCell::new`（const fn；wt-b 先例 + 本次实证） |
| vstd_extra `Count<T,N>`（FracMetadataPerm） | ✓* | `Count::alloc`（proof fn；要求 T 可铸） |
| `Guards` | ✓ | pub 字段结构字面量（`Set::empty()`） |
| `OwnerSubtree` | ✓ | `TreeNode::tracked_new_val` + 可铸 `EntryOwner`（Absent 变体、`TreePath::new(Seq::empty())`）——main6 实证其产物满足 `inv()` |
| `MetaRegionOwners` | 部分 | pub 字段字面量可构造（空映射），但 `inv()` 不可满足（main6 实证：axiom 在 `old(regions).inv()` 处拒绝） |
| vstd `simple_pptr::PointsTo` | ✗ | `PPtr::new/empty` 均 `#[cfg(feature = "std")]`；字段私有 |
| vstd_extra `array_ptr::PointsTo` | ✗ | `ArrayPtr::new` std 门控；字段私有 |
| `FrameRef` / `PageTableGuard` 值 | ✗ | tracked 字段依赖上述；mm 内部构造器 `borrow_paddr` 为 `pub(in crate::mm)`（E0624） |
| `MetadataPerm` / `FracMetadataPerm` / `MetaSlotOwner` / `NodeOwner` / `CursorOwner` / `CursorContinuation` | ✗ | 传递阻断 |
| `Tracked::assume_new` | ✗ | verus 编译级阻断（M2 s1 归档先例，本轮未重复归档） |

## 运行命令（可重放）

```
# 叶证据 run（预期构建失败，失败日志即证据）
timeout 600 nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  'cd ~/vie/work/AES/spec-check && ./target/debug/cargo-spec-check \
     analyses-vacuity \
     --manifest examples/vostd-guard/construction/construction-fab-blocked.json \
     --cargo-verus .../cargo-verus --z3 .../z3 --vstd .../vstd \
     --out /tmp/vac-construction-fableaf6'
# verifying run（归档运行）
timeout 880 nix develop ... 同上 --manifest .../construction-manifest.json \
  --out /tmp/vac-construction-main6'
```

注入机制（runner 自动）：注入 `ostd/src/spec_check_probes/`（每条目一
模块，`#[cfg(ktest)]` 门控）+ lib.rs 一行声明；运行后工作区恢复原字节
（fableaf6 / main6 均已核对：`ostd/src` 无 `spec_check_probes` 残留；
replay 副本在各自 `out/injections/`）。

## spike 阶段（node_lock 单点，成败闸门）

`spike-manifest.json`（node_lock fab + holder，已并入最终 manifest 后
移除）两轮预编译（`/tmp/vac-construction-spike`、`-spike2`）：
第 1 轮抓出导入路径错误（`crate::task::DisabledPreemptGuard` 再导出、
`pcell_maybe_uninit` 内类型名为 `PCell`）；第 2 轮给出决定性发现——
NodeOwner 铸造链在 `PPtr::new` / `ArrayPtr::new` 处 **E0599**（std 门控，
no_std 目标无铸造 API）。spike 结论：**构造判据可迁移性分层**——

1. 铸造链完整 ⇒ 判据按 wt-b 原型迁移（伪造调用被 verifier 拒绝 →
   ConfirmedVacuous）；
2. 铸造链结构性缺失 ⇒ 判据以**通道缺席**形态迁移（编译级锚点 +
   文档化通道穷尽），不产生"拒绝型"机器 ConfirmedVacuous。

由此 row15/16 走完整铸造伪造面，其余 8 条目走叶证据路线（不虚构机器
判定）。

## 叶证据 run（construction-fab-blocked.json → fableaf6）

8 个伪造面模块（每条目一模块）全部在预编译回路
（`cargo-verus check --no-verify`）命名解析失败，**13 个错误全部落在
铸造调用点、无杂音**（`/tmp/vac-construction-fableaf6/precheck-build.log`）：

| 模块（条目） | 错误 | 归因行 |
|---|---|---|
| node_lock_construction | E0599 `PPtr::new` ×1 + E0599 `ArrayPtr::new` ×1 | :50, :51 |
| make_guard_unchecked_construction | 同上 ×2 | :50, :51 |
| alloc_if_none_construction | 同上 ×2 | :87, :88 |
| split_if_mapped_huge_construction | 同上 ×2 | :103, :104 |
| alloc_absent_child_construction | 同上 ×2 | :123, :124 |
| try_traverse_and_lock_subtree_root_construction | E0624 `borrow_paddr` private ×1 | :75 |
| path_slot_as_mut_construction | E0624 ×1 | :68 |
| tracked_restore_construction | E0624 ×1 | :49 |

即：NodeOwner 铸造通道 = 无 no_std 构造器（E0599）；PageTableGuard 值
铸造通道 = mm 内部构造器 + tracked 字段双重阻断（E0624）。该 run 不产
出判定（构建失败即证据形态本身），预注册预期"8 模块命名解析失败、
每模块 ≥1 处铸造调用错误"**8/8 一致**。

## verifying run（construction-manifest.json → main6）

目标 workspace：**1508 verified / 3 errors**（ktest 基线 1497 + 14 个
探针 fn；3 errors = 3 个伪造面的预期拒绝）。判定分布：
**ConfirmedVacuous 3 / ConfirmedLive 11**（14 条目，含 2 个 mutex 锚点）。

| entry | 预注册 | 实际 | 一致? | 契约 |
|---|---|---|---|---|
| node_lock_holder_face | ConfirmedLive | **ConfirmedLive** | ✓ | 真实 `PageTableNodeRef::lock` |
| make_guard_unchecked_holder_face | ConfirmedLive | **ConfirmedLive** | ✓ | 真实 `make_guard_unchecked` |
| try_traverse_..._holder_face | ConfirmedLive | **ConfirmedLive** | ✓ | stub（私有 fn） |
| alloc_if_none_holder_face | ConfirmedLive | **ConfirmedLive** | ✓ | 方法形 stub |
| split_if_mapped_huge_holder_face | ConfirmedLive | **ConfirmedLive** | ✓ | 方法形 stub |
| alloc_absent_child_holder_face | ConfirmedLive | **ConfirmedLive** | ✓ | 方法形 stub |
| path_slot_as_mut_holder_face | ConfirmedLive | **ConfirmedLive** | ✓ | stub（私有 fn） |
| vm_space_cursor_embedded_construction | ConfirmedVacuous | **ConfirmedVacuous** | ✓ | 真实 axiom |
| vm_space_cursor_embedded_holder_face | ConfirmedLive | **ConfirmedLive** | ✓ | 真实 axiom |
| vm_space_cursor_mut_embedded_construction | ConfirmedVacuous | **ConfirmedVacuous** | ✓ | 真实 axiom |
| vm_space_cursor_mut_embedded_holder_face | ConfirmedLive | **ConfirmedLive** | ✓ | 真实 axiom |
| tracked_restore_holder_face | ConfirmedLive | **ConfirmedLive** | ✓ | 真实 `proof fn` |
| mutex_guard_new_construction（锚点） | ConfirmedVacuous | **ConfirmedVacuous** | ✓ | wt-b stub 原样复用 |
| mutex_guard_new_holder_face（锚点） | ConfirmedLive | **ConfirmedLive** | ✓ | 同上 |

伪造拒绝的 span 归因（target-build.log 逐字核对）：
- `vm_space_cursor_embedded_construction.rs:70` 调用点 ← **真实 axiom
  requires `old(regions).inv()`**（specs/mm/embedding/cursor.rs:97）；
  mut 版同（:138）。 fabricated VmSpaceOwner 通过了 `vm_space.inv()`
  （空 `OwnerSubtree` 满足不变式、`active=false` 短路其余子句）——失败
  精确落在空映射 `MetaRegionOwners` 的 meta-slot 覆盖域子句上，与
  预注册"预期 verifier 在 axiom requires 处拒绝"一致且更精确。
- `mutex_guard_new_construction.rs:36` ← stub requires
  `perm.id() == m.cell_id()`（wt-b 归档判定 3/3 复现）。

**14/14 预注册一致**；预注册分布（ConfirmedVacuous=3 / ConfirmedLive=11）
与实测一致。

## 与 quantifier shape 判定的对照（10 条目逐条）

| 条目（CSV 行） | quantifier 判定 | 本轮构造判定 | 升级情况 |
|---|---|---|---|
| try_traverse_and_lock_subtree_root (2,3) | CandidateInconclusive | 叶证据：铸造通道缺席（E0624）+ holder 面机器 ConfirmedLive | **部分升级**：机器证据从"双探针皆败"升级为"holder 面可调用性确认 + 伪造通道缺席编译锚点"；恒假结论仍为文档化通道穷尽论证，**不虚构机器 ConfirmedVacuous** |
| alloc_if_none (5,7) | CandidateInconclusive | 同上（E0599×2） | 同上 |
| split_if_mapped_huge (6,8) | CandidateInconclusive | 同上（E0599×2） | 同上 |
| alloc_absent_child (9,10) | CandidateInconclusive | 同上（E0599×2） | 同上 |
| node_lock (11,13) | CandidateInconclusive | 同上（E0599×2） | 同上 |
| make_guard_unchecked (12,14) | CandidateInconclusive | 同上（E0599×2） | 同上 |
| path_slot_as_mut (4) | CandidateInconclusive | 同上（E0624；满足点需 PageTableGuard 值，铸造被墙） | 同上 |
| vm_space_cursor_embedded (15) | CandidateInconclusive | **机器 ConfirmedVacuous**（真实 axiom 伪造拒绝，归因 `old(regions).inv()`） | **升级为机器判定**（模型相对：调用面 = 已铸 VmSpaceOwner + 可构造 regions） |
| vm_space_cursor_mut_embedded (16) | CandidateInconclusive | **机器 ConfirmedVacuous**（同上） | 同上 |
| tracked_restore (17) | CandidateInconclusive | 叶证据：E0624（child 铸造被墙）+ holder 面 ConfirmedLive | 部分升级（同 8 条目口径） |

未升级为机器 ConfirmedVacuous 的 8 条目：quantifier 层的不完备性
（uninterp 谓词双探针皆败）由"铸造通道缺席"的编译级证据补强，但其
恒假性证明的最后一步（"不存在任何可写铸造通道"的穷尽量化）仍是
文档化论证——机器锚点为：每个叶类型的具体阻断错误（本 run）+
`assume_new` 编译级阻断（M2 s1 归档）+ holder 面调用可满足性确认
（本 run）。registry 按此口径记证据升级，不虚构机器判定。

## 证据边界（重要）

1. **模型相对性**：调用面模型 = 注入点可见的 pub 面 + 上表铸造通道。
   row15/16 的 ConfirmedVacuous 限定于"外部调用者（注入点可见面）+
   no_std 铸造通道"模型；若项目真实 API 在其它配置/目标（std）下输出
   匹配 tracked 权限，判定应翻转。6 个 stub 锚点契约面为转写面
   （alloc 三联为方法形转写、接收者保留；try_traverse/path_slot 为
   自由 fn 转写），同 wt-b 口径。
2. **通道穷尽是文档化论证**：机器证据是"每个具体铸造调用点的阻断/
   拒绝"（逐行 span），不是"无通道"的全称定理；`assume_new` 信任边界
   按 M2 纪律监控（生成源与 stub 文本无 `assume_specification`，
   vacuity.rs 信任边界检查在发射前执行）。
3. **holder 面 ConfirmedLive 的含义**：契约 requires 在调用面可满足
   时调用通过（面参数承担义务），不是对 ensures 内容的验证。
4. **转写偏差记录（机械、语义保持）**：① `self` → 自由 fn 形参更名
   （holder 面参数名 `entry`/`node`/`cont`）；② 泛型实例化
   `C`→`UserPtConfig`、`A`→`DisabledPreemptGuard`（pub 具体类型）；
   ③ `Self::` → 具体类型路径；④ alloc_absent 的
   `relate_guard(*old(self))` 在 stub 中写作 `relate_guard(*old(self).node)`
   ——裸形式仅被 verified 契约的 verus 检查器接受（外部体契约文本
   rustc 可见，E0308 实证），`.node` 形式与 split_if_mapped_huge 自身
   对同一谓词的子句写法一致；⑤ try_traverse ensures 的 `C::Item` 写作
   `<UserPtConfig as PageTableConfig>::Item`；⑥ 属性绑定 `r` 统一更名
   `res` 并具名返回。逐字文本见 `out/injections/` 与 manifest。

## manifest 修复迭代记录（预算外如实记录）

叶证据路线 5 轮（fableaf1–5：导入路径 2 轮；Inv/OwnerOf 导入 +
`Entry<'_,'_,_>` 双生存期 + `relate_guard` `.node` 偏差各 1 轮）、
verifying 路线 5 轮（main1–5：axiom 导入、stub 调用绑定、`&vs_fab`/
`&mut regions_fab`、tracked 结构字面量的 spec 模式字段需
`Seq::tracked_empty()`/`Map::tracked_empty()`）。工具代码零改动
（manifest v1.1 全部机制原样承载）；全部修复在生成器/manifest 侧。

## 运行产物

- 叶证据 run：`/tmp/vac-construction-fableaf6/`
  （precheck-build.log = 13 错误证据；injections/ = 8 模块 replay）
- verifying run：`/tmp/vac-construction-main6/`
  （report.json = 14 判定；target-build.log = 1508/3 与 span 归因；
  injections/ = 14 模块 replay + lib.rs.injected/orig）
- 中间迭代目录：`/tmp/vac-construction-spike{,2}`（spike）、
  `/tmp/vac-construction-fableaf{1..5}`、`/tmp/vac-construction-main{1..5}`
  （修复迭代，保留供审计）
