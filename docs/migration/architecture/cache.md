# 缓存与增量

## 分层顺序

1. 先只缓存完整对象，键包含全部语义依赖。
2. 再逐层缓存已检查 Body、实例、优化图。

初期可以过度失效，但不能为命中率省略语义依赖。旧 Graph 缓存、新语义缓存、LLVM reference 产物分别使用独立的 namespace、schema 和输出目录；新链不读旧键。

## 键必须包含

- 编译器版本、schema 版本
- 源语义和宏卫生身份（纯诊断坐标可以分离，hygiene 和定义身份不行）
- edition、cfg、features
- target、CPU 特性
- 类型、布局、ABI
- panic、overflow、优化级别、调试信息策略
- 依赖的接口和泛型体、静态常量、内联体摘要

环境变量、文件、外部工具等输入若无法完整跟踪，该任务不跨构建复用。build script 和过程宏的结果不默认视为纯函数。

## 命中要可解释

每次命中都能说明匹配了哪些依赖版本，对象来源可追溯。缓存不确定时回退为重新编译，不能回退到另一个代码生成后端。

## 失效测试

逐项修改并确认预期的命中/失效：注释、函数体、公开签名、trait impl、泛型体、宏/hygiene、target feature、编译器升级、被调用者、布局、panic/overflow、pass 配置。另见 [调度与缓存故障测试](../validation/pipeline-faults.md)。
