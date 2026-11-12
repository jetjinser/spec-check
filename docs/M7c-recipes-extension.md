# M7c 配方库扩展：iterator / 状态量化骨架（设计）

**运行前写定**（2026-11-12，先于实现与任何合成/注入运行）。本文件是
M7c 合成器的**增量**设计：在 `docs/M7c-obligation-synthesizer.md` 的
4 族骨架、recipe schema 与预注册规则之上，定义新增族骨架、schema 扩展、
可表达性判据与降级规则。旧行为保持逐字节兼容；判定只以 divergence
实跑为准，本文件不记录运行结果。

## 1. 目标

M7b 首站 22 个非 blocked 候选中只有 2 条能被现有骨架表达。剩余 20 条
的缺口不是"没有断言"，而是**骨架无法承载断言的形状**：

- 9 条 `util::ops::test::range_difference_*`：义务是 `Iterator::next`
  序列（`assert_eq!(res.next(), Some(val))` 直至 `None`），现有
  `value-readback`/`is_none-demand` 只能做一次不可变绑定调用；
- 2 条 `cpu::set::test::*`：义务是集合构造后的空性/成员关系，涉及
  `CpuSet@` 的集合视图与 `AtomicCpuSet::add` 后的查询；
- 9 条页表/create_page_table/page_properties：受 tracked owner/ghost
  guards、`external_body` 与私有 helper 阻断（见 §6）。

本扩展只加**通用语义骨架**，不把目标类型写进工具：如何构造迭代器、
消费几次、断言哪些值，全部由项目侧 recipe 提供。

## 2. 新增模板族

新增两个族。族由 recipe 的 `obligation_family` 选择（§4），旧的
4 族行为不变。

### 2.1 `iterator-sequence`（迭代器序列骨架）

```rust
<type.setup>
<bind_attr?>  let [mut] recv = <construct>;   // construct 可空 = 无接收者
<interface.bridge>                             // 可使用 {recv}
let mut it = <interface.call>;
<interface.steps...>                           // 可消费 {it}，多行 exec 语句
<interface.probes...>                          // 可选，steps 之后
assert(<interface.assert>);
```

语义：`call` 必须产出一个迭代器；`steps` 按顺序消费它（通常是
`let r0 = {it}.next();`），`assert` 检查序列。与 `value-readback` 的
区别是迭代器必须 `mut` 绑定且允许任意步消费；与 `final-relation` 的
区别是断言对象是序列/输出，不是接收者后状态。

校验（合成期，失败即整体报错）：

- `call` 非空；`steps` 至少一条；
- `{it}` 只在 `iterator-sequence` 可用（旧族 recipe 出现 `{it}` 报错）；
- `assert` 必须引用 `{it}` 或 `steps` 中 `let` 绑定的名字之一，防止
  "消费了迭代器但断言与序列无关"；
- `{out}`/`{handle}` 在该族不可用。

### 2.2 `state-invariant`（状态 + 查询量化骨架）

```rust
<type.setup>
<bind_attr?>  let [mut] recv = <construct>;   // construct 可空
<interface.bridge>                             // 可使用 {recv}
let out = <interface.call>;                    // 可空；空 = 纯状态断言
<interface.probes...>                          // 可使用 {recv}/{out}
assert(<interface.assert>);
```

语义：在**同一接收者状态**上先做一组变更（`bridge`），再取一个查询
输出（`call`），最后断言状态视图或查询结果。用于集合/原子状态的
成员关系、空性、计数等"量化状态"义务；`{recv}@` 的视图与谓词完全由
项目侧 recipe 书写。

校验：

- `call` 为空时（纯状态断言）`assert` 必须引用 `{recv}`；
- `call` 非空时 `assert` 必须引用 `{recv}` 或 `{out}`；
- `{it}`/`{handle}` 在该族不可用。

> 量化（`forall`/集合成员）不是骨架硬编码：骨架只提供"变更 + 查询 +
> 断言"的位置，量词与视图表达式属于 recipe。这样工具保持零项目专有
> 知识，同时不再需要为每个集合 API 加新族。

## 3. 多候选同函数：`candidate_interfaces`

9 条 `range_difference_*` 的 `function` 都是 `range_difference`，但
a/b 实参与期望序列各不相同。现有 `interfaces` 以函数为键，无法区分。
新增**按候选 id 索引**的覆盖表：

```jsonc
{
  "candidate_interfaces": {
    "K145": { /* 与 interfaces 同构 */ },
    "K146": { /* ... */ }
  }
}
```

解析优先级：

1. `candidate_interfaces[<candidate.id>]`（精确命中，最高优先）；
2. `interfaces[<function 原文>]`；
3. `interfaces[<去泛型后的 canonical function>]`。

这是纯增量：缺省空的 `candidate_interfaces` 等价于旧行为。重复键仍由
候选的 `dedup_key` 去重；不同候选 id 的条目互不影响。

## 4. schema 扩展

`InterfaceRecipe` 新增字段（全部可选、向后兼容）：

| 字段 | 类型 | 语义 |
|---|---|---|
| `steps` | `[]string` | `iterator-sequence` 的迭代器消费语句；可用 `{it}` |
| `obligation_family` | `string` | 骨架族选择；可不同于候选 `family` |
| `family_basis` | `string` | 与候选 `family` 不同时**必填**，进入报告审计 |

`Recipes` 新增 `candidate_interfaces: { <candidate id>: InterfaceRecipe }`。

约束：

- `obligation_family` 必须能 `parse`；解析失败整体报错；
- 与候选 `family` 相同时不要求 basis；不同时缺 `family_basis` 整体报错；
- 原有 `family` 字段仍是**交叉校验**（必须等于候选 family），不被
  `obligation_family` 取代；
- 报告的 `entries[]` 增加 `obligation_family` 与 `family_basis`，
  运行前即可审计"骨架是否被改写"。

## 5. 可表达性判据与降级规则

对每条缺口候选，按以下顺序判定：

1. **构造可达**：类型/静态/帧是否能在 crate 根探针内构造，且不依赖
   私有 module 或 `pub(crate)` 之外的项；
2. **tracked 参数可铸造**：`#[verus_spec(with Tracked(...), Ghost(...))]`
   的调用者参数能否由公开 API 或 recipe preamble 提供；
3. **义务形状匹配**：序列 → `iterator-sequence`；变更+查询 → 
   `state-invariant`；单次值读回 → `value-readback`；
4. **前置可满足**：调用的 `requires` 能由探针上下文与 vstd 默认
   broadcast 证明；
5. **断言可写**：断言中的视图/谓词在探针内可见（`View`、`@`、
   spec 函数）。

降级：

| 情形 | 处理 |
|---|---|
| 1–5 全过 | 写 recipe，进入主批量 manifest |
| 构造可达但前置/断言需假设 | recipe 仍可写，但**不得**使用 `assume`/`axiom`；运行判定为 gap/Inconclusive 如实记录 |
| tracked 参数不可铸造 / 私有 helper / `external_body` 无语义 | **不进入主 manifest**；在预注册与运行文档中列源码证据 |
| 同函数多实例 | `candidate_interfaces` 按 id 覆盖 |

不可表达项不写"占位义务"进主 manifest：divergence 的 phase-1 编译失败
会把**整批**条目降为 Inconclusive，污染干净判定面。需要编译证据的
阻断项单独成批尝试，结果如实记 Inconclusive。

## 6. 页表类阻断的判据（预注册前结论）

以下为 M7b/M7c 已有证据的**结构化重述**，运行文档会再给源码行号：

- `PageTable::empty()` / `empty_with_owner()` 为 `#[verifier::external_body]`；
- `cursor_mut` / `entry` / `is_node` / `to_ref` 的 `with Tracked(owner),
  Ghost(root_guard), Tracked(regions), Tracked(guards)` 需要
  `PageTableOwner` + `MetaRegionOwners` + `Guards`，而
  `MetaRegionOwners::inv` 要求**全槽 map**，冻结面无铸造入口；
- `PageTable::new_kernel_page_table` 为 `external_body` 且 body 是
  `unimplemented!()`；
- 页表测试的 setup（`create_user_pt_mapped_at`、
  `setup_page_table_with_two_frames`、`check_map_with_property`）是私有
  helper，crate 根探针不可达；
- `cursor_mut` 的 `0 < va.end` 前置使空/负向 range 义务在客户端不可写。

因此 9 条页表类候选**不在本轮可表达集**；不写 recipe，只登记阻断。

## 7. 预注册策略

- 运行前写 `RECIPE-EXT-EXPECTED.md`：逐条列出合成后的义务摘要、
  预期判定（`supported`/`gap`）、依据与"未运行"项；
- recipe 的 `expect` 只在义务形态与候选草稿不同时使用，且必须带
  `expect_basis`；运行后不改写；
- `expect_match` 只记录机器可查假设是否命中，不参与判定；
- 同一输入两次合成必须逐字节一致（无时间戳/哈希）。

## 8. 产物

- `examples/vostd-ktest/recipes-m7b-ext.json`：可表达候选的 recipe；
- `examples/vostd-ktest/divergence-manifest-m7b-ext.json` + `.report.json`：
  批量合成产物；
- `examples/vostd-ktest/RECIPE-EXT-EXPECTED.md`：运行前预注册；
- `examples/vostd-ktest/RECIPE-EXT-RUN.md`：运行判定表与阻断清单；
- `examples/vostd-ktest/run-m7b-ext.sh`：复现命令。

综合报告新增 `obligation_family`/`family_basis` 字段仅用于审计，不被
divergence runner 消费。
