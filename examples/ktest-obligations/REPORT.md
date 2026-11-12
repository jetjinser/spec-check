# ktest 断言义务候选：原型报告（M7b 前置）

日期：2026-11-12  
输入：`/tmp/vostd-43da079-ktest-chan/ostd/src`（vostd rev `43da079`，`#[ktest]` 门控）  
工具：`/tmp/m7b-ktest/extract.py`（正则 + 花括号配对，只读、不构建）  
产物：`/tmp/m7b-ktest/candidates.json`（schema `m7b-ktest-candidates-v1`）

---

## 1. 结论摘要

- 冻结源中共发现 **153 个 `#[ktest]` 测试**；提取出 **139 个非排除候选**，排除 14 个。
- 候选评级：**direct 8 / needs_model_helper 10 / needs_model_quantified 4 / blocked 117 / excluded 14**。
- 断言级：**459 条断言**（候选内保留 446 条：value 294、existence 115、invariant 37）；**310 条成功需求**（`unwrap`/`expect`/`let Some|Ok|Err`，候选内 301 条）。
- 候选级主类别：**value_readback 61 / existence_acquisition 37 / state_transition 26 / invariant_consistency 15**。
- 冻结 proof 快照的可回放性是主要约束：117 个测试带 `ktest_capture_gap`（其中 105 个非排除候选、12 个本身被排除），另有 8 个模块断连测试。**117 个 blocked 候选 = 105 capture-gap + 8 模块断连 + 4 调度器/环境依赖**。真正可直接注入验证的只有 22 个非 blocked 候选（8 direct + 10 helper + 4 quantified）。
- 与研究档案逐项对照：`mutex-try-lock-demand`、`memory-region-push-test`、`range-allocator-alloc-specific`、`vmio-fallible-write` 的已知义务均被准确复现；`guard-transitions` 不是 ktest 来源，冻结语料中也没有 rwlock/rwmutex ktest，无法由本路径复现。

---

## 2. 语料与运行

```sh
python3 /tmp/m7b-ktest/extract.py /tmp/vostd-43da079-ktest-chan/ostd/src \
  --out /tmp/m7b-ktest/candidates.json
```

- 冻结源：`ostd/src/**/*.rs`，共 203 个 `.rs` 文件；mask 掉注释/字符串后有 **153 个有效 `#[ktest]` 测试**（原始文本中有 159 处 `#[ktest`，其中 6 处位于块注释：`lib.rs` 3、`task` 2、`frame::allocator` 1）。
- 模块路径规则与 HIR 清单（`ktest-hir-inventory/vostd-import-v016`）的运行时路径在 **146/153** 个测试上逐字一致：
  - 本提取多 7 个：`arch/riscv/cpu/extension.rs` 的 6 个（冻结 `lib.rs` 选择 x86 `arch` 路径，riscv 测试未编译）+ `mm::page_table::test::navigation::jump_from_end_and_query_huge_middle`（运行时旧清单之后新增）。
  - 本提取少 6 个：`lib.rs` 的 3 个自测、`task::test::{create_task,spawn_task}`、`mm::frame::allocator::test_alloc_dealloc`；它们在冻结树中被块注释掉，mask 后不参与提取。
- 行号/位置核验：对 **769 条**断言与需求位置逐条检查源码行，全部落在含 `assert`/`unwrap`/`expect`/`let` 的行上（脚本见 §6）。

### 2.1 评级分布

| 评级 | 数量 | 含义 |
|---|---:|---|
| `direct` | 8 | 公共 API + 可观察值/形状断言，可直接写 obligation client |
| `needs_model_helper` | 10 | 断言在本地 `assert*/check*` helper 中，需要内联调用轨迹 |
| `needs_model_quantified` | 4 | 循环/迭代器/集合，需要量化不变量 |
| `blocked` | 117 | 105 个 capture-gap、8 个模块断连、4 个调度器/环境依赖 |
| `excluded` | 14 | 10 个 `should_panic` + 2 个私有观察 + 2 个无断言 |

### 2.2 候选级类别分布

| 类别 | 数量 | 例子 |
|---|---:|---|
| `value_readback` | 61 | `assert_eq!(frame.meta().value, 42)`、`assert_eq!(val, read_val)`、`assert_eq!(regions[0].base(), 0)` |
| `existence_acquisition` | 37 | `assert!(lock.try_lock().is_none())`、`allocator.acquire(..).is_some()`、`rw1.get() == Some(..)` |
| `state_transition` | 26 | lock→try_lock、drop→realloc、map→unmap→query、clone→get、wake 两次 |
| `invariant_consistency` | 15 | `frame1.start_paddr() == frame2.start_paddr()`、`reference_count() == 2`、集合成员关系 |

### 2.3 断言级类别分布

| 类别 | 数量（候选内保留） | 备注 |
|---|---:|---|
| `value_readback` | 294 | 含 buffer 往返、属性保持、查询谓词 |
| `existence_acquisition` | 115 | `is_*`、`unwrap/expect` 之外的显式形状断言 |
| `invariant_consistency` | 37 | 跨对象/引用计数/属性关系 |
| 成功需求（全部） | 310 | 候选内 301：292 条变体未知，7 条 Some，2 条 Err |
| 排除的私有观察 | 3 | mutex 的 3 条 `lock.lock.load` |
| 排除的测试假设 | 2 | `frame_drop`/`segment_drop` 的 paddr 复用 |

> 注：459 是全量断言次数，446 是候选内保留次数。差额 13 = 5 条候选内排除（3 私有 + 2 分配器复用）+ 8 条落在被排除测试（`should_panic` 等）中的断言。helper 内联使 `util::ops` 的 2 条断言在 9 个测试中重复出现（共 18 次），统计时已注明。

---

## 3. 与研究档案对照

| 研究档案 | 档案已知结论 | 本提取结果 | 判定 |
|---|---|---|---|
| `mutex-try-lock-demand` | 从 `assert!(lock.try_lock().is_none())` 生成 client；保留 1 条断言，省略 3 条私有 `lock.lock.load` 读 | **K131**：`assert!(lock.try_lock().is_none())`（sync/mutex.rs:320）保留；3 条私有读逐条排除 | ✅ 精确复现 |
| `memory-region-push-test` | 5 次 `MemoryRegionArray::push(...).unwrap()` + 16 条排序/结果断言；生成 5 条 `is_ok` 义务 | **K007**：5 条 push 成功需求（行 519/526/533/541/549）+ 16 条 value 断言（行 553–572） | ✅ 精确复现 |
| `range-allocator-alloc-specific` | `conflict_region` 的 6 个 `IoMemAllocator::acquire`，`Option::is_some/is_none`；冻结 crate root 未声明 `io` 模块 | **K016**：6 条 acquire 形状断言（2 some / 4 none，行 166–193）；`disconnected=true` | ✅ 需求复现；可回放性同样为 blocked（与档案一致） |
| 同上（负向控制） | 越界/非法区间应返回 `None` | **K015** `illegal_region`：4 条 `acquire(...).is_none()`（行 148–151） | ✅ 复现 |
| `vmio-fallible-write` | `read_write_val_fallible` 的 3 个义务：写成功、读成功、值相等 | **K109**：2 条 unwrap 成功需求 + 1 条 `assert_eq!(val, read_val)`，共 3 条 | ✅ 精确复现 |
| `guard-transitions` | 3 个 guard 转换义务来自 VIR owner-field 扫描，不是 ktest | 冻结 ktest 语料中 **0 个 rwlock/rwmutex 测试**；无候选 | ❌ 不适用（来源不同） |
| `ktest-hir-inventory` | 运行时 rev 153 注册 / 152 条记录；386 个断言宏位置 | 冻结树 153 测试；459 条断言次数；146/153 路径一致 | ⚠️ 数量不可直比：修订不同、6 个测试被注释、7 个测试新增、helper 重复计数 |
| `success-demands` | 全 MIR 范围 36 条硬成功需求（27 个 caller） | 本提取是 ktest 范围 310 条成功需求 | ⚠️ 范围不同；mutex `is_none` 等重叠项一致，不能作为全量 oracle |

---

## 4. Top 10 候选（按 M7b 价值）

> “可转写”指能直接写成 divergence 的 obligation client（真实 API 调用 + `assert(义务)`）。`blocked` 项需求确定，但当前冻结树需恢复捕获/模块后才能注入。

| # | ID | 测试（源码位置） | 评级 / 类别 | 义务 | 价值 |
|---|---|---|---|---|---|
| 1 | K131 | `sync::mutex::test::test_mutex_try_lock_does_not_unlock`（sync/mutex.rs:310；断言 320） | direct / state_transition | `try_lock().is_none()`（持有 guard 时） | 研究锚点；divergence 已有同形 client；1 条断言、3 条私有读正确排除 |
| 2 | K007 | `boot::memory_region::test::test_sort_full_non_overlapping`（boot/memory_region.rs:509；断言 553–572） | direct / value_readback | 5×`push(...).unwrap()` + 16 条 base/len/typ 读回 | 研究锚点；直线前缀完整，最适合批量生成 |
| 3 | K016 | `io::io_mem::allocator::test::conflict_region`（io/io_mem/allocator.rs:154；断言 166–193） | blocked（disconnected）/ state_transition | 6×`acquire(range).is_none/is_some` | range-allocator 锚点；需恢复 `io` 顶层模块（档案同结论） |
| 4 | K136 | `sync::rwarc::test::lockless_get`（sync/rwarc.rs:195；断言 198/201/204/207） | blocked（capture-gap）/ state_transition | `get()` 的 `Some(1)`/`None` 四种状态读回 | 与 divergence manifest 的 rwarc 家族相邻；Option 形状清晰 |
| 5 | K109 | `mm::test::io::read_write_val_fallible`（mm/test.rs:352；需求 359/365，断言 366） | blocked（capture-gap）/ value_readback | 写成功、读成功、`val == read_val` | vmio-fallible-write 锚点；3 条义务与研究完全一致 |
| 6 | K070 | `mm::page_table::test::range_checks::range_check`（mm/page_table/test.rs:202；断言 211/214/215） | direct / existence_acquisition | `cursor_mut(valid)` Ok；`invalid`/`kernel` Err | 纯形状义务，参数为字面量区间；无需 tracked 资源 |
| 7 | K071 | `mm::page_table::test::range_checks::boundary_conditions`（mm/page_table/test.rs:218；断言 225/229/237） | direct / existence_acquisition | 空区间/越界/未对齐 → `Err` | 负向边界义务，确定性强 |
| 8 | K078 | `mm::page_table::test::navigation::jump`（mm/page_table/test.rs:462；断言 472–483） | direct / state_transition | `virt_addr` 读回、`query().unwrap().1.is_none()`、`jump(...).unwrap()` 后查询 | 6 条断言 + 4 条需求；setup helper 需内联 |
| 9 | K040 | `mm::frame::test::frame::frame_clone`（mm/frame/test.rs:35；断言 42–45） | blocked（capture-gap）/ invariant_consistency | clone 后两 frame 的 paddr/meta 相同、refcount 均为 2 | 引用计数/身份不变量的典型；需要 refcount 关系谓词 |
| 10 | K145 | `util::ops::test::range_difference_contained`（util/ops.rs:162；helper 断言 158/159） | needs_model_helper / existence_acquisition | `range_difference(0..10, 3..7)` 依次返回 `[0..3, 7..10]`，最后 `None` | 纯函数、参数全字面量；helper 内联后可转写 |

**次选**：K015 `illegal_region`（4 条 `is_none` 负向控制）、K072/K073 `start/end_boundary_mapping`（`page_walk(...).is_some()`）、K080 `find_next`（cursor 导航读回）、K093 `read_write_u32_infallible`（buffer 往返）。

---

## 5. 误报与可信度审计

- **位置核验**：769 条断言/需求位置逐条检查，全部落在对应源码行；无凭空候选。
- **已确认的过近似**：
  1. `map_preserves_page_property`（K075）：`.map(|f| PageFlags::from_bits(f).unwrap())` 的 unwrap 被计为“逐元素成功需求”；实际是迭代器元素，应归 `needs_model_quantified` 而非单次义务。脚本当前因 helper 归因把它评为 `needs_model_helper`，并保留 2 条需求，属于过近似。
  2. `frame_clone` 的 `reference_count() == 2` 被逐条归为 `value_readback` 而非 `invariant_consistency`；主类别仍正确，子类别有偏差。
  3. `assert!(set_cpus.len() == num_cpus)` 被归为 value；该测试是环境依赖（`all_cpus`/`num_cpus`），最终评级 `blocked`，不影响可用性。
  4. helper 内联：`assert_range_difference` 的 2 条断言在 9 个 `range_difference_*` 候选中重复出现；统计已单列 `assertions_via_helper_tests=10`。
- **已确认的排除**：
  - 10 个 `#[should_panic]` 测试全部识别；冻结树中另有 4 个 expected-panic 测试已被块注释（HIR 运行时清单有、冻结树无）。
  - mutex 的 3 条私有原子读被排除，只保留 `try_lock().is_none()`（与研究档案的 omitted_assertions 一致）。
  - `frame_drop`/`segment_drop` 的 paddr 复用断言标记为测试假设；`frame_drop` 其余 3 条断言仍保留（但测试整体 capture-gap blocked）。
- **没有覆盖到的**：`guard-transitions`/`mutex-lock-contract`/`spinlock-return-contract` 中的 owner/guard 关系义务不是从 ktest 断言提取的；本语料也没有 spinlock/rwlock/rwmutex 的 ktest。M7b 若需要这些家族，仍需 VIR/owner-field 路径。

---

## 6. 局限

1. **无类型信息**：`unwrap/expect` 的变体默认 `Some or Ok`（292/301 条）；只有显式 `let x: Result/Option` 或 `Some/Err` 模式能确定。产品化应使用 typed HIR。
2. **无控制流/生命周期**：`let ... else`、`if let` 的分支路径被拍平；guard 存活、drop 时点仅按文本顺序记录。
3. **helper 只做同文件一层**：仅内联名字以 `assert/check` 开头的本地 helper；`setup_page_table_with_two_frames` 等 setup helper 未展开。
4. **门控是启发式**：识别 `ktest_capture_gap`、父模块 `mod test;` 声明、`lib.rs` 注释与 `arch/riscv` 旁路；任意 `cfg`/feature/target 组合未建模。
5. **“direct”不等于已验证**：只表示语法上可写成 obligation client，未检查 proof-side API 是否存在、签名是否匹配；仍需 divergence 工具注入验证。
6. **117/139 候选 blocked**：大部分需求位于冻结 proof 快照之外（mm/test、frame、dma、kspace、rwarc、rcu、wait、irq、io、riscv）。这说明“提取规则”必须与“冻结树可回放性”分开评估；完整需求面应在运行时 HIR 上提取，再按 gate manifest 标注可验证性。
7. **修订差异**：冻结 vostd `43da079` 与研究档案的运行时 `4e78653b`/`7a398167` 不是同一棵树；行号与测试集合有差异（146/153 路径一致），对照时以测试路径与断言文本为准，不以行号为准。

---

## 7. 产出文件与复现

| 文件 | 内容 |
|---|---|
| `/tmp/m7b-ktest/EXTRACTION-RULES.md` | 模式分类、提取规则、排除规则、转写形式、局限 |
| `/tmp/m7b-ktest/extract.py` | 原型提取器（只读、无构建依赖） |
| `/tmp/m7b-ktest/candidates.json` | 153 条测试记录（139 候选 + 14 排除）、统计、断言/需求/调用序列/评级 |
| `/tmp/m7b-ktest/REPORT.md` | 本报告 |

运行：

```sh
python3 /tmp/m7b-ktest/extract.py /tmp/vostd-43da079-ktest-chan/ostd/src \
  --out /tmp/m7b-ktest/candidates.json
```

位置核验（本次一次性审计）：

```sh
python3 - <<'PY'
import json, pathlib
root = pathlib.Path('/tmp/vostd-43da079-ktest-chan/ostd/src')
d = json.load(open('/tmp/m7b-ktest/candidates.json'))
bad = []
for c in d['candidates']:
    lines = (root / c['file']).read_text(errors='replace').splitlines()
    for a in c['assertions']:
        if 'assert' not in lines[a['line'] - 1]:
            bad.append((c['test'], a['line']))
    for x in c['demands']:
        if not any(k in lines[x['line'] - 1] for k in ('unwrap', 'expect', 'let')):
            bad.append((c['test'], x['line']))
print('bad:', len(bad), bad[:10])
PY
```

本次核验结果：`bad: 0`（769 条位置）。
