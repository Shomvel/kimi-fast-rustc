# 本轮性能回退诊断

首次21轮交错矩阵显示，H1+address+checked的累计候选虽然改善base64和timestamp格式化，但semver长identifier耗时中位数回退约19%（逐轮配对比值中位数约+16.1%，两种统计不混用），总体运行收益仅约0.5%。因此没有直接接受此候选，先对同类布局问题静态排查。

## identifier：已完成可证伪实验

比较实际driver所调用闭包：27个唯一函数、32个对象定义的符号内机器码及归一重定位在start、H1、address间相同，动态指令约36.57亿也相同。寄存器分配、spill及热路径操作增加被排除；最终函数地址mod64与快慢组吻合。详细证据见[静态记录](identifier-regression-static.md)。

对all的已有object作诊断变换，只改变目标函数section对齐及前置填充，保持759B函数体、全部23个对象内函数和归一重定位一致。11轮同环境交错计时：start220.240ms、all254.462ms、layout32约218.989ms；动态指令相同。布局变化足以消除本次回退，但该实验不能确定某个特定微架构事件，也不是按函数名写入编译器的优化。

## timestamp parse：同类相邻风险

10函数可达闭包中仅两项机器码改变：parse_rfc3339_weak2574→2571B（tail地址共享和相应寄存器/分支布局）；Duration::new175→178B（checked和复用）。其余8函数不变。动态指令4,853,262,121→4,859,262,091（约+0.124%），cycles929,595,977→961,029,761。不能用指令数小幅变化直接解释全部时间回退；将与通用对齐候选一起回归。

## 修复边界

通用循环入口布局已实现。分支长度与padding必须联合收敛，label、call、地址/TLS relocation同时重映射；函数section对齐必须匹配相对循环对齐。最终保留方案仍以正确性、四库运行、编译成本和代码体积验收为准。


## 通用策略筛选

16/32/64 B三个候选均通过x86正确性与四库语料。7负载筛选中，16 B使identifier恢复基线、timestamp parse约+0.46%，同时保留base64及duration收益；32/64 B未恢复identifier，64 B还使timestamp parse约回退8.9%。因此选择16 B，而不是认为更大对齐必然更快。

16 B相对未对齐累计候选all的四库代码增量分别为167/89/114/49 B（semver/base64/Unicode/humantime）；这项代价单独记录。该默认值是本次EPYC环境与所测调用链的工程选择，不保证消除所有未来链接相位敏感性。选定后另外运行21轮全13负载与完整编译配对，不用筛选样本代替最终验收。


## Unicode ASCII：保留不确定性

最终selected的ASCII耗时中位数比start高约8.7%，但逐round配对中位数为1.06760，95%区间[0.99980,1.14525]跨1且较宽；独立perf指令只差64（约19.6亿），cycles反而下降。该轮不能定性为稳定回退。

静态核验确认：forward的next_boundary/is_boundary由LLVM driver实例化且地址保持不变；ASCII调用的KFRC分类helper在all/selected同址、热路径字节相同。Unicode全部14处新增padding均在自然循环，ASCII不执行这些padding。详见[Unicode证据](static-unicode.md)。

静态排查另外发现机器后向边不等于自然回边：寄存器分配后置critical-edge块也会向前面跳。最终实现改用调度器已有支配分析提供的循环头，不再以机器跳转方向推断。这个通用策略修复不能写成ASCII波动的已证原因。
