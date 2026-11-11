# 预注册预期（写于运行前，2026-11-11）

Manifest 固定后、运行 `analyses-vacuity` 之前写下。判定只以探针运行为准；
若实际与预期不符，如实记录，不改此文件迁就结果。

| # | entry | 预期判定 | 依据 |
|---|-------|----------|------|
| 1 | `mutex_guard_new_perm_cell_shape` | **ConfirmedVacuous** | audit 档案（research/experiments/spec-quality-two-day）判断 `MutexGuard::new` 的 stub 前置（`perm.id() == mutex.cell_id()`，Tracked permission）在调用面上恒假/不可达。本探针检验该断言在形状层是否被机器支持。 |
| 2 | `mutex_new_any_value_control` | **ConfirmedLive** | 阴性对照：`Mutex::new` 接受任意值（形状化为非平凡的值域谓词 `val_id < 8`，见证 0）。 |
| 3 | `arith_bounds_live_control` | **ConfirmedLive** | 阴性对照：简单算术前置 `i < len`，见证 `(0, 1)` 可满足。 |
| 4 | `arith_oob_vacuous_control` | **ConfirmedVacuous** | 阳性空洞对照：`i < len && i >= len` 恒假（spike 判据在本运行的再现）。 |
| 5 | `vmspace_cursor_query_shape` | **CandidateInconclusive** | 形状再现（shape reproduction, not literal contract）：tracked ghost 资源抽象为不透明 id，不变式谓词 uninterp——孤立探针既无法证 forall 否定，也无法给出见证。预期验证器不完备导致 Inconclusive，不计缺陷。 |
| 6 | `vmspace_cursor_jump_shape` | **CandidateInconclusive** | 同 #5。 |
| 7 | `vmspace_cursor_mut_map_shape` | **CandidateInconclusive** | 同 #5。 |

## 预期不匹配时的解读预案

- #1 若机器判 **ConfirmedLive**（相等谓词平凡可满足）：说明形状再现丢失了
  权限不可伪造性（PointsTo 是线性 ghost 资源，只能由 `PCell::new` 产出），
  audit 的"调用面不可达"是**上下文相关**判断，孤立形状探针无法确认也无法
  否证——按边界如实记录，不视为任一方被证伪。
- #5–#7 若意外得到 Confirmed*：检查 uninterp 谓词是否被意外退化，复核后再采信。
