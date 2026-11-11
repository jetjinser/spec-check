# vostd guard 清单 60 接口全量运行归档（contract-vacuity 全量化，2026-11-11）

对象：`guard-return-inventory.csv` 全部 60 行（M1 只覆盖 7 项恒假 stub；
本轮为全量化：14 行锚点复用 + 15 行自动提取 + 31 行如实记录为
extraction_skipped）。冻结源 `/tmp/vostd-43da079-ktest-chan/ostd/src`
（rev 43da0797bbfa）。提取脚本与逐条转写对照：`extract_full_manifest.py`、
`EXTRACTION-NOTES.md`（**运行前预注册**，含 E 规则与逐条预期）。
判定只以探针运行为准；本文件不改写预注册。

## 运行命令

```
timeout 900 nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  'cd ~/vie/work/AES/spec-check && \
   ./target/debug/cargo-spec-check analyses-vacuity \
     --manifest examples/vostd-guard/full-manifest.json \
     --cargo-verus .../vostd/tools/verus/source/target-verus/release/cargo-verus \
     --z3 .../vostd/tools/verus/source/z3 \
     --vstd .../vostd/tools/verus/source/vstd \
     --out /tmp/vac-vostd-full60-run1'
```

混合 manifest（quantifier + construction）单次运行 **4m20s**，900s 预算
内完成，未拆分。目标捕获缓存复用：构造阶段目标是 
`/tmp/vostd-43da079-ktest-chan`（即 `/tmp/ktest-chan-capture` 捕获的
同一工作区），注入后运行、运行后恢复原字节（`spec_check_probes` 目录
已移除，lib.rs 无残留声明；replay 副本在 out/injections/）。

## 机器判定汇总

- vstd 依赖：2060 verified / 0 errors；
- 探针 crate（26 quantifier 条目 × 3 fns）：42 verified / 36 errors
  （31 条 `failed this postcondition` + 5 条 witness `assertion failed`：
  后者恰为 5 个含 uninterp 合取项的带见证条目——M1 根因复现）；
- 目标 workspace（3 construction 条目）：1498 verified / 1 errors
  （1 error = MutexGuard::new 伪造面 `precondition not satisfied`，span
  归因 spec_check_probes/mutex_guard_new_construction.rs:36）；
- 判定分布：**ConfirmedVacuous 2 / ConfirmedLive 16 /
  CandidateInconclusive 10 / Anomaly 0**（28 条目）。

## 逐契约判定表（60 行覆盖 + 对照）

预注（EXPECTED）= EXTRACTION-NOTES.md 预注册；锚点预注 = M1 已归档判定；
构造/形状对照预注 = tracked/VALIDATION.md 已归档判定。

| # | entry | CSV 行 | 建模 | 预期 | 实际 | 一致? |
|---|-------|--------|------|------|------|------|
| 1 | `try_traverse_and_lock_subtree_root_shape` | 2, 3 | 锚点复用 | CandidateInconclusive | CandidateInconclusive | ✓ |
| 2 | `alloc_if_none_shape` | 5, 7 | 锚点复用 | CandidateInconclusive | CandidateInconclusive | ✓ |
| 3 | `split_if_mapped_huge_shape` | 6, 8 | 锚点复用 | CandidateInconclusive | CandidateInconclusive | ✓ |
| 4 | `alloc_absent_child_shape` | 9, 10 | 锚点复用 | CandidateInconclusive | CandidateInconclusive | ✓ |
| 5 | `node_lock_shape` | 11, 13 | 锚点复用 | CandidateInconclusive | CandidateInconclusive | ✓ |
| 6 | `make_guard_unchecked_shape` | 12, 14 | 锚点复用 | CandidateInconclusive | CandidateInconclusive | ✓ |
| 7 | `mutex_guard_new_shape` | 26, 27 | 锚点复用 | ConfirmedLive | ConfirmedLive | ✓ |
| 8 | `row00_read_inner` | 0 | 自动提取 | ConfirmedLive | ConfirmedLive | ✓ |
| 9 | `row01_read_inner` | 1 | 自动提取 | ConfirmedLive | ConfirmedLive | ✓ |
| 10 | `row04_path_slot_as_mut` | 4 | 自动提取 | CandidateInconclusive | CandidateInconclusive | ✓ |
| 11 | `row15_vm_space_cursor_embedded` | 15 | 自动提取 | CandidateInconclusive | CandidateInconclusive | ✓ |
| 12 | `row16_vm_space_cursor_mut_embedded` | 16 | 自动提取 | CandidateInconclusive | CandidateInconclusive | ✓ |
| 13 | `row17_tracked_restore` | 17 | 自动提取 | CandidateInconclusive | CandidateInconclusive | ✓ |
| 14 | `row25_try_lock` | 25 | 自动提取 | ConfirmedLive | ConfirmedLive | ✓ |
| 15 | `row29_read` | 29 | 自动提取 | ConfirmedLive | ConfirmedLive | ✓ |
| 16 | `row37_try_read` | 37 | 自动提取 | ConfirmedLive | ConfirmedLive | ✓ |
| 17 | `row39_try_write` | 39 | 自动提取 | ConfirmedLive | ConfirmedLive | ✓ |
| 18 | `row44_try_upgrade` | 44 | 自动提取 | ConfirmedLive | ConfirmedLive | ✓ |
| 19 | `row47_try_read` | 47 | 自动提取 | ConfirmedLive | ConfirmedLive | ✓ |
| 20 | `row52_lock` | 52 | 自动提取 | ConfirmedLive | ConfirmedLive | ✓ |
| 21 | `row53_try_lock` | 53 | 自动提取 | ConfirmedLive | ConfirmedLive | ✓ |
| 22 | `row59_get_freelist_guard` | 59 | 自动提取 | ConfirmedLive | ConfirmedLive | ✓ |
| 23 | `mutex_new_any_value_control` | — | M1 对照（阴性） | ConfirmedLive | ConfirmedLive | ✓ |
| 24 | `arith_bounds_live_control` | — | M1 对照（阴性） | ConfirmedLive | ConfirmedLive | ✓ |
| 25 | `arith_oob_vacuous_control` | — | M1 对照（阳性空洞） | ConfirmedVacuous | ConfirmedVacuous | ✓ |
| 26 | `mutex_guard_new_construction` | — | tracked 复用（构造探针） | ConfirmedVacuous | ConfirmedVacuous | ✓ |
| 27 | `mutex_guard_new_holder_face` | — | tracked 复用（构造探针） | ConfirmedLive | ConfirmedLive | ✓ |
| 28 | `mutex_guard_new_quantifier_shape` | — | tracked 复用（形状对照） | ConfirmedLive | ConfirmedLive | ✓ |

**28/28 一致**（预注册 vs 运行；含锚点 7/7 复现 M1、构造对照
3/3 复现 wt-b、阳性/阴性对照 3/3）。

## 与已知锚点的一致性结论

1. **锚点（7 项已知恒假 stub，14 行 CSV）**：判定逐一复现 M1 归档
   （FULL-RUN.md）：6 项 CandidateInconclusive（tracked 资源抽象为
   usize id 后双探针皆败）、`mutex_guard_new` 形状 ConfirmedLive。
   抽取脚本与 M1 手工转写的机械/抽象边界在判定层等价——全量化管道
   未引入漂移。
2. **构造探针（wt-b）**：`mutex_guard_new_construction` 伪造调用在
   `expect_fail_requires` 处被拒 → ConfirmedVacuous；holder 面
   ConfirmedLive。复现 tracked/VALIDATION.md 归档判定（模型相对）。
3. **E 规则全部成立**：11 个无 requires 契约条目（E1）全 ConfirmedLive；
   4 个全 uninterp 条目（E2）与 1 个混合条目（E3）全
   CandidateInconclusive；E6（标量层恒假）未出现——与本轮预期一致，
   60 接口中不存在标量层自相矛盾的入口前置。
4. **无新缺陷**：全部 ConfirmedVacuous 均为既有锚点（V-001 构造 + 阳性
   对照）；全量运行未产生新的恒假证据，registry 不新增条目（纪律：
   不虚构）。V-002 按 probe_run 证据更新。

## 证据边界（重要）

- **15 个自动提取条目中 11 个为"无 requires"契约**（ensures-only 或
  裸 `#[verus_spec]`，源文本层面入口前置为空合取）：探针 requires 编码
  为字面 `true`，ConfirmedLive 表达的是"入口前置平凡可满足"，不是对
  ensures 内容的任何验证。
- **31 行 extraction_skipped**：冻结源对应 span 处无 `#[verus_spec]`、
  签名内亦无 requires（guard trait impl、阻塞式 `read/upgrade/
  downgrade` 包装、`disable_preempt` 等 external_body 无契约接口）。
  这些接口在 VIR 层记录的契约无显式 requires 子句，不存在可探针的恒假
  候选；按纪律不硬造条目，逐行清单见 EXTRACTION-NOTES.md。
- 锚点 7 项与 row04/row15/row16/row17 均为 shape reproduction：
  tracked 参数抽象为不透明 id、ghost 结构谓词 uninterp；
  CandidateInconclusive 不计缺陷（验证器不完备）。恒假性若存在，
  位于被抽象掉的 ghost 资源/不变式层（V-002 静态判读不变）。
- 见证取值是标量层可满足性的纸面论证；机器判定以探针运行为准
  （M1 修正后的见证策略）。

## 运行产物

- 最终轮：`/tmp/vac-vostd-full60-run1/`（本文件所有数字来源；
  report.json / probe-build.log / injections/）
- 冒烟轮（quantifier-only 编译验证，同一 manifest 去掉构造条目，
  判定与最终轮 26 quantifier 条目完全一致）：
  `/tmp/kilo/extract/vac-smoke/`（首跑误在 nix 环境外执行，
  `cargo metadata` 失败全 Anomaly，属环境错误非工具缺陷，重跑即正）

## manifest 修复迭代

首轮即通过 cargo-verus 编译与触发推断（28 条目；26 quantifier + 3
construction 注入 + 3 M1 对照），未消耗 manifest 修复迭代；提取脚本
开发中的 3 处参数/见证绑定缺陷在运行前由预检修正（未进入探针运行）。

