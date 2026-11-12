# M7a census 运行记录

## 1. 环境与版本

- 工具仓库：`~/vie/work/AES/spec-check`（master，未 push）。
- Verus SDK：`/home/jinser/vie/work/AES/vostd/tools/verus`（`vostd-pr830`
  与 `vostd` 共用同版本）。
  - `VERUS_USE_RUSTUP=0 .../verus --version` →
    `0.2026.10.08.d128dda`（release）。
  - Rust toolchain：`1.98.1`。
- VIR 导出工作树：`~/vie/work/AES/vostd-pr830`
  （`43da0797bbfa4085749efee9585231e43c656a98`，干净）。
- 校准 artifact：`/tmp/jinser-bzyA6G/ensures-coverage/`
  （workflow ref `b6db6df974cf0af881255cf88613836613fc6ce4`，
  Verus `0.2026.11.11.04fc24f`）。
- 所有构建/测试在 nix 内：
  `nix develop /home/jinser/vie/work/AES/vostd/nix -c ...`。

## 2. 导出 VIR（43da079）

`vostd-pr830` 的 `dv` CLI 使用 `--targets`（b6db6df 的较新 `dv` 使用 `-p`）：

```bash
cd ~/vie/work/AES/vostd-pr830
nix develop /home/jinser/vie/work/AES/vostd/nix -c bash -c '
  export VERUS_USE_RUSTUP=0 \
         VERUS_Z3_PATH=/home/jinser/vie/work/AES/vostd-pr830/tools/verus/source/z3 \
         RUSTC_BOOTSTRAP=1
  cargo dv focus --targets ostd -- \
    --no-verify --log vir --log-dir /tmp/m7a-vir-export2
'
```

- 输出：`/tmp/m7a-vir-export2/crate.vir`（98 MB）。
- `--no-verify` 只跳过证明；VIR 仍写出。若 cargo 缓存导致不写文件，
  使用新的 `--log-dir` 并清理目标 crate（本轮首次导出已产生完整 VIR）。
- VIR 含 ostd 及其依赖（vstd/vstd_extra/bitflags 等）的函数记录；
  census 按 `:name :path` 的 `ostd::` 前缀筛选，并用 `_VERUS_VERIFIED_`
  规范化包装函数。

## 3. 运行 census

```bash
cd ~/vie/work/AES/spec-check
nix develop /home/jinser/vie/work/AES/vostd/nix -c bash -c '
  cd /home/jinser/vie/work/AES/spec-check
  cargo build --offline
  ./target/debug/cargo-spec-check census \
    --vir /tmp/m7a-vir-export2/crate.vir \
    --crate ostd \
    --out /tmp/m7a-census-43da079
'
```

输出：`/tmp/m7a-census-43da079/census.json`（2.7 MB），stdout 汇总：

```
census: 1468 functions (826 exec, 642 proof), 1268 with postcondition
(1311 outputs, 52 findings), 200 without (171 outputs, 171 findings)
```

单测：

```bash
cargo test --offline
# 100 passed / 0 failed / 72 ignored（其中 7 个 census 单测）
```

## 4. 校准

```bash
cd ~/vie/work/AES/spec-check
python3 examples/vostd-census/calibrate.py \
  --artifact /tmp/jinser-bzyA6G/ensures-coverage \
  --census /tmp/m7a-census-43da079/census.json \
  --out /tmp/m7a-calibration.json

python3 examples/vostd-census/summarize.py \
  --census /tmp/m7a-census-43da079/census.json \
  --out examples/vostd-census/census-summary.json
```

`calibrate.py` 用 `head.results` 的 `(ret ...)` / `(mut ...)` 描述符作为
逐函数 finding 事实（描述符只在实际 finding 时出现；总数 235 与
`all.md` 汇总一致），因此不需要解析散文报告的分组标题。

## 5. 精确修订 b6db6df 的尝试（失败）

```bash
cd ~/vie/work/AES/vostd
git fetch --no-tags upstream b6db6df974cf0af881255cf88613836613fc6ce4
git worktree add /tmp/vostd-b6db6df b6db6df974cf0af881255cf88613836613fc6ce4
cd /tmp/vostd-b6db6df
git submodule update --init dv    # 5ba6ba4ebf2e5e59f05b6eaddfe8dfc8554fb740
# tools/verus 不在该修订的仓库内；临时 symlink 到本地 SDK
ln -sfn ~/vie/work/AES/vostd-pr830/tools/verus tools/verus

nix develop /home/jinser/vie/work/AES/vostd/nix -c bash -c '
  cd /tmp/vostd-b6db6df
  export VERUS_USE_RUSTUP=0 \
         VERUS_Z3_PATH=/tmp/vostd-b6db6df/tools/verus/source/z3 \
         RUSTC_BOOTSTRAP=1
  cargo dv focus -p ostd -- --no-verify --log vir --log-dir /tmp/m7a-vir-b6db6df
'
```

失败 1：`x86_64 0.15.5` 的 `Step::forward_overflowing/backward_overflowing`
在 Rust 1.98.1 中不存在（artifact 修订的 `rust-toolchain.toml` 要求
1.99.1）。临时把 `x86_64` crate 拷到 `/tmp/x86_64-0.15.5-patched` 并删掉
这两个方法后重试。

失败 2：`vstd_extra` 的
`use vstd::std_specs::{cmp::*, nonzero::*, ptr::*};` 无法解析
`std_specs::{nonzero, ptr}`——本地 vstd 0.2026.10.08 缺少 artifact
所用 Verus 0.2026.11.11 的新 API。

结论：精确修订需要 Rust 1.99.1 + Verus 0.2026.11.11；本轮没有该组合，
所以 census 在 43da079 上运行，校准按路径交集进行，差异归因见
`CALIBRATION.md` §3.4–3.5。临时 worktree 只用于源码对照，未修改
`~/vie/work/AES/vostd` 工作树文件。
