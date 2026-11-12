# M7a — 自研 VIR 规格普查（spec census）

本文件先于实现与校准运行写定。它定义 census 的输入、语义规则、JSON
形态、CLI 与校准方法；校准的实际数字只回填到
`examples/vostd-census/CALIBRATION.md` 的 “Actual” 节，不改写本文和该校准
文档的预注册节。

## 1. 定位

普查是自有漏斗的顶端：只输出**事实**，不判定缺陷。

- 输入：一个 Verus crate 的 VIR（`--log vir` 产物），以及该 crate 的名字。
- 输出：每个 Exec/Proof 函数的
  - 输出列表（返回值 / `#[verus_spec]` 注入的 ghost 返回值 / `&mut` 参数），
  - 每个输出是否被有效后置条件提及，
  - 后置条件来源（own / trait declaration / 两者 / 无），
  - `external_body`、trait、可见性、源码位置、requires 摘要。
- 不做：不注入探针、不调用验证器、不产生 “缺陷” 判定。未提及只是候选
  事实，供 M7b/M7c/M7d 使用。

工具代码不含任何项目专有路径或统计；`ostd` 只作为 `--crate` 输入和
`examples/vostd-census/` 的校准材料。

## 2. VIR 事实来源

| census 字段 | VIR 来源 |
|---|---|
| 函数路径 | `Function :name (Fun :path P)` |
| 模式 | `:mode Exec` / `:mode Proof`（`Spec` 排除） |
| 源码位置 | 外层 `(@ "file:line:col: line:col (#N)" ...)` |
| 可见性 | `:visibility (Visibility :restricted_to None|...)` |
| external body | `:attrs (FunctionAttrs ... :is_external_body true ...)` |
| trait impl | `:kind (FunctionKind TraitMethodImpl :method (Fun :path M) :trait_path T ...)` |
| trait 声明 | `:kind (FunctionKind TraitMethodDecl/TraitMethodDefault ...)` |
| own requires | `:require (...)` |
| own ensures | `:ensure (tuple (regular...) (defaults...))` |
| own returns | `:returns <expr>`（`None` 表示没有） |
| 返回值 | `:inner_ret` / `:outer_ret` 的 `Param` 类型与名字 |
| `&mut` 参数 | `:params` 中类型含 `Typ MutRef` 的 `Param`（含 `Tracked<&mut _>`、`Ghost<&mut _>` 与 `#[verus_spec] with` 参数） |
| 返回元组分量 | own/defer 的 `ensure` 首个子句 `Stmt Decl :pattern (Pattern Constructor (Dt Tuple N) ...)` |
| 类型不变式 | `Datatype :user_defined_invariant_fn (FunWithVis ... :fun (Fun :path ...::type_inv))` |

### 2.1 `#[verus_spec]` 的两类记录

Verus 对 `#[verus_spec]` 可能同时产生：

- 原路径的 wrapper（通常 `:body None`、契约被简化）；
- `::_VERUS_VERIFIED_<name>` 的真实验证体（完整契约与 body）。

census 以 `::_VERUS_VERIFIED_` 记录为准，去掉该段后作为**规范路径**
（declared path）。没有该记录时使用原路径记录。这样：

- 输出与 artifact 的函数键（VIR 路径）可比；
- 不会把 wrapper 的空契约误判为 “无后置条件”。

### 2.2 返回元组分量的识别

当 `#[verus_spec]` 把 “实际返回值 + `with ->` ghost 返回值” 打包成元组时，
该函数**自己的** ensures 首个子句会出现顶层解构：

```
(Stmt Decl :pattern (Pattern Constructor (Dt Tuple N) tuple%N
  ((-> 0 (Pattern Var (PatternBinding :name (VarIdent r ...) :typ T0 ...)))
   (-> 1 (Pattern Var (PatternBinding :name (VarIdent g ...) :typ T1 ...)))
   ...)))
```

处理规则：

- 找到该 pattern 时，每个分量是一个输出（第 0 个是实际返回值，其余是
  ghost 返回值）；若第 0 个分量的类型是 unit，则它不产生输出。
- 找不到该 pattern 时，返回值整体是一个输出（类型非 unit 时），不拆解
  源码层面的 tuple 返回类型。
- 对 trait impl，输出数量与名字取自 **impl 自己的签名/pattern**；trait
  declaration 的 pattern 只用于继承后的提及扫描，不用来拆分 impl 的
  返回值（否则 `item_into_raw` 一类 `(post decl)` 的外部体 impl 会被
  declaration 的 4 元组误拆成 4 个输出，而 artifact 记为 1 个）。
- 无论哪种情况，每个 `&mut` 参数都是独立输出。

## 3. 语义规则（逐条对应同事 ensures-coverage 规则）

| # | 规则 | census 实现 |
|---|---|---|
| S1 | 输出 = 返回值、`#[verus_spec]` 返回元组的每个分量、每个 `&mut` 参数 | §2.2 + `Typ MutRef` 扫描 |
| S2 | 提及 = ensures 中出现返回值的名字，或写了 `returns` | 有效 regular ensures 的 `VarIdent` 名集合；`returns` 存在则实际返回值提及 |
| S3 | `&mut` 参数 x 被提及 = ensures 某处出现 `final(x)`；只写 `old(x)` 不算 | 仅收集 `UnaryOp MutRefFuture (MutRefFutureSourceName Final)` 子树里的 `VarIdent` |
| S4 | trait 方法实现继承 trait 声明（`(post decl)`）的 ensures/returns | `TraitMethodImpl :method` → 查找同 VIR 的 `TraitMethodDecl/TraitMethodDefault` 函数；合并其 regular ensures/returns 用于**提及扫描**，并用声明侧的返回/参数名匹配 impl 的合成名（`%return`/`_VERUS_ret_ident`） |
| S5 | `default_ensures` 不算后置条件 | 只取 `:ensure` tuple 的第 1 分量（regular）；第 2 分量仅用于 “mentioned only in default_ensures” 注记 |
| S6 | 条件式 ensures 也算提及（哪怕条件恒假） | 不求解条件，纯语法扫描全部 regular clauses |
| S7 | 只提类型不变式需标注 “Type invariant only” | 未提及输出的顶层类型有 `user_defined_invariant_fn` 时置 `type_invariant_only=true` |
| S8 | external_body 需标注 | `:is_external_body` |
| S9 | 无 postcondition（只有 requires 也算无）单独分组 | `present = own_regular>0 \|\| own_returns \|\| decl_regular>0 \|\| decl_returns` |
| S10 | 一个输出的 finding 类别 | return/ghost → `return_unmentioned`；`&mut` → `mut_state_unmentioned` |

### 3.1 后置条件来源标签

- `own`：own regular ensures 或 own `returns` 非空；
- `decl`：仅 trait declaration 的 regular ensures/returns 非空；
- `own_decl`：两者都非空；
- `none`：都没有；
- `decl_unavailable`：trait impl 的声明函数不在该 VIR 中（artifact 对这类
  实现按 trait method 合并；census 如实标注，不用推测补齐）。

### 3.2 注记字段

- `default_ensures_mention`：输出仅在 `default_ensures` 中出现；仍是
  finding，但附注来源。
- `type_invariant` / `type_invariant_only`：见 S7。
- `ghost`：`&mut` 参数或返回分量被 `Tracked`/`Ghost` 装饰。
- `named`：返回值的源码名字；`%return`/`_VERUS_ret_ident` 视为未命名。

## 4. JSON schema（`census.json`）

```json
{
  "schema": "spec-check.census/1",
  "vir": {"path": "...", "sha256": "..."},
  "crate": "ostd",
  "source_root": "...",
  "scope": "Exec and Proof functions whose VIR path starts with `<crate>::`; _VERUS_VERIFIED_* canonicalized to declared paths; Spec functions excluded.",
  "counts": {
    "functions": 0, "exec": 0, "proof": 0,
    "pub": 0, "non_pub": 0,
    "external_body": 0, "trait_impl": 0, "trait_decl": 0,
    "decl_unavailable": 0,
    "with_postcondition": {
      "functions": 0, "outputs": 0, "findings": 0,
      "return_unmentioned": 0, "mut_state_unmentioned": 0,
      "type_invariant_only": 0
    },
    "without_postcondition": {
      "functions": 0, "outputs": 0, "findings": 0,
      "return_unmentioned": 0, "mut_state_unmentioned": 0,
      "type_invariant_only": 0
    }
  },
  "functions": [
    {
      "path": "ostd::...",
      "name": "method",
      "mode": "exec|proof",
      "kind": "static|trait_impl|trait_decl|trait_default",
      "visibility": {"kind": "pub|non_pub", "restricted_to": null},
      "source": {"span": "...", "file": "...", "line": 0},
      "external_body": false,
      "trait": {"path": "...", "method": "...", "post_source": "own|decl|own_decl|none|decl_unavailable"},
      "pattern": {"found": true, "source": "own", "components": 2},
      "module": "...",
      "postcondition": {
        "present": false,
        "source": "own|decl|own_decl|none|decl_unavailable",
        "own_clauses": 0, "decl_clauses": 0, "default_clauses": 0,
        "returns": false, "decl_returns": false,
        "default_ensures_only": false,
        "decl_unavailable": false,
        "requires_count": 0,
        "requires_preview": ["..."]
      },
      "outputs": [
        {
          "kind": "return|ghost_return|mut_param",
          "name": "r",
          "mention_name": "r",
          "named": true,
          "type": "(Typ ...)",
          "ghost": false,
          "mentioned": false,
          "mention": "variable|returns|final|none",
          "default_ensures_mention": false,
          "type_invariant": "type_inv",
          "type_invariant_only": false,
          "finding": "return_unmentioned|mut_state_unmentioned|null"
        }
      ],
      "findings": ["return_unmentioned"]
    }
  ],
  "limits": ["..."]
}
```

`requires_preview` 只保留前 3 条、每条截断到 300 字符；完整 VIR 在
输入文件中。

M7e 追加的 additive 字段（schema 仍为 `spec-check.census/1`，旧消费者
不受影响）：每个函数新增 `generics {count,names}`、`params
[{name,type,kind,mode}]`、`requires [{index,span,file,line,text,class,ast,
ast_vars,ast_consts,vars,consts,calls,features}]`。`class=mechanical` 表示
该子句可机械转写为标量/布尔表达式（附结构化 `ast`）；`text` 在提供
`--source-root` 时按 VIR span 从源码切出（多行 span 也支持）。这些是
筛查输入事实，不含缺陷判定。详见 `docs/M7e-requires-screening.md`。

## 5. CLI

```
cargo-spec-check census --vir FILE --crate NAME --out DIRECTORY [--source-root DIRECTORY]
```

- 纯分析，不注入、不验证；不解析目标源码（`--source-root` 用于把
  `source.file` 变为相对路径，并在 M7e 中按 VIR span 切出 requires
  原文）。
- 输出 `DIRECTORY/census.json`；stdout 打印汇总。
- `cargo-spec-check` 与 `spec-check-core` 的二进制入口都注册该命令。

## 6. 校准方法（先写定）

基准：`/tmp/jinser-bzyA6G/ensures-coverage/`（同事 ensures-coverage artifact）。

1. 解析 `head.results` 的 S 表达式逐函数事实（path、mode、`(outs N)`、
   `(post own|decl|own decl)`、`(trait ...)`、`external-body`、可见性）。
2. 解析 `all.md` 的逐函数 findings（含 trait declaration 合并/实现组），
   按函数名映射回 `head.results` 的 VIR 路径。
3. 以 **VIR 路径交集**逐函数比较：
   - 输出数量 `outs`；
   - 后置条件存在性与来源；
   - finding 集合（`return_unmentioned` / `mut_state_unmentioned` + 输出名），
     计算 precision/recall 与完全一致率；
   - 汇总表（有/无后置条件的函数/输出/findings）。
4. 差异分类（预注册 ID）：
   - `D-FN`：函数集合差异（修订/重命名/新增/删除）；
   - `D-SIG`：同名函数签名变化导致输出数量不同（含 `with ->` ghost 输出、
     `&mut` 参数增删）；
   - `D-VERUS`：Verus 版本导致的 VIR 编码差异（wrapper、tuple pattern 等）；
   - `D-DECL`：trait 声明不在 VIR 中（`decl_unavailable`）；
   - `D-DEFAULT`：`default_ensures` 注记/计数的差异；
   - `D-TYPEINV`：类型不变式注解差异；
   - `D-PARSER`：artifact 的 196 个源码解析器缺口（census 以 VIR 为键，
     不受该缺口影响，但显示名/类型可能不同）；
   - `D-MENTION`：提及规则差异；
   - `D-NONE`：无法归类的剩余差异。
5. 预注册期望（运行前）：
   - 输出数量一致率（交集）≥ 95%；
   - 后置条件存在性一致率（交集）≥ 98%；
   - finding 集合完全一致率 ≥ 90%，`D-MENTION`/`D-NONE` 合计 ≤ 10%；
   - 汇总数字受修订差异影响，预计与 artifact 的差距在函数数 ±5%、
     输出/ findings ±10% 量级；所有差距逐类给出实例。
6. 归档：`examples/vostd-census/` 下保存
   - `CALIBRATION.md`（预注册 + Actual + 差异实例）；
   - `census-summary.json`（计数 + findings；完整 census 路径记录在文档）；
   - `RUN.md`（VIR 导出命令、工具命令、版本事实）。

## 7. 修订/版本事实

- 校准 artifact 的 VIR 来自工作流 ref
  `b6db6df974cf0af881255cf88613836613fc6ce4`，运行时 Verus
  `0.2026.11.11.04fc24f`。
- 本地冻结快照 `43da0797b`（`borrowed-repr` 分支，工作树干净）可用本地
  Verus `0.2026.10.08.d128dda` 导出 VIR；census 在 43da079 上运行。
- 我们尝试在精确修订 b6db6df 上导出 VIR：该修订要求 Rust 1.99.1；
  本地 SDK 为 Rust 1.98.1/Verus 0.2026.10.08，依赖 `x86_64 0.15.5`
  与旧 `Step` trait 不兼容；对 `x86_64` 做临时 patch 后又在
  `vstd_extra` 的 `std_specs::{nonzero, ptr}` 导入处失败（旧 vstd 缺
  新 API）。因此精确修订 VIR 未产出，校准按路径交集进行，修订差异
  逐类解释。
