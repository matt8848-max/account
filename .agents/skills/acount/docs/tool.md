---
title: "工具链"
---

## MiKTeX

Windows 下的 TeX 发行版，提供 xelatex 编译器和宏包管理。

### 核心能力
- 包管理（mpm / miktex-console）
- xelatex 编译流程
- 字体管理
- 宏包安装与更新
- 编译错误排查

---

## Cargo

Rust 的包管理器与构建工具。

### 核心能力
- 包管理（Cargo.toml / Cargo.lock）
- 代码正确性检查（cargo check）— 仅检查语法和类型，不生成二进制，比 build 快
- 构建与发布（cargo build / publish）
- 测试（cargo test）
- 代码质量（cargo clippy / fmt）
- 文档生成（cargo doc）
- Workspace 多包管理
- 自定义子命令（cargo-xxx）

### 代码检查流程

修改代码后应使用以下流程验证正确性，按速度排序：

| 命令 | 作用 | 速度 | 使用时机 |
|------|------|------|---------|
| `cargo check` | 检查语法和类型错误，不生成二进制 | 最快 | 开发中频繁使用 |
| `cargo build` | 编译生成二进制 | 较慢 | 需要运行或部署时 |
| `cargo test` | 运行所有测试 | 中等 | 提交前验证 |

**核心原则**：开发过程中使用 `cargo check` 快速验证代码正确性，只有需要运行二进制时才使用 `cargo build`。

### 依赖管理最佳实践

依赖必须使用 `cargo add` 添加，而不是手动编辑 `Cargo.toml`。示例：

```bash
# 带 features 的常用库
cargo add clap --features derive
cargo add serde --features derive
cargo add tokio --features full

# 日志库：log（通用门面）+ env_logger（终端输出实现）
cargo add log
cargo add env_logger
```

原因：
- `cargo add` 自动查询最新兼容版本，写入正确的语义化版本范围
- `cargo add --features` 确保 features 声明正确
- 手动编辑容易写错版本号或忘记 lockfile 更新

如果需要查看 crate 的最新版本：
```bash
cargo search <crate-name> --limit 1
```

### 测试数据清理规范

**原则：测试不得在项目目录产生临时文件垃圾。**

- 数据库等有状态的测试优先使用**内存库**（如 SQLite `open_in_memory`），彻底避免磁盘残留
- 若必须使用临时文件：
  - 使用唯一文件路径（防止并行测试互相覆盖）
  - 测试结束立即删除（`std::fs::remove_file`、`drop` 守卫或临时目录 crate）
  - 不在项目根目录写文件——用系统临时目录（如 `std::env::temp_dir()`）或 `tempfile` crate
- 每次开发结束检查项目目录：`dir /b *.db`、`git status` 确认无残留

### 跨平台注意事项（Windows）

#### B. cmd 中 `cd dir && cargo xxx` 前缀可能失效

Windows `cmd.exe` 执行 `cd dir && cargo new server` 时，`cd` 可能不生效（尤其目录路径包含中文或特殊字符时），cargo 会在**当前目录**而非目标目录创建包。

**正确做法**：不依赖 `cd` 前缀，直接用路径参数或 `--manifest-path`：

```bash
# 创建包到指定目录（不 cd）
cargo new ledger/server --name ledger-server

# 对指定包执行命令（不 cd）
cargo add tokio --manifest-path path/to/Cargo.toml
cargo check --manifest-path path/to/Cargo.toml
cargo test --manifest-path path/to/Cargo.toml
```

#### E. edition 2024 workspace 必须显式 `resolver = "3"`

edition 2024 的 workspace 若不指定 resolver，cargo 会警告 `virtual workspace defaulting to resolver = "1"`，导致解析行为错误。

**正确做法**（虚拟 workspace 根）：

```toml
[workspace]
resolver = "3"
members = ["server", "client"]
```

注意：`resolver = "2"` 是 edition 2021 的默认只是约定俗成；edition 2024 必须写 `"3"`。

#### F. Windows 下测试并行产生同名临时文件导致 SQLite 锁冲突

`cargo test` 默认多线程并行跑测试。若多个测试用**相同文件名**的临时 SQLite 库（如 `test_{pid}.db`），WAL 模式下会报 `database is locked`（Error code 5）。

**正确做法**：
1. 首选内存库：`Connection::open_in_memory()` — 每个连接独立，无文件锁冲突
2. 必须用文件时，每个测试用**全局递增计数器**生成唯一路径：

```rust
static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
let path = format!("test_{}_{n}.db", std::process::id());
```
