# M7b ktest 义务首站预注册（运行前写定，运行后不改写；2026-11-12）

对象：vostd 冻结源（rev `43da079`，`/tmp/vostd-43da079-ktest-chan`）中由
`cargo-spec-check extract-ktest` 提取、经 `synthesize` 生成的 **2 条 ktest
义务 client**，由 `analyses-divergence` 执行。判定只以 divergence 实跑为
准；本文件是运行前假设，不参与判定，也不在运行后修改。

## 1. 冻结输入

| 输入 | 路径 | sha256 |
|---|---|---|
| 提取候选（153 条记录 = 139 候选 + 14 排除） | `examples/vostd-ktest/extracted-candidates.json` | `6b99754cd8c602365564de10ea674cc5c3d21b84d38995d8331ecb8731b5219a` |
| 项目侧配方 | `examples/vostd-ktest/recipes-m7b.json` | `82366c089abc962220acc1d6e3da0666dc8a4ab70e51a8b9b4fd91b0630a0756` |
| 合成 manifest（2 条） | `examples/vostd-ktest/divergence-manifest-m7b.json` | `a18105ce4a05e14e4bf1405c9806a722b217014ae7b6b7f6c8c5511da8b23e9f` |
| 合成报告 | `examples/vostd-ktest/divergence-manifest-m7b.report.json` | `d12483f55c0c918911701132c6d89a53f789de013845ebac5642c9fa232cb37e` |

合成统计（机械）：153 条输入记录中 `--select K007,K131` 选中 **2 条**，
151 条跳过（`not-selected`；其中 117 blocked、10 needs-model-helper、
4 needs-model-quantified、其余 excluded/other）。跳过项不进入 manifest。

## 2. 选条：为什么只有 2 条

提取面 139 个非排除候选，其中 **22 个非 blocked**（8 direct + 10
needs_model_helper + 4 needs_model_quantified）。本首站只选"当前冻结树
+ 现有 M7c 骨架 + 项目 recipe 能表达"的候选：

| 候选 | 测试 | 函数 | 评级 | 选入理由 |
|---|---|---|---|---|
| K007 | `boot::memory_region::test::test_sort_full_non_overlapping` | `MemoryRegionArray::push` | direct | 研究锚点 `memory-region-push-test`；`push` 有 ensures，可验证"成功 + 后状态长度" |
| K131 | `sync::mutex::test::test_mutex_try_lock_does_not_unlock` | `Mutex::try_lock` | direct | 研究锚点 `mutex-try-lock-demand`；`try_lock().is_none()` 是干净的契约缺口形状 |

**未选入的 20 个非 blocked 候选**（如实记录，不硬造）：

| 候选组 | 数量 | 原因 |
|---|---:|---|
| `util::ops::range_difference_*`（K145–K153） | 9 | helper 内联后义务是 `Iterator::next` 序列；M7c 骨架只有 `let out = call()` 的不可变绑定，无法消费/断言迭代器序列；需要新的 iterator 骨架或项目侧 helper 模型 |
| `mm::page_table::test::*`（K070/K071/K072/K073/K078/K080） | 6 | `PageTable::cursor_mut` 的 `with Tracked(owner)/Ghost(root_guard)/Tracked(regions)/Tracked(guards)` 需要项目侧 ghost owner 铸造；`empty()` 是 `external_body`；负向 range 还要求 `0 < va.end` 前置，空区间义务在契约下不可表达 |
| `mm::page_table::test::create_page_table::*`（K068/K069） | 2 | 同上，且需要 `MOCK_KERNEL_PT` 静态与页表 owner |
| `mm::page_table::test::page_properties::map_preserves_page_property`（K075） | 1 | 断言在 `check_map_with_property` helper + 闭包 `map(|f| ...unwrap())`；需要量化/谓词模型 |
| `cpu::set::test::*`（K012/K014） | 2 | `iter().collect()` 的集合空性/成员关系需要量化不变量；`AtomicCpuSet` 还需并发模型 |

研究锚点 K016（`IoMemAllocator::acquire`，`io` 模块断连）和 K109
（`VmWriter::write_val` / `VmReader::read_val`，`mm/test` capture-gap）
本身是 blocked，不在运行面；K136（RwArc）与 K040（Frame clone）同样
blocked。提取器仍保留它们的候选与义务文本。

## 3. 逐条预测

期望值映射：`supported → ObligationSupported`、`gap → ContractGapCandidate`；
`Inconclusive`/`Anomaly` 不作为预注册值。

| # | entry | 函数 | 族 | 合成后的义务（摘要） | 预期 | 运行前依据 |
|---|---|---|---|---|---|---|
| 1 | `c007_memoryregionarray_push` | `MemoryRegionArray::push` | value-readback | 4 次前置 push + 第 5 次 `push`，断言 `out.is_ok()` 且 `recv@.len() == 5` | **ObligationSupported** | `push` ensures：`old(self)@.len() < LEN ==> final(self)@ == old(self)@.push(region) && ret.is_ok()`；`LEN=64`，第 5 次前 len=4，契约字面可推 |
| 2 | `c131_mutex_try_lock` | `Mutex::try_lock` | is_none-demand | 持有 `lock()` guard 后 `try_lock()`，断言 `out.is_none()` | **ContractGapCandidate** | `try_lock` 的 ensures 只有 `ret is Some ==> ret->0.rel_mutex(*self)`；不约束返回 `None`。"持锁时 try_lock 返回 None" 是运行时/文档事实，契约不表达 |

**预注册分布**：ObligationSupported 1 / ContractGapCandidate 1。

## 4. 已知运行风险（不改变预测）

1. K007 的 `recv@.len() == 5` 依赖 `View` 在 probe 模块内可展开；若
   Verus 不能把 `push` ensures 的 `final(self)@` 与局部 `recv@` 连接，
   该条会判 gap（预注册 supported 不命中），按实测改判并在 RUN 文档
   逐条归因。
2. K007 的 4 次前置 `push(...).unwrap()` 在 Verus 下需要 `Result::unwrap`
   的可证前置；若编译阶段失败，phase-1 会把**全部**条目判 Inconclusive。
3. K131 的 `let _guard = recv.lock();` 若因 `Mutex::lock` 的 guard
   生命周期/类型检查失败，同样 phase-1 全 Inconclusive。
4. 义务 fail ≠ 运行时反例；`expect_match` 只作机器可查假设的命中记录。
5. baseline 预期：vstd 依赖 0 errors；目标 crate 探针模块外 0 errors。
   若 baseline 不干净，分类器会判 Anomaly，按实测记录。

## 5. 运行命令（预注册）

```
timeout 900 nix develop ~/vie/work/AES/vostd/nix -c bash -c \
  'cd ~/vie/work/AES/spec-check && \
   ./target/debug/cargo-spec-check analyses-divergence \
     --manifest examples/vostd-ktest/divergence-manifest-m7b.json \
     --cargo-verus /home/jinser/vie/work/AES/vostd/tools/verus/source/target-verus/release/cargo-verus \
     --z3 /home/jinser/vie/work/AES/vostd/tools/verus/source/z3 \
     --vstd /home/jinser/vie/work/AES/vostd/tools/verus/source/vstd \
     --out /tmp/vostd-divergence-m7b-run1'
```

## 6. 证据边界（预注册）

- 本轮只覆盖 2 条 ktest 需求，不代表 139 候选的整体判定；22 个非 blocked
  中另外 20 条因骨架/ghost/量化缺口未运行，117 个 blocked 未进入运行面。
- K007 的断言只覆盖 `push` 的成功与后状态长度，**不覆盖**原测试的 16 条
  `into_non_overlapping` 值读回（该函数是 `external_body`，无 ensures）。
- K131 只覆盖 `try_lock().is_none()`，不覆盖原测试的 3 条私有
  `lock.lock.load` 读（提取阶段已逐条排除）。
- `ContractGapCandidate` 是证明论结论：义务在原契约下不可证且基线干净，
  不是运行时反例，也不证明实现错误。

## 6. 编写期预检（注册运行前）

### 6.1 预检 v1（`/tmp/vostd-divergence-m7b-precheck1`）

- 使用首版 recipe（无 `receiver_mut`）与 manifest（sha256
  `db073297…`，已被本节后的修正取代）。
- 结果：**2/2 Inconclusive**；phase-1 `cargo-verus check --no-verify` 报
  `E0596: cannot borrow recv as mutable, as it is not declared as mutable`
  （`c007_memoryregionarray_push.rs:14`）。两项按 divergence 规则落
  Inconclusive，不计缺陷。
- 根因：M7c 的 value-readback/is_none-demand 骨架硬编码 `let recv = ...`；
  `MemoryRegionArray::push` 需要 `&mut self`。这是 client 可表达性问题，
  不是契约缺口。
- 修正：合成器新增通用 `receiver_mut` recipe 字段（`let mut recv`），
  K007 recipe 置 true；K131（`&self`）保持 false。新增单测
  `mutable_receiver_binding_is_supported`。预测（§3）不变。
- 本节记录发生在**注册运行之前**；§1 哈希已更新为修正后的输入。
