# M7b 设计笔记：ktest 断言 → 调用者义务候选提取

**运行前写定**（2026-11-12，先于实现与首站运行）。本文件钉定
`cargo-spec-check extract-ktest` 的输入、提取规则、输出 schema、降级策略
与预注册边界。判定只以 divergence 实跑为准；本文件不记录运行结果。

## 1. 目标与非目标

目标：把冻结 Rust 源中的 `#[ktest]` 测试，机械地提取为**可审计的调用者
义务候选**，输出与 `synthesize --candidates` 兼容的 JSON。提取面是"所有
ktest 的需求"，运行面是"当前冻结树能注入验证的子集"；两者**解耦**：

```
source tree ──► extract-ktest ──► candidates.json ──► synthesize ──► divergence
                     │                    │
                     │                    └─ 项目侧 recipes（构造/调用/断言）
                     └─ 需求存在性 + 可回放性分级（direct / needs-model / blocked）
```

非目标：

- 不发明断言。每条候选都带源码位置、原始片段与触发规则。
- 不预测运行判定。`constructibility` 只描述"能否写成 client"，
  `ContractGapCandidate` 只由 divergence 实跑产生。
- 不把 blocked 当"无需求"。blocked 是"需求已提取、当前冻结 proof 树
  不可回放"，必须保留并计数。
- 不修改目标源码。提取器只读。

工具代码 **零项目专有知识**：`#[ktest]`/断言宏/`unwrap` 是通用 Rust
语义；项目特定的私有观察模式、环境依赖、门控 cfg、方法变体表、研究
锚点与函数覆盖都在 `--config` 的项目侧配置里。Rust 源码与单测不得出现
目标项目类型名。

## 2. CLI 与输出

```
cargo-spec-check extract-ktest \
  --source-root DIR \
  --out CANDIDATES.json \
  [--config FILE] \
  [--stats FILE]
```

- `--source-root`：递归扫描 `*.rs` 的根（例如 `ostd/src`）。
- `--out`：**裸 JSON 数组**，每个元素是一条候选，字段是
  `synthesize::Candidate` 的超集（未知字段被 synthesize 忽略）。
- `--config`：项目侧 JSON；缺省时只用通用默认规则。
- `--stats`：可选统计 sidecar；同时打印到 stdout。

输出按 `priority_score` 降序、测试路径升序排序；ID 在发现阶段按
"文件名排序 + 源码顺序"分配（`K001...`），保证同一输入逐字节确定。

### 2.1 候选 schema（synthesize 消费字段加粗）

| 字段 | 含义 |
|---|---|
| **id** | `K###`，发现顺序稳定 |
| **function** | 最佳努力的主被测函数路径（`Type::method` 或裸函数名） |
| **family** | 映射到 M7c 模板族：`value-readback` / `is_none-demand` / `final-relation` / `other` |
| **constructibility** | `constructible` / `needs-recipe` / `blocked` |
| **constructibility_reason/note** | 分级依据 |
| **dedup_key** | 默认 `ktest:<id>`（一测试一候选，不跨测试去重） |
| **obligation_draft** | 审计用草稿；不直接执行 |
| test/file/line/body_span | 测试身份与位置 |
| assertions/demands/call_sequence | 断言、成功需求、调用序列 |
| categories/category | 断言级 + 候选级类别 |
| rating/rating_reasons | 可转写性评级（见 §5） |
| capture_gap/disconnected | 冻结树可回放性 gate |
| function_source/functions | 函数推导来源与全部目标 |
| model_requirements | 下游需要的模型能力（helper/quantified/控制流/ghost） |

## 3. 提取架构

### 3.1 词法层（无类型信息也可靠）

1. 读原文件为 `raw`；
2. 生成等长 `masked`：块注释（可嵌套）、行注释、字符串/字符字面量、
   raw 字符串（`r#"..."#` / `br#"..."#`）替换为空格，保留换行与**字节
   偏移**；
3. 所有结构匹配在 `masked` 上做，片段与行号从 `raw` 取；
4. 花括号/圆括号/方括号配对用通用 `match_delim`。

等长 mask 是硬约束：任何"片段取自 raw、位置取自 masked"的操作都
要求两者偏移一致。

### 3.2 测试发现与门控

- 匹配配置的测试属性（默认 `#[ktest]`，允许 `#[ktest(...)]`），向后
  600 字节内找 `fn name(`，花括号配对取函数体。
- 记录属性串：`should_panic`（整测试排除）、`capture_gap_cfgs`
  （proof 侧捕获缺口）。
- 模块路径 = 文件模块前缀（`lib.rs`/`mod.rs` 去掉）+ 花括号包含该测试
  的内层 `mod` 名 + 函数名。
- 三层门控：
  1. 内层 `mod` 或测试属性含 `ktest_capture_gap`；
  2. 父模块文件里 `mod <stem>;` 声明窗口含 `ktest_capture_gap`；
  3. crate 根：顶层模块未声明（或被块注释）、或配置的 arch 旁路规则
     命中（例如 `arch/riscv` 未被 `lib.rs` 选中）。
- 门控结果是 `capture_gap` / `disconnected`，**不参与需求存在性**，
  只影响评级。

### 3.3 断言与需求

断言宏：`assert!`、`assert_eq!`、`assert_ne!`（含 `::core::assert!`
前缀；从 `assert` 起取片段）。成功需求：

- `.unwrap()` / `.expect(...)`；
- `let`/`if let`/`while let` 左值含 `Some(`/`Ok(`/`Err(` 的模式绑定，
  包括 `let ... else` 与元组解构（`let (va, Some(item)) = ...`）；
- 每条记录：类别、原文、行号、目标调用、作用域（single/quantified）、
  控制流上下文（none/if/if-let/loop/match/let-else/iterator）、
  helper 归因。

**闭包/迭代器**：在 `.map/.filter_map/.filter/.for_each/.any/.all/...`
实参 span 或 `for/while` 体内的断言/需求标 `scope=quantified`。这直接
消除原型"闭包内 unwrap 当成单次成功需求"的过近似；下游不会把
`scope=quantified` 的 demand 计入单次义务。

**控制流**：`if let`、`let ... else`、`match`、循环内的位置记录
`control_flow`。`let ... else panic` 是"成功需求 + panic 分支"，不视为
义务弱化；真正的条件分支（断言只在某分支成立）标 `if-let`/`match`，
下游必须显式建模或判 Inconclusive。

**类型/变体信息**：没有 typed HIR 时，用局部证据推断
`Option/Result` 变体：

| 证据 | 置信度 | `variant_source` |
|---|---|---|
| `let x: Option<_>` / `Result<_,_>` 注解 | high | `type-annotation` |
| 表达式含 `Some/None/Option::` 或 `Ok/Err/Result::` | high | `constructor` |
| `let`/`if let` 左值模式 | high | `pattern` |
| 配置 `method_variants`（项目侧方法表） | medium | `method-table` |
| 以上都无 | low | `unknown` |

无法判定时保留 `Some or Ok` 并标 `unknown`，**不猜单一变体**。

### 3.4 调用序列与函数推导

调用序列按源码位置记录 `recv.method(...)`、`Type::assoc(...)`、裸函数；
不展开宏、不建借用图。主被测函数按优先级：

1. 配置 `function_overrides[test]`；
2. helper 内联时，helper 体内的首个非断言/非迭代器 callee
   （例如 `assert_range_difference` 里的 `range_difference`）；
3. 断言/需求的目标调用 + 局部 `let name = Type::...` 的接收者类型
   （得到 `Type::method`）；
4. 关联调用 `Type::assoc`；
5. 只有方法名时输出裸方法名，`function_source=method-name-only`
   （需要项目 recipe 或覆盖）。

`functions[]` 保留全部可推导目标，供人工/recipe 选择；`function` 只是
默认主目标，不宣称唯一。

### 3.5 helper 归因

- 同文件、名字以配置前缀（默认 `assert`/`check`）开头的本地 helper：
  若测试体自身无断言，则一层内联其断言，记 `via_helper`/`via_helper_file`；
- 可选跨文件同名前缀 helper（`cross_file_helpers`，默认关闭）：仅当
  名字在整个源根唯一时内联，避免错误归因；
- setup helper（`setup_*`/`create_*`）**不展开**，只保留调用名并要求
  下游建模；这是有意降级，不硬造。

## 4. 断言种类 → 模板族

| ktest 形态 | 类别 | synthesize 族 | 义务转写 |
|---|---|---|---|
| `assert_eq!(g.value(), 5)` / 写后读回 | `value_readback` | `value-readback` | `let out = call(); assert(out == 5)` |
| `.is_none()/.is_some()/.is_ok()/.is_err()` | `existence_acquisition` | `is_none-demand` | `let out = call(); assert(out.is_none())` |
| `unwrap`/`expect` | `existence_acquisition` | `is_none-demand` | `assert(out.is_ok())`（变体确定时） |
| `assert!(acquire(..).is_some())` | `existence_acquisition` | `is_none-demand` | 形状义务 |
| 获取/释放/再获取、`drop` 后查询 | `state_transition` | `is_none-demand` | 保留调用链与 guard 生命周期（recipe 的 `bridge`/`hold`） |
| 两对象关系、引用计数、paddr/vaddr | `invariant_consistency` | `final-relation` | 需要导出关系谓词；否则 `needs-model` |
| `matches!` / 模式绑定 | `existence_acquisition` | `is_none-demand` | 变体确定后断言形状 |

`family` 只表示"用哪个骨架"，不表示语义已可证；关系谓词缺失时
`constructibility=needs-recipe`，不生成占位义务。

## 5. 可回放性分级（constructibility / rating）

评级只看**可转写性**，不看需求价值：

| 评级 | 条件 | constructibility |
|---|---|---|
| `direct` | 公共 API + 可观察值/形状断言；无 helper/循环/关系/环境依赖 | `constructible` |
| `needs_model_helper` | 断言在断言 helper 中，需内联 | `needs-recipe` |
| `needs_model_quantified` | 循环/迭代器/集合，需要量化不变量 | `needs-recipe` |
| `needs_model_invariant` | 跨对象关系，需导出谓词/tracked 模型 | `needs-recipe` |
| `blocked` | capture-gap / 模块断连 / 调度器环境依赖 | `blocked` |
| `excluded` | `should_panic`、纯私有观察、无断言无需求、恒真、测试假设 | `blocked` |

`direct` **不等于已验证**：只表示语法上可写成 client；实际可表达性
（前置条件、ghost owner、可见性）由 divergence 运行裁决。首站即出现
"提取 direct、但契约前置使负向义务不可表达"的例子，报告必须区分。

## 6. 排除规则（项目侧可配）

| 规则 | 默认 | 说明 |
|---|---|---|
| `should_panic` | 通用 | panic 是预期行为，现有 client 无 panic 模型 |
| `exclude_file_suffixes` | 空 | 例如 crate 根 `lib.rs` 的框架自测 |
| `private_observation_patterns` | 空 | 私有字段/原子读/全局表；项目配置 |
| `environment_patterns` | 空 | 调度器/硬件/全局运行时；标 blocked 而非 excluded |
| `allocator_reuse_patterns` | 空 | "释放后立即复用同一物理页"是测试假设，不是 API 契约 |
| 恒真断言 | 通用 | `assert_eq!(0, 0)` 等字面量对字面量 |
| 无断言无需求 | 通用 | 仅执行不 panic，不构成可断言义务 |

**不排除**：proof 侧不可回放的测试（capture-gap / 断连）、环境依赖的
真实属性、`needs-recipe` 的关系义务——这些都保留为需求记录。

## 7. 降级策略（不虚报）

| 情况 | 输出 | 下游 |
|---|---|---|
| 变体未知 | `demand=Some or Ok`, `variant_source=unknown` | recipe 必须显式指定变体，或判 Inconclusive |
| 断言在 helper 中 | `via_helper` + `needs_model_helper` | 内联调用轨迹后再写 recipe |
| 闭包/迭代器 | `scope=quantified` | 不进入单次 demand 计数；需量化模型 |
| 条件分支 | `control_flow=if-let/match` | 显式建模，或 Inconclusive |
| 控制流/生命周期无法重建 | `model_requirements` 记录 | 不猜 drop 时点 |
| setup helper 未展开 | 调用名保留 | 人工/项目 recipe 补全 |
| 冻结树不可回放 | `blocked` + 原因 | 保留需求，不进入首站运行面 |

## 8. 与 synthesize / divergence 的接口

1. `extract-ktest` 输出裸数组；`synthesize --candidates` 直接消费。
2. `family` 决定骨架；`constructibility` 决定无 recipe 时的跳过原因
   （`blocked` / `needs-recipe` 分别进合成报告）。
3. **预注册不写在提取器里**。候选不带 `prereg_expected`；运行前假设写在
   `KTEST-EXPECTED-M7B.md`，需要机器可查时由项目 recipe 的
   `expect` + `expect_basis` 显式覆盖（合成报告记录 `recipe-override`）。
4. `dedup_key=ktest:<id>`：不同测试即使调用同一 API 也不去重，因为
   输入/期望不同；如需合并由项目侧另写候选。
5. `obligation_draft` 只进合成报告，不执行。

## 9. 首站口径

- 提取面：冻结源全部 `#[ktest]`（原型 153 测试 → 139 非排除候选）。
- 运行面：只选**当前冻结树可表达且有项目 recipe** 的候选；其余
  `blocked`/`needs-recipe` 如实记录，不硬造。
- 差异审计：与原型 `examples/ktest-obligations/candidates.json` 按 ID/
  测试路径逐条对照；评级/需求计数差异必须逐条解释（类型推断、闭包
  作用域、控制流、解构模式、门控）。
- 运行命令、span 归因、baseline、expect_match 写入
  `examples/vostd-ktest/KTEST-RUN-M7B.md`；运行后工作副本逐字节恢复并
  核对 sha256。

## 10. 测试面

单测至少覆盖：mask/偏移、断言分类、变体推断（注解/构造/方法表/未知）、
闭包作用域、控制流、helper 内联、排除规则、门控（capture-gap/断连）、
函数推导、确定性输出。全测在 nix 内 `cargo test --offline` 通过。
