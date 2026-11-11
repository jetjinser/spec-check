# construction 探针端到端验证记录（wt-b，2026-11-11）

对象：vostd 冻结工作区副本 `/tmp/vostd-43da079-ktest-chan`（ktest 通道捕获
所用副本，rev 43da0797bbfa + ktest-channel 补丁）。manifest：
`examples/vostd-guard/tracked/manifest.json`。runner：`crates/
spec-check-analyses/src/vacuity.rs`（manifest v1.1，两阶段 construction
探针，接口契约 `docs/ANALYSIS-CONTRACT.md`）。

## 运行命令（可重放）

```
timeout 900 nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  'cd /tmp/spec-check-b && cargo build --offline && \
   ./target/debug/cargo-spec-check analyses-vacuity \
     --manifest examples/vostd-guard/tracked/manifest.json \
     --cargo-verus ~/vie/work/AES/vostd/tools/verus/source/target-verus/release/cargo-verus \
     --z3 ~/vie/work/AES/vostd/tools/verus/source/z3 \
     --vstd ~/vie/work/AES/vostd/tools/verus/source/vstd \
     --out /tmp/vac-construction-run3'
```

两阶段执行（runner 自动）：(1) 预编译回路
`cargo-verus check -p ostd --lib --target x86_64-unknown-none --
 --no-verify --cfg ktest --cfg ktest_capture_gap`（54 s，退出 0，
`precheck-build.log`）；(2) 验证构建
`cargo-verus build … --cfg ktest --cfg ktest_capture_gap`
（`target-build.log`）。注入模块 `ostd/src/spec_check_probes/` 在运行后
删除、`ostd/src/lib.rs` 恢复原字节（已核对与 `injections/lib.rs.orig`
逐字节一致）；`out/injections/` 保留全部注入文件副本与 `replay.json`。

## 逐探针判定 vs 预注册预期

| # | entry | 探针 | 预期 | 实际 | 一致? |
|---|-------|------|------|------|-------|
| 1 | `mutex_guard_new_construction` | construction（fab 面：新铸 `PCell` 权限调契约面 stub） | ConfirmedVacuous | **ConfirmedVacuous** | ✓ |
| 2 | `mutex_guard_new_holder_face` | construction（holder 面：`face_requires` 携带 `perm.id() == m.cell_id()`） | ConfirmedLive | **ConfirmedLive** | ✓ |
| 3 | `mutex_guard_new_quantifier_shape` | quantifier（M1 #7 形状：真实等式 + 见证 (0,0)） | ConfirmedLive（同 M1） | **ConfirmedLive** | ✓ |

机器证据（run 3，`report.json`/`target-build.log`）：

1. ostd 目标构建 **1498 verified / 1 errors**（基线 1497 + holder 面 1；
   唯一 error 即预期拒绝）：
   `error: precondition not satisfied`
   `--> ostd/src/spec_check_probes/mutex_guard_new_construction.rs:36:14`
   （调用点），secondary span 落在 stub requires
   `perm.id() == m.cell_id()`（第 26 行）——失败归因按 span 命中 fab 面
   entry fn 的构造调用。依赖逐 crate：vstd 2060/0、1908/44/737/1/23/12，
   与 ktest 捕获基线一致。
2. holder 面 entry fn 在同一构建中 verified（1498 之内），归因器在该
   entry 文件无任何 error 事件。
3. standalone quantifier 探针 crate：**2 verified / 1 errors**（空洞探针
   forall¬pre 被见证值否证 = `vacuous_probe_failed=true`；活性探针带见证
   verified）→ ConfirmedLive，与 M1 #7 判定一致。

## 修复迭代记录（预算 3，实际用 2）

1. run 1（`/tmp/vac-construction-run1`）：预编译回路拦截生成器缺陷——
   注入声明写成 `#[ktest]`（属性宏）而非 `#[cfg(ktest)]`，且模块文件缺
   `use vstd::prelude::*;`（`verus!` 宏不可见）。两处均为 `vacuity.rs`
   生成器修复（对既有 quantifier 路径零改动）。
2. run 2（`/tmp/vac-construction-run2`）：quantifier 对照判
   CandidateInconclusive ≠ M1 的 ConfirmedLive——根因是本次 manifest 把
   `perm_matches_cell` 声明成 uninterp，而 M1 manifest 是机械转写的真实
   等式（见证 (0,0) 可证）。manifest 修正为 M1 原形状；工具行为如实（
   uninterp 谓词上见证 assert 本就不可证，与 M1 #1–#4 结论一致）。
3. run 3：三条判定全部与预期一致，作为归档运行。

## 契约缺口（如实报告，不改契约文档）

1. **target 构建的 target 三元组缺失**：契约 schema 的
   `target_workspace` 无 `target` 字段，但 ostd 类 kernel crate 有
   `[target.<triple>.dependencies]`（acpi 等），host 构建必败（run 0
   预检实证：8 个 E0432/E0433）。实现增加可选 `target` 字段。
2. **额外 cfg 缺失**：ktest 通道副本的 14 个排除模块以
   `#[cfg(all(ktest, not(ktest_capture_gap)))]` 门控；仅传
   `gate_cfg=ktest` 会让它们全部复活并破坏构建。实现增加可选
   `extra_cfgs`（转发给两个构建阶段；注入模块本身仍只由 `gate_cfg`
   门控）。
3. **fab/holder 调用面无区分字段**：分类表区分"外部铸造调用被拒"
   （ConfirmedVacuous）与"holder 面调用通过"（ConfirmedLive），但 schema
   没有字段说明 entry fn 是哪个面。实现增加可选 `face_requires`
   （默认 false = fab 面，不携带 requires；true = holder 面，entry fn
   携带 `requires expect_fail_requires`）。
4. **可见性墙（结构性发现）**：真实 `MutexGuard::new` 是
   `sync::mutex` 模块私有（frozen 基线与本副本一致），任何注入模块
   （crate 根或 `sync/` 子模块）都无法命名它——Rust 可见性在 verus
   检查之前拦截，构造探针无法直接调用真实 stub。本验证的 fab/holder
   探针因此走**契约面 stub**（`external_body` + 发散体的最小转写，
   requires 逐字对应 mutex.rs:217–224，类型用真实 `Mutex<u64>` /
   `MutexGuard<'a, u64>` / vstd `PointsTo<u64>`，`Mutex::new` 无 ghost
   ensures、`cell_id()` `pub closed` 外部不可展开——忠实性论据与 M2
   spike 的模型相对性一致，见 `examples/tracked-modeling-spike/
   RESULT.md` 证据边界）。ConfimedVacuous 因此是**模型相对**判定：
   "经转写契约面的唯一可写铸造通道被 verifier 拒绝"；若目标项目真实
   API 可输出匹配 tracked 权限，判定应翻转。契约文档的 inject 机制
   未提及目标侧可见性约束，属缺口；合并时建议在契约文档补记。

## 与预测一致性的边界说明

- 判据为构造判据：空洞证据 = 唯一可写伪造通道被拒绝（span 归因）+
  通道穷尽的文档化论证（参数通道被调用面模型排除、铸造通道被运行拒绝
  、`assume_new` 不可从 verified 代码调用——M2 s1 归档）；非 SMT 层
  公理化。
- run 3 的注入文件、构建日志、report.json 均保留在
  `/tmp/vac-construction-run3/`；`examples/vostd-guard/tracked/
  manifest.json` 为归档 manifest。
