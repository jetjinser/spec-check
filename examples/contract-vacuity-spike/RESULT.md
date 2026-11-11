# 里程碑①：判定判据 spike（2026-11-11）

探针 crate（本目录）经 cargo-verus（vostd SDK rev，`VERUS_USE_RUSTUP=0
VERUS_Z3_PATH=.../z3 RUSTC_BOOTSTRAP=1`，包元数据 `verify = true`）运行：

- `dead_entry_is_vacuous`（恒假契约的空洞证明）：**verified** → Confirmed 空洞
- `live_entry_is_live`（活性证明 + 显式见证 `s = 0`）：**verified** → Confirmed 非空洞
- `dead_entry` / `live_entry`（契约定义）：verified
- `live_entry_is_vacuous_should_fail`（对可满足契约的空洞声明）：**failed（预期）** → 判据正确区分

## 判定判据（里程碑②③遵循）

- 空洞证明（`forall !pre`）验证成功 → **Confirmed 空洞**
- 活性证明（`exists pre` + 显式见证）验证成功 → **Confirmed 非空洞**
- 两者皆失败 → **Candidate**（验证器不完备，需人工；不计为缺陷）

## 已知实现注意

- 量化器需可推断触发：谓词写成具名 `spec fn` 应用（字面比较会报
  "Could not automatically infer triggers"）。
- `exists` 证明需要在函数体里显式 `assert(P(witness))`（SMT 不自动合成见证）。
- cargo-verus 默认不验证 primary 包：需包元数据
  `[package.metadata.verus] verify = true`。
- 环境变量：`VERUS_USE_RUSTUP=0`、`VERUS_Z3_PATH=.../z3`、
  `RUSTC_BOOTSTRAP=1`（与 vostd 捕获环境一致）。
