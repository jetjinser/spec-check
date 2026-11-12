# REQUIRES-EXPECTED.md — vostd 全函数 requires 筛查：预注册（写于探针运行前）

生成：`cargo-spec-check screen-requires`（M7e）；输入 `/tmp/m7e-census/census.json`
（VIR `/tmp/m7a-vir-export2/crate.vir`，vostd rev 43da079，Verus 0.2026.10.08，
crate `ostd`，source-root `/tmp/vostd-43da079-ktest-chan`）。
本文件在 **Tier A/B 探针运行之前** 写定，含规则与逐条预期；运行结果只写入
`REQUIRES-RUN.md`，不改写本文件。

## 漏斗（生成阶段事实）

- 函数总数 **1468**；有 requires **774**；无 requires（空合取，不筛查）**694**。
- Tier A **9**（11 条全机械子句）；Tier B **89**（230 条子句，其中部分机械 + 部分 uninterp）；Tier C **676**（2266 条子句，不生成探针）。
- Tier C 阻塞原因（按函数去重）：generic **505**、
  ghost_param **168**、reference_param **366**（可重叠；原因组合见 `screening.json`）。
- 可筛查候选 **98** = Tier A 9 + Tier B 89；分三批运行：A=9、B1=45、B2=44。

**Tier C 不是「无缺陷」**：它只表示本工具当前无法建模该 requires（tracked/ghost、
引用参数、泛型）。其条款文本与原因逐条归档在 `screening.json`，未运行即未筛查。

## 预注册判定规则（P 规则）

| 规则 | 条目形状 | 预期 | 意外结局预案 |
|---|---|---|---|
| P1 | Tier A，见证搜索成功（有 witness） | **ConfirmedLive** | 非 Live → 探针生成/触发推断回归，整批作废并排查 |
| P2 | Tier A，搜索无见证 | **CandidateInconclusive** | ConfirmedVacuous = E6 真阳性（标量层恒假），必须人工复核并登记；Anomaly 排查分类器 |
| P3 | Tier B（含 uninterp 子句） | **CandidateInconclusive** | ConfirmedVacuous = 机械部分联立恒假，属真阳性，必须复核；ConfirmedLive 在本模型下不应出现 → Anomaly 排查 |
| P4 | 任一批编译失败 | 该批无效，不产生判定 | 修生成器后整批重跑，不逐条挑选 |
| P5 | Tier C | 不运行 | 不计入缺陷、不计入 Live；覆盖缺口如实报告 |

校准顺序：先跑 Tier A（9 条，全部 P1）验证生成器与触发推断；A 全 Live 后再放量
B1/B2。B 批预期全部 P3；若出现非 Inconclusive 必须逐条解释。

## Tier A 条目（逐条预注册：全部 ConfirmedLive）

| entry | function | params | requires | witness | expected |
|---|---|---|---|---|---|
| `r0106_lemma_encode_prop_flags_matches` | `ostd::arch::mm::lemma_encode_prop_flags_matches` | `cache_flags: usize` | `p106_k0(cache_flags)` | `0usize` | ConfirmedLive |
| `r0127_new` | `ostd::boot::memory_region::impl&%19::new` | `base: usize, len: usize, max_paddr: usize` | `p127_k0(base, len, max_paddr)` | `(0usize, 0usize, 0usize)` | ConfirmedLive |
| `r0166_lemma_u64_distinct_bits_differ` | `ostd::cpu::set::lemma_u64_distinct_bits_differ` | `x: usize, y: usize` | `p166_k0(x, y)` | `(0usize, 1usize)` | ConfirmedLive |
| `r0167_lemma_u64_nonzero_has_set_bit` | `ostd::cpu::set::lemma_u64_nonzero_has_set_bit` | `word: usize` | `p167_k0(word)` | `1usize` | ConfirmedLive |
| `r0169_lemma_u64_unit_bit_projection` | `ostd::cpu::set::lemma_u64_unit_bit_projection` | `b: usize` | `p169_k0(b)` | `0usize` | ConfirmedLive |
| `r0732_lemma_paddr_to_vaddr_properties` | `ostd::specs::arch::x86::lemma_paddr_to_vaddr_properties` | `pa: usize, linear_mapping_base_vaddr: usize, vmalloc_base_vaddr: usize` | `p732_k0(pa, linear_mapping_base_vaddr, vmalloc_base_vaddr)` | `(0usize, 0usize, 1usize)` | ConfirmedLive |
| `r0733_lemma_vaddr_to_paddr_properties` | `ostd::specs::arch::x86::lemma_vaddr_to_paddr_properties` | `va: usize, linear_mapping_base_vaddr: usize, vmalloc_base_vaddr: usize` | `p733_k0(va, linear_mapping_base_vaddr, vmalloc_base_vaddr)` | `(0usize, 0usize, 1usize)` | ConfirmedLive |
| `r1059_lemma_split_sub_page_big_j` | `ostd::specs::mm::page_table::cursor::page_size_lemmas::lemma_split_sub_page_big_j` | `level: usize, nr_levels: usize, i: usize, nr_entries: usize` | `p1059_k0(level, nr_levels) && p1059_k1(i, nr_entries)` | `(2usize, 2usize, 1usize, 2usize)` | ConfirmedLive |
| `r1196_lemma_vaddr_make_eq_page_size` | `ostd::specs::mm::page_table::owners::lemma_vaddr_make_eq_page_size` | `idx: usize, nr_levels: usize, offset: usize, nr_entries: usize` | `p1196_k0(idx, nr_levels) && p1196_k1(offset, nr_entries)` | `(0usize, 1usize, 0usize, 1usize)` | ConfirmedLive |

## Tier B 批次 B1（45 条，预注册全部 CandidateInconclusive）

| entry | function | clauses (mech/unsupported) |
|---|---|---|
| `r0028_lemma_path_pairs_step` | `ostd::arch::kernel::acpi::remapping::lemma_path_pairs_step` | 2 (0/2) |
| `r0068_lemma_eq_from_bits` | `ostd::arch::mm::impl&%15::lemma_eq_from_bits` | 1 (0/1) |
| `r0069_lemma_from_bits_bits` | `ostd::arch::mm::impl&%15::lemma_from_bits_bits` | 1 (0/1) |
| `r0111_lemma_x86_page_flags_wf` | `ostd::arch::mm::lemma_x86_page_flags_wf` | 1 (0/1) |
| `r0112_lemma_x86_priv_flags_wf` | `ostd::arch::mm::lemma_x86_priv_flags_wf` | 1 (0/1) |
| `r0113_lemma_x86_set_prop_decode_bits` | `ostd::arch::mm::lemma_x86_set_prop_decode_bits` | 4 (1/3) |
| `r0114_lemma_x86_set_prop_preserves_entry_bits` | `ostd::arch::mm::lemma_x86_set_prop_preserves_entry_bits` | 3 (0/3) |
| `r0115_lemma_x86_set_prop_roundtrip` | `ostd::arch::mm::lemma_x86_set_prop_roundtrip` | 4 (0/4) |
| `r0140_lemma_type_inv_range` | `ostd::cpu::lemma_type_inv_range` | 1 (0/1) |
| `r0156_with_capacity_val` | `ostd::cpu::set::impl&%6::with_capacity_val` | 2 (0/2) |
| `r0158_contains` | `ostd::cpu::set::impl&%9::contains` | 2 (0/2) |
| `r0159_load` | `ostd::cpu::set::impl&%9::load` | 1 (0/1) |
| `r0160_new` | `ostd::cpu::set::impl&%9::new` | 1 (0/1) |
| `r0162_store` | `ostd::cpu::set::impl&%9::store` | 2 (0/2) |
| `r0168_lemma_u64_nonzero_has_set_bit_aux` | `ostd::cpu::set::lemma_u64_nonzero_has_set_bit_aux` | 3 (2/1) |
| `r0185_check_and_insert_dma_mapping` | `ostd::mm::dma::check_and_insert_dma_mapping` | 1 (0/1) |
| `r0301_lemma_meta_region_inv_at` | `ostd::mm::frame::linked_list::lemma_meta_region_inv_at` | 2 (0/2) |
| `r0314_frame_to_meta` | `ostd::mm::frame::meta::mapping::frame_to_meta` | 1 (0/1) |
| `r0315_meta_to_frame` | `ostd::mm::frame::meta::mapping::meta_to_frame` | 2 (0/2) |
| `r0435_lemma_kernel_range_valid` | `ostd::mm::kspace::kvirt_area::lemma_kernel_range_valid` | 3 (0/3) |
| `r0436_sum_page_sizes_extend_right` | `ostd::mm::kspace::kvirt_area::sum_page_sizes_extend_right` | 1 (0/1) |
| `r0437_sum_page_sizes_mono` | `ostd::mm::kspace::kvirt_area::sum_page_sizes_mono` | 1 (0/1) |
| `r0438_paddr_to_vaddr` | `ostd::mm::kspace::paddr_to_vaddr` | 1 (0/1) |
| `r0475_lemma_eq_from_bits` | `ostd::mm::page_prop::impl&%26::lemma_eq_from_bits` | 1 (0/1) |
| `r0476_lemma_from_bits_bits` | `ostd::mm::page_prop::impl&%26::lemma_from_bits_bits` | 1 (0/1) |
| `r0516_lemma_eq_from_bits` | `ostd::mm::page_prop::impl&%51::lemma_eq_from_bits` | 1 (0/1) |
| `r0517_lemma_from_bits_bits` | `ostd::mm::page_prop::impl&%51::lemma_from_bits_bits` | 1 (0/1) |
| `r0591_lemma_replace_cur_entry_mapping_sets` | `ostd::mm::page_table::cursor::lemma_replace_cur_entry_mapping_sets` | 3 (0/3) |
| `r0731_lemma_meta_frame_vaddr_properties` | `ostd::specs::arch::x86::lemma_meta_frame_vaddr_properties` | 2 (0/2) |
| `r0779_lemma_accounting_inv_at` | `ostd::specs::mm::embedding::lemma_accounting_inv_at` | 2 (0/2) |
| `r0780_lemma_accounting_inv_intro` | `ostd::specs::mm::embedding::lemma_accounting_inv_intro` | 3 (0/3) |
| `r0781_lemma_accounting_preserved_by_pt_alloc` | `ostd::specs::mm::embedding::lemma_accounting_preserved_by_pt_alloc` | 4 (0/4) |
| `r0782_lemma_coverage_preserved_slots_eq` | `ostd::specs::mm::embedding::lemma_coverage_preserved_slots_eq` | 3 (0/3) |
| `r0784_lemma_frame_drop_pre_derivable` | `ostd::specs::mm::embedding::lemma_frame_drop_pre_derivable` | 3 (0/3) |
| `r0791_lemma_handle_count_insert_fresh` | `ostd::specs::mm::embedding::lemma_handle_count_insert_fresh` | 1 (0/1) |
| `r0792_lemma_handle_count_remove` | `ostd::specs::mm::embedding::lemma_handle_count_remove` | 1 (0/1) |
| `r0793_lemma_segment_cover_contains` | `ostd::specs::mm::embedding::lemma_segment_cover_contains` | 2 (0/2) |
| `r0794_lemma_segment_cover_insert_inside` | `ostd::specs::mm::embedding::lemma_segment_cover_insert_inside` | 2 (0/2) |
| `r0795_lemma_segment_cover_insert_outside` | `ostd::specs::mm::embedding::lemma_segment_cover_insert_outside` | 2 (0/2) |
| `r0796_lemma_segment_cover_remove_inside` | `ostd::specs::mm::embedding::lemma_segment_cover_remove_inside` | 2 (0/2) |
| `r0797_lemma_segment_cover_remove_outside` | `ostd::specs::mm::embedding::lemma_segment_cover_remove_outside` | 2 (0/2) |
| `r0798_lemma_segment_cover_shrink_front` | `ostd::specs::mm::embedding::lemma_segment_cover_shrink_front` | 8 (0/8) |
| `r0799_lemma_segment_cover_split` | `ostd::specs::mm::embedding::lemma_segment_cover_split` | 11 (3/8) |
| `r0800_lemma_segment_cover_witness` | `ostd::specs::mm::embedding::lemma_segment_cover_witness` | 1 (0/1) |
| `r0824_lemma_step_segment_from_unused_accounting` | `ostd::specs::mm::embedding::lemma_step_segment_from_unused_accounting` | 14 (0/14) |

## Tier B 批次 B2（44 条，预注册全部 CandidateInconclusive）

| entry | function | clauses (mech/unsupported) |
|---|---|---|
| `r0834_lemma_structural_inv_cursor_frame` | `ostd::specs::mm::embedding::lemma_structural_inv_cursor_frame` | 3 (0/3) |
| `r0835_lemma_structural_inv_frame` | `ostd::specs::mm::embedding::lemma_structural_inv_frame` | 2 (0/2) |
| `r0836_lemma_structural_inv_segment` | `ostd::specs::mm::embedding::lemma_structural_inv_segment` | 4 (0/4) |
| `r0917_lemma_frame_to_index_injective` | `ostd::specs::mm::frame::mapping::lemma_frame_to_index_injective` | 3 (1/2) |
| `r0918_lemma_frame_to_meta_soundness` | `ostd::specs::mm::frame::mapping::lemma_frame_to_meta_soundness` | 1 (0/1) |
| `r0919_lemma_index_to_frame_biinjective` | `ostd::specs::mm::frame::mapping::lemma_index_to_frame_biinjective` | 1 (0/1) |
| `r0920_lemma_meta_to_frame_soundness` | `ostd::specs::mm::frame::mapping::lemma_meta_to_frame_soundness` | 2 (0/2) |
| `r0921_lemma_meta_to_paddr_biinjective` | `ostd::specs::mm::frame::mapping::lemma_meta_to_paddr_biinjective` | 2 (0/2) |
| `r0922_lemma_paddr_to_meta_biinjective` | `ostd::specs::mm::frame::mapping::lemma_paddr_to_meta_biinjective` | 1 (0/1) |
| `r0923_axiom_mmio_paddr_huge_page_closed` | `ostd::specs::mm::frame::meta_owners::axiom_mmio_paddr_huge_page_closed` | 2 (1/1) |
| `r0936_lemma_contains_valid_frame_paddr` | `ostd::specs::mm::frame::meta_region_owners::impl&%1::lemma_contains_valid_frame_paddr` | 2 (0/2) |
| `r0945_internal_invariants_imply_top_level_properties` | `ostd::specs::mm::impl&%0::internal_invariants_imply_top_level_properties` | 1 (0/1) |
| `r1062_lemma_va_plus_page_size_no_overflow` | `ostd::specs::mm::page_table::cursor::page_size_lemmas::lemma_va_plus_page_size_no_overflow` | 1 (0/1) |
| `r1110_lemma_extend_empty_mapping_range` | `ostd::specs::mm::page_table::mapping_set_lemmas::lemma_extend_empty_mapping_range` | 4 (2/2) |
| `r1111_lemma_mapping_set_cardinality_bound` | `ostd::specs::mm::page_table::mapping_set_lemmas::lemma_mapping_set_cardinality_bound` | 2 (0/2) |
| `r1112_lemma_mapping_set_cardinality_fits_usize` | `ostd::specs::mm::page_table::mapping_set_lemmas::lemma_mapping_set_cardinality_fits_usize` | 2 (0/2) |
| `r1113_lemma_mapping_set_cardinality_in_range` | `ostd::specs::mm::page_table::mapping_set_lemmas::lemma_mapping_set_cardinality_in_range` | 3 (1/2) |
| `r1114_lemma_sub_mapping_pa_compose` | `ostd::specs::mm::page_table::mapping_set_lemmas::lemma_sub_mapping_pa_compose` | 10 (0/10) |
| `r1115_lemma_wf_subset` | `ostd::specs::mm::page_table::mapping_set_lemmas::lemma_wf_subset` | 2 (0/2) |
| `r1116_lemma_wf_union` | `ostd::specs::mm::page_table::mapping_set_lemmas::lemma_wf_union` | 3 (0/3) |
| `r1192_lemma_rec_vaddr_aligned` | `ostd::specs::mm::page_table::owners::lemma_rec_vaddr_aligned` | 2 (0/2) |
| `r1193_lemma_rec_vaddr_push_tail` | `ostd::specs::mm::page_table::owners::lemma_rec_vaddr_push_tail` | 3 (1/2) |
| `r1194_lemma_rec_vaddr_suffix_bound` | `ostd::specs::mm::page_table::owners::lemma_rec_vaddr_suffix_bound` | 2 (0/2) |
| `r1195_lemma_rec_vaddr_unfold` | `ostd::specs::mm::page_table::owners::lemma_rec_vaddr_unfold` | 2 (0/2) |
| `r1199_lemma_vaddr_strict_bound` | `ostd::specs::mm::page_table::owners::lemma_vaddr_strict_bound` | 2 (0/2) |
| `r1200_lemma_vaddr_top_index_cell` | `ostd::specs::mm::page_table::owners::lemma_vaddr_top_index_cell` | 2 (0/2) |
| `r1210_lemma_flush_preserves_inv` | `ostd::specs::mm::tlb::impl&%1::lemma_flush_preserves_inv` | 1 (0/1) |
| `r1211_lemma_update_preserves_consistent` | `ostd::specs::mm::tlb::impl&%1::lemma_update_preserves_consistent` | 4 (0/4) |
| `r1215_lemma_read_bytes_eq_pointwise` | `ostd::specs::mm::virt_mem::impl&%1::lemma_read_bytes_eq_pointwise` | 2 (0/2) |
| `r1216_lemma_split_join_identity` | `ostd::specs::mm::virt_mem::impl&%1::lemma_split_join_identity` | 3 (0/3) |
| `r1217_lemma_split_preserves_read` | `ostd::specs::mm::virt_mem::impl&%1::lemma_split_preserves_read` | 1 (0/1) |
| `r1218_lemma_split_preserves_transl` | `ostd::specs::mm::virt_mem::impl&%1::lemma_split_preserves_transl` | 1 (0/1) |
| `r1231_from_vaddr` | `ostd::specs::mm::virt_mem::impl&%5::from_vaddr` | 1 (0/1) |
| `r1236_split` | `ostd::specs::mm::virt_mem::impl&%5::split` | 3 (0/3) |
| `r1237_sub` | `ostd::specs::mm::virt_mem::impl&%5::sub` | 1 (0/1) |
| `r1242_lemma_va_mapping_unique` | `ostd::specs::mm::virt_mem::impl&%7::lemma_va_mapping_unique` | 2 (0/2) |
| `r1333_lemma_aligned_addr_clears_tag_bit` | `ostd::sync::rcu::non_null::either::lemma_aligned_addr_clears_tag_bit` | 3 (0/3) |
| `r1459_new` | `ostd::util::range_alloc::impl&%4::new` | 1 (0/1) |
| `r1461_lemma_alloc_specific_model` | `ostd::util::range_alloc::lemma_alloc_specific_model` | 4 (0/4) |
| `r1462_lemma_alloc_suffix_model` | `ostd::util::range_alloc::lemma_alloc_suffix_model` | 5 (0/5) |
| `r1464_lemma_free_set_contains_range` | `ostd::util::range_alloc::lemma_free_set_contains_range` | 1 (0/1) |
| `r1465_lemma_insert_free_range` | `ostd::util::range_alloc::lemma_insert_free_range` | 5 (0/5) |
| `r1466_lemma_merge_right_neighbor` | `ostd::util::range_alloc::lemma_merge_right_neighbor` | 9 (0/9) |
| `r1467_lemma_remove_left_neighbor` | `ostd::util::range_alloc::lemma_remove_left_neighbor` | 8 (0/8) |

## 与 M1 60 接口的关系（预注册对照口径）

- M1 的 60 行是人工清单，覆盖 7 项已知 stub + 15 项可提取契约 + 31 项无契约；
  本文件的候选集是 census 全量 1468 函数中 98 个 A/B 可筛查函数，**不是**对 60 行的重新挑选。
- 若某 M1 锚点函数出现在本候选集，其判定应与 M1 归档一致；M1 锚点多为
  tracked/引用参数（Tier C）或形状对照，预计不在 A/B 内。运行后逐项对照。
- M1 的 E1–E6 规则与本文 P1–P3 同源；区别是见证域从 0..=7 扩大到
  0..8/15/16/31/32/63/64/255/256/4096，并对常量做了参数提升（抽象，见设计文档 §4）。

## 变更纪律

运行后不改写本文件的预期/表格；任何规则修订只能追加到 `REQUIRES-RUN.md` 的
「规则修订」小节，并说明旧规则为何失效。
