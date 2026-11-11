# EXTRACTION.md — 7 项恒假契约的形状再现（原契约 → 转写对照）

对象：`guard-return-inventory.csv` 中 `entry_precondition_vacuous=true` 的 7 行
（全部 `external_body` stub）。契约原文摘自冻结源
`/tmp/vostd-43da079-ktest-chan/ostd/src`（rev 43da0797bbfa）各 span 处的
`#[verus_spec(...)]`。

**边界声明：本 manifest 全部 7 项契约条目均为 shape reproduction（形状再现），
不是原文契约。** tracked 参数（`Tracked<&mut CursorOwner>` / `Tracked<&mut
NodeOwner>` / `Tracked<&mut OwnerSubtree>` / `Tracked<&mut MetaRegionOwners>` /
`Tracked<&mut Guards>` / `Tracked<PointsTo<T>>` / `Tracked<&NodeOwner>`）不被
manifest 支持，统一抽象为不透明 usize id。转写规则：

- **机械转写**：requires 中对标量字段的比较/算术（`level() == NR_LEVELS`、
  `path.len() == level()`、`level + parent_level == INC_LEVELS` 等）逐字保留
  为具名 `open spec fn`。verus spec 算术是数学语义，`level < inc_levels - 1`
  在 `inc_levels == 0` 时良定义（false），与原意一致。
- **uninterp 抽象**：涉及 ghost 结构（集合/映射成员、不变式、所有权关系、
  路径谓词）的子句机械不可表达，声明为具名 `pub uninterp spec fn`（保住
  约束结构的名字与参数形状，丢掉定义体）。占位返回 true 的写法会丢约束，
  不采用；uninterp 声明的抽象损失逐条记录于下表。
- **见证策略**：requires 含可满足的机械标量约束的条目给出显式 witness
  （证明形状层可满足性，检验抽象是否吃掉恒假性）；requires 全为 uninterp
  谓词的条目不给 witness。**【运行后更正 2026-11-11】**本条初版假定
  "uninterp 谓词上的 assert 平凡成功"——首次运行证伪：`assert(f(args))`
  对 uninterp `f` **不可证**（没有任何公理强制 `f(args)` 为真），见证
  assert 失败，于是含 uninterp 合取项的条目双探针皆败 → 按判据落
  CandidateInconclusive（既非"平凡 ConfirmedLive"也不可判定为假）。
  witness 保留在 manifest 中：其标量取值仍作为"机械子句层可满足性"的
  论证记录，但机器判定以探针运行为准。详见 FULL-RUN.md。

对照集（非契约条目）：`mutex_new_any_value_control`、`arith_bounds_live_control`
（阴性，带见证）、`arith_oob_vacuous_control`（阳性算术矛盾）。前两者沿用
形状运行（git ae0f897）已验证的形状，`Mutex::new` 在冻结源中无显式 requires，
值域谓词是建模假设（非原文契约）。

---

## 1. `try_traverse_and_lock_subtree_root` — `ostd/src/mm/page_table/cursor/locking.rs`（attr 294–383，fn 384）

原 requires（tracked：`cursor_own: Tracked<&mut CursorOwner<'rcu, C>>`、
`regions: Tracked<&mut MetaRegionOwners>`、`guards: Tracked<&mut Guards>`）：

```text
old(cursor_own).level == NR_LEVELS,
old(cursor_own).continuations.dom().contains(NR_LEVELS - 1),
old(cursor_own).continuations[NR_LEVELS - 1].all_some(),
```

| 原子句 | 转写 | 类型 |
|---|---|---|
| `old(cursor_own).level == NR_LEVELS` | `cursor_level_eq_nr_levels(cursor_level, nr_levels)` := `cursor_level == nr_levels` | 机械 |
| `.continuations.dom().contains(NR_LEVELS - 1)` | `cont_dom_contains(cursor_own_id, nr_levels)` | uninterp（损失：映射域与成员关系） |
| `.continuations[NR_LEVELS - 1].all_some()` | `cont_all_some(cursor_own_id, nr_levels)` | uninterp（损失：continuation 树形状，`-1` 折入 uninterp 参数） |

`regions`/`guards` 仅出现在 ensures，requires 无引用，不建参数。
witness `(3,3,0)`：满足 `3 == 3`（机械层）。【运行后更正】含 uninterp
合取项，整条 requires 的 assert 不可证，机器活性未确认，判
CandidateInconclusive。

## 2. `alloc_if_none` — `ostd/src/mm/page_table/node/entry.rs`（attr 516–613，fn 616，impl&%2）

原 requires（tracked：`owner: Tracked<&mut OwnerSubtree<C>>`、
`parent_owner: Tracked<&mut NodeOwner<C>>`、`regions: Tracked<&mut MetaRegionOwners>`、
`guards: Tracked<&mut Guards>`）：

```text
old(self).invariants(old(owner).value(), *old(regions)),
old(owner).inv(),
old(owner).value().path.len() == old(owner).level(),
old(self).node_matching(old(owner).value(), *old(parent_owner), *old(self).node),
old(owner).level() < INC_LEVELS - 1,
old(owner).level() + old(parent_owner).level() == INC_LEVELS,
old(parent_owner).metaregion_sound_node(*old(regions)),
in_count_window::<C>(old(parent_owner).level(), old(self).idx as int),
```

| 原子句 | 转写 | 类型 |
|---|---|---|
| `old(self).invariants(owner.value(), *regions)` | `entry_invariants(entry_id, owner_id, regions_id)` | uninterp（损失：Entry 不变式定义） |
| `old(owner).inv()` | `owner_inv(owner_id)` | uninterp |
| `path.len() == level()` | `path_len_eq_level(owner_path_len, owner_level)` := `==` | 机械（`path.len()` 提升为标量参数） |
| `old(self).node_matching(...)` | `node_matching(entry_id, owner_id, parent_owner_id)` | uninterp |
| `level() < INC_LEVELS - 1` | `level_lt_inc_minus_1(owner_level, inc_levels)` := `level < inc_levels - 1` | 机械 |
| `level() + parent.level() == INC_LEVELS` | `levels_sum_eq_inc(owner_level, parent_level, inc_levels)` | 机械 |
| `metaregion_sound_node(*regions)` | `metaregion_sound_node(parent_owner_id, regions_id)` | uninterp |
| `in_count_window::<C>(parent.level(), idx)` | `in_count_window(parent_level, idx)` | uninterp（损失：`C` 配置常量决定的窗口定义） |

witness `(0,0,1,0,0,2,2,3,2)`：`path_len 0==0`、`0 < 2-1`、`0+2==2` 均成立（机械层；整条 requires 因 uninterp 合取项 assert 不可证，机器活性未确认，见 FULL-RUN.md）。

## 3. `split_if_mapped_huge` — `entry.rs`（attr 851–966，fn 968，impl&%2）

原 requires（tracked 同 #2）：

```text
old(regions).inv(),
old(owner).inv(),
old(owner).value().path.len() == old(owner).level(),
old(self).wf(old(owner).value()),
old(parent_owner).relate_guard(*old(self).node),
old(parent_owner).inv(),
old(parent_owner).level() == old(owner).value().parent_level,
old(parent_owner).level() < NR_LEVELS,
old(owner).level() + old(parent_owner).level() == INC_LEVELS,
old(self).node_matching(old(owner).value(), *old(parent_owner), *old(self).node),
old(parent_owner).metaregion_sound_node(*old(regions)),
old(owner).value().is_frame() && old(parent_owner).level() > 1 ==>
    old(owner).value().metaregion_sound(*old(regions)),
```

| 原子句 | 转写 | 类型 |
|---|---|---|
| `old(regions).inv()` | `regions_inv(regions_id)` | uninterp |
| `old(owner).inv()` / `old(parent_owner).inv()` | `owner_inv(owner_id)` / `owner_inv(parent_owner_id)`（同一 fn 两个 id 实例） | uninterp |
| `path.len() == level()` | `path_len_eq_level` | 机械 |
| `old(self).wf(owner.value())` | `entry_wf(entry_id, owner_id)` | uninterp |
| `parent_owner.relate_guard(*old(self).node)` | `relate_guard(parent_owner_id, entry_id)` | uninterp |
| `parent.level() == owner.value().parent_level` | `parent_level_eq_owner_parent_level(parent_level, owner_parent_level)` := `==` | 机械（`parent_level` 字段提升为标量参数） |
| `parent.level() < NR_LEVELS` | `level_lt_nr_levels(parent_level, nr_levels)` | 机械 |
| `level() + parent.level() == INC_LEVELS` | `levels_sum_eq_inc` | 机械 |
| `old(self).node_matching(...)` | `node_matching(entry_id, owner_id, parent_owner_id)` | uninterp |
| `metaregion_sound_node(*regions)` | `metaregion_sound_node(parent_owner_id, regions_id)` | uninterp |
| `is_frame() && parent.level() > 1 ==> metaregion_sound(*regions)` | `frame_split_metaregion(owner_is_frame, parent_level, owner_id, regions_id)` := `!(owner_is_frame && parent_level > 1) \|\| metaregion_sound(...)` | 半机械（布尔 `is_frame` 提升为参数；蕴含体 uninterp） |

witness `(0,1,1,1,2,false,2,2,3,4,3)`：`1==1`、`2==2`、`2<4`、`1+2==3`、
frame 蕴含取假前件成立（机械层；整条 requires 因 uninterp 合取项 assert
不可证，机器活性未确认，见 FULL-RUN.md）。

## 4. `alloc_absent_child` — `entry.rs`（attr 2047–2140，fn 2141，impl&%3）

原 requires（tracked 同 #2）：

```text
old(owner).inv(),
old(owner).value().path.len() == old(owner).level(),
old(owner).value().is_absent(),
old(owner).level() < INC_LEVELS - 1,
old(owner).value().metaregion_sound(*old(regions)),
old(parent_owner).inv(),
old(parent_owner).relate_guard(*old(self)),
old(parent_owner).level() == old(owner).value().parent_level,
old(parent_owner).level() > 1,
old(owner).level() + old(parent_owner).level() == INC_LEVELS,
old(parent_owner).metaregion_sound_node(*old(regions)),
idx < NR_ENTRIES,
in_count_window::<C>(old(parent_owner).level(), idx as int),
old(owner).value().match_pte(
    old(parent_owner).children_perm.value()[idx as int],
    old(owner).value().parent_level,
),
old(regions).inv(),
old(regions).slots.contains_key(old(parent_owner).slot_index),
```

| 原子句 | 转写 | 类型 |
|---|---|---|
| `owner.inv()` / `parent_owner.inv()` | `owner_inv(owner_id)` / `owner_inv(parent_owner_id)` | uninterp |
| `path.len() == level()` | `path_len_eq_level` | 机械 |
| `is_absent()` | `owner_is_absent: bool`（布尔枚举成员提升为参数） | 机械（值域仍抽象） |
| `level() < INC_LEVELS - 1` | `level_lt_inc_minus_1` | 机械 |
| `metaregion_sound(*regions)` | `metaregion_sound(owner_id, regions_id)` | uninterp |
| `parent_owner.relate_guard(*old(self))` | `relate_guard(parent_owner_id, entry_id)` | uninterp |
| `parent.level() == owner.value().parent_level` | `parent_level_eq_owner_parent_level` | 机械 |
| `parent.level() > 1` | `parent_level_gt_1(parent_level)` | 机械 |
| `level() + parent.level() == INC_LEVELS` | `levels_sum_eq_inc` | 机械 |
| `metaregion_sound_node(*regions)` | `metaregion_sound_node(parent_owner_id, regions_id)` | uninterp |
| `idx < NR_ENTRIES` | `idx_lt_nr_entries(idx, nr_entries)` | 机械 |
| `in_count_window(parent.level(), idx)` | `in_count_window(parent_level, idx)` | uninterp |
| `match_pte(children_perm.value()[idx], parent_level)` | `match_pte(owner_id, parent_owner_id, idx, owner_parent_level)` | uninterp（损失：children_perm 数组索引与 PTE 匹配定义） |
| `old(regions).inv()` | `regions_inv(regions_id)` | uninterp |
| `regions.slots.contains_key(parent_owner.slot_index)` | `slots_contains_key(regions_id, parent_slot_index)` | uninterp（`slot_index` 字段提升为标量参数） |

witness `(0,0,16,1,1,1,true,2,2,2,0,3,3)`：`1==1`、`1<2`、`2==2`、`2>1`、
`1+2==3`、`0<16` 均成立（机械层；整条 requires 因 uninterp 合取项 assert
不可证，机器活性未确认，见 FULL-RUN.md）。

## 5/6. `lock` / `make_guard_unchecked` — `ostd/src/mm/page_table/node/mod.rs`（attr 369–383 / 395–412，fn 375 / 401，impl&%2）

两者 requires 完全相同（tracked：`owner: Tracked<&NodeOwner<C>>`（lock）/
`Tracked<&NodeOwner<C>>`（make_guard_unchecked）、`guards: Tracked<&mut Guards>`）：

```text
self.inner@.invariants(*owner),
self.inner@.external_meta_wf(owner.frame_permission.resource(), ()),
old(guards).unlocked(owner.slot_vaddr()),
```

| 原子句 | 转写 | 类型 |
|---|---|---|
| `self.inner@.invariants(*owner)` | `node_invariants(node_id, owner_id)` | uninterp（损失：NodeOwner 不变式） |
| `self.inner@.external_meta_wf(owner.frame_permission.resource(), ())` | `external_meta_wf(node_id, owner_id)`（frame_permission 资源折入 owner_id） | uninterp |
| `old(guards).unlocked(owner.slot_vaddr())` | `guards_unlocked(guards_id, slot_vaddr)` | uninterp（`slot_vaddr` 提升为标量参数；集合成员关系抽象） |

**无 witness**：requires 全为 uninterp 谓词，无机械标量内容可见证。
预期按判据落在 CandidateInconclusive（不可判定，不计缺陷）。两项共享同一
形状但作为独立清单条目分别判定。

## 7. `MutexGuard::new` — `ostd/src/sync/mutex.rs`（attr 220–228，fn 229，impl&%7）

原 requires（tracked：`perm: Tracked<PointsTo<T>>`）：

```text
perm.id() == mutex.cell_id(),
```

| 原子句 | 转写 | 类型 |
|---|---|---|
| `perm.id() == mutex.cell_id()` | `perm_matches_cell(perm_id, mutex_id)` := `perm_id == mutex_id` | 机械 |

witness `(0,0)`（沿用形状运行）。**已知抽象损失（git ae0f897 已记录）**：
`PointsTo<T>` 是线性 ghost 权限，只能由 `PCell::new` 产出、不可伪造；
抽象成 usize id 后相等谓词平凡可满足。audit 的"调用面恒假/不可达"是
上下文相关判断，孤立形状探针无法确认也无法否证——此条目在 findings
registry V-001 中保持 `probe_disputed` 状态，本次运行不改变该状态。

---

## Abstraction 损失汇总

1. 所有 tracked 线性资源 → 不透明 id：所有权不可伪造性全部丢失。audit 的
   7 项"恒假"判读若依赖"外部调用者拿不到权限"，形状探针原则上无法复现该
   不可能性。
2. ghost 结构谓词（`invariants`、`inv`、`metaregion_sound*`、`match_pte`、
   `slots.contains_key`、`continuations` 树等）→ uninterp：定义体丢失，
   对应条目上 SMT 无法判定可满足性方向。
3. 机械保留的只有标量等式/不等式/算术与两个布尔成员（`is_frame`、
   `is_absent`）。因此运行结果只能区分两类：标量约束可满足 → 预期
   ConfirmedLive（说明恒假性不在标量层）；无标量约束 → 预期
   CandidateInconclusive。任一条目若得 ConfirmedVacuous，说明其 requires
   在纯标量层即自相矛盾——那将是超出 audit 静态判读的新证据，须复核。
