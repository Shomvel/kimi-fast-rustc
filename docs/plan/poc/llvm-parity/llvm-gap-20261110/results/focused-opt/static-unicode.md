# Unicode ASCII 变化与循环对齐静态核查

结论：现有证据不支持把 selected 的 ASCII 中位数增加认定为稳定回退，也不支持把它归因于动态执行了更多 padding。完整 Unicode 产物中的14个新增 padding 位置均对应自然循环；没有找到此次 crate 内的假循环实例。其他构造暴露的“机器地址向后边不等于自然回边”问题仍应修复，但不能把它写成已解释此次 ASCII 变化。

## 测量边界

同一轮21次测试，start ASCII 中位数122.613 ms，selected 133.294 ms；逐round配对比值中位数1.06760，bootstrap 95%区间[0.99980,1.14525]，跨1。不能将中位数相除的+8.7%改写成稳定因果。

独立perf的ASCII instructions为1,962,774,141→1,962,774,205，差64；cycles反而为460,498,665→446,768,403。perf与正式计时是不同运行，不能用此单次cycles证明加速，但这些结果不支持执行新增NOP导致8.7%时间增加。

mixed instructions增加1,132,015（约0.079%），reverse增加47,145,370（约0.922%）；较早独立perf中reverse的start→all已有约3872万指令增加；跨两组perf作量级比较，all→selected另增约842万，不能当同轮配对的精确拆因。因此这些工作负载确有不同动态工作量，不能把所有变化统一归类为纯地址布局。

## 实际代码归属与链接路径

3arm（start/all/selected）的6个LLVM driver object逐一同SHA-256，local text均37496 B。源码的inline边界必须以最终对象为准：

- `GraphemeCursor::next_boundary` 的实际forward调用是LLVM driver内实例，最终地址3arm均`0x325f0`，大小1156 B。
- 它调用的`is_boundary`是LLVM副本，3arm均`0x32040`、1137 B。二进制还保留KFRC同名副本，地址`0x36f90`、start2082 B/all和selected2118 B；不能按同名符号误选这一份作为forward热代码。
- LLVM next_boundary通过GOT调用KFRC `GraphemeCursor::grapheme_category`。GOT relocation已核实：start slot`0x7eca8`→`0x37c10`；all slot`0x7ecf8`→`0x37c40`；selected slot`0x7ed08`→`0x37c40`。该helper均145 B，ASCII走`ch≤0x7e`的本地分支，不调用Unicode分类表。
- all→selected的这个ASCII helper热路径字节和最终地址相同。LLVM forward函数地址、原driver对象相同；链接后的GOT displacement、其他调用/data重定位变化，不应声称最终整个函数字节相同。
- Unicode ASCII输入只有可打印ASCII及CR/LF，不进入emoji/regional/InCB handler与Unicode分类binary_search的新增padding。其instructions近乎相等与这一静态路径相符。

仍可能有地址相关微架构因素，但目前没有需要再猜测具体硬件事件的证据；没有将其写成已定位根因。

## 所有新增循环padding逐项核对

比较all→selected：13个函数体改变、总code增加114 B。脚本从最终汇编建立控制流图并计算支配关系：14处padding、111字节、16条NOP；3字节净差来自其他编码布局变化。每处padding后的目标均支配相应的向后分支源，因此全14处都有自然回边支持。

| 函数/实体 | padding范围（函数相对offset，半开） | 字节 | 回边源 | 自然循环 |
|---|---|---:|---|---|
| grapheme category binary_search | 0x47..0x50 | 9 | 0xc3 | 是 |
| word category binary_search | 0x47..0x50 | 9 | 0xc3 | 是 |
| sentence category binary_search | 0x47..0x50 | 9 | 0xc3 | 是 |
| emoji category binary_search | 0x47..0x50 | 9 | 0xc3 | 是 |
| range binary_search（CGU0） | 0x47..0x50 | 9 | 0xc3 | 是 |
| range binary_search（CGU1） | 0x47..0x50 | 9 | 0xc3 | 是 |
| GraphemeCursor::prev_boundary | 0x12c..0x130 | 4 | 0x4e3 | 是 |
| handle_emoji | 0xd7..0xe0 | 9 | 0x14e | 是 |
| handle_regional | 0x59..0x60 | 7 | 0x11f | 是 |
| handle_incb_consonant | 0x64..0x70 | 12 | 0xfa | 是 |
| Chars::any(is_alphanumeric) | 0xa..0x10 | 6 | 0xea | 是 |
| sentence match_sb8 | 0xcd..0xd0 | 3 | 0x157 | 是 |
| AsciiWordBoundIter::next_back，首个loop | 0x93..0xa0 | 13 | 0x113 | 是 |
| AsciiWordBoundIter::next_back，第二loop | 0x21d..0x220 | 3 | 0x51a | 是 |

真实循环也不等于padding免费：从前驱fallthrough进入时会执行NOP；循环回边直接跳到padding之后。binary_search每次调用的初次进入多一条9B NOP；handler和反向路径也可能增加循环入口执行量。这解释存在动态指令增加的机制，但未测出各位置的动态命中次数，不能按静态NOP数直接计算workload代价。

相邻风险包括：无profile的冷循环padding增加code；非循环向后跳误判；嵌套循环fallthrough重复执行padding；branch放宽与padding共同改变其他地址；函数/数据/GOT布局影响未改源码的调用者。当前自然loop信息修复能处理误判，不能自动解决其余成本，最终仍须固定同一轮四库完整验收。

## 复核材料

- `unicode-regression-evidence/`：3arm库objects、LLVM driver objects、最终driver和原静态汇编，下载包SHA-256 `dc2dd311458386b121d19118d1051a795685b9de89f462d7e4d7dd98ed1e168a`。
- `audit_unicode_alignment.py`：逐padding的控制流/支配核查脚本。
- `unicode-regression-evidence/alignment-audit.json`：完整14项记录，含object、symbol、padding长度和回边。
- `{start,all,selected}-driver.nm/.asm`及`*-category.asm`：最终符号地址、forward路径和ASCII helper。
- `suite/runtime/unicode-segmentation.json`：最终同轮原始样本；`suite/runtime-first/`是较早all实验，不能混用其绝对时间计算本轮比值。

本次没有修改生产源码，也没有启动额外benchmark或stress。
