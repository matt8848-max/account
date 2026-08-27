---
name: acount
description: 管理简历数据的技能。使用渐进式 Markdown 条目存储个人信息，通过 Rust CLI 生成 LaTeX→PDF 简历。当需要更新简历内容、添加技能条目、生成简历 PDF 时自动触发。
---

# Resume 技能

管理个人简历数据的技能。核心能力：

## 文档索引

按需读取以下文档：

- [Rust](docs/rust.md) — Rust 开发知识
- [工具链](docs/tool.md) — MiKTeX, Cargo
- [编码规范](docs/coding-standards.md) — 注释规范、测试规范、代码风格（语言无关）
- [测试规范](docs/testing.md) — 测试注释、边界/集合测试策略、测试数据（语言无关）

## 渐进式披露

1. `SKILL.md` — 摘要和索引
2. `docs/*.md` — 按需深入读取详情

## 使用流程

1. 读取 `SKILL.md` 了解有哪些技能
2. 按需读取 `docs/rust.md`、`docs/latex.md` 或 `docs/tool.md` 获取具体知识
3. 编码时参照 `docs/coding-standards.md` 的规范
