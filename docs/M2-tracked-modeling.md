# M2 设计笔记：tracked 资源建模（2026-11-11）

状态：设计完成，spike 见 `examples/tracked-modeling-spike/`（运行结果以
该目录 RESULT.md 为准；本笔记的预测写于运行之前，若与运行结果不符，
以 RESULT.md 的如实记录为准）。

## 问题定义

`MutexGuard::new`（vostd `sync/mutex.rs:217–228`）stub 契约
`perm.id() == mutex.cell_id()`（`perm: Tracked<PointsTo<T>>`）在 M1 形状
探针（usize id 抽象）判 ConfirmedLive，audit 静态判读为调用面恒假，
registry V-001 保留 `rule_classified_probe_disputed`。根因：`PointsTo`
权限的**不可伪造性**是 term-level 性质，不是契约逻辑形式上的一阶性质：

- `PointsTo` 是 `tracked struct` + `external_body`（vstd
  `cell/pcell.rs:99–105`）：无公开构造器，verified 代码中唯一的铸造来源是
  `PCell::new`（`external_body`，ensures `pt.1@.id() == pt.0.id()`，铸造
  发生在函数内部）。唯一可以"无中生有"产生 tracked 值的
  `Tracked::assume_new()` 标记 `#[cfg_attr(verus_keep_ghost,
  verifier::external)]`（builtin `src/lib.rs:505`），只在 external_body
  函数体内使用。
- vostd 的铸造链：`Mutex::new`（`sync/mutex.rs:53–64`）**内部**调
  `PCell::new` 并把 perm 埋进原子不变式（`lock: AtomicBool<_,
  Option<PointsTo<T>>, _>`，wf 不变式 `g is Some ==> g->0.id() ==
  val.id()`），API 不输出任何 tracked 值。外部调用者因此**结构上拿不到**
  匹配的 perm——audit 的"调用面恒假"是这个**分布性/可达性**判断，不是
  契约等式的可满足性判断。
- 抽象为 `perm_id: usize` 后，等式 `perm_id == mutex_id` 平凡可满足——
  usize 无来源（provenance），这正是抽象丢失的信息。

**核心论点（三条路线共同的评价基准）**：不可伪造性在 SMT 一阶层面
**不可公理化**——它不是"id 上任何可写的约束"，而是"verified 项宇宙中
不存在相应构造项"。因此任何在 manifest 层用谓词/公理"重建"不可伪造性
的尝试，要么退化为不可判定（Inconclusive），要么直接假设结论（把铸造集
为空写成公理/ensures，违反建模纪律）。唯一能利用该性质的层面是
**verus 自己的项纪律**（路线 c）：verifier 对 tracked 项的线性类型检查
本身就是不可伪造性的机械实现。

## 路线 (a)：uninterp spec fn 建模铸造集 —— 否定

建模：manifest 增加 `tracked_stubs`（如 `pub uninterp spec fn
forgeable(id: usize) -> bool`），契约 requires 增补铸造来源约束
`!forgeable(perm_id)`。

判定分析（先验）：

- **空洞方向**：`forall mutex_id, perm_id: !(perm_id == mutex_id &&
  !forgeable(perm_id))` 等价于 `forall id: forgeable(id)`。`forgeable`
  是 uninterp，SMT 对其任何取值模型都自洽（取恒真即反例），谓词**不可
  证** → 空洞探针必败。
- **活性方向**：需要见证 `(w, w)` 使 `!forgeable(w)` 可证——对不透明
  谓词同样不可证 → 活性探针必败。
- **结论**：双败 → CandidateInconclusive，不是 ConfirmedVacuous。这
  不是运行偶然，是结构性的：**uninterp 谓词只能给模型集合加元素（增加
  可满足性），永远不会支持否定方向的证明**；要让 `¬pre` 可证必须钉死
  `forgeable` 的行为，而钉死为空集（`forall id: !forgeable(id)` 作公理）
  恰好就是 audit 的结论本身（"外部调用者拿不到任何权限"）——违反建模
  纪律，排除。
- **经验佐证**：M1 全量运行（`examples/vostd-guard/FULL-RUN.md`）中
  requires 全为 uninterp 谓词的条目 #5/#6 全部落 CandidateInconclusive，
  与本分析一致。
- **对照误报风险**：无（标量对照不受影响）。但目标判定无法达成。

**结论：(a) 不可行。** 不为它另跑 spike——M1 已含等价形状的机器证据。

## 路线 (b)：vstd ghost 结构（真实 tracked 类型 + 平面量词探针）—— 预期否定，spike 顺带验证

建模：探针 crate 链接 vstd，`PointsTo<T>` 用真类型进参数与量词：

```verus
pub open spec fn pre(cell_id: CellId, perm: PointsTo<u64>) -> bool {
    perm.id() == cell_id
}
proof fn vacuous() ensures forall|cell_id: CellId, perm: PointsTo<u64>| !pre(cell_id, perm) {}
proof fn live() ensures exists|cell_id: CellId, perm: PointsTo<u64>| pre(cell_id, perm) { /* 见证 */ }
```

判定分析（先验）：

- **空洞方向**：`forall cell_id, perm: perm.id() != cell_id` 在 vstd
  语义下是**假命题**——`PCell::new` 的 ensures 直接给出配对反例。可靠的
  验证器必然拒绝 → 空洞探针必败（这正是真实语义的忠实反映）。
- **活性方向**：需要 `PointsTo<u64>` 的见证**项**。verified 代码里唯一
  的构造项是铸造：在 proof 体内 `let (cell, Tracked(perm)) =
  PCell::new(5u64);` 然后 `assert(pre(cell.id(), perm))`。verus 的
  proof 代码可以包含 exec 调用（执行时擦除），故预期可证 → 活性探针
  预期通过 → **ConfirmedLive**。
- 不确定点（运行检验）：proof fn 内调 exec `PCell::new` 是否被 verus
  接受；tracked struct 作 forall/exists 绑定类型与触发推断是否顺利。
- **对照误报风险**：低——活性方向要么 ConfirmedLive 要么
  Inconclusive，不会把可满足契约误判为空洞。

**结论（预期）：(b) 不能产生 ConfirmedVacuous——但这本身是重要结果**：
它把 V-001 的层级分歧从"形状抽象丢失信息"精确化为"**契约在孤立 vstd
语义下固有可满足（ConfirmedLive 是正确判定），audit 的恒假是分布性
（铸造被 Mutex::new 内部消费）判断，平面量词探针原则上无法表达**"。
spike 中作为对照组运行，记录机器事实。

## 路线 (c)：源级构造探针（construction probe）—— 最有希望，spike 主体

建模：探针 crate 含**忠实的最小模型**（两模块，字段私有对齐 vostd）：

- `model::MutexModel { cell: PCell<u64>, perm: Tracked<PointsTo<u64>> }`
  ——字段私有（外部模块碰不到埋藏的 perm，复刻 vostd 字段隐私）；
  `model::mutex_new(val)` 内部 `PCell::new` 铸造并埋藏，**无任何 ghost
  ensures**（忠实于冻结源 `Mutex::new` 无 `#[verus_spec]`：调用者对
  埋藏 perm 一无所知）；
- `model::guard_new(m, Tracked(perm)) -> GuardModel`，`external_body`
  stub 面，requires `perm.id() == cell_id(m)`（MutexGuard::new 形状原样）。

探针判据从"量词证明"换成"**构造**"：

- **空洞**（c1）：外部调用者世界（crate root 模块）尝试唯一可写的伪造
  通道——铸造新配对并调用 stub：
  `let (_cell2, Tracked(pt2)) = PCell::new(7u64);
  model::guard_new(m, Tracked(pt2))`。**预期**：verus 拒绝——
  `pt2.id() == cell_id(m)` 不可证（新铸造的 id 与已有 mutex 的 cell id
  无任何可证关系；`Mutex::new` 铸造链在模型内、其 ghost 事实不外泄），
  报 `failed this precondition` 于该 requires → 按**构造判据**记
  ConfirmedVacuous。
- **活性对照**（c2，holder 面）：持有匹配 perm 的调用面
  （`requires perm.id() == cell_id(m)` 直接给参数，复刻 vostd 内部
  try_lock 调用者从锁不变式取出 perm 的情形）调 stub——预期通过 →
  ConfirmedLive。
- **阴性对照**（c3，外部可构造的 tracked 契约）：`PCell::read` 形状
  （requires `self.id() == perm.id()`）——调用者自铸自用，预期通过 →
  ConfirmedLive。证明该判据不把"可构造的 tracked 契约"误报为空洞：
  MutexGuard::new 的恒假性来自埋藏，不来自 tracked 类型本身。
- **健全性探针**（s1）：尝试 `Tracked::assume_new()` 直接伪造后调 stub。
  预期：要么编译拒绝（external 项不可从 verified 代码调用），要么验证
  失败于 id-match（assume_new 无 ensures）。任一结果都保全不可伪造性；
  如实记录实际形态。

**证据形态的诚实边界**：c1 的空洞证据 = "唯一伪造通道被 verifier 拒绝"
+ **通道穷尽的结构性论证**（外部 tracked 项只能来自参数或铸造；参数通道
被调用面模型排除，铸造通道被运行拒绝）——通道穷尽本身是文档化论证，
不是机器验证的定理。这与 (a)/(b) 试图在 SMT 层公理化不可伪造性不同：
(c) 复用 verus 自己的可靠性机制（tracked 线性纪律），把"不可伪造"
从待证命题变为 verifier 的判定行为。若 (c) 成立，对 manifest 生成器的
实现建议见 RESULT.md。

**对照误报风险**：主要风险在 c3（若 verus 拒绝合法的 read 调用形状，
则阴性对照误报 = 路线失败，如实记录）。

## 先验预测汇总（运行前写定）

| 探针 | 预测 | 判据归点 |
|---|---|---|
| b1 `forall ... !pre`（真实 PointsTo） | failed | 被铸造反例否定 |
| b2 `exists ... pre`（铸造见证） | verified → ConfirmedLive | 孤立语义固有可满足 |
| c1 外部铸造后调 guard_new | failed（`failed this precondition` 于 id-match） | ConfirmedVacuous（构造判据） |
| c2 持有 perm 调 guard_new | verified → ConfirmedLive | holder 面 |
| c3 自铸自用 read | verified → ConfirmedLive | 阴性对照零误报 |
| s1 assume_new 伪造调用 | 编译拒绝或 failed 于 id-match | 健全性边界 |

成功标准：c1 = ConfirmedVacuous（构造判据）且 c2/c3 = ConfirmedLive。
若 c3 误报或 c1 通过验证（不可伪造性被击穿），按失败如实记录。

## 对 manifest 生成器的含义（spike 验证后回填）

见 `examples/tracked-modeling-spike/RESULT.md` 末节。
