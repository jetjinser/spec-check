# M7c 配方库扩充运行归档（2026-11-12）

预注册：`RECIPE-EXT-EXPECTED.md`（运行前写定；§9 记录一次编写期预检，
发生在注册运行之前）。判定只以 `analyses-divergence` 实跑为准。

## 1. 运行命令

复现脚本：`examples/vostd-ktest/run-m7b-ext.sh`。

合成（11 条）：

```sh
cargo-spec-check synthesize \
  --candidates examples/vostd-ktest/extracted-candidates.json \
  --recipes examples/vostd-ktest/recipes-m7b-ext.json \
  --out examples/vostd-ktest/divergence-manifest-m7b-ext.json \
  --select K145,K146,K147,K148,K149,K150,K151,K152,K153,K012,K014
```

注册运行（单次，900s 预算内完成）：

```sh
timeout 900 nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  'cd ~/vie/work/AES/spec-check && \
   ./target/debug/cargo-spec-check analyses-divergence \
     --manifest examples/vostd-ktest/divergence-manifest-m7b-ext.json \
     --cargo-verus /home/jinser/vie/work/AES/vostd/tools/verus/source/target-verus/release/cargo-verus \
     --z3 /home/jinser/vie/work/AES/vostd/tools/verus/source/z3 \
     --vstd /home/jinser/vie/work/AES/vostd/tools/verus/source/vstd \
     --out /tmp/vostd-divergence-m7b-ext-run2'
```

- 独立副本 `/tmp/vostd-43da079-ktest-chan-b`（`cp -a` 自冻结源；440 文件
  sha256 逐一一致）。运行后 `ostd/src/spec_check_probes/` 已移除，
  `lib.rs` 无注入声明；副本 440 文件逐字节恢复到运行前（`diff` 为空）。
- 冻结源 `/tmp/vostd-43da079-ktest-chan` 未触碰。

## 2. 机器判定汇总

- vstd 依赖：**1908 verified / 0 errors**；
- 目标 workspace（ostd）：**1498 verified / 10 errors**；
- **baseline clean = true**：全部 10 个 error 都落在 11 个 obligation fn
  内的最终 `assert`；探针模块外零错误事件；
- 判定分布：**ObligationSupported 1 / ContractGapCandidate 10 /
  Inconclusive 0 / Anomaly 0**；
- **`expect_match` 11/11 = true**（预注册 1 supported + 10 gap，逐条命中）。

编写期预检（注册前）：`/tmp/vostd-divergence-m7b-ext-run1`，11/11
Inconclusive，phase-1 `error: cannot use function ostd::cpu::CpuId::bsp
which is ignored ...`（`c014_...rs:16:14`）。改用带 `#[verus_verify]` 的
`CpuId::try_from(0usize).unwrap()` 后重跑注册运行；预注册 §9 已记录，
预测未改。

## 3. 逐条目判定表（预测 vs 实际）

| # | entry | 函数 / ktest 来源 | 骨架 | 预期 | 实际 | 一致 | 失败 span / 归因 |
|---|---|---|---|---|---|---|---|
| 1 | `c145_range_difference` | `range_difference` / `range_difference_contained` | iterator-sequence | gap | **ContractGapCandidate** | ✓ | `c145_...rs:18:12` 序列断言；契约未暴露迭代器义务 |
| 2 | `c146_range_difference` | `range_difference` / `range_difference_all_same` | iterator-sequence | gap | **ContractGapCandidate** | ✓ | `c146_...rs:16:12` |
| 3 | `c147_range_difference` | `range_difference` / `range_difference_left_same` | iterator-sequence | gap | **ContractGapCandidate** | ✓ | `c147_...rs:17:12` |
| 4 | `c148_range_difference` | `range_difference` / `range_difference_right_same` | iterator-sequence | gap | **ContractGapCandidate** | ✓ | `c148_...rs:17:12` |
| 5 | `c149_range_difference` | `range_difference` / `range_difference_b_empty` | iterator-sequence | gap | **ContractGapCandidate** | ✓ | `c149_...rs:17:12` |
| 6 | `c150_range_difference` | `range_difference` / `range_difference_a_empty` | iterator-sequence | gap | **ContractGapCandidate** | ✓ | `c150_...rs:16:12` |
| 7 | `c151_range_difference` | `range_difference` / `range_difference_all_empty` | iterator-sequence | gap | **ContractGapCandidate** | ✓ | `c151_...rs:16:12` |
| 8 | `c152_range_difference` | `range_difference` / `range_difference_left_intersected` | iterator-sequence | gap | **ContractGapCandidate** | ✓ | `c152_...rs:17:12` |
| 9 | `c153_range_difference` | `range_difference` / `range_difference_right_intersected` | iterator-sequence | gap | **ContractGapCandidate** | ✓ | `c153_...rs:17:12` |
| 10 | `c012_cpu_set_test_test_empty_cpu_set_iter_is_empty` | `CpuSet::new_empty` / `test_empty_cpu_set_iter_is_empty` | final-relation | supported | **ObligationSupported** | ✓ | 无 error；`new_empty` ensures 空集合视图 |
| 11 | `c014_atomiccpuset_contains` | `AtomicCpuSet::contains` / `test_atomic_cpu_set_multiple_sizes` | state-invariant | gap | **ContractGapCandidate** | ✓ | `c014_...rs:19:12` `assert(out)` |

不一致项：**无**。

## 4. 覆盖率读数

| 指标 | M7b 首站 | 本轮扩展 | 变化 |
|---|---:|---:|---|
| 非 blocked 候选 | 22 | 22 | — |
| 可运行（有 recipe + 可编译） | 2 | **13** | +11 |
| 其中本轮新增 | — | **11** | 9 iterator + 2 CpuSet |
| 仍阻断 | 20 | **9** | 页表/create_page_table/page_properties |
| 本轮判定 | 2/2 expect_match | 11/11 expect_match | — |

新增可表达数 **11**，全部通过 phase-1 编译并进入 phase-2；其中 1 条
supported、10 条契约缺口候选。

## 5. 语义读数

### 5.1 range_difference（9 条 gap）

- 契约依据：`ostd/src/util/ops.rs:80-97` 的 ensures 是

  ```
  ret.obeys_prophetic_iter_laws() && ret.will_return_none() ==> {
      remaining() == range_difference_spec(*a, *b) && ...
  }
  ```

  `obeys_prophetic_iter_laws()` 与 `will_return_none()` 是**前件**，
  没有被 ensures 建立。`Iterator::next` 的 spec 只在
  `obeys_prophetic_iter_laws()` 成立时把 `ret` 与
  `remaining()[0]` 联系起来（vstd `std_specs/iter.rs:44-58`）。
- 实际失败 span 全部落在最终 `assert`（不是 `range_difference` 调用
  前置），说明前置 `obeys_cmp::<usize>`/`finite_range_matches_ord`/
  `cloned` 已由 vstd 默认 broadcast 证明；缺口是**契约没有暴露迭代器
  行为**。
- 9 条覆盖 `range_difference` 的 8 个测试形态（`contained` 有 2 个
  区间，其余 0/1 个区间），每条按测试的 `a`/`b` 与期望序列生成。
- 这不是"断言错误"：`range_difference_spec` 与测试期望一致；缺口在
  契约把结论包在未建立的前件里（`will_return_none` 从不被保证）。

### 5.2 K012 `CpuSet::new_empty`（supported）

- 义务：`let recv = CpuSet::new_empty(); assert(recv@.len() == 0);`
- 契约依据：`new_empty` 的 ensures 是
  `ret@ == Set::empty() && ret.inv()`；`View` 是 `Set<int>`，空集合
  长度为 0。
- **未覆盖**：原测试的 `set.iter().collect::<Vec<_>>().is_empty()`
  迭代器适配器。本义务只证明构造器给出的集合视图为空，不证明
  `iter()` 的序列。

### 5.3 K014 `AtomicCpuSet::contains`（gap）

- 义务：空 `AtomicCpuSet` → `add(CPU0)` → `contains(CPU0)`。
- 契约依据：`contains` 的 ensures 只有
  `cpu_id@ / 64 >= num_parts() ==> ret == false`；`add` 没有后置条件。
  因此"add 后 contains 为真"不可由契约推出；失败 span 在
  `assert(out)`。
- 覆盖范围：循环体在 CPU0 上的一个实例；**未覆盖** 10 种容量、
  `%3` 模式、`load`/`store` 分支。

## 6. 仍阻断的 9 条（逐条）

| 候选 | 函数 | 确切阻断 |
|---|---|---|
| K070 | `PageTable::cursor_mut` | `cursor_mut`（`page_table/mod.rs:1425-1435`）需要 `Tracked(PageTableOwner)`、`Ghost(root_guard)`、`Tracked(MetaRegionOwners)`、`Tracked(Guards)`；`MetaRegionOwners.inv`（`specs/mm/frame/meta_region_owners.rs:38-60`）要求全槽 map，冻结面无铸造入口 |
| K071 | `PageTable::cursor_mut` | 同上；且 `0 < va.end` 前置使 `0..0` 空区间的 Err 义务在 Verus 客户端不可表达 |
| K072 | `page_walk` | setup `create_user_pt_mapped_at` 是 `page_table/test.rs:20` 的私有 helper；`FrameAllocOptions::alloc_frame_with` + cursor tracked 参数不可得 |
| K073 | `page_walk` | 同上；`page_walk` 仅在 `#[cfg(ktest)]` 下存在（`page_table/mod.rs:1415-1417`） |
| K078 | `query` | setup `setup_page_table_with_two_frames` 是 `page_table/test.rs:414` 的私有 helper；cursor tracked 参数 |
| K080 | `find_next` | 同上 |
| K068 | `start_paddr` | `create_user_page_table` 需 `Tracked(kernel_owner)`（`page_table/mod.rs:955`）；`MOCK_KERNEL_PT` 是测试内静态（`test.rs:151`）；`new_kernel_page_table` 为 `external_body` + `unimplemented!()`（`mod.rs:917-923`） |
| K069 | `entry` | `entry` 需要 `Tracked(child_owner/owner/regions)`（`node/mod.rs:445-452`）；`is_node` 需要 `Tracked(EntryOwner/NodeOwner)`（`node/entry.rs:123-134`）；`new_kernel_page_table` 无 ensures |
| K075 | `disable_preempt` | 断言在私有 helper `check_map_with_property`（`test.rs:282`）；`cursor_mut` tracked 参数；闭包 `from_bits(...).unwrap()` 需量化模型 |

这 9 条**不是无需求**；它们是需求已提取、但当前冻结 proof 面无法写成
可编译/可验证 client 的项。未写 recipe、未进入 manifest，按纪律不以
占位义务污染整批判定。

## 7. 运行后恢复

- 运行前/后对副本 `ostd/src` 全部文件做 sha256 清单比对：
  **逐字节一致**（`/tmp/vostd-m7b-ext-before.sha256` vs
  `/tmp/vostd-m7b-ext-after.sha256`，`diff` 为空）。
- `ostd/src/spec_check_probes/` 已移除；`lib.rs` 无注入声明。
- replay 副本在 `/tmp/vostd-divergence-m7b-ext-run2/injections/`；
  判定报告归档为 `examples/vostd-ktest/divergence-report-m7b-ext.json`。
- 预检 v1 证据在 `/tmp/vostd-divergence-m7b-ext-run1/`。

## 8. 证据边界

1. **ContractGapCandidate 是证明论结论**：10 条义务在原契约下不可证且
   基线干净；不是运行时反例，不证明实现错误。
2. **iterator 缺口的根因是契约前件**：`range_difference` 的 ensures 把
   结论放在 `obeys_prophetic_iter_laws() && will_return_none()` 之后；
   修复方向是契约补前件/暴露 `will_return_none`，不是改测试。
3. **K012/K014 是窄义务**：K012 不覆盖 `iter().collect()`；K014 只覆盖
   CPU0 的 add→contains 实例。
4. **可表达数 11 不是 20**：9 条页表类受 tracked owner/external_body/
   私有 helper 阻断，见 §6；若上游补铸造入口，需新运行证据。
5. **骨架是通用的**：`iterator-sequence` 与 `state-invariant` 只提供
   "消费序列"/"变更+查询"的位置；`range_difference`、`CpuSet`、
   `AtomicCpuSet`、`CpuId` 均只出现在项目侧 recipe。
