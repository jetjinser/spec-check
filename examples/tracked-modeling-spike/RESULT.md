# M2 spike：tracked 资源建模构造探针归档（2026-11-11）

设计笔记：`docs/M2-tracked-modeling.md`（预测写于运行前）。本文件记录
路线对比、逐探针运行结果、与预测的出入、结论与生成器实现建议。

## 运行方法（可重放）

```
nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  'cd ~/vie/work/AES/spec-check/examples/tracked-modeling-spike && \
   VERUS_USE_RUSTUP=0 \
   VERUS_Z3_PATH=~/vie/work/AES/vostd/tools/verus/source/z3 \
   RUSTC_BOOTSTRAP=1 \
   ~/vie/work/AES/vostd/tools/verus/source/target-verus/release/cargo-verus build'
```

vstd 依赖 2060 verified / 0 errors（本目录 `probe-build.log`）。
健全性子 crate：同命令，目录 `s1-assume-new/`（日志
`s1-assume-new/probe-build.log`）。

## 逐探针运行结果 vs 先验预测

主 crate `verification results:: 4 verified, 2 errors`，逐项核对：

| 探针 | 预测（笔记先写） | 实际 | 一致? | 判定 |
|---|---|---|---|---|
| `b1_guard_new_flat_vacuous_should_fail`（forall∀¬pre，真实 `PointsTo<u64>`） | failed（被铸造反例否定） | **failed**：`postcondition not satisfied` 于 forall ensures | ✓ | 路线 (b) 空洞方向不可达 |
| `b2_guard_new_flat_live`（exists + 铸造见证） | verified → ConfirmedLive | **verified** | ✓（形态修正见下） | **ConfirmedLive**：契约在孤立 vstd 语义下固有可满足 |
| `model::mutex_new`（铸造+埋藏模型） | — | **verified** | — | 模型自身良构 |
| `c1_external_caller_guard_attempt`（外部伪造唯一通道：新铸 PCell 配对后调 stub） | failed：`failed this precondition` 于 `perm.id() == cell_id(m)` | **failed**：`precondition not satisfied`，失败 span 精确落在 `model::guard_new` 的 requires `perm.id() == cell_id(m)`（lib.rs:88）并指向调用点 lib.rs:105 | ✓ | **ConfirmedVacuous（构造判据）** |
| `c2_holder_face_guard_call`（持有 perm 的调用面） | verified → ConfirmedLive | **verified** | ✓ | **ConfirmedLive** |
| `c3_pcell_read_external_caller`（阴性对照：自铸自用 read，同形 tracked requires） | verified → ConfirmedLive | **verified** | ✓ | **ConfirmedLive**——阴性对照零误报 |
| `s1_assume_new_fabrication_attempt`（assume_new 直接伪造） | 编译拒绝或 id-match 验证失败（记 (A)/(B)） | **编译拒绝**（(A)）：`error: 'verus_builtin::impl&%6::assume_new' is not supported`，逐字归档 | ✓（命中 (A)） | 伪造原语不可从 verified 代码调用 |

**成功标准达成：c1 = ConfirmedVacuous（构造判据）且 c2/c3 = ConfirmedLive。**

## 与预测的出入（如实记录，不改预测）

1. **b2 模式修正**：笔记标注的不确定点被运行证伪——verus 拒绝在
   `proof fn` 体内调用 exec 函数（`error: cannot call function
   vstd::cell::pcell::impl&%3::new with mode exec`）。b2 改为 exec fn +
   `proof { assert(...) }` 关闭 exists（见证项来自 exec 体的铸造）。
   实质预测（铸造见证使 exists 可证）不变、成立。
2. **`cell_id` 必须 `pub closed`**：私有字段上的 `pub open spec fn` 被
   verus 拒绝（`disallowed: field expression for an opaque datatype`）。
   与 vostd 自己的选择一致（`sync/mutex.rs:43` `pub closed spec fn
   cell_id`）；closed 恰好也是忠实语义——外部调用者可调用但不可展开。
3. **s1 移出主 crate**：编译级拒绝会毒化整 crate 的验证（首轮运行已证
   实：4 个 E0425 使其余探针全部未验证）。s1 独立成 expected-fail 子
   crate，其运行日志即证据。
4. 其余全部命中，c1 的失败归因 span 与预测完全一致。

## 路线对比结论

- **(a) uninterp `forgeable` 建模：不可行（先验否定，M1 机器证据佐证，
  未另跑）。** uninterp 谓词只能给模型集合加元素：空洞方向
  `∀id: forgeable(id)` 不可证，活性方向见证穿过不透明谓词亦不可证 →
  结构性 CandidateInconclusive；要得到 ConfirmedVacuous 必须公理化
  铸造集为空，那正是结论本身（违反建模纪律）。M1 中 requires 全
  uninterp 的条目 #5/#6 全落 CandidateInconclusive 与此一致。
- **(b) 真实 tracked 类型 + 平面量词探针：不能产生 ConfirmedVacuous，
  且给出了 V-001 分歧的精确刻画。** 空洞 forall 在 vstd 语义下被
  `PCell::new` 的 ensures 配对反例直接否证（b1 failed，verus 拒绝证假
  命题——可靠性在位）；活性方向铸造见证通过（b2 verified）。即：
  **`perm.id() == mutex.cell_id()` 作为孤立契约是固有可满足的，
  ConfirmedLive 是该语义层的正确判定**。audit 的恒假是分布性判断
  （perm 由 `Mutex::new` 内部铸造并埋进锁不变式，外部无来源），平面
  量词探针原则上无法表达"无来源"。
- **(c) 源级构造探针：可行（本次 spike 主体）。** 判据从"量词证明"
  换成"构造"：恒假 = 唯一可写伪造通道被 verifier 拒绝（c1，失败归因
  于被预测的 requires）+ 通道穷尽的结构性论证（外部 tracked 项只能
  来自参数或铸造；参数通道被调用面模型排除——`c1` 签名只取
  `&MutexModel`，忠实于 vostd 外部调用者面；铸造通道被运行拒绝；伪造
  原语 `assume_new` 被 verus 编译级阻断，s1 归档）。活性 = 构造成功
  （c2 holder 面、c3 外部可构造对照，全 verified）。
  **(c) 不含任何直接假设结论的公理**：不可伪造性不是被断言的，而是
  verifier 对 tracked 项纪律的判定行为本身。

**对 V-001 的含义**（registry 更新在 research 仓库，本次不动）：
M1 的 ConfirmedLive 与 audit 判读并不矛盾，而是两个不同问题的答案；
构造探针问的是 audit 的问题，并在机器上得到与 audit 一致的判定
（ConfirmedVacuous，限定于"外部调用者"调用面模型）。spike 内模型是
最小化忠实转写（Mutex::new 无 ghost 契约、字段私有、perm 埋藏），
非 vostd 原文。

## 证据边界

- (c) 的 ConfirmedVacuous 是**模型相对**判定：相对 spike 中转写的铸造
  链契约（`mutex_new` 无 ghost ensures、埋藏 perm 私有）。若目标项目
  的真实 API 输出匹配的 tracked 权限，判定应翻转——通道穷尽论证依赖
  manifest 提供的 API 面描述。
- 通道穷尽本身是文档化论证，不是机器验证的定理；机器证据是"唯一可写
  通道被拒绝"（c1）+ "伪造原语被阻断"（s1）。
- `assume_specification` 可给 `Tracked::assume_new` 挂任意 spec 使伪造
  重新可行（verus 报错提示原文即含此建议）。这是项目侧的信任决定，
  等价于注入假公理；生成器必须把探针 crate 中出现 tracked 构造器的
  `assume_specification` 视为异常（见下节建议 4）。
- b1/b2 使用真 `vstd::cell::pcell::PointsTo`；c 模型同样基于真 PCell，
  未引入自定义 tracked 类型。

## 对 `vacuity.rs` 生成器的实现建议（M3+ 实施）

1. **manifest 条目增加探针种类字段** `"probe"`（默认 `"quantifier"`，
   保持 M1 行为不变）：`"construction"` 条目提供 `model_preamble`
   （含 tracked 模型的 verus 源，注入 `verus!{` 之后，机制与现有
   preamble 相同）、`params`（调用面模型：外部调用者的可见参数）、
   `construct`（构造体源码）、`expect_fail_requires`（被预测失败的
   requires 源码文本，用于归因）。
2. **生成器**（`generate_probe_lib`）：`construction` 条目只生成
   `pub fn entry_<i>_<name>(<params>) { <construct> }`，不生成
   pre/vacuous/live 三件套；`quantifier` 条目照旧。params 含真实
   tracked 类型（`Tracked<PointsTo<u64>>` 等）时 forall 量词发射原样
   可用（b1 已验证可编译可判定）；tracked 活性见证不能写在 proof 模式
   （mode-exec 拒绝），有 `"live_construct"` 字段时按 b2 形态生成
   exec fn + proof 块，否则跳过活性探针（记 Inconclusive）。
3. **分类器**（`classify`）：failure 标记集合补充
   `precondition not satisfied` / `failed this precondition`；construction
   条目按 span 归因——失败行落在 `expect_fail_requires` 对应 span（或
   其调用点）→ ConfirmedVacuous；该条目无失败 → ConfirmedLive；失败在
   别处 → CandidateInconclusive / Anomaly。现有 span 行匹配
   （trim_start 后取 `--> src/lib.rs:` 行号）可复用。
4. **信任边界**：发射探针 crate 前检查生成源与 model_preamble 中不得
   含对 tracked 构造器的 `assume_specification` 声明；出现即 Anomaly
   （伪造被重新打开，判定失效）。
5. **报告 detail**：construction 判据的 ConfirmedVacuous 在 detail 中
   注明证据形态（"fabrication channel rejected + channel-exhaustion
   argument (documented)"），与量词判据区分。
