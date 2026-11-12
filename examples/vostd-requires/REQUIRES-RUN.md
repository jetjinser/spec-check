# REQUIRES-RUN.md — vostd 全函数 requires 筛查：首站运行归档（2026-11-12）

对象：`ostd` crate 的 1468 个 Exec/Proof 函数（census 全量）。VIR：
`/tmp/m7a-vir-export2/crate.vir`，vostd rev 43da079，Verus 0.2026.10.08；
生成器：`cargo-spec-check screen-requires`（M7e，`spec-check-core::requires`）。
预注册规则与逐条预期见 `REQUIRES-EXPECTED.md`（**写于运行前，运行后未改写**）；
设计见 `docs/M7e-requires-screening.md`。判定只以探针运行为准。

## 复现命令

```sh
# 1) 扩展 census（秒级，含 requires 结构化事实与源码 span 文本）
cargo-spec-check census --vir /tmp/m7a-vir-export2/crate.vir --crate ostd \
  --source-root /tmp/vostd-43da079-ktest-chan --out /tmp/m7e-census

# 2) Tier A 校准批（9 条）
cargo-spec-check screen-requires --census /tmp/m7e-census/census.json \
  --out /tmp/m7e-run-a --tier A \
  --cargo-verus <SDK>/target-verus/release/cargo-verus \
  --z3 <SDK>/z3 --vstd <SDK>/vstd

# 3) Tier B 两批（45 + 44）
cargo-spec-check screen-requires --census /tmp/m7e-census/census.json \
  --out /tmp/m7e-run-b1 --tier B --batch 0 --batch-size 45 \
  --cargo-verus <SDK>/target-verus/release/cargo-verus --z3 <SDK>/z3 --vstd <SDK>/vstd
cargo-spec-check screen-requires --census /tmp/m7e-census/census.json \
  --out /tmp/m7e-run-b2 --tier B --batch 1 --batch-size 45 \
  --cargo-verus <SDK>/target-verus/release/cargo-verus --z3 <SDK>/z3 --vstd <SDK>/vstd
```

运行环境：`nix develop ~/vie/work/AES/vostd/nix -c bash -c '...'`；
SDK = `/home/jinser/vie/work/AES/vostd/tools/verus/source`；
每批独立 900s 预算，实际耗时（含 nix 进入与构建）：**A 35.9s / B1 38.1s / B2 37.5s**，
单批从未接近上限。三批均首次编译通过，未消耗 manifest 修复迭代。

校准批说明：vostd 的 Tier A 实际只有 9 条（严格的「全机械 + 非泛型 +
非 ghost/tracked + 非引用」规则），少于步骤建议的 30–60 条；因此 Tier A
全量作为校准批直接运行，没有把 Tier B/C 混入 Tier A 来凑数。

## 漏斗统计（census 全量）

| 层 | 数量 | 说明 |
|---|---|---|
| 函数总数 | 1468 | Exec 826 / Proof 642 |
| 有 requires | 774 | 进入分层判定 |
| 无 requires | 694 | 空合取，前置平凡可满足；不是筛查缺口 |
| Tier A | 9 | 全机械子句；11 条子句 |
| Tier B | 89 | 含 uninterp；230 条子句 |
| Tier C | 676 | 不可筛查；2266 条子句 |
| 可运行候选 | 98 | A 9 + B 89 |
| 实际运行 | 98 | A 9 + B1 45 + B2 44，100% 候选覆盖 |

Tier C 阻塞原因（按函数去重，可重叠）：
generic **505**、ghost_param **168**、
reference_param **366**。
原因组合：gen-only 252 / ref-only 143 / gen+ref 113 / gen+ghost+ref 94 /
gen+ghost 46 / ghost+ref 16 / ghost-only 12。**Tier C 不视为「无缺陷」**。

参数形态（774 个有 requires 的函数，参数实例计数）：int 531、bool 7、opaque 669、
reference 481、ghost/tracked 368；按函数去重：含 ghost 175、含引用 388、泛型 505。
这解释了 Tier C 的主导来源：泛型与引用覆盖面远大于标量层。

子句层（census 全量）：2507 条 requires 子句 = mechanical 97 / unsupported 2410；
源码 span 文本恢复 2391 条（95.4%，多行 span 已支持；失败项保留 span 与 VIR 预览）。
unsupported 特征计数（可重叠）：call 2130、field_access 968、quantifier 146、
unary_opr 117、trigger 80、ctor 72；`old`/`final` 未在 requires 顶层出现。

## 机器判定汇总

| 批 | 条目 | 预注册 | ConfirmedVacuous | ConfirmedLive | CandidateInconclusive | Anomaly | expect 一致 |
|---|---|---|---|---|---|---|---|
| A | 9 | 全部 ConfirmedLive | 0 | 9 | 0 | 0 | 9/9 |
| B1 | 45 | 全部 CandidateInconclusive | 0 | 0 | 45 | 0 | 45/45 |
| B2 | 44 | 全部 CandidateInconclusive | 0 | 0 | 44 | 0 | 44/44 |
| **合计** | **98** | — | **0** | **9** | **89** | **0** | **98/98** |

Verus 汇总：
- A：vstd `verification results:: 2060 verified, 0 errors`；探针 `verification results:: 18 verified, 9 errors`
- B1：vstd `verification results:: 2060 verified, 0 errors`；探针 `verification results:: 45 verified, 90 errors`
- B2：vstd `verification results:: 2060 verified, 0 errors`；探针 `verification results:: 44 verified, 88 errors`

错误数解释：每条 manifest 条目生成 entry fn + vacuous/live 两个 proof fn。
A（Live）：entry 9 + live 9 = 18 verified，vacuous 9 errors；
B（Inconclusive）：entry 45/44 verified，vacuous+live 90/88 errors。
全部错误为 `failed this postcondition`，无 witness assert 错误（B 不生成见证），
与本模型预期一致。

## Tier A 逐条判定（9/9 与预注册一致）

| entry | function | witness | 预期 | 实际 | 一致 |
|---|---|---|---|---|---|
| `r0106_lemma_encode_prop_flags_matches` | `ostd::arch::mm::lemma_encode_prop_flags_matches` | `0usize` | ConfirmedLive | **ConfirmedLive** | ✓ |
| `r0127_new` | `ostd::boot::memory_region::impl&%19::new` | `(0usize, 0usize, 0usize)` | ConfirmedLive | **ConfirmedLive** | ✓ |
| `r0166_lemma_u64_distinct_bits_differ` | `ostd::cpu::set::lemma_u64_distinct_bits_differ` | `(0usize, 1usize)` | ConfirmedLive | **ConfirmedLive** | ✓ |
| `r0167_lemma_u64_nonzero_has_set_bit` | `ostd::cpu::set::lemma_u64_nonzero_has_set_bit` | `1usize` | ConfirmedLive | **ConfirmedLive** | ✓ |
| `r0169_lemma_u64_unit_bit_projection` | `ostd::cpu::set::lemma_u64_unit_bit_projection` | `0usize` | ConfirmedLive | **ConfirmedLive** | ✓ |
| `r0732_lemma_paddr_to_vaddr_properties` | `ostd::specs::arch::x86::lemma_paddr_to_vaddr_properties` | `(0usize, 0usize, 1usize)` | ConfirmedLive | **ConfirmedLive** | ✓ |
| `r0733_lemma_vaddr_to_paddr_properties` | `ostd::specs::arch::x86::lemma_vaddr_to_paddr_properties` | `(0usize, 0usize, 1usize)` | ConfirmedLive | **ConfirmedLive** | ✓ |
| `r1059_lemma_split_sub_page_big_j` | `ostd::specs::mm::page_table::cursor::page_size_lemmas::lemma_split_sub_page_big_j` | `(2usize, 2usize, 1usize, 2usize)` | ConfirmedLive | **ConfirmedLive** | ✓ |
| `r1196_lemma_vaddr_make_eq_page_size` | `ostd::specs::mm::page_table::owners::lemma_vaddr_make_eq_page_size` | `(0usize, 1usize, 0usize, 1usize)` | ConfirmedLive | **ConfirmedLive** | ✓ |

## Tier B1 逐条判定（45/45 CandidateInconclusive）

| entry | 预期 | 实际 | 一致 |
|---|---|---|---|
| `r0028_lemma_path_pairs_step` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0068_lemma_eq_from_bits` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0069_lemma_from_bits_bits` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0111_lemma_x86_page_flags_wf` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0112_lemma_x86_priv_flags_wf` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0113_lemma_x86_set_prop_decode_bits` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0114_lemma_x86_set_prop_preserves_entry_bits` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0115_lemma_x86_set_prop_roundtrip` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0140_lemma_type_inv_range` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0156_with_capacity_val` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0158_contains` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0159_load` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0160_new` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0162_store` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0168_lemma_u64_nonzero_has_set_bit_aux` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0185_check_and_insert_dma_mapping` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0301_lemma_meta_region_inv_at` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0314_frame_to_meta` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0315_meta_to_frame` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0435_lemma_kernel_range_valid` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0436_sum_page_sizes_extend_right` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0437_sum_page_sizes_mono` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0438_paddr_to_vaddr` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0475_lemma_eq_from_bits` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0476_lemma_from_bits_bits` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0516_lemma_eq_from_bits` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0517_lemma_from_bits_bits` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0591_lemma_replace_cur_entry_mapping_sets` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0731_lemma_meta_frame_vaddr_properties` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0779_lemma_accounting_inv_at` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0780_lemma_accounting_inv_intro` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0781_lemma_accounting_preserved_by_pt_alloc` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0782_lemma_coverage_preserved_slots_eq` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0784_lemma_frame_drop_pre_derivable` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0791_lemma_handle_count_insert_fresh` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0792_lemma_handle_count_remove` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0793_lemma_segment_cover_contains` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0794_lemma_segment_cover_insert_inside` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0795_lemma_segment_cover_insert_outside` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0796_lemma_segment_cover_remove_inside` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0797_lemma_segment_cover_remove_outside` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0798_lemma_segment_cover_shrink_front` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0799_lemma_segment_cover_split` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0800_lemma_segment_cover_witness` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0824_lemma_step_segment_from_unused_accounting` | CandidateInconclusive | CandidateInconclusive | ✓ |

## Tier B2 逐条判定（44/44 CandidateInconclusive）

| entry | 预期 | 实际 | 一致 |
|---|---|---|---|
| `r0834_lemma_structural_inv_cursor_frame` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0835_lemma_structural_inv_frame` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0836_lemma_structural_inv_segment` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0917_lemma_frame_to_index_injective` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0918_lemma_frame_to_meta_soundness` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0919_lemma_index_to_frame_biinjective` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0920_lemma_meta_to_frame_soundness` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0921_lemma_meta_to_paddr_biinjective` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0922_lemma_paddr_to_meta_biinjective` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0923_axiom_mmio_paddr_huge_page_closed` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0936_lemma_contains_valid_frame_paddr` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r0945_internal_invariants_imply_top_level_properties` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1062_lemma_va_plus_page_size_no_overflow` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1110_lemma_extend_empty_mapping_range` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1111_lemma_mapping_set_cardinality_bound` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1112_lemma_mapping_set_cardinality_fits_usize` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1113_lemma_mapping_set_cardinality_in_range` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1114_lemma_sub_mapping_pa_compose` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1115_lemma_wf_subset` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1116_lemma_wf_union` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1192_lemma_rec_vaddr_aligned` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1193_lemma_rec_vaddr_push_tail` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1194_lemma_rec_vaddr_suffix_bound` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1195_lemma_rec_vaddr_unfold` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1199_lemma_vaddr_strict_bound` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1200_lemma_vaddr_top_index_cell` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1210_lemma_flush_preserves_inv` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1211_lemma_update_preserves_consistent` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1215_lemma_read_bytes_eq_pointwise` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1216_lemma_split_join_identity` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1217_lemma_split_preserves_read` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1218_lemma_split_preserves_transl` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1231_from_vaddr` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1236_split` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1237_sub` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1242_lemma_va_mapping_unique` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1333_lemma_aligned_addr_clears_tag_bit` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1459_new` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1461_lemma_alloc_specific_model` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1462_lemma_alloc_suffix_model` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1464_lemma_free_set_contains_range` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1465_lemma_insert_free_range` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1466_lemma_merge_right_neighbor` | CandidateInconclusive | CandidateInconclusive | ✓ |
| `r1467_lemma_remove_left_neighbor` | CandidateInconclusive | CandidateInconclusive | ✓ |

## 与 M1 60 接口结论的对照

把 `guard-return-inventory.csv` 的 60 行逐一映射到 census 全函数分类：
- **42 行 = `no_requires`**（空合取；M1 中这些接口本来就没有 requires，
  M1 的判定是对 ensures/guard-return 面的分析，不是 requires 面）；
- **18 行 = Tier C**（tracked/ghost/引用/泛型阻塞，其中包含 M1 的 7 项已知
  vacuous stub 锚点：`entry_precondition_vacuous=true` 的 7 行全部在 Tier C）；
- **0 行 = Tier A/B** —— M7e 的 98 个候选与 M1 的 60 行清单**完全不相交**。

因此：
1. M7e 没有重测 M1 的任何条目，也没有改写 M1 的判定（2 ConfirmedVacuous /
   16 ConfirmedLive / 10 CandidateInconclusive）。两套结论不冲突，覆盖的是
   不同函数群体：M1 是 guard/return 接口清单，M7e 是 requires 全函数普查。
2. M1 的 7 项已知恒假 stub 在 M7e 中全部是 **Tier C（未筛查）**，这正好
   验证了「Tier C 不等于无缺陷」：如果把 Tier C 当无缺陷，就会漏掉 M1
   已经用构造/形状探针证明过的那批契约。
3. M1 的 E1/E2/E3 规则与本轮 P1/P2/P3 同源；本轮没有出现 E6 型新真空洞。

## 新缺陷与台账

- 本轮 **0 个 ConfirmedVacuous**（Tier A 9/9 Live；Tier B 89/89 Inconclusive）。
  因此 registry 不新增条目；M7e 只提供「无新标量层真空洞」的负面证据。
- Tier B 的 Inconclusive 不计缺陷（uninterp 模型下验证器不完备）。
- 不能把 0 Vacuous 解读为「774 个 requires 都健康」：676 个 Tier C 未筛查，
  其 requires 位于 tracked/ghost/引用/泛型层。

## 边界与未跑部分

- **Tier C 676 个函数未运行**：generic 505、ghost_param 168、reference_param 366
  （按函数去重，可重叠）。其中包含全部 18 个 M1 requires 接口。
- **常量提升**：`ConstVar` 被提升为自由参数（M1 先例），对真空洞是保守近似：
  可能漏报、不会误报。
- **见证域有界**：Tier A 的 witness 搜索域为 0..8/15/16/31/32/63/64/255/256/4096；
  「无见证」只用于选择 CandidateInconclusive，不是不可满足证明。
- **Tier B 无见证**：uninterp 合取项无法见证，双探针皆败是模型预期。
- **未做**：Tier C 的 ghost/引用/泛型建模；跨批结果缓存；夜班默认接入。
- 运行后未向冻结源 `/tmp/vostd-43da079-ktest-chan` 注入任何文件（quantifier
  探针在独立 probe crate 中构建），因此无需恢复源树。
- 补充诊断（非探针判定，不计入上表）：676 个 Tier C 中 63 个仍含至少一条机械子句；
  对这 63 个的机械子集按 Tier A 同域做有界见证搜索，63/63 可满足（常量按提升处理）。
  这只说明其标量核心未显示矛盾，不改变 Tier C「未筛查」的状态，也不构成缺陷判定。

## 产物

- `examples/vostd-requires/screening.json`：1468 函数的逐函数分层、Tier C 原因、
  98 条候选的预注册预期与生成参数。
- `manifest-a.json` / `manifest-b1.json` / `manifest-b2.json`：三批 quantifier manifest。
- `runs/{a,b1,b2}/`：`verdicts.json`（含 dependency/probe 汇总、逐条
  verdict/expected/match）、`report.json`、`probe-build.log`、`probe-src.lib.rs`。
- `controls/`：生成器阳性/阴性对照（zero-arg `false` → ConfirmedVacuous、
  bool 可满足 → ConfirmedLive、bool 矛盾 → ConfirmedVacuous），3/3 符合预期；
  证明 0-vacuity 不是生成器对该形状的盲区（`controls/RESULT.md`）。
- 原始运行目录（probe crate、日志、编译产物）：`/tmp/m7e-run-{a,b1,b2}/`。