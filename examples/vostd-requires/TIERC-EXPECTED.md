# TIERC-EXPECTED.md — M7e-ext Tier C 建模：预注册（探针运行前）

对象：`ostd` 全函数 requires 筛查的 Tier C 扩展轮。输入 census 与 screening
由本节所述命令生成，**规则与逐类期望在任何 Tier C 探针运行前确定**；运行结果只
写入 `TIERC-RUN.md`，不修改本文件。设计规则见 `docs/M7e-tierc-modeling.md`
（§3 G / §4 R,H / §7 T 规则）。本文是那份预注册的归档转录。

## 复现命令（生成阶段，不含探针）

```sh
cargo-spec-check census --vir /tmp/m7a-vir-export2/crate.vir --crate ostd \
  --source-root /tmp/vostd-43da079-ktest-chan --out /tmp/m7e-tierc-census
cargo-spec-check screen-requires --census /tmp/m7e-tierc-census/census.json \
  --out /tmp/m7e-tierc-screen --config examples/vostd-requires/config-tierc.json
```

配置：`examples/vostd-requires/config-tierc.json`
（`generic_instances: [{"match":"*","types":{"*":"u64", ...}}]`，
`reference.mode=opaque`，`ghost.mode=opaque`）。

## 分层事实（探测前）

| 层 | baseline（无 config） | config 后 | 变化 |
|---|---|---|---|
| Tier A | 9 | **12** | +3（泛型实例化后全机械） |
| Tier B | 89 | **633** | +544（C→B 抽象映射） |
| Tier C | 676 | **129** | −547 |
| 有 requires | 774 | 774 | — |
| 可运行候选 | 98 | **645** | +547 |

C→A/B 的机制（按函数，可重叠的机制只记首次组合；合计 547）：

| 机制 | C→A | C→B | 合计 |
|---|---|---|---|
| 仅 generic | 3 | 249 | 252 |
| 仅 reference | 0 | 137 | 137 |
| 仅 ghost | 0 | 8 | 8 |
| generic + reference | 0 | 87 | 87 |
| generic + ghost | 0 | 16 | 16 |
| generic + reference + ghost | 0 | 42 | 42 |
| reference + ghost | 0 | 5 | 5 |
| **合计** | **3** | **544** | **547** |

- 泛型实例化覆盖 505/505 个泛型有 requires 函数（`match:"*"` + `"*":"u64"` 默认）。
- 引用映射：316 个函数 / 364 个参数；ghost 映射：121 个函数 / 211 个参数。
- 仍为 Tier C 的 129 个函数：`ghost_param_unmapped` 115 个参数、
  `reference_param_unmapped` 66 个参数（函数级可重叠）；逐函数原因在
  `screening-tierc.json` 的 `functions[].reasons`。

## 预注册 P 规则（探针判定）

| 规则 | 条目形状 | 预期 | 意外结局预案 |
|---|---|---|---|
| P1 | Tier A，见证搜索成功（12/12 均有 witness） | **ConfirmedLive** | 非 Live → 生成器/触发推断回归，整批作废排查 |
| P2 | Tier A，搜索无见证 | **CandidateInconclusive** | ConfirmedVacuous = E6 真阳性，必须复核登记 |
| P3 | Tier B，纯 uninterp（633 条全部） | **CandidateInconclusive** | ConfirmedLive 在抽象模型下不应出现 → 视为模型异常并逐条解释；ConfirmedVacuous = 抽象形状恒假，是 E6 真阳性，必须复核登记 |
| P4 | 任一批编译失败 | 该批无效 | 修生成器后整批重跑，不逐条挑选 |
| P5 | Tier C | 不运行 | 不计缺陷、不计 Live；覆盖缺口如实报告 |
| P6 | baseline（无 config）复跑 | 9 A / 89 B / 676 C，逐函数 tier 与 M7e 归档一致 | 任何差异视为回归 |

**Inconclusive 不计缺陷。** 引用/ghost 映射的 id 不是真实引用值；其 Live 只说明
抽象可满足，不能当作真实 liveness；其 Vacuous 才是真阳性（relaxation 论证见
`docs/M7e-tierc-modeling.md` §6）。

## 逐类期望

### Tier A — 12 条，全部 ConfirmedLive

原有 9 条（与 M7e 相同，未重跑结论改写）加 3 条泛型实例化新条目：

| entry | function | params | requires | witness |
|---|---|---|---|---|
| `r0560_new_pt` | `ostd::mm::page_table::PageTableEntryTrait::new_pt` | `paddr: usize, max_paddr: usize` | `p560_k0(paddr, max_paddr)` | `(0usize, 1usize)` |
| `r1152_lemma_count_present_upto_bound` | `ostd::specs::mm::page_table::node::owners::lemma_count_present_upto_bound` | `n: usize` | `p1152_k0(n)` | `0usize` |
| `r1153_lemma_count_present_upto_monotone` | `ostd::specs::mm::page_table::node::owners::lemma_count_present_upto_monotone` | `n1: usize, n2: usize` | `p1153_k0(n1, n2)` | `(0usize, 0usize)` |

依据：泛型参数按配置实例化后，函数不再因 `generic` 阻塞；子句本就只引用标量
参数（`paddr`, `n`, `n1`, `n2`），保持 mechanical；因此按 Tier A 生成并做有界见证。
其余 9 条沿用 M7e 的 A 规则（见 `REQUIRES-EXPECTED.md`）。

### Tier B — 633 条，全部 CandidateInconclusive

| 类别 | 条数 | 依据 |
|---|---|---|
| generic-only（实例化后仍有 uninterp 子句） | 249 | G4：实例化不重解析子句文本，unsupported 仍按 uninterp |
| reference-only（安全 place 使用） | 137 | R2：id + 形状保留的 uninterp |
| ghost-only（安全 place 使用） | 8 | H1 |
| generic + reference | 87 | G4 + R2 |
| generic + ghost | 16 | G4 + H1 |
| generic + reference + ghost | 42 | G4 + R2 + H1 |
| reference + ghost | 5 | R2 + H1 |
| baseline Tier B（M7e 原有） | 89 | P3（未变） |
| **合计** | **633** | |

### Tier C — 129 条，不运行

全部因至少一个被引用参数存在**不安全使用**（`plain_root`、静态调用参数、
量化器/触发器等无法归因的访问），因此映射不成立：

- `reference_param_unmapped`：66 个参数实例（如 `self` 41、`this` 9、`pt` 3）；
- `ghost_param_unmapped`：115 个参数实例（如 `owner` 27、`perm` 19、`regions` 11）。

这些函数**不是「无缺陷」**，只是当前抽象规则拒绝映射。逐条原因见
`screening-tierc.json`。
