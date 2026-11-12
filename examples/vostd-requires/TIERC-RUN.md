# TIERC-RUN.md — M7e-ext Tier C 建模：运行归档（2026-11-12）

对象：`ostd` crate 1468 个 Exec/Proof 函数的 requires 全量筛查；VIR
`/tmp/m7a-vir-export2/crate.vir`，vostd rev 43da079，Verus 0.2026.10.08；
工具：`cargo-spec-check`（`spec-check-core::census` + `requires`）。
规则与逐类期望见 `TIERC-EXPECTED.md`（探针运行前确定，运行后未改写）与
`docs/M7e-tierc-modeling.md`。判定只以探针运行为准。

## 复现命令

```sh
# 1) census（新增 param_uses / param_occurrences / Old 特征）
cargo-spec-check census --vir /tmp/m7a-vir-export2/crate.vir --crate ostd \
  --source-root /tmp/vostd-43da079-ktest-chan --out /tmp/m7e-tierc-census

# 2) baseline（无 --config，M7e 回归）
cargo-spec-check screen-requires --census /tmp/m7e-tierc-census/census.json \
  --out /tmp/m7e-tierc-baseline --tier C

# 3) 配置筛查（12 A / 633 B / 129 C）
cargo-spec-check screen-requires --census /tmp/m7e-tierc-census/census.json \
  --out /tmp/m7e-tierc-screen --config examples/vostd-requires/config-tierc.json

# 4) 探针运行：13 批，前 12 批 50 条、最后 45 条
#    每批独立 900s 预算；命令模板：
cargo-spec-check screen-requires --census /tmp/m7e-tierc-census/census.json \
  --out /tmp/m7e-tierc-runs/batch-K \
  --config examples/vostd-requires/config-tierc.json \
  --batch K --batch-size 50 \
  --cargo-verus <SDK>/target-verus/release/cargo-verus \
  --z3 <SDK>/z3 --vstd <SDK>/vstd
```

运行环境：`nix develop ~/vie/work/AES/vostd/nix -c bash -c '...'`；
SDK = `/home/jinser/vie/work/AES/vostd/tools/verus/source`。
工具内部设置 `VERUS_USE_RUSTUP=0`、`VERUS_Z3_PATH`、`RUSTC_BOOTSTRAP=1` 等探针环境；每批独立 probe
crate。

## T1 回归：baseline 未变

无 `--config` 的 baseline 与 M7e 归档逐函数一致：

- 计数：**9 A / 89 B / 676 C**；Tier C 原因 `generic 505 /
  ghost_param 168 / reference_param 366`。
- 与 `examples/vostd-requires/screening.json` 对比：**0 个函数 tier 差异**。
- 说明：新增的 census 字段与 `old_or_final` 特征修正不影响分层。

## before → after

| 层 | baseline（M7e） | 本轮 config | 变化 |
|---|---|---|---|
| Tier A | 9 | **12** | +3 |
| Tier B | 89 | **633** | +544 |
| Tier C | 676 | **129** | −547 |
| 可运行候选 | 98 | **645** | +547 |
| 实际运行 | 98（历史） | **645** | 100% 本轮候选覆盖 |

C→A/B 的机制（547）：仅 generic 252（+3 A / 249 B）、仅 reference 137 B、
仅 ghost 8 B、generic+reference 87 B、generic+ghost 16 B、
generic+reference+ghost 42 B、reference+ghost 5 B。

建模量：

- 泛型实例化：505/505 个泛型有 requires 函数（配置 `match:"*"` + `"*":"u64"`，
  见 G5：不参与标量子句的类型参数被实例化为任意具体类型均是惰性的）。
- 引用映射：316 个函数 / 364 个参数（`self`、`this`、`order` 等）；
  记录为 `screening-tierc.json` 的 `modeling.reference_mapping`（含每参数
  `uses[{kind,callee,field,argc}]` 审计账本）。
- ghost 映射：121 个函数 / 211 个参数（`owner`、`perm`、`regions` 等），
  记录在 `modeling.ghost_mapping`。

## 探针运行（13 批，全部成功）

| 批 | 条目 | A/B | 墙钟 | probe summary |
|---|---|---|---|---|
| 0 | 50 | 5/45 | 35s | 55 verified, 95 errors |
| 1 | 50 | 0/50 | 33s | 50 verified, 100 errors |
| 2 | 50 | 1/49 | 34s | 51 verified, 99 errors |
| 3 | 50 | 0/50 | 35s | 50 verified, 100 errors |
| 4 | 50 | 2/48 | 33s | 52 verified, 98 errors |
| 5 | 50 | 0/50 | 34s | 50 verified, 100 errors |
| 6 | 50 | 0/50 | 34s | 50 verified, 100 errors |
| 7 | 50 | 0/50 | 33s | 50 verified, 100 errors |
| 8 | 50 | 0/50 | 32s | 50 verified, 100 errors |
| 9 | 50 | 1/49 | 34s | 51 verified, 99 errors |
| 10 | 50 | 2/48 | 33s | 52 verified, 98 errors |
| 11 | 50 | 1/49 | 32s | 51 verified, 99 errors |
| 12 | 45 | 0/45 | 32s | 45 verified, 90 errors |
| **合计** | **645** | **12/633** | **434s** | — |

每批 `dependency_summary` 均为 vstd `2060 verified, 0 errors`；13 批均首次
编译通过，无 manifest 修复迭代，单批远低于 900s。校准先导：在正式分批前用
`--filter 'ostd::cpu::set::impl'` 跑了 15 条（引用 `self_id` 与 ghost
`ordering_id` 等映射形状），15/15 `CandidateInconclusive`，确认生成器可编译后
再放量。

错误数解释与 M7e 相同：每条 manifest 条目生成 entry + vacuous/live 两个
proof fn。B 条目 entry 通过、两个探针失败（每 B 条目 1 verified + 2 errors）；
A 条目 entry+live 通过、vacuous 失败（每 A 条目 2 verified + 1 error）。

## 判定分布与 expect 一致性

| 批集合 | 条目 | 预注册 | ConfirmedVacuous | ConfirmedLive | CandidateInconclusive | Anomaly | 一致 |
|---|---|---|---|---|---|---|---|
| 全部 645 | 645 | A 12 Live / B 633 Inconclusive | **0** | **12** | **633** | **0** | **645/645** |

按层：A 12/12 `ConfirmedLive`；B 633/633 `CandidateInconclusive`。
`match` 字段逐条为 true；无 `Anomaly`，无 `ConfirmedVacuous`。

新增 3 条 Tier A（泛型实例化后全机械）全部 Live：

| entry | function | witness | 实际 |
|---|---|---|---|
| `r0560_new_pt` | `ostd::mm::page_table::PageTableEntryTrait::new_pt` | `(0usize, 1usize)` | ConfirmedLive |
| `r1152_lemma_count_present_upto_bound` | `ostd::specs::mm::page_table::node::owners::lemma_count_present_upto_bound` | `0usize` | ConfirmedLive |
| `r1153_lemma_count_present_upto_monotone` | `ostd::specs::mm::page_table::node::owners::lemma_count_present_upto_monotone` | `(0usize, 0usize)` | ConfirmedLive |

原有 9 条 Tier A 与本轮 baseline 完全一致，未出现新的 E6。

## 每一类建模的收益与代价

| 类 | 收益 | 代价 / 边界 |
|---|---|---|
| 泛型实例化 | 505 个函数脱离 `generic` 阻塞；252 个 gen-only 变为 B（249）或 A（3） | 不重解析子句文本：`T` 上的算术语义仍按 uninterp，因此主要是 B 收益；实例化表是项目侧知识（`config-tierc.json`） |
| 引用抽象 | 364 个引用参数 / 316 个函数进入 B（含 inv-only、字段、方法接收者） | id 无语义，clause 体仍 uninterp；不能给出真实引用见证；`plain_root`（直接比较/作为普通实参）全部拒绝 |
| ghost/tracked 抽象 | 211 个 ghost 参数 / 121 个函数进入 B | 同 id 抽象；quantifier/trigger 或直接使用仍拒绝；ghost 资源语义未被解释 |
| 组合 | 150 个函数同时用到两类以上抽象 | 仍全部 B/Inconclusive，不产生 Live 声称 |

对 M1 60 接口清单的影响（按 canonical path 映射）：42 行仍 `no_requires`；
18 行有 requires 的行中 **12 行 C→B、6 行仍 C**（3 个不同函数：
`page_table::node::impl&%2::lock`、`...::make_guard_unchecked`、
`sync::mutex::impl&%7::new`，均因 ghost/reference 直接使用）。没有 M1 条目进入
Tier A，M1 的既有判定不被改写。

## 仍为 Tier C 的 129 个函数

全部原因是「被引用参数存在不安全使用」，没有泛型阻塞：

- `ghost_param_unmapped`：115 个参数实例；单 ghost 67 个函数，ref+ghost 21 个；
- `reference_param_unmapped`：66 个参数实例；单 ref 41 个函数，ref+ghost 21 个；
- 高频名称：`self` 41、`this` 9、`pt` 3；ghost `owner` 27、`perm` 19、`regions` 11。

典型拒绝原因（逐条见 `screening-tierc.json`）：

- 引用作为普通实参/直接比较，而非字段、解引用或方法接收者，例如
  `sync::mutex::impl&%3::unlock` 的 `perm.id() == self.cell_id()`（两侧均为
  `plain_root`）；
- 旧值/槽位被直接传递给 spec 函数，例如 `vm_space_specs::activate_reader`
  的 `reader.wf(*old(owner_r))`（`reader` 直接实参）；
- ghost 权限值被整体传参，例如 `page_table::cursor::impl&%3::new` 的
  `pt_own` / `root_guard`。

这些不是「无缺陷」，是「当前保守规则拒绝映射」。下一轮若要继续缩小，应优先
定义显式的参数到 spec-fn 映射（项目侧），而不是放宽 id 抽象的 use 规则。

## 新缺陷与台账

- 本轮 **0 个 ConfirmedVacuous**：645 条全部符合预注册（A Live / B
  Inconclusive）。因此 research registry **不新增条目**；本轮只提供
  「抽象后的 Tier B 形状未出现可证恒假」的负面证据。
- 注意：不能把 0 Vacuous 解读为 129 个 Tier C 或 633 个 B 都健康。B 的
  Inconclusive 是模型不完备，不计缺陷；Tier C 未运行。

## 产物

- `screening-tierc.json`：1468 函数的 before/after 分层、每函数
  `baseline_tier/reasons`、`modeling`（实例化匹配、引用/ghost 映射及 use 账本）、
  645 条候选的预注册预期。
- `manifest-tierc.json`：645 条 quantifier manifest（A 12 + B 633）。
- `runs-tierc/verdicts.json`：13 批合并后的逐条 verdict/expected/match；
  原始 probe crate、日志、编译产物在 `/tmp/m7e-tierc-runs/batch-{0..12}/`。
- `config-tierc.json`：项目侧实例化与策略开关。
- `TIERC-EXPECTED.md`：预注册规则与逐类期望。

## 边界

- 引用/ghost 的 `ConfirmedLive` 不会出现；即使出现也只是抽象可满足，不能当作
  真实 liveness。`ConfirmedVacuous` 才是真阳性（relaxation 论证）。
- 类型替换是字符串级、惰性的；不建模 trait bound、布局、`size_of`。
- 引用/ghost 的字段与方法访问只作为审计账本，不进入子句语义；同一参数在不同
  子句中的访问不保证关联（每个子句是独立 uninterp）。
- 未做：跨批结果缓存、夜班 `check` 默认接入、Tier C 的显式 spec-fn 映射表。
- 运行未修改冻结源 `/tmp/vostd-43da079-ktest-chan`；探针在独立 crate 构建。
- 批次预算：13 批 × 900s 上限，实际总墙钟 434s；无 timeout、无编译失败。
