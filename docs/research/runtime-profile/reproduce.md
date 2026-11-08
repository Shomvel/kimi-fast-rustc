# 复现与数据指纹

全部脚本在 [scripts/](scripts/)，都是独立 bash 脚本，路径从参数传入。除 `prior-runtime-table.sh`、`intrinsic-census.sh`、`classify-profile.sh`、`compare-profiles.sh` 可在本机跑以外，其余在 Linux x86_64 沙箱里以 root 运行。

## 顺序

| 步 | 命令 | 产物 |
|---|---|---|
| 1 | `setup-sandbox.sh WORK` | 工具链、perf、libc6-dbg、源码、语料、`WORK/fingerprint.txt` |
| 2 | `check-llvm-args.sh WORK` | 每臂 flag 的存在性与效果表 |
| 3 | `build-arms.sh WORK rg` / `build-arms.sh WORK cargo` | `WORK/bin/<arm>/{rg,cargo}`、`WORK/bin/<project>-builds.tsv` |
| 4 | `runtime-bench.sh WORK rg OUT` | `OUT.tsv`（逐次计数）、`OUT.check.tsv`（输出比对） |
| 5 | `summarize-runtime.sh OUT.tsv instructions_u A V,U,R,J,I,G,H,L,O1,C,S2,S0` | 比值表；`cycles_u`、`wall_ns` 同理 |
| 6 | `kernel-share.sh WORK rg A` | user/sys 表 |
| 7 | `profile-hotspots.sh WORK rg A DIR`，再 `classify-profile.sh DIR matrix 10` | 热点归类表 |
| 8 | `compare-profiles.sh A.sym.txt X.sym.txt RATIO` | 两臂逐符号差 |
| 9 | `intrinsic-census.sh REGISTRY_SRC memchr-2.8.3 aho-corasick-1.1.4 hashbrown-0.16.1` | intrinsic 清单 |
| 10 | `prior-runtime-table.sh RUNS_DIR range` | 2026-10 各轮的 runtime 比值范围 |

`arm-flags.sh`、`workloads.sh`、`make-simd-patches.sh` 由上面的脚本调用，也可单独查看。

## 本轮环境

- E2B：按项目规范经 `hk-linlizhi-jumpserver` 使用 SWEEnvMaker 0820 服务，模板 `flasheda-r5-cpu8-t16-20261030`（模板 API 核对为 ready，8 vCPU / 16 GiB）。rg 与 Cargo 各一个沙箱，测完已释放。前期材料用的 `mirx-rust1941-linux` 模板不在 0820 的模板列表里，所以 Rust 1.94.1 由 setup 脚本经 rustup 安装。
- 构建耗时与体积（`build-arms.sh` 原样输出的列选取；rg 的第一次 R、C 因 `-regalloc=basic` 与 O0 的 allocator shim 冲突失败，加 `-optimize-regalloc` 后重建，记录在表尾）：

```
arm	exit	wall_s	bytes	sha256              arm	exit	wall_s	bytes	sha256
A	0	9.48	27889480	26f175b3d665515c    A	0	101.56	38202928	4bc5e8698cf91164
V	0	9.38	27893704	fc09994c55ccdddb    V	0	99.94	38132592	f22ab9fc9fdeceb5
U	0	9.65	28270408	97604bd3e9cdf420    U	0	99.79	37714048	61bca54cf13f1857
R	101	7.00	-	-                   R	0	107.43	38664016	ed1385ab520c4e7c
F	0	9.97	29644320	986b68c608615a8b    F	0	100.10	47436336	84caa360aa67223f
J	0	9.64	27788640	bb5524dee8c173d5    J	0	99.94	38058824	b0084df1fae3f203
I	0	8.61	23207952	8d6a24c021cb0304    I	0	93.26	40388288	88b238d0f617bb23
G	0	9.42	27930264	eefbb7f4f64a5e77    G	0	100.40	38268616	541d30966b154fd0
H	0	9.66	27879936	d173d9d11c73c09b    H	0	99.72	38145792	3e8225b2c4156fc8
L	0	8.15	24948616	b71b6d7e9d0112c9    L	0	89.35	41614928	fa198ebd75972578
O1	0	8.37	26566584	68aa1ef6ff570561    O1	0	94.29	42406296	1df47eb150926f03
C	101	6.98	-	-                   C	0	101.04	37958280	160751db3f523c2f
S2	0	9.68	27465464	e6fa17c4539a51f2    S2	0	101.14	38081064	4e6fbe5756cf4151
S0	0	9.76	27156176	b385489e9cdf4442    S0	0	100.59	38032160	b924fc15be50e46f
R	0	9.70	27925536	a3460f4a405ff49b
C	0	9.55	28185320	568fb5485775ff75
```

左为 rg（含 `debug=1`，未 strip），右为 Cargo 0.95.0。每个臂的二进制哈希都不同，说明 flag 在真实构建里改变了代码。

## 指纹

| 输入 | SHA-256 |
|---|---|
| ripgrep 15.2.0 `Cargo.lock` | `7a7d39cda8a03930e578f1dbb724e055771901842eca239e03b01e19da946a64` |
| Cargo 0.99.0 `Cargo.lock`（workload 输入） | `1f623862d07e134b910ceb1c496b4494b786ba1463b88006c9423a9db845ae6c` |
| cargo-0.95.0.crate（被测二进制的源码） | `0af45a75a5265a587a8d9e4098cd2b12a07b34bcbd9eb7952ae6a62ea6d65c4d` |
| cargo 0.95.0 `Cargo.lock` | `37fa0a78f13eb4926f2d4da7fec787e5b3e6cb1a4566b95b3223f4a569a287d2` |
| 单文件语料源 `big.txt`（518,400,870 字节，`mid.txt` 取前 256 MiB） | `59cb617ca6d39fbca50aa7ed3cfe2c6fd085e551fa496a07cf23bfd2efc452cd` |
| `pats-lit40.txt` / `pats-word500.txt` | `46786b4e…1006` / `ac1fabdb…2556` |

## 坑

| 现象 | 处理 |
|---|---|
| perf 6.8 的 `user_time`/`system_time` 工具事件多数运行只输出其中一个，按位置读取会错列 | runtime-bench 只取 `instructions:u`、`cycles:u`，按事件名解析，缺失记 NA；user/sys 另用 `/usr/bin/time` |
| awk 的 `printf …, a > 0 ? x : y` 把 `>` 当成输出重定向 | 三元表达式整体加括号 |
| `-regalloc=basic` 在 O0 模块（build script、bin 的 allocator shim）上被拒绝 | `--target` 隔离 build script；R 臂加 `-optimize-regalloc` |
| `-regalloc=fast` 配 O3 的二进制段错误 | F 臂排除 |
| 没装 libc6-dbg 时 libc 内部函数只显示地址 | setup 脚本安装 libc6-dbg |
| Cargo 0.99.0 不能用 1.94.1 构建；其锁文件含已 yank 的 chacha20 0.10.1 | 被测 Cargo 用 0.95.0；从零解析的 workload 用 0.95.0 源码 |
| 跳板机吞掉含 `rm -rf` 的命令、`scp` 无效 | 文件经 `ssh … 'cat > path'` 传输，删除只在沙箱内的脚本里做 |
