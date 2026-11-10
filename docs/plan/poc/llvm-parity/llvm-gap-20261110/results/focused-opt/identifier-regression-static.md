# semver identifier 回退的静态定位

本次只读检查现有 start、h1、address、checked、h1_address、all ELF；没有重跑性能，没有修改生产代码。

## 已确认

- 真实 driver identifier 工作负载对 1024 字节 `a` 调用 Prerelease::new、clone、as_str、eq、drop。numeric_identifier 的 checked 算术优化不是这条热循环。
- 从这5个入口按对象 relocation 建立的闭包有27个唯一函数符号，在多个对象中共32个定义。start→h1 和 start→address 的32个定义，其符号内机器码及归一后的 relocation offset/kind/target/addend 全部相同。
- parse::identifier 所有6个arm均759 B，原始对象内函数SHA-256为 `316ba947f8d818a3eb292bea5e93cc13a4b5ff07b239e9f5698c972704a1cf81`。寄存器、spill、分支及相对跳转没有改变。
- E2B验证agent提供的独立perf：identifier instructions start=3,657,263,566，address=3,657,263,311；cycles=808,254,656→927,912,773。这与“地址CSE增加热路径工作”不符。

| arm | linked parse::identifier入口 | mod64 | 已观察运行组 |
|---|---:|---:|---|
| start | 0x1d2e0 | 32 | 快 |
| checked | 0x1d2e0 | 32 | 快 |
| h1 | 0x1d2b0 | 48 | 慢 |
| address | 0x1d2b0 | 48 | 慢 |
| h1_address | 0x1d280 | 0 | 慢 |
| all | 0x1d280 | 0 | 慢 |

ASCII `a` 热循环执行函数相对偏移 [+0x55,+0x75)、[+0x7b,+0x81)、[+0x87,+0x9f)、[+0xa4,+0xb5)、[+0x2e6,+0x2f7)，回边 +0x2f2→+0x55。函数入口相对64B边界变化使循环取指分布变化。以上只建立布局假说，不证明具体微架构事件或全部回退的原因。

## 最小可证伪实验

`pad_identifier_object.py` 对 all 的现有 cgu.01 object 做独立诊断变换：目标函数 `.text` alignment 64、前置32B NOP、函数symbol value与相关relocation offset增32，函数原字节不变。不会按函数名加入编译器规则。原始输入object SHA-256：`76fb5a80830e1a2d25197fe739ef10d866ef020a5fb0fd8cf34f43a875f51af8`。

本地已核验该object全部23个函数的机器码和归一relocation保持相同。仅目标.text、对应.rela、.symtab payload变化。重新链接时使用原driver object和库对象，仅替换这个object；确认最终函数size759、入口mod64=32，先corpus与checksum，再少量paired identifier计时和独立perf。若布局32恢复时间而指令数不变，支持布局因果；若不恢复，检查链接变换是否有效及其他地址/微架构因素，不能认定CSE错误。

## 本地证据

- `compare_semver_objects.py`：ELF函数字节与重定位比较脚本。
- `semver-byte-map.json`：6arm符号数据（唯一名称汇总），重复定义另做逐object校验。
- `identifier-closure.json`：入口和可达函数清单。
- `regression-evidence/{arm}-driver.nm`、`{arm}-identifier.asm`：最终链接地址与函数反汇编。
- `layout-experiment/padded.o.json`：诊断对象变换回执。

动态布局实验由E2B验证agent执行，本文不把尚未返回的结果记为完成。
