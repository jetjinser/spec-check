# M7a census × ensures-coverage 校准

本文的 “Pre-registration” 节在运行自研 census 与校准脚本之前写定；
“Actual” 节只在运行后回填，不改写预注册内容。

## 0. 基准事实（来自同事 artifact，非本工具输出）

- artifact：`/tmp/jinser-bzyA6G/ensures-coverage/`
  - `head.results`：1502 个 exec/proof 函数，S 表达式逐函数事实。
  - `all.md`：全量报告；汇总（有后置条件）1270 函数 / 1335 输出 /
    42 findings（return 16 / `&mut` 26）；（无后置条件）232 函数 /
    193 输出 / 193 findings（return 133+25 / `&mut` 34+1）。
  - `check.out`：196 个函数源码解析器找不到，名字按 VIR 推断。
- artifact VIR 修订：`b6db6df974cf0af881255cf88613836613fc6ce4`；
  Verus `0.2026.11.11.04fc24f`。
- 本地 census 运行修订：`43da0797bbfa4085749efee9585231e43c656a98`
  （`vostd-pr830`，干净）；Verus `0.2026.10.08.d128dda`；
  VIR 由 `cargo dv focus --targets ostd -- --no-verify --log vir` 导出
  （43da079 的 dv CLI；b6db6df 的较新 dv 使用 `-p`）。

## 1. 预注册：实现语义

见 `docs/M7a-spec-census.md` §2–§3。要点：

- 输出：实际返回值 + `#[verus_spec]` 注入的 ghost 返回值（顶层解构
  pattern 的每个分量；unit 实际返回值不计）+ 每个 `&mut` 参数。
- 提及：返回/ghost 输出 = 有效 regular ensures 中出现其名字，或实际返回
  有 `returns`；`&mut` = 仅在 `final(x)` 中出现才算。
- 后置条件：own regular ensures 或 own `returns`；trait impl 合并
  declaration 的 regular ensures/returns；`default_ensures` 不算。
- 类型不变式仅作为未提及输出的注记，不改变 finding 类别。
- `_VERUS_VERIFIED_*` 规范化为声明路径，并以验证体记录为准。

## 2. 预注册：校准指标与期望

按 VIR 路径交集逐函数比较：

| 指标 | 预注册期望 |
|---|---|
| 函数集合 | 报告交集/ours-only/artifact-only；受修订差异影响，不设硬阈值 |
| 输出数量 `outs` 一致率 | ≥ 95% |
| 后置条件存在性一致率 | ≥ 98% |
| finding 集合完全一致率 | ≥ 90% |
| `D-MENTION` + `D-NONE` | ≤ 10% 的差异 |
| 汇总数字 | 函数数差 ≤ 5%，输出/findings 差 ≤ 10%（修订差异） |

预注册差异类别：

- `D-FN`：函数集合差异（修订/重命名/新增/删除）。
- `D-SIG`：同名函数签名变化（`with ->` ghost 输出、`&mut` 参数增删）。
- `D-VERUS`：Verus 版本/编码差异（wrapper、tuple pattern）。
- `D-DECL`：trait 声明不在 VIR 中。
- `D-DEFAULT`：`default_ensures` 注记/计数差异。
- `D-TYPEINV`：类型不变式注解差异。
- `D-PARSER`：artifact 的 196 个源码解析器缺口（VIR 键不受影响）。
- `D-MENTION`：提及规则差异。
- `D-NONE`：无法归类。

## 3. Actual

运行：2026-11-12。census 输入为 43da079（`vostd-pr830`）导出的 ostd VIR；
基准为同事 artifact（b6db6df 修订，Verus 0.2026.11.11.04fc24f）。逐函数
比较以 VIR 路径为键。

### 3.1 本次运行事实

- census：1468 函数（exec 826 / proof 642；pub 1202 / non-pub 266）；
  有后置条件 1268 函数 / 1311 输出 / 52 findings（return 15、`&mut` 37）；
  无后置条件 200 函数 / 171 输出 / 171 findings（return 146、`&mut` 25）。
- artifact：1502 函数（exec 895 / proof 607；pub 1214 / non-pub 288）；
  有后置条件 1270 / 1335 / 42（return 16、`&mut` 26）；
  无后置条件 232 / 193 / 193（return 158、`&mut` 35）。
- 路径交集 1210；ours-only 258；artifact-only 292。
- 196 个 artifact “源码解析器缺口”（`inferred`）函数中，188 个在本次
  census 中按 VIR 路径存在；8 个因修订差异不在 43da079。
- `decl_unavailable` 6：均为自带后置条件的
  `FallibleVmRead/Write::{read,write}_fallible` impl；其 trait 声明不在
  43da079 VIR 中。artifact 报告 70 个声明缺失的 impl 并按其 trait method
  合并；本次 census 如实保留 own 结果。
- `default_ensures_only` 3：`AnyFrameMeta::is_untyped` 声明 + 两个 impl。
  artifact 只在声明行标 `default-only`，但两者都把 `default_ensures` 排除在
  后置条件之外，且都产生 return finding。

### 3.2 汇总对比

| 指标 | artifact | ours (43da079) | 差 | 相对 |
|---|---:|---:|---:|---:|
| 函数 | 1502 | 1468 | -34 | -2.3% |
| exec | 895 | 826 | -69 | -7.7% |
| proof | 607 | 642 | +35 | +5.8% |
| pub | 1214 | 1202 | -12 | -1.0% |
| non-pub | 288 | 266 | -22 | -7.6% |
| 有后置条件函数 | 1270 | 1268 | -2 | -0.2% |
| 有后置条件输出 | 1335 | 1311 | -24 | -1.8% |
| 有后置条件 findings | 42 | 52 | +10 | +23.8% |
| — return findings | 16 | 15 | -1 | -6.2% |
| — `&mut` findings | 26 | 37 | +11 | +42.3% |
| 无后置条件函数 | 232 | 200 | -32 | -13.8% |
| 无后置条件输出 | 193 | 171 | -22 | -11.4% |
| 无后置条件 findings | 193 | 171 | -22 | -11.4% |
| — return findings | 158 | 146 | -12 | -7.6% |
| — `&mut` findings | 35 | 25 | -10 | -28.6% |
| Type invariant only（有/无） | 0 / 26 | 0 / 26 | 0 | 0% |

其他事实：ours external_body 129（artifact 127）；trait impl 246
（artifact 266）；trait decl 72（artifact `(module ...)` 76）。

### 3.3 交集指标 vs 预注册

| 指标 | 实际 | 预注册期望 | 结论 |
|---|---:|---:|---|
| 函数集合 | 交集 1210 / ours 258 / artifact 292 | 报告，不设阈值 | — |
| 输出数量一致率 | 1202 / 1210 = **99.34%** | ≥ 95% | 通过 |
| 后置条件存在性/来源一致率 | 1203 / 1210 = **99.42%** | ≥ 98% | 通过 |
| finding count 一致率 | 1192 / 1210 = **98.51%** | — | — |
| finding 集合完全一致率（类别+名字） | 1192 / 1210 = **98.51%** | ≥ 90% | 通过 |
| 类别相同但名字/集合不同 | 0 | — | — |
| D-MENTION/D-NONE 规则差异 | 0 / 1210 | ≤ 10% | 通过 |
| 汇总函数数差 | -2.3% | ≤ 5% | 通过 |
| 汇总输出/ findings 差 | 有后置：-1.8% / +23.8%；无后置：-11.4% / -11.4% | ≤ 10% | **部分未通过** |

预注册的汇总容差有两处未完全满足：有后置条件 findings +23.8%、无后置条件
输出/findings -11.4%。这两处均由**函数集合与契约文本的修订差异**解释，
不是交集上的语义实现差异（见 3.4/3.5）；交集上的三项核心指标全部通过。

### 3.4 差异分类与实例

受影响的交集函数共 21 个（18 个 finding 数不同、8 个输出数不同、7 个后置
条件来源不同；类别完全一致，无 D-MENTION/D-NONE 规则差异）。全部可追溯到
b6db6df↔43da079 的源码签名或契约文本变化，归为 `D-SIG`。

#### D-FN（函数集合）

- artifact-only 292：`ostd/src/arch/x86` 56 个（例如
  `ostd::arch::kernel::apic::Apic::id`、`ApicTimer::timer_current_count`、
  `ostd::arch::ex_table::read_table`）、`ostd/specs/mm/page_table` 53、
  `ostd/specs/mm/virt_mem.rs` 28、`ostd/src/mm/frame` 61、
  `ostd/src/sync/*` 35。43da079 的 arch/kernel 模块被 cfg 掉，或这些
  函数在该修订中尚未加入/已改名。
- ours-only 258：`ostd/specs` 130（源码 span 形如
  `ostd/src/../specs/...`，已归一化为 `ostd/specs/...`；函数路径仍按
  VIR 原样比较）、`ostd/src/mm/frame` 62、`ostd/src/sync/rcu` 20、
  `ostd/src/boot/memory_region.rs` 13 等；属于 43da079 新增/改名的函数。
- 196 个 artifact `inferred` 函数：188 个在 ours 中存在（VIR 原生支持，
  没有源码解析器缺口）；8 个缺失示例：`ostd::boot::impl&%6::clone`、
  `ostd::arch::task::impl&%3::clone`、
  `ostd::specs::arch::VERUS_layout_of_usize` 等。

#### D-SIG（同名函数的签名/契约变化）

代表性证据（b6db6df → 43da079）：

1. `LinkedList::pop_front`（artifact outs 5，ours 3）：
   b6db6df 的 `#[verus_spec]` 有 `with -> frame_owner: Tracked<...>` 和
   `Tracked(owner): Tracked<&mut LinkedListOwner<M>>`；43da079 把
   owner 改为按值 `Tracked<LinkedListOwner<M>>`，返回类型改为
   `Option<(UniqueFrame, Tracked<...>)>`，没有 `with ->` ghost 输出。
   因此 ours 少 2 个输出，且 43da079 的 ensures 不再提 `final(self)`，
   多出 `self` finding。
2. `LinkedList::cursor_back_mut`（artifact 0 findings，ours 1）：
   b6db6df 的 ensures 含 `*final(self) == *final(r.list)`；43da079 改为
   返回 `(CursorMut, Tracked<CursorOwner>)` 并重写 ensures，未提
   `final(self)` → ours 的 `self` finding 是 43da079 的真实事实。
3. `LinkedList::contains`（artifact outs 2/0 findings，ours outs 4/3
   findings）：b6db6df 使用 `Tracked<&MetaRegionOwners>` /
   `Tracked<&LinkedListOwner<M>>` 共享引用并写 `returns ... final(self)`；
   43da079 改为 `Tracked<&mut ...>` 且没有 `returns`，ours 因此多 2 个
   `&mut` 输出和 3 个 findings。
4. `RcuOptionReadGuard::get`、`RcuOptionReadGuard::compare_exchange`、
   `RcuReadGuard::get/compare_exchange`、`RcuReadGuard::read_with`、
   `Mutex::lock`（artifact 有 post own/0 findings，ours 无 post/1
   return finding）：b6db6df 增加了
   `#[verus_spec(r => ensures r.inv())]`（或等价 ensures），43da079 没有；
   ours 的“无后置条件 + return finding”是该修订的事实。
5. `dfs_acquire_lock`（artifact 0 findings，ours 1 `cur_node` finding）：
   43da079 增加 `A: InAtomicMode` 泛型并改写契约文本；签名/契约均变化。
6. `create_user_page_table` / `VmSpace::new`（artifact 2 findings，ours 1）：
   43da079 的 ensures 提及返回 `r`（或新增 `regions/guards` 约束），
   artifact 修订中 `r` 未提及；输出数也因 `&mut`/ghost 参数不同而变。
7. `from_pte`、`to_ref`（artifact outs 1，ours 2）：b6db6df 的
   `regions` 是共享引用；43da079 改为 `Tracked<&mut MetaRegionOwners>`，
   ours 多一个 `&mut` 输出（该参数在 43da079 的 ensures 中没有 `final`）。

其余 D-SIG 路径：`cursor_at_ghost_mut`、`cursor_front_mut`、
`cursor_mut_at`、`pop_back`、`push_back`、`push_front`、
`take_current`（linked-list 同族修改）。

#### D-DECL（trait 声明缺失）

ours 6 个 `decl_unavailable`：
`ostd::mm::io::impl&%10..12::read_fallible`、
`ostd::mm::io::impl&%13..15::write_fallible`。这些 impl 自带后置条件，
`post_source` 仍为 `own`；trait 声明（`FallibleVmRead/Write`）在
43da079 VIR 中不作为函数记录出现。artifact 报告 70 个声明缺失的 impl，
按 trait method 合并；本次没有用推测补齐，而是显式标记。

#### D-DEFAULT（`default_ensures`）

`AnyFrameMeta::is_untyped`：declaration 的 regular ensures 为空、default
ensures 有 1 条并提及 `res`。census：`postcondition.present=false`、
`default_ensures_only=true`、return finding、`default_ensures_mention=true`；
artifact 同样产生 finding 并标注 “Mentioned only in default_ensures”。
两个 impl 继承该 default-only 状态；artifact 只在声明行标
`default-only`，这是**注解粒度差异**，不影响 finding。

#### D-TYPEINV（类型不变式）

总数为有后置 0 / 无后置 26，与 artifact 完全一致。artifact 的 26 个中
25 个是 return、1 个是 `&mut`（`XApic::enable`，属于 artifact-only 的
apic 模块）；ours 的 26 个全部是 return。差异只来自函数集合，不是
类型不变式识别规则差异。实例：`AtomicCpuSet::new` →
`type_invariant_only=true`、`type_invariant="type_inv"`。

#### D-PARSER

artifact 的 196 个 `inferred` 函数中，188 个在 ours 中；8 个因修订缺失。
census 不使用源码解析器，函数键直接来自 VIR，因此不存在“196 个找不到”
的同类缺口；代价是显示名是 VIR 短名（`name`），不是 artifact 的
demangled 名称。

#### D-VERUS / D-MENTION / D-NONE

- `D-VERUS`：精确修订 b6db6df 的 VIR 未能在本地导出（见 3.5），所以
  无法单独量化 Verus 0.2026.10.08↔0.2026.11.11 的编码差异。但在
  43da079 上，`::_VERUS_VERIFIED_` wrapper 规范化、注入 tuple pattern
  和 `Tracked`/`Ghost` 参数均按预期工作；交集上的 21 个差异全部可由
  源码修订解释，没有观察到独立的编码差异。
- `D-MENTION`：0 个规则差异；18 个 finding 差异全部是修订后的 ensures
  文本变化（归入 D-SIG）。
- `D-NONE`：0。

### 3.5 精确修订 VIR 的尝试（失败）

为消除修订差异，我们按 artifact 的 workflow ref
`b6db6df974cf0af881255cf88613836613fc6ce4` 做了两次导出尝试：

1. `git fetch upstream b6db6df...` 成功；在 `/tmp/vostd-b6db6df` 建
   worktree，初始化 `dv` 子模块到 `5ba6ba4e...`。
2. 使用本地 Verus 0.2026.10.08 / Rust 1.98.1 执行
   `cargo dv focus -p ostd -- --no-verify --log vir`：
   `x86_64 0.15.5` 的 `forward_overflowing`/`backward_overflowing`
   不是 Rust 1.98.1 `Step` trait 的成员（artifact 修订要求 Rust 1.99.1）。
3. 临时 patch 掉这两个方法后重试：`vstd_extra` 在
   `use vstd::std_specs::{cmp::*, nonzero::*, ptr::*};` 处失败——本地
   vstd 0.2026.10.08 没有 `std_specs::{nonzero, ptr}`（artifact 使用
   Verus 0.2026.11.11）。

结论：精确修订 VIR 需要 Rust 1.99.1 + Verus 0.2026.11.11；本地没有该
组合。因此校准按路径交集进行，21 个差异逐条归因到修订/版本变化。
b6db6df 的临时 worktree 仅用于源码对照，未修改 vostd 工作树文件。

### 3.6 预注册偏差说明

- 预注册的“汇总输出/findings 差 ≤10%”未完全满足（有后置 findings
  +23.8%，无后置输出/findings -11.4%）。原因：
  - 有后置 findings 增加来自 43da079 中 18 个函数的真实 `&mut`/return
    未提及（D-SIG；artifact 修订的 ensures 更强或签名不同）。
  - 无后置函数少 32 个：artifact 的 196 个解析器缺口与 apic/ex_table
    等模块在 43da079 中不存在或已加后置条件；这些函数的输出/findings
    随之减少。
- 交集上的核心指标（输出数 99.34%、后置来源 99.42%、finding 集合
  98.51%）满足预注册；没有 D-MENTION/D-NONE 规则差异。

### 3.7 归档

- `census-summary.json`：本次 census 的计数、逐函数输出/finding 计数和
  223 条 findings（完整 `census.json` 2.7 MB，未入库；路径
  `/tmp/m7a-census-43da079/census.json`）。
- `CALIBRATION.json`（运行产物，未入库；可由 `calibrate.py` 重放）：
  `/tmp/m7a-calibration.json`。
- 导出/运行命令与版本事实见 `RUN.md`。
