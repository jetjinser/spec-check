# ktest 断言 → 调用者义务候选：提取规则（原型）

- 目标：为 spec-check M7b（divergence 线的需求提取）提供确定性需求来源与提取规则。
- 输入：冻结源 `/tmp/vostd-43da079-ktest-chan/ostd/src`（vostd rev `43da079`，`#[ktest]` 门控）。
- 工具：`extract.py`（Python 3，正则 + 花括号配对；不构建、不依赖 Rust 工具链、只读）。
- 输出：`candidates.json`（schema `m7b-ktest-candidates-v1`）。
- 本文件只描述**规则**；统计、档案对照与 top 10 见 `REPORT.md`。

---

## 1. 语料与发现规则

### D1. 文件扫描
递归扫描 `ostd/src/**/*.rs`。`extract.py` 对每个文件构造两套文本：

1. `raw`：原样源码（用于记录断言片段与行号）；
2. `masked`：把注释、字符串、字符字面量替换为等长空格，保留换行与偏移（用于结构匹配）。

等长 mask 是硬约束：断言片段取自 `raw`、位置取自 `masked`，两者偏移必须一致，否则行号会漂移。

### D2. 测试定位
匹配 `#[ktest]`（允许 `#[ktest(...)]`），向后找到 `fn <name>(`，再以花括号配对取函数体。
测试的模块路径由三部分拼接：

```
文件路径模块前缀（lib.rs 丢弃、mod.rs 丢弃）
  + 花括号包含该测试的内层 mod 名
  + 函数名
```

示例：`sync/mutex.rs` 内的 `mod test` → `sync::mutex::test::test_mutex_try_lock_does_not_unlock`；`mm/test.rs` 内的 `mod io` → `mm::test::io::read_write_val_fallible`。该规则与 HIR 清单的运行时路径在 146/153 个测试上逐字一致（差异见 REPORT）。

### D3. 属性与门控
除 `#[ktest]` 外记录两类属性：

- `#[should_panic]` / `#[should_panic(expected=...)]`：整测试标记为 expected-panic；
- `#[cfg(... ktest_capture_gap ...)]`：proof-side 捕获缺口。

同时检测三个层级的门控：

1. **内层 mod**：`#[cfg(all(ktest, not(ktest_capture_gap)))] mod test { ... }`；
2. **文件模块声明**：`mm/mod.rs` 的 `#[cfg(...)] mod test;`、`mm/frame/mod.rs`、`mm/dma/mod.rs`、`mm/kspace/mod.rs` 等；
3. **crate 根断连**：`io` 顶层模块在 `lib.rs` 中被块注释掉；`arch` 选择的是 x86 路径而非 `arch/riscv`。

判定结果写为 `capture_gap` / `disconnected`，两者都是**可提取但当前 proof 环境不可回放**，不是“无需求”。

### D4. 断言与需求捕获
在函数体内捕获：

- `assert!(...)`、`assert_eq!(...)`、`assert_ne!(...)`（花括号/圆括号配对，支持跨行）；
- `.unwrap()`、`.expect(...)`（成功需求）；
- `let Some(...)` / `let Ok(...)` / `let Err(...)`（模式匹配需求，含 `let ... else panic` 形式）。

每条记录：类别、原始片段、行号、目标调用（接收者与方法名）。

### D5. 调用序列
按源码位置排序记录 `recv.method(...)`、`Type::assoc(...)`、裸函数 `helper(...)`。用于还原 obligation client 的直线前缀；不构建控制流图。

---

## 2. 断言模式分类

候选级（测试级）类别按优先级取：`state_transition > existence_acquisition > invariant_consistency > value_readback`。同一测试可同时带多个类别，`assertions[].category` 保留逐条判定。

| 类别 | 判据（源码形态） | 典型例子 | 可转写的义务 |
|---|---|---|---|
| `value_readback` | `assert_eq!/assert_ne!`，一侧是方法/字段读回，另一侧是字面量、参数派生值或另一读回；写后读回；布尔查询谓词 | `assert_eq!(frame.meta().value, 42)`；`assert_eq!(val, read_val)`；`assert_eq!(segment.size(), range.len())`；`assert!(result.contains(IsaExtensions::I))` | `let r = api(args); assert(r == expected)`；写后读回保留两次调用 |
| `existence_acquisition` | `.is_none()/.is_some()/.is_ok()/.is_err()`；`unwrap/expect`；`let Some/Ok/Err`；`matches!`；对返回 bool 的获取调用直接 `assert!` | `assert!(lock.try_lock().is_none())`；`let v: u64 = reader.read_val().unwrap()`；`assert!(allocator.acquire(range).is_some())`；`assert_eq!(rw1.get(), None)` | `let r = api(args); assert(r.is_none())`；`unwrap` → `assert(r.is_ok())`；`expect` 同 unwrap（消息忽略） |
| `state_transition` | 获取/释放/再获取；`drop` 后再分配/查询；lock→try_lock；map→unmap→query；clone→get；wake 两次/drop 后 wake；名字含 `try_lock|wake|twice|drop|unmap|alloc_and_free|activ|...` | mutex 测试；`drop(rw2); assert_eq!(rw1.get(), Some(1))`；`IrqLine::alloc_specific` 的 alloc→drop→realloc；`kvirt_area_*_drop` | 按序保留调用与绑定；`drop` 表达为作用域/所有权结束；对后置状态调用做断言 |
| `invariant_consistency` | 两个不同接收者的读回相等/关系；引用计数、paddr/vaddr 关系、属性保持、集合成员关系 | `assert_eq!(frame1.start_paddr(), frame2.start_paddr())`；`assert_eq!(frame1.reference_count(), 2)`；`assert_eq!(new_segment.start_paddr(), paddr_before)`；`assert_eq!(set_cpus, all_cpus)` | 需要导出关系谓词（如 `guard.rel_mutex`）或 tracked owner 模型；否则只能记为 `needs_model_invariant` |

统计（本语料）：断言级 `value_readback` 294、`existence_acquisition` 115、`invariant_consistency` 37；成功需求全部提取 310 条（非排除候选内 301 条：292 条无法仅凭正则判定 `Option::Some` 还是 `Result::Ok`，7 条 `Some`、2 条 `Err` 来自显式模式/类型）。候选级：value 61、existence 37、state 26、invariant 15。

### 2.1 逐条分类的细则

- `assert_eq!(x, Some(...))` / `assert_eq!(x, None)`：归入 existence（Option 形状 + 载荷值），而不是纯 value；例如 `rwarc::lockless_get`。
- `assert!(x.wake_up())` / `assert!(x.acquire(...))`：归入 existence（获取成功/失败），不是普通布尔值。
- `assert!(x.contains(y))`：若测试体内有循环或接收者是集合类 → invariant（成员一致性）；否则 value（查询谓词）。
- 两个不同接收者的方法读回比较 → invariant；同一接收者的标量读回 vs 字面量 → value。
- `assert!(cond.load(...))`、`.lock.load`、`INNERS[..]`、`.root`、`.callbacks`：私有/测试内观察，逐条排除（见 X4），但同一测试中保留的公共 API 断言仍可评级。

---

## 3. 提取规则（可产品化条目）

### R1. 定位被测函数
- 优先取**断言表达式中最内层的方法调用**（如 `lock.try_lock().is_none()` → `Mutex::try_lock`）；
- 若断言比较两个读回，取各自目标调用（如 `frame1.start_paddr` / `frame2.start_paddr`）；
- 若断言在 helper 内（如 `assert_range_difference`），取测试体对 helper 的调用参数作为被测函数输入，并把 helper 的断言内联；
- 若测试体只有 `unwrap/expect`，取被 unwrap 的表达式目标（`MemoryRegionArray::push`）。

原型用正则取“最后一个方法调用 + 关联函数调用”，不可靠时写 `target_call` 为空并在 `note` 记录；产品化应使用 typed HIR 的 resolved def span（研究档案 `ktest-hir-inventory` 已证明可行）。

### R2. 调用序列
- 保留 `let` 绑定、方法调用、`drop(...)`、裸 helper 调用；
- 记录行号以保持顺序；
- 不展开宏（`vec!`、`cpu_local!`）、不展开闭包、不做借用检查；
- obligation client 只保留到最后一个被断言调用之前的直线前缀（见研究档案的 `generated-client.rs`）。

### R3. 断言表达式
- `assert!(e)` → `assert(e)`；
- `assert_eq!(a, b)` → `assert(a == b)`；
- `assert_ne!(a, b)` → `assert(a != b)`；
- `.is_none()` → `assert(r.is_none())`；`.is_some()` / `.is_ok()` / `.is_err()` 同理；
- `unwrap()` → `assert(r.is_ok())`（`Option` 则为 `is_some()`）；`expect(msg)` 相同，消息不进入证明；
- `let Some(x) = r else { panic!() }` → `assert(r.is_some())` + 绑定 x；
- `matches!(r, Err(Error::InvalidArgs))` → `assert(r is Err(InvalidArgs))`（需要类型信息，原型只保留原文）。

### R4. 状态转换
按源码顺序生成调用链，例如：

```
let lock = Mutex::new(0);
let guard1 = lock.lock();
let r = lock.try_lock();
assert(r.is_none());
let _held = guard1;   // 保持 guard 存活到断言之后
```

关键点：drop/guard 生命周期必须显式保留（研究档案 `mutex-try-lock-demand` 的生成 client 即保留 `guard1`）。原型只记录调用序列与 `drop` 位置，生命周期需要产品化阶段用 HIR/MIR 重建。

### R5. 评级（translatability）
| 评级 | 含义 | 本语料数量 |
|---|---|---|
| `direct` | 公共 API + 可观察值/形状断言，无需额外模型 | 8 |
| `needs_model_helper` | 断言在本地 helper 中，需内联调用轨迹 | 10 |
| `needs_model_quantified` | 循环/迭代器/集合，需要量化不变量 | 4 |
| `blocked` | 当前 proof 快照不可回放（capture-gap / 模块断连 / 调度器环境） | 117 |
| `excluded` | 不是调用者义务（should_panic、私有观察、无断言等） | 14 |

评级只看**可转写性**，不看需求价值：`blocked` 的 mutex/allocator 类需求仍是高价值候选，只是当前冻结树无法直接注入验证。

### R6. helper 断言归属
仅对同文件、名字以 `assert`/`check` 开头的本地 helper 做一层内联（`assert_range_difference`、`check_map_with_property`）。归因后保留 `via_helper` 字段；同一 helper 被 9 个测试复用时，断言会在 9 个候选中重复出现（统计时注明）。不做跨文件 helper 解析。

---

## 4. 排除规则

| 编号 | 排除对象 | 例子 | 理由 |
|---|---|---|---|
| X1 | `#[should_panic]` 测试 | `failing_assertion`、`read_beyond_buffer_infallible`、`unaligned_map`、`max_segment_creation` | panic 是预期行为，现有 obligation client 无 panic 模型；HIR 生成器同样记为 `expected-panic-not-modeled` |
| X2 | 测试框架自测 | `lib.rs` 的 `trivial_assertion`（冻结树中已被块注释） | 与 API 无关 |
| X3 | 无断言且无成功需求 | `queue_wake_one`、`queue_wake_all` | 仅执行不 panic，不构成可断言义务 |
| X4 | 私有/测试内观察 | `assert!(lock.lock.load())`、`INNERS[..].callbacks.len()`、`page_table.root.borrow()`、`BAR.load()` | 外部 client 不可见；mutex 测试正是排除 3 条私有观察、只保留 `try_lock().is_none()` |
| X5 | 恒真断言 | `assert_eq!(0, 0)` | 非需求（冻结树中未出现） |
| X6 | 分配器复用假设 | `frame_drop`/`segment_drop` 的 `new_*.start_paddr() == paddr_before` | 假设“释放后立即复用同一物理页”，是测试对分配器策略的假设，不是 API 契约 |
| X7 | 环境/调度器假设 | `all_cpus()`/`num_cpus()`、`KERNEL_PAGE_TABLE`、`VmSpace::activate`、`TaskOptions`/`wait`/`wake_up`、IRQ 全局表 | 是真实属性但依赖运行时环境；标为 `blocked` 而非 `excluded`，保留需求记录 |
| X8 | proof-side 捕获缺口 | `ktest_capture_gap` 门控模块、`io` 顶层模块被注释、`arch/riscv` 未被编译 | 需求存在，但冻结 proof 树没有对应 API/模块；标为 `blocked`，并在报告中作为 M7b 的“需恢复捕获”清单 |
| X9 | 性能/计时断言 | 本语料无 | — |

注意：X3 的 `queue_wake_one/all` 是“只验证不 panic”的测试；若 M7b 需要 no-panic 义务，应新增独立类别，而不是塞进现有类别。

---

## 5. 不可提取 / 高误报模式

1. **闭包内的 unwrap/expect**：`SubsetIter::new(...).map(|f| PageFlags::from_bits(f).unwrap())` 被计为“成功需求”，但它是逐元素、循环语义；`map_preserves_page_property` 因此出现 2 条过近似需求。需 HIR/MIR 区分“单次调用”与“迭代器元素”。
2. **控制流分支**：`if let Some(..)` / `let ... else panic` 只记录存在性需求，不建模分支路径；真正的义务可能是“满足某前置时返回 Some”。
3. **宏生成代码**：`cpu_local!`、`vec!`、`assert!` 内部的嵌套宏不展开；`cpu_local` 测试的 `BAR`/`FOO` 只能按测试内符号处理。
4. **私有字段/全局表**：`.root`、`.inner`、`.callbacks`、`INNERS` 无法在外部 client 表达；`guard-transitions` 档案明确指出“外部 client 需要导出的 spec predicate”。
5. **生命周期与借用**：引用计数、guard 存活、`drop` 的精确时点只能从 HIR/MIR 恢复；正则版只记录文本顺序。
6. **`unwrap` 的变体**：纯文本无法区分 `Option::unwrap` 与 `Result::unwrap`；默认写 `Some or Ok`，产品化需类型/HIR。
7. **跨文件 helper**：原型只内联同文件 `assert*/check*`；`setup_page_table_with_two_frames` 等 setup helper 保留为调用名，需要人工/产品化展开。
8. **门控判定是启发式**：只识别 `ktest_capture_gap` 与 `lib.rs`/`arch` 两类断连；任意 `cfg` 组合、features、target 条件未建模。

---

## 6. 断言 → proof 断言形式对照

| ktest 形态 | obligation client 形态 | 备注 |
|---|---|---|
| `assert!(m.try_lock().is_none())` | `let r = m.try_lock(); assert(r.is_none());` | 保留 `guard` 存活 |
| `assert!(alloc.acquire(rg).is_some())` | `let r = alloc.acquire(rg); assert(r.is_some());` | 前置：构造 allocator 的 range |
| `assert_eq!(g.value(), 5)` | `let r = g.value(); assert(r == 5);` | 值读回 |
| `let v: u64 = reader.read_val().unwrap()` | `let r = reader.read_val(); assert(r.is_ok());` | 变体需类型 |
| `assert_eq!(rw1.get(), Some(1).as_ref())` | `let r = rw1.get(); assert(r == Some(&1));` | Option 形状 + 载荷 |
| `let Some(va) = cursor.find_next(x) else { panic!() }` | `let r = cursor.find_next(x); assert(r.is_some());` + 绑定 | 分支被拍平 |
| `assert_eq!(f1.start_paddr(), f2.start_paddr())` | 需要 `assert(f1.same_owner(f2))` 或导出 paddr 关系谓词 | 无谓词则 `needs_model_invariant` |
| `assert!(waker.wake_up())` / `!waker.wake_up()` | `let r = waker.wake_up(); assert(r)` / `assert(!r)` | 获取布尔语义 |

client 形态遵循 `spec-check/examples/vostd-divergence/divergence-manifest.json`：真实 API 调用 + `assert(义务)`；`Option` 用 `if let Some(g)` 解包，`Result` 用 `assert(r.is_ok())`。

---

## 7. 复现

```sh
python3 /tmp/m7b-ktest/extract.py /tmp/vostd-43da079-ktest-chan/ostd/src \
  --out /tmp/m7b-ktest/candidates.json
```

输出 `stats` 块包含测试数、候选数、评级/类别分布、断言与需求计数；`candidates[]` 按 `priority_score` 降序，每条含 `test/file/line/body_span/capture_gap/disconnected/category/assertions/demands/call_sequence/rating/rating_reasons/research_anchor`。

### 产品化建议（M7b）
1. 用 typed HIR 替换正则：resolved def span、`Option/Result` 变体、顶层语句、精确 span（研究档案已给出生成器）。
2. 用 VIR 判定可见性/导出谓词，避免把 `.inner` 类关系写成外部 client。
3. 把“冻结树可回放性”做成独立 gate manifest（`ktest_capture_gap`、模块断连、crate root 注释），不要与“需求是否存在”混为一谈。
4. helper 内联与生命周期恢复放在 HIR 层做；正则原型只用于候选发现与人工审阅。
