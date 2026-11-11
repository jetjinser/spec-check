# CONSTRUCTION 预注册（运行前写定，运行后不改写；2026-11-11）

对象：60 接口 quantifier 全量运行中 10 个 CandidateInconclusive 条目（6 个恒假 stub 锚点 + row04/15/16/17）。生成器：`extract_construction_manifest.py`。预测只基于冻结源静态判读与 M2/wt-b 已归档判据；运行判定以 manifest 实测为准。

## 探针架构与铸造策略表（静态判读，先于运行）

| 契约 | 面类型 | 伪造通道 | 阻断叶（如有） |
|------|--------|----------|----------------|
| node_lock | direct | blocked | NodeOwner mint: array_ptr::PointsTo (ArrayPtr::new is #[cfg(std)]) + simple_pptr::PointsTo (PPtr::new/empty are #[cfg(std)]; fields private) have no no_std constructor; FracMetadataPerm transitively blocked |
| make_guard_unchecked | direct | blocked | same as node_lock (identical tracked-arg set) |
| try_traverse_and_lock_subtree_root | stub | blocked | CursorOwner/ CursorContinuation mint: continuation guard field is a PageTableGuard value (blocked tracked fields; mm-internal borrow_paddr invisible at root injection); regions/ guards otherwise literal-mintable |
| alloc_if_none | stub | blocked | parent_owner: NodeOwner mint blocked (array_ptr + simple_pptr leaves); owner subtree / regions / guards otherwise mintable |
| split_if_mapped_huge | stub | blocked | same as alloc_if_none (NodeOwner mint blocked) |
| alloc_absent_child | stub | blocked | same as alloc_if_none (NodeOwner mint blocked) |
| path_slot_as_mut | stub | blocked | satisfying path value mint: PageTableGuard value blocked (FrameRef tracked fields; borrow_paddr is pub(in crate::mm) — visibility wall at root injection) |
| vm_space_cursor_embedded | direct | mint | — |
| vm_space_cursor_mut_embedded | direct | mint | — |
| tracked_restore | direct | blocked | CursorContinuation mint blocked (guard field is a PageTableGuard value) |

## 预注册预测（verifying run，construction-manifest.json）

| entry | 预测 | 理由 |
|-------|------|------|
| node_lock_holder_face | ConfirmedLive | holder 面：face requires 即契约 requires，调用点义务由面参数平凡 discharge（direct 条目直接调用真实契约面，stub 条目调用逐字转写面）|
| make_guard_unchecked_holder_face | ConfirmedLive | holder 面：face requires 即契约 requires，调用点义务由面参数平凡 discharge（direct 条目直接调用真实契约面，stub 条目调用逐字转写面）|
| try_traverse_and_lock_subtree_root_holder_face | ConfirmedLive | holder 面：face requires 即契约 requires，调用点义务由面参数平凡 discharge（direct 条目直接调用真实契约面，stub 条目调用逐字转写面）|
| alloc_if_none_holder_face | ConfirmedLive | holder 面：face requires 即契约 requires，调用点义务由面参数平凡 discharge（direct 条目直接调用真实契约面，stub 条目调用逐字转写面）|
| split_if_mapped_huge_holder_face | ConfirmedLive | holder 面：face requires 即契约 requires，调用点义务由面参数平凡 discharge（direct 条目直接调用真实契约面，stub 条目调用逐字转写面）|
| alloc_absent_child_holder_face | ConfirmedLive | holder 面：face requires 即契约 requires，调用点义务由面参数平凡 discharge（direct 条目直接调用真实契约面，stub 条目调用逐字转写面）|
| path_slot_as_mut_holder_face | ConfirmedLive | holder 面：face requires 即契约 requires，调用点义务由面参数平凡 discharge（direct 条目直接调用真实契约面，stub 条目调用逐字转写面）|
| vm_space_cursor_embedded_construction | ConfirmedVacuous | fab 面：新铸 VmSpaceOwner（tracked_new_val 空 OwnerSubtree）+ 空映射 MetaRegionOwners 调真实 axiom；`old(regions).inv()` 的 meta-slot 覆盖域子句对空映射不可满足，预期 verifier 在 axiom requires 处拒绝，span 归因于 entry 调用点 |
| vm_space_cursor_embedded_holder_face | ConfirmedLive | holder 面：face requires 即契约 requires，调用点义务由面参数平凡 discharge（direct 条目直接调用真实契约面，stub 条目调用逐字转写面）|
| vm_space_cursor_mut_embedded_construction | ConfirmedVacuous | fab 面：新铸 VmSpaceOwner（tracked_new_val 空 OwnerSubtree）+ 空映射 MetaRegionOwners 调真实 axiom；`old(regions).inv()` 的 meta-slot 覆盖域子句对空映射不可满足，预期 verifier 在 axiom requires 处拒绝，span 归因于 entry 调用点 |
| vm_space_cursor_mut_embedded_holder_face | ConfirmedLive | holder 面：face requires 即契约 requires，调用点义务由面参数平凡 discharge（direct 条目直接调用真实契约面，stub 条目调用逐字转写面）|
| tracked_restore_holder_face | ConfirmedLive | holder 面：face requires 即契约 requires，调用点义务由面参数平凡 discharge（direct 条目直接调用真实契约面，stub 条目调用逐字转写面）|

## 预注册预测（leaf evidence run，construction-fab-blocked.json）

8 个 fab 面（每契约一个注入模块）预期在预编译回路（`cargo-verus check --no-verify`）触发 rustc 命名解析失败：

- `node_lock_construction`：铸造调用处报 E0599/E0603 族错误（NodeOwner mint: array_ptr::PointsTo (ArrayPtr::new is #[cfg(std)]) + simple_pptr::PointsTo (PPtr::new/empty are #[cfg(std)]; fields private) have no no_std constructor; FracMetadataPerm transitively blocked）
- `make_guard_unchecked_construction`：铸造调用处报 E0599/E0603 族错误（same as node_lock (identical tracked-arg set)）
- `try_traverse_and_lock_subtree_root_construction`：铸造调用处报 E0599/E0603 族错误（CursorOwner/ CursorContinuation mint: continuation guard field is a PageTableGuard value (blocked tracked fields; mm-internal borrow_paddr invisible at root injection); regions/ guards otherwise literal-mintable）
- `alloc_if_none_construction`：铸造调用处报 E0599/E0603 族错误（parent_owner: NodeOwner mint blocked (array_ptr + simple_pptr leaves); owner subtree / regions / guards otherwise mintable）
- `split_if_mapped_huge_construction`：铸造调用处报 E0599/E0603 族错误（same as alloc_if_none (NodeOwner mint blocked)）
- `alloc_absent_child_construction`：铸造调用处报 E0599/E0603 族错误（same as alloc_if_none (NodeOwner mint blocked)）
- `path_slot_as_mut_construction`：铸造调用处报 E0599/E0603 族错误（satisfying path value mint: PageTableGuard value blocked (FrameRef tracked fields; borrow_paddr is pub(in crate::mm) — visibility wall at root injection)）
- `tracked_restore_construction`：铸造调用处报 E0599/E0603 族错误（CursorContinuation mint blocked (guard field is a PageTableGuard value)）

该 run 不产出判定（构建失败即证据形态本身）；8 条目据此**不**升级为机器 ConfirmedVacuous——其恒假结论保持为文档化通道穷尽论证（铸造通道缺席，编译级锚点 + s1 assume_new 归档），quantifier 判定 CandidateInconclusive 维持"验证器不完备"口径，registry 记证据升级但不虚构机器判定。

## 预测汇总

verifying run 预测：ConfirmedVacuous=2，ConfirmedLive=10（合计 12，含 2 个 mutex 锚点）；leaf run 预测：8 模块命名解析失败（每模块 ≥1 处铸造调用错误）。
