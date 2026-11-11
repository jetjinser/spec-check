# vostd 真实契约 contract-vacuity 探针归档（2026-11-11）

对象：vostd 冻结源 `/tmp/vostd-43da079-ktest-chan/ostd/src`（rev 43da0797bbfa）。
`EXPECTED.md` 为运行前预注册；本文件记录运行命令、逐契约机器判定、与预注册
预期的对比及证据边界。

## 运行命令

```
timeout 600 nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  'cd ~/vie/work/AES/spec-check && \
   ./target/debug/cargo-spec-check analyses-vacuity \
     --manifest examples/vostd-guard/manifest.json \
     --cargo-verus .../vostd/tools/verus/source/target-verus/release/cargo-verus \
     --z3 .../vostd/tools/verus/source/z3 \
     --vstd .../vostd/tools/verus/source/vstd \
     --out /tmp/vac-vostd'
```

验证结果（`/tmp/vac-vostd/probe-build.log`）：vstd 依赖 2060 verified / 0
errors；探针 crate 11 verified / 10 errors。10 个失败逐一对应下表探针
（vacuous_0/1/2、live_3、vacuous_4–6、live_4–6），与判定完全一致。

## 逐契约判定 vs 预注册预期

| # | entry（冻结源出处） | 预期 | 实际 | 一致? |
|---|--------------------|------|------|-------|
| 1 | `mutex_guard_new_perm_cell_shape`（sync/mutex.rs:217–228，`MutexGuard::new`，requires `perm.id() == mutex.cell_id()`） | ConfirmedVacuous（依 audit：stub 前置在调用面恒假/不可达） | **ConfirmedLive** | ✗ 不一致 |
| 2 | `mutex_new_any_value_control`（sync/mutex.rs:53–64，`Mutex::new` 阴性对照） | ConfirmedLive | ConfirmedLive | ✓ |
| 3 | `arith_bounds_live_control`（简单算术阴性对照） | ConfirmedLive | ConfirmedLive | ✓ |
| 4 | `arith_oob_vacuous_control`（算术矛盾阳性空洞对照） | ConfirmedVacuous | ConfirmedVacuous | ✓ |
| 5 | `vmspace_cursor_query_shape`（mm/vm_space.rs:566–589，`Cursor::query`） | CandidateInconclusive | CandidateInconclusive | ✓ |
| 6 | `vmspace_cursor_jump_shape`（mm/vm_space.rs:670–688，`Cursor::jump`） | CandidateInconclusive | CandidateInconclusive | ✓ |
| 7 | `vmspace_cursor_mut_map_shape`（mm/vm_space.rs:909–930，`CursorMut::map`） | CandidateInconclusive | CandidateInconclusive | ✓ |

6/7 与预期一致；#1 不一致，解读见下。未出现 Anomaly。

## #1 不一致的解读（如实记录，不改预期）

audit 档案（research/experiments/spec-quality-two-day）判断 `MutexGuard::new`
的 stub 前置在调用面上恒假/不可达；预注册据此预期 ConfirmedVacuous。机器对
形状谓词 `perm_matches_cell(perm_id, mutex_id) := perm_id == mutex_id` 给出
ConfirmedLive（空洞探针 `forall` 失败于见证 `(0,0)`，活性探针验证成功）。

两者并不矛盾，而是层级不同：真实前置要求持有**线性 ghost 权限**
`Tracked<PointsTo<T>>` 且 `perm.id() == mutex.cell_id()`；该权限不可伪造
（只能由 `PCell::new` 产出），所以 audit 的"不可达"是**调用面上下文相关**
判断——外部调用者拿不到权限。形状再现只保留了相等谓词的逻辑形式，抽象
id 上相等显然可满足，因此孤立形状探针**既不能确认也不能否证** audit 的
不可达断言。EXPECTED.md 的解读预案预判了这一结局；结论：#1 的机器判定
证实形状抽象丢失权限所有权信息，audit 断言仍需调用图级别的证据（未在本
里程碑内验证），不据此修改任何预期。

## 证据边界（重要）

- **#1、#5–#7 均为 shape reproduction, not literal contract**：真实契约的
  tracked 参数（`Tracked<PointsTo<T>>`、`Tracked<&mut CursorOwner<...>>`、
  `Tracked<&mut MetaRegionOwners>`、`Tracked<&mut Guards>`、
  `Tracked<EntryOwner<...>>`、`Tracked<&mut TlbModel>`）无法用普通 manifest
  参数表达，全部抽象为不透明 usize id。
- #5–#7 的不变式谓词（`CursorOwner`/`MetaRegionOwners`/`Guards` 等的
  `invariants`、各 `*_panic_condition`、`item_wf`、`TlbModel::inv`）为
  uninterp 声明：冻结定义涉及深层 ghost 状态（页表视图、RCU owner、TLB
  模型），未在本 manifest 复现。因此其 CandidateInconclusive 是**验证器
  对不透明形状的不完备**，按判据不计为缺陷；要获得可判定探针需链接
  vstd_extra/vostd 的完整 ghost 模型（后续里程碑）。
- `may_panic` 按冻结源原文声明为 uninterp
  （`verified_libs/vstd_extra/src/panic.rs:11`），此处忠实转写。
- #2 的 `Mutex::new` 在冻结源中无显式 `#[verus_spec]` requires；对照形状
  （值域谓词 `val_id < 8`）是建模假设，非原文契约。
- 判定判据沿用 spike（examples/contract-vacuity-spike/RESULT.md）：
  空洞证明通过 ⇒ ConfirmedVacuous；活性证明+见证通过 ⇒ ConfirmedLive；
  双败 ⇒ CandidateInconclusive（验证器不完备，不计缺陷）。

## 工具修正（运行期间发现并修复，均在 spec-check 仓库内）

1. `crates/spec-check-analyses/src/vacuity.rs` 生成器：多参数见证绑定原先生成
   `let a, b = ...;`（非法 Rust），改为多参数时生成元组模式
   `let (a, b) = ...;`。
2. 同文件分类器：span 行匹配原为 `strip_prefix("  --> src/lib.rs:")`（恰好
   两个空格）；verus 会按最大行号宽度对齐 `-->` 标记，3 位数行号时带三个
   空格，导致失败归因丢失、误报 Anomaly。改为先 `trim_start` 再匹配。
   修复前首轮运行 6/7 误报 Anomaly，修复后与日志逐探针核对一致。

Manifest 首轮即通过 cargo-verus 编译与触发推断，未消耗修复迭代。
