# 统一测量与产物账本

这是产物审阅的数据附录。性能候选统一为 transport；ranges 仅有静态产物和正确性验证。原始样本使用21轮逐round配对比值中位数，区间由4000次bootstrap产生，只描述本次实验内部波动。各workload等权几何平均不是线上流量权重。

## 运行与编译

| crate / workload | transport / baseline | 95%区间 | transport / LLVM | 动态指令 / LLVM |
|---|---:|---|---:|---:|
| semver / parse | 0.9270 | [0.9176, 0.9360] | 3.755× | 3.592× |
| semver / req | 0.9113 | [0.8987, 0.9233] | 6.851× | 5.627× |
| semver / identifier | 0.9989 | [0.9947, 1.0083] | 1.693× | 1.648× |
| base64 / encode | 0.8501 | [0.8398, 0.8569] | 8.249× | 4.355× |
| base64 / decode | 0.8464 | [0.8301, 0.8507] | 6.823× | 4.162× |
| base64 / short | 0.8575 | [0.8496, 0.8646] | 3.830× | 2.786× |
| unicode-segmentation / graphemes-mixed | 0.9882 | [0.9838, 0.9931] | 1.323× | 1.329× |
| unicode-segmentation / graphemes-ascii | 0.9939 | [0.9872, 0.9969] | 1.089× | 1.045× |
| unicode-segmentation / graphemes-rev | 0.9857 | [0.9733, 1.0035] | 3.731× | 3.510× |
| humantime / dur-parse | 0.8289 | [0.8249, 0.8395] | 8.660× | 6.669× |
| humantime / dur-format | 0.9486 | [0.9379, 0.9574] | 1.133× | 1.150× |
| humantime / ts-parse | 1.0003 | [0.9918, 1.0095] | 3.117× | 4.105× |
| humantime / ts-format | 1.0351 | [1.0335, 1.0387] | 1.714× | 1.973× |

| crate | 编译wall / baseline | 95%区间 | 编译CPU / baseline | 编译指令 / baseline |
|---|---:|---|---:|---:|
| semver | 1.1081 | [1.0986, 1.1473] | 1.0361 | 1.0720 |
| base64 | 1.1190 | [1.1142, 1.1368] | 1.0577 | 1.1234 |
| unicode-segmentation | 1.0508 | [1.0340, 1.0691] | 1.0341 | 1.0442 |
| humantime | 0.8338 | [0.8150, 0.8440] | 0.8641 | 0.8235 |

## 全部函数体盘点

| crate | arm | 函数体 | 符号字节 | 指令 | 静态call | 静态branch |
|---|---|---:|---:|---:|---:|---:|
| base64 | baseline | 90 | 28175 | 6860 | 279 | 778 |
| base64 | llvm | 35 | 8648 | 2168 | 53 | 277 |
| base64 | ranges | 72 | 24444 | 5961 | 180 | 614 |
| base64 | transport | 72 | 26196 | 6336 | 219 | 698 |
| humantime | baseline | 98 | 58034 | 14423 | 317 | 2835 |
| humantime | llvm | 32 | 10426 | 2651 | 60 | 450 |
| humantime | ranges | 82 | 36305 | 9136 | 322 | 1479 |
| humantime | transport | 82 | 37393 | 9442 | 322 | 1551 |
| semver | baseline | 194 | 32705 | 9500 | 512 | 1390 |
| semver | llvm | 75 | 18515 | 5061 | 189 | 831 |
| semver | ranges | 171 | 31089 | 9014 | 459 | 1310 |
| semver | transport | 171 | 31114 | 9021 | 459 | 1312 |
| unicode-segmentation | baseline | 78 | 17878 | 5314 | 218 | 936 |
| unicode-segmentation | llvm | 24 | 8398 | 2356 | 46 | 450 |
| unicode-segmentation | ranges | 76 | 17535 | 5225 | 211 | 926 |
| unicode-segmentation | transport | 76 | 17535 | 5225 | 211 | 926 |

完整1428行见 [function-inventory.csv](function-inventory.csv)。LLVM别名可能对应同一反汇编函数体，不重复计；静态call site不能代替动态执行次数。函数外对齐不计入符号范围，整个.text仍包括这些字节。

## ELF对象增长的精确归因

下表为transport减baseline，单位B。headers含ELF header与section headers；strings含全部SHT_STRTAB；symbols与relocations分别按ELF section type计数。code按SHF_EXECINSTR，rodata仅按`.rodata*`名称，其他含`.data.rel.ro*`等，所以本表rodata与旧suite汇总的只读数据口径不同。NOBITS不计磁盘字节，padding/gaps包含所有未归类文件间隙。各项与总文件长度严格相等，200个object均核对了原始SHA-256。

| crate | object总增量 | headers | strings | code | relocations | symbols | rodata | other | gaps |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| semver | +88504 | +45760 | +47063 | -2938 | -1440 | -888 | -498 | -96 | +1541 |
| base64 | +76328 | +40064 | +42229 | -2582 | -2208 | -1200 | -492 | -96 | +613 |
| unicode-segmentation | +75328 | +31680 | +43923 | -955 | -216 | -96 | -7 | +0 | +999 |
| humantime | +81680 | +49216 | +52665 | -21316 | -192 | +696 | -195 | -24 | +830 |

机器码收缩与object增大可以同时成立。单个base64根函数的GC consumer实验中，链接保留code由13654B降到31B（LLVM27B），只读数据由5059B降到0；真实stripped consumer总文件由376904B降到350160B（LLVM350192B）。这证明该consumer的链接回收有效，不代表四库所有消费者已接近LLVM。

## 证据与复现

持久证据目录：`/Users/shom/Downloads/kfrc-artifact-fixes-20261110/systematic-review-evidence/evidence/results`。

接手时已从现有9个分片拼接完整`review-evidence.tar.gz`（70,021,780 B），逐片、完整压缩包及归档内3,496个清单文件均通过SHA-256核验。完整归档保存在上述`results`目录的上两级；日常分析使用已展开的compact/index证据。归档回执见[evidence-receipt.json](evidence-receipt.json)。重新运行两份复算脚本后，函数CSV、静态汇总、ELF字节账本、测量汇总四份文件与原结果逐字节一致。这是已有证据复核，未新增运行或编译测量。

- `static-review/`：四arm汇编、函数表和对象哈希。
- `suite/{arm}/{crate}/ref/objects`：原始ELF对象。
- `suite/runtime/*.json`：transport运行原始样本与perf计数。
- `suite/compile-final.json`：transport编译配对样本。
- `graphs/`、`wrappers-graphs/`：较早候选的选定S/L dump；必须结合command文件识别版本，不能当作ranges图。
- [inputs.json](inputs.json)：源码覆盖文件及哈希。
- [measurement-summary.json](measurement-summary.json)：统计结果与方法。
- [object-sections.json](object-sections.json)：全arm ELF字节账本。

复算：

```sh
python3 ../artifact-fixes/summarize.py "$RESULTS" measurement-summary.json
python3 index_artifacts.py "$RESULTS" .
```

本轮只整理证据和计划；现有编译器修改维持未提交状态。测试证据包括四库语料/13项checksum与LLVM一致、transport集成测试semver34/base6413/Unicode8通过；完整upstream unit仍分别受fabsf64、atomic_xsub、atomic_load缺失阻塞，不能宣称全套通过。
