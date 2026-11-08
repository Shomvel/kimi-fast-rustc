# 来源记录与许可

## 每次移植的记录

首次移植代码时在 `docs/migration/sources/records/` 下建立记录，一份对应一个可审阅的语义闭包：

1. 官方仓库 URL、tag（若有）、精确 commit、源文件、内容摘要；若用派生实现，单列基线和补丁。
2. 采用方式：复制后修改 / 算法移植 / 仅行为参考；本地目标文件与负责角色。
3. 保留的不变量、主动改变的表示、没有移植的行为和显式拒绝范围。
4. 原始许可与 notice 依据及保留位置；代码和复制的测试分别记录。
5. 来源测试 ID/路径、自己的差分用例、oracle 版本/配置、原始运行结果。
6. 上游更新影响：数据格式、语义变化、本地差异、需要重跑的验收集。

缺这些字段的改动只算调研草稿，不能标为完成移植。

## 许可

参考树的 [REUSE.toml](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/REUSE.toml) 对多数 compiler/library/tests 文件标注 MIT 或 Apache-2.0，但也有覆盖和例外。复制前逐文件检查 header、REUSE 注释、第三方代码和测试附件，不从根目录的双许可推断所有文件。保留适用的版权、许可证和 NOTICE，记录本地修改。

## 测试复用

见 [差分测试：借用上游测试](../validation/differential-testing.md#借用上游测试)。移植实现时连同上游的边界用例一起迁移。
