# M7c 设计笔记：义务 client 合成器（obligation synthesizer）

**运行前写定**（2026-11-12，先于实现与任何合成/注入运行）。本文件是
`cargo-spec-check synthesize` 的接口钉定：输入、模板族、recipe schema、
预注册继承、降级规则与 census 的关系。判定只以 divergence 实跑为准；
本文件不记录任何运行结果。

## 1. 目标与非目标

目标：把"M7c 候选（`candidates.json`）+ 项目侧类型配方（`recipes.json`）"
机械地降维成 divergence 可执行的 **obligation manifest**，使新增一个类型
族的成本是"一个 recipe 块"，而不是"每个函数手写一条义务"。产出必须是
可复现的纯函数：同一输入产生逐字节相同的 manifest 与合成报告。

非目标：

- 不发明义务。义务的语义（调用什么、断言什么）来自项目侧 recipe；
  工具只提供族骨架、占位符、校验、去重、预注册继承与报告。
- 不修改目标源码。合成器只写 manifest 与 report；注入/验证仍由
  `analyses-divergence` 完成。
- 不做运行时语义推断。`ContractGapCandidate` 是证明论结论，`Inconclusive`
  不计缺陷；合成器不预测运行结果，只继承运行前预注册。

工具代码 **零项目专有知识**：模板族是通用语义；vostd 的
`VmReader::from(...)`、`Either::<u64, u64>::Left(5)` 等配方只允许出现在
`examples/vostd-*/recipes.json`。Rust 源码与单测不得出现目标项目类型名。

## 2. 流水线

```
candidates.json ──┐
                  ├─► synthesize ──► divergence-manifest.<m7c>.json  (可执行)
recipes.json ─────┘                └─ synthesize report.json       (逐候选去向)
                                          │
                                          ▼
                                  analyses-divergence (注入 + 验证 + 判定)
```

候选侧只消费结构化字段：`id`、`function`、`family`、`constructibility`
(+`reason`/`note`)、`prereg_expected`、`dedup_key`、`registry_overlap`、
`obligation_draft`（审计用，不直接执行为义务）。`obligation_draft` 是运行前
假设草稿，可能含 Verus 不合法的形态（如 `final(owned_local)` 或 exec 调用
落在 `assert` 实参内）；合成器只把它记录进报告，实际义务由 recipe 渲染。

选择与顺序：候选按输入顺序处理，保证输出确定。`--select` 给出逗号分隔的
token，匹配 `id`（忽略 `-`/大小写）、`function` 原文、去泛型后的
`Type::method`、或生成的 entry 名；任一 token 未命中即报错（防止拼写错误
静默空跑）。未给 `--select` = 全部候选。

去重：以候选的 `dedup_key` 为准，重复键保留首个，后续进报告
`skipped/dedup`。entry 名同时做 sanitize 后唯一性检查，冲突报错。

## 3. 模板族（通用骨架）

四个族与 divergence/研究线既有语义一致。占位符是**字面 token**
（`{recv}`/`{out}`/`{handle}`），由工具替换为固定变量名 `recv`/`out`/
`handle`；未知或该族不可用的占位符一律报错（不静默降级）。

### 3.1 value-readback / is_none-demand（绑定-断言骨架）

```rust
<type.setup>
let recv = <type.construct>;
<interface.bridge>
let out = <interface.call>;
<interface.probes>          // 可选：对 out 再做一次 exec 调用并绑定，例如 flag
assert(<interface.assert>);
<interface.hold ? "let _held = out;" : "">
```

`call` 必填。`assert` 通常使用 `{out}`；`probes` 用于绕开"exec 调用不能
出现在 assert 实参内"的 Verus 限制（例如先 `let flag = out.is_none();`
再 `assert(flag);`）。`hold = true` 时在断言后持有句柄（guard 生命周期
纪律）。两族的差别是义务语义（读回值 vs 布尔需求），骨架相同；在族校验
上分别记账，recipe 侧按语义选择。

### 3.2 guard-identity（句柄-关系骨架）

```rust
<type.setup>
let recv = <type.construct>;
<interface.bridge>
let handle = <interface.call>;
<interface.probes>
assert(<interface.assert>);
<interface.hold ? "let _held = handle;" : "">
```

`call` 必填；`assert`/`probes` 必须引用 `{handle}`（否则是"取得句柄但
不检查关系"，族校验拒绝）。身份谓词是否可在冻结面表达属项目侧 recipe
责任；不可表达时 recipe 不应登记该接口。

### 3.3 final-relation（接收者后状态骨架）

```rust
<type.setup>
let mut recv = <type.construct>;
<interface.bridge>
<interface.snapshot>        // 可选：调用前快照，如 let before = recv.cursor.vaddr;
<interface.call>;           // 可空：构造器自身的后状态义务
<interface.probes>
assert(<interface.assert>);
```

`construct` 可空（静态函数/无接收者）；`assert` 必须引用 `{recv}`（接收者
后状态），否则拒绝。`call` 为空时表示"构造完成后的状态断言"。

**转写口径（与 candidates 的 `obligation_draft` 差异，预注册前承认）**：
Verus 的 `final(x)` 要求 `x` 是 `&mut` 类型的位置；候选草稿里的
`final(r)`（`r` 是自有局部）不可编译。模板把"接收者后状态"降为对**同一
个自有局部在调用后的当前值**断言（`recv.remain_spec() <= 4`），语义等价
（都是 `*self` 的后状态），但形式不同。此差异必须逐条写进运行前预注册。

### 3.4 other

无通用骨架。family = `other` 的候选（trait 派生、错误语义、
external_body、proof-only 等）不进入合成 manifest，进报告
`skipped/no-template-family`。若未来需要，必须先在本文新增族语义与骨架，
不得用 `value-readback` 冒充。

## 4. recipe schema（项目侧输入）

```jsonc
{
  "target_workspace": { /* divergence v1.1 同构，原样透传 */ },

  "types": {
    "<TypeName>": {
      "imports": ["use ...;"],        // 可选：use 行，逐行拼接
      "setup": "let buf = [0u8; 8];", // 可选：接收者/参数构造前的准备语句
      "construct": "<表达式>",         // 生成 `let recv = ...;`；空串=无接收者
      "model_preamble": "..."          // 可选：额外 verus 源（辅助 spec/struct）
    }
  },

  "interfaces": {
    "<function 原文或去泛型 Type::method>": {
      "receiver_type": "<types 键>",
      "receiver_mut": false,            // 可选：true => `let mut recv = ...`（&mut self 方法）
      "family": "value-readback",       // 可选：与候选 family 交叉校验
      "bridge": "let ro = {recv}.clone_ro();", // 可选：接收者与调用之间
      "call": "{recv}.limit(4)",        // value/is_none/guard 必填；final 可空
      "snapshot": "let before = {recv}.cursor.vaddr;", // final 可选
      "probes": ["let flag = {out}.is_none();"],       // 可选，可多行
      "assert": "{out} == Some(5u64)",
      "hold": false,                    // 可选
      "expect": "gap",                  // 可选：gap | supported | none（覆盖）
      "expect_basis": "…"               // 覆盖时必须写明依据（运行前）
    }
  }
}
```

约束（合成期校验，失败即整体报错）：

- `receiver_type` 必须存在；`types` 中未引用的块允许存在（便于共享）。
- `receiver_mut = true` 只影响接收者绑定（`let mut recv`），用于
  `&mut self` 方法；final-relation 骨架本来就用 `let mut recv`。
- 接口 `family`（若给）必须与候选 `family` 一致。
- 三个族骨架的必填项与非空校验见 §3；占位符必须在该族可用集合内。
- `expect` 覆盖必须带 `expect_basis`；该字段进入报告，便于审计
  "预注册是否被改写"。

recipe 文件是 CLI 的显式输入，不内嵌于候选或目标仓库；同一份 candidates
可以用不同 recipe 生成不同强度的 manifest（对照实验）。

## 5. 预注册（expect）继承规则

运行前写定，运行后不改写；`expect` 只用于报告的 `expect_match`，不参与
判定（divergence 既有纪律）。

1. 默认继承候选的 `prereg_expected`：`gap → "gap"`、`supported →
   "supported"`、`inconclusive → 无 expect`（divergence 没有
   Inconclusive 预注册值；不虚构）。
2. recipe 的 `expect` 覆盖优先，但必须同时给 `expect_basis`。覆盖只在
   **义务形态被模板改变**时使用（例如候选草稿含不可编译的
   `final(owned_local)`，或 recipe 选择了一个更弱/更强的可表达义务），
   并在 `DIVERGENCE-EXPECTED-M7C.md` 中逐条写出原值、新值、依据。
3. 报告记录每条 entry 的 `expect` 与 `expect_source`
   （`candidate-prereg` / `recipe-override` / `none`）；运行前把报告与
   预注册文档一起提交，运行后不得重生成覆盖旧报告（用 git 历史固定）。

## 6. 不可合成降级与失败语义

| 情形 | 处理 | 报告 reason |
|---|---|---|
| 有 interface recipe，渲染成功 | 写入 manifest | `synthesized` |
| family = `other` | 跳过 | `no-template-family` |
| `constructibility = needs-recipe` 且无 recipe | 跳过 | `needs-recipe` |
| `constructibility = blocked` 且无 recipe | 跳过 | `blocked` |
| `constructible` 但无 recipe | 跳过 | `no-interface-recipe` |
| 同 `dedup_key` 已合成 | 跳过 | `dedup` |
| `--select` 未命中 | 跳过（仅报告） | `not-selected` |
| recipe 配置错误（缺字段/占位符/族不符/重名） | **整个合成失败** | CLI 非零 |
| recipe 存在但候选被标 blocked | 仍尝试（recipe 是对可行性的新主张） | — |

"recipe 存在即尝试"是有意的：M7c 的第二个任务是验证 needs-recipe 能否被
项目侧配方提升为可执行义务；失败由 divergence 的编译期 Inconclusive 或
静态评估如实记账，不由合成器预先替运行下结论。

跳过项 **不写入 divergence manifest**：divergence 没有"skip"判定，
manifest 里塞不可编译的占位义务会污染整批构建（phase-1 失败会把全部条目
降为 Inconclusive）。因此 skip 只存在于合成报告；manifest 只包含将要
真实注入的条目，判定面保持干净。运行期 Inconclusive 仍只表示"义务不可
表达/探针不净"，两者口径不同。

## 7. 与 census（M7a）的关系

- census 是**事实层**：VIR 上逐函数枚举输出、后置来源、提及判定；不产生
  义务也不判定缺陷。
- candidates（M7b 前置）在 census 之上按族与可合成性分层，给出
  `prereg_expected`。
- 合成器消费 candidates，不重做 census：`function`/`family`/
  `constructibility` 是上游结论；`vir_path`/`frozen_census_status` 仅进
  报告审计。若 census 判定"无后置条件"，合成出的 gap 仍然是**契约缺口
  候选**（无后置本身就是最强的缺口形态），但归档时必须注明其来源是
  census 的 `without_postcondition` 行，不能与"有后置但未提及该输出"
  混为一谈。

## 8. 产物与可复现性

- `--out FILE`：divergence manifest（确定性 JSON，2 空格缩进 + 末尾换行）。
- `--report FILE`（缺省 = `<out stem>.report.json`）：逐候选去向、entry
  审计信息（含渲染后的义务正文与 expect_source）。
- 两次同输入运行：manifest 与 report 逐字节一致（无时间戳、无哈希）。
- 合成器不接触目标工作区；注入/恢复完全由 divergence runner 负责。
