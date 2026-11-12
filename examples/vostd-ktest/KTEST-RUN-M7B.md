# M7b ktest 义务提取 + 首站运行归档（2026-11-12）

预注册：`KTEST-EXPECTED-M7B.md`（运行前写定；§6 记录一次编写期预检，
发生在注册运行之前）。判定只以 `analyses-divergence` 实跑为准。

## 1. 运行命令

提取（冻结源只读）：

```sh
~/vie/work/AES/spec-check/target/debug/cargo-spec-check extract-ktest \
  --source-root /tmp/vostd-43da079-ktest-chan/ostd/src \
  --config examples/vostd-ktest/extract-config.json \
  --out examples/vostd-ktest/extracted-candidates.json \
  --stats examples/vostd-ktest/extract-stats.json
```

合成：

```sh
cargo-spec-check synthesize \
  --candidates examples/vostd-ktest/extracted-candidates.json \
  --recipes examples/vostd-ktest/recipes-m7b.json \
  --out examples/vostd-ktest/divergence-manifest-m7b.json \
  --select K007,K131
```

注册运行（单次，900s 预算内完成）：

```sh
timeout 900 nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  'cd ~/vie/work/AES/spec-check && \
   ./target/debug/cargo-spec-check analyses-divergence \
     --manifest examples/vostd-ktest/divergence-manifest-m7b.json \
     --cargo-verus /home/jinser/vie/work/AES/vostd/tools/verus/source/target-verus/release/cargo-verus \
     --z3 /home/jinser/vie/work/AES/vostd/tools/verus/source/z3 \
     --vstd /home/jinser/vie/work/AES/vostd/tools/verus/source/vstd \
     --out /tmp/vostd-divergence-m7b-run1'
```

- 冻结源 `/tmp/vostd-43da079-ktest-chan`（rev `43da079` + ktest 通道），
  注入 `ostd/src/spec_check_probes/`，`#[cfg(ktest)]` +
  `--cfg ktest_capture_gap`，target `x86_64-unknown-none`；两阶段
  （`cargo-verus check --no-verify` → `cargo-verus build`）。
- 输入哈希见预注册 §1；提取/合成命令与输出逐字节确定。

## 2. 机器判定汇总

- vstd 依赖：**1908 verified / 0 errors**；
- 目标 workspace（ostd）：**1498 verified / 1 errors**；
- **baseline clean = true**：唯一 error 落在 `c131_mutex_try_lock.rs:18`
  的 obligation fn 内；探针模块外零错误事件；
- 判定分布：**ObligationSupported 1 / ContractGapCandidate 1 /
  Inconclusive 0 / Anomaly 0**；
- **`expect_match` 2/2 = true**（预注册 1 supported + 1 gap，逐条命中）。

## 3. 逐条目判定表（预测 vs 实际）

| # | entry | 函数 / ktest 来源 | 族 | 预期 | 实际 | 一致 | 归因 |
|---|---|---|---|---|---|---|---|
| 1 | `c007_memoryregionarray_push` | `MemoryRegionArray::push`；K007 `boot::memory_region::test::test_sort_full_non_overlapping` | value-readback | ObligationSupported | **ObligationSupported** | ✓ | 义务验证通过；`push` ensures 覆盖成功与后状态长度 |
| 2 | `c131_mutex_try_lock` | `Mutex::try_lock`；K131 `sync::mutex::test::test_mutex_try_lock_does_not_unlock` | is_none-demand | ContractGapCandidate | **ContractGapCandidate** | ✓ | `ostd/src/spec_check_probes/c131_mutex_try_lock.rs:18:12` `assert(flag)`；无 precondition/postcondition 事件 |

不一致项：**无**。

编写期预检（注册前）：`/tmp/vostd-divergence-m7b-precheck1`，2/2
Inconclusive，phase-1 `E0596`（`let recv` 非 mut，`push` 需要
`&mut self`）。修正为合成器通用 `receiver_mut` recipe 字段后重跑注册
运行；预注册 §6 已记录，预测未改。

## 4. 语义读数

### 4.1 K007 `MemoryRegionArray::push`（supported）

- 义务：4 次前置 push 后，第 5 次 `push` 断言 `out.is_ok()` 且
  `recv@.len() == 5`。
- 契约依据：`push` 的 ensures
  `old(self)@.len() < LEN ==> final(self)@ == old(self)@.push(region) && ret.is_ok()`；
  `LEN=64`，第 5 次前 `len=4`，因此成功与长度 5 都是**契约字面可推**。
- 这是本轮唯一的 supported 面，且是非平凡 supported（需要把 `View`
  的 `Seq` 后状态与局部 `recv@` 连接），不是"空 ensures 碰巧通过"。
- **未覆盖**：原测试的 16 条 `into_non_overlapping` 值读回（merge/排序/
  typ 结果）。`into_non_overlapping` 在冻结树是 `#[verifier::external_body]`
  + 运行时 body，无 ensures；因此这些读回不在本轮义务内，不能由本运行
  推断 supported/gap。

### 4.2 K131 `Mutex::try_lock`（gap）

- 义务：持有 `lock()` guard 后 `try_lock()` 返回 `None`。
- 契约依据：`try_lock` 的 ensures 只有
  `ret is Some ==> ret->0.rel_mutex(*self)`；没有"锁已被持有 ⇒ 返回
  None"的分支。`lock()` 本身也没有把"已被持有"作为调用者可见事实。
- 失败 span 精确落在 obligation fn 的 `assert(flag)`（`:18:12`），
  baseline 干净 ⇒ 证明论缺口候选，**不是运行时反例**。
- 与 registry `F-013`（`mutex-try-lock-is-none-demand`，
  `obligation_fail`）是同一语义关系；本轮是 ktest 提取路径的独立复现，
  不新增缺陷条、不升级 `machine_status`（按维护规则只补证据）。
- 原测试的 3 条私有 `lock.lock.load` 读在提取阶段已逐条排除，不在
  client 内；client 只保留 `try_lock().is_none()`。

## 5. 提取结果与原型对照（139 / 22 口径）

| 指标 | 原型 `examples/ktest-obligations/candidates.json` | 本轮 `extract-ktest` | 差异 |
|---|---:|---:|---|
| `#[ktest]` 测试 | 153 | 153 | 一致 |
| 非排除候选 | 139 | 139 | 一致 |
| 排除 | 14 | 14 | 一致（10 should_panic + 2 私有观察 + 2 无断言） |
| capture-gap 测试 | 117 | 117 | 一致（含 12 个被排除测试） |
| 模块断连测试 | 8 | 8 | 一致 |
| `by_rating` | direct 8 / helper 10 / quantified 4 / blocked 117 / excluded 14 | 同左 | 一致 |
| `by_category` | value 61 / existence 37 / state 26 / invariant 15 | 同左 | 一致 |
| kept assertions | 446 | 446 | 一致 |
| all assertions | 459 | 459 | 一致 |
| assertion categories | value 294 / existence 115 / invariant 37 / private 3 / test-assumption 2 | 同左 | 一致 |
| all success demands | 310 | **311** | +1（见下） |
| non-excluded success demands | 301 | **302** | +1（同一处） |
| blocked 构成 | 105 capture-gap + 8 断连 + 4 环境 | 同左 | 一致 |

**差异逐条解释**：

1. **+1 demand（311 vs 310）**：K078 `mm::page_table::test::navigation::jump`
   的 `let (queried_va, Some(queried_item)) = cursor.query().unwrap() else {...}`
   是**元组解构**中的 `Some` 模式；原型正则只匹配 `let Some(` /
   `if let Some(`，漏掉该形态。本轮 `let` 左值扫描将其记为
   `kind=let-pattern, demand=Some, control_flow=let-else`。这是产品化
   改进，不是回归。
2. **变体推断**：原型 292/301 条 demand 只能记 `Some or Ok`；本轮用
   显式类型注解、构造器、`let` 模式和项目侧 `method_variants` 表推断：
   `Some 36 / Ok 82 / Err 2 / Some or Ok 191`（按全部 311 条 demand），
   `variant_source` = `constructor 14 / method-table 96 / pattern 10 /
   unknown 191`。**unknown 仍有 191 条（61%）**，没有把方法表猜到的
   变体写成 high confidence；`method-table` 标 medium，未知仍保留
   `Some or Ok`。
3. **闭包/迭代器作用域**：42 条断言 + 12 条 demand 标
   `scope=quantified`（迭代器实参或循环体）。K075
   `map_preserves_page_property` 的 2 条 `from_bits(...).unwrap()` 在
   原型中作为单次成功需求（过近似）；本轮标 `scope=quantified`，
   下游不会把它们计入单次义务。评级仍为 `needs_model_helper`（断言在
   helper），不改变 139/22 口径。
4. **控制流**：14 条 demand 标 `control_flow=let-else`、6 条 `loop`、
   2 条 `iterator`；断言侧 2 条 `if-let`、9 条 `iterator`、30 条
   `loop`。原型只记录文本顺序；本轮把分支/循环上下文显式交给下游。
5. **函数推导**：67 条 `receiver-type-inference`、26 条
   `associated-call`、10 条 `bare-call`、22 条 `method-name-only`、
   28 条 `unknown`。unknown 集中在动态/解构/宏生成接收者；研究锚点
   与首站选条均已精确解析（K007/K131/K016/K136/K015；K040/K109 用
   项目侧 `function_overrides` 钉定）。
6. **门控分类**：与原型逐条一致；未把 blocked 当"无需求"。

### 5.1 22 个非 blocked 候选的可运行性（首站边界）

| 组 | 数量 | 首站是否运行 | 原因 |
|---|---:|---|---|
| K007/K131（研究锚点） | 2 | ✅ | 公共 API + 现有骨架 + 项目 recipe |
| K145–K153 `range_difference_*` | 9 | ❌ | 义务是 `Iterator::next` 序列；骨架只有不可变 `let out = call()`，无法消费/断言迭代器 |
| K070/K071/K072/K073/K078/K080 页表 | 6 | ❌ | `cursor_mut` 的 tracked owner/ghost guards 需要项目侧铸造；`empty()` external_body；负向 range 受 `0 < va.end` 前置限制 |
| K068/K069 create_page_table | 2 | ❌ | 同上 + `MOCK_KERNEL_PT` 静态 |
| K075 page_properties | 1 | ❌ | helper + 闭包谓词，需要量化模型 |
| K012/K014 CpuSet | 2 | ❌ | 集合/迭代器量化 + 并发模型 |

因此"22 可注入"是**冻结树可回放性口径**（非 blocked），不等于"22 条
现有骨架可直接合成"；首站实际运行 2 条。这一区分是 M7b 的核心产出之一。

## 6. 117 个 blocked 的构成与影响

| 子类 | 数量 | 含义 |
|---|---:|---|
| `ktest_capture_gap` | 105 | 需求已提取；冻结 proof 快照未捕获该模块/测试 |
| 模块断连 | 8 | `io` 顶层模块在 `lib.rs` 被块注释；`arch/riscv` 未被选中 |
| 调度器/硬件/全局状态 | 4 | 真实属性但依赖运行时环境（`all_cpus`/`num_cpus`/激活页表等） |

另有 12 个带 capture-gap 的测试本身被排除（should_panic/无断言/
私有观察），所以 `capture_gap_tests=117` 与 `blocked=117` 不是同一集合。
blocked 候选全部保留在 `extracted-candidates.json`，没有被首站运行
"清洗"掉；它们是下一轮恢复 proof 捕获或补 recipe 的需求清单。

## 7. 与研究 anchors 对照

| 研究档案 | 本轮提取 | 首站运行 | 结论 |
|---|---|---|---|
| `mutex-try-lock-demand` | K131 `Mutex::try_lock`，`existence_acquisition`，3 条私有读排除 | ✅ ContractGapCandidate | 精确复现；与 registry F-013 同关系 |
| `memory-region-push-test` | K007 `MemoryRegionArray::push`，5 条 push 需求 + 16 条值断言 | ✅ ObligationSupported（push 成功 + len=5） | 需求复现；值读回未覆盖（`into_non_overlapping` external_body） |
| `range-allocator-alloc-specific` | K016 `IoMemAllocator::acquire`，6 条形状断言；`disconnected=true` | ❌ blocked（模块断连） | 需求复现；可回放性同档案结论 |
| 同上（负向控制） | K015 `illegal_region`，4 条 `is_none` | ❌ blocked | 需求复现 |
| `vmio-fallible-write` | K109 `VmWriter::write_val`/`VmReader::read_val`，2 unwrap + 1 值相等 | ❌ blocked（mm/test capture-gap） | 需求复现；不可回放 |
| `frame/refcount identity` | K040 `Frame::clone`（override），4 条关系断言 | ❌ blocked | 需求复现 |
| `guard-transitions` | 冻结 ktest 语料无 rwlock/rwmutex 测试 | — | 不适用（来源是 VIR owner-field，不是 ktest） |

## 8. 运行后恢复

- 运行前/后对 `ostd/src` 全部 **216 个文件**做 sha256 清单比对：
  **逐字节一致**（`/tmp/vostd-ktest-before.sha256` vs
  `/tmp/vostd-ktest-after.sha256`，`diff` 为空）。
- `ostd/src/spec_check_probes/` 已移除；`lib.rs` 无注入声明。
- 关键文件：`lib.rs f1413d24…`、`sync/mutex.rs eea4b78a…`、
  `boot/memory_region.rs f06a75ab…`（与运行前一致）。
- replay 副本在 `/tmp/vostd-divergence-m7b-run1/injections/`；
  预检 v1 证据在 `/tmp/vostd-divergence-m7b-precheck1/`。
- 判定报告副本：`examples/vostd-ktest/divergence-report-m7b.json`
  （与 `/tmp/vostd-divergence-m7b-run1/report.json` 相同）。

## 9. 证据边界

1. **ContractGapCandidate 是证明论结论**：K131 的义务在原契约下不可证
   且基线干净；不是运行时反例，不证明实现错误。
2. **supported 面是窄的**：K007 只覆盖 `push` 成功 + 后状态长度；不覆盖
   `into_non_overlapping` 的 16 条值读回。
3. **extraction rating 是语法口径**：`direct` 不等于"可表达"；页表负向
   range 的例子说明契约前置会让"运行时 Err"义务在 Verus 客户端不可写。
   首站只运行了 2 条，不能把这 2 条的判定外推到 139 候选。
4. **22 非 blocked ≠ 22 可合成**：只有 2 条有现成 recipe；其余 20 条
   缺 iterator/量化/ghost owner/集合模型，已在 §5.1 逐组记录。
5. **117 blocked 不是无需求**：105 capture-gap + 8 断连 + 4 环境，
   需求文本与断言位置都在候选 JSON 中。
6. **变体推断仍是局部的**：191/311 条 demand 无法从源码局部证据确定
   `Option`/`Result`；`method-table` 是项目侧 medium-confidence 表，
   不是 typed HIR。
7. **预检 v1 不计缺陷**：phase-1 `E0596` 是 client 形态问题，分类器按
   规则判 Inconclusive；修正 `receiver_mut` 后注册运行才产生判定。
