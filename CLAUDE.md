# CLAUDE.md

本文件为 Claude Code 提供本项目（记账软件）的开发上下文。**必须保持与代码实现一致**。

## 项目概述

记账软件，功能为账单管理（个人记账）。采用客户端-服务器（C/S）架构：

- **计算端（`account_server`）**：负责存储数据，提供 HTTP API（HTTP/2 或 HTTP/3）
- **客户端（`account_client`）**：GPUI 桌面应用，负责请求数据并展示

无账户系统，单用户使用。C/S 架构仅为了保证客户端多样性。

## 技术栈

| 层 | 技术 | 说明 |
|---|---|---|
| 后端 | Rust + axum 0.8 | HTTP/2（axum-server TLS）或 HTTP/3（quinn + h3 + h3-axum） |
| 数据库 | SQLite + rusqlite (bundled) | 同步轻量库；按月分表 + 对象月度明细 |
| 前端 | GPUI + gpui-kit 0.10 | 原生桌面渲染（Windows / Linux / macOS） |
| 日志 | log + env_logger | 前后端均有日志 |

> 0.5.0 起**移除 Dioxus 与 WebAssembly**：只保留 GPUI 桌面客户端。
> gpui-kit 使用 0.10（依赖 gpui-pre 0.3.12）；API 有疑问时查 `cargo doc -p gpui-kit` 或源码。

## 项目结构

```text
记账软件/
├── CLAUDE.md
├── README.md
├── 更新日志.md
├── skills-lock.json        ← npx skills 安装锁（项目技能）
├── .agents/skills/         ← 项目技能：gpui-kit / rust-best-practices / sqlite-best-practices
├── .github/workflows/
│   └── release.yml         ← push main 自动构建 + 以 Cargo 版本为 tag 创建 Release
└── account/
    ├── Cargo.toml
    ├── .cargo/config.toml  ← RUSTFLAGS: --cfg reqwest_unstable（HTTP/3）
    ├── assets/             ← 全部图片资产（icon.ico / icon-256.png 等）
    ├── build.rs            ← Windows：把 assets/icon.ico 编入 exe（资源 ID=1）
    ├── certs/              ← 运行期自签名证书（gitignore）
    ├── bin/
    │   ├── account_server.rs
    │   └── account_client.rs
    └── src/
        ├── lib.rs          ← 只注册 server / ui
        ├── server.rs       ← 后端父模块（只注册/导出）
        ├── server/
        │   ├── money.rs
        │   ├── models.rs
        │   ├── export.rs
        │   ├── api.rs
        │   └── db.rs + db/month.rs + db/counterparty_monthly.rs
        └── ui.rs + ui/
            ├── app.rs / main_layout.rs / state.rs
            ├── date.rs        ← DatePicker 日期/月份选择封装
            ├── assets.rs      ← 项目 assets + gpui-kit 资产源、窗口图标
            ├── http.rs / confirm.rs
            ├── connect.rs / records.rs / master.rs / stats.rs
            └── export.rs / status_bar.rs / update.rs
```

**模块约定**：

- `lib.rs`、`server.rs`、`ui.rs` **只负责注册子模块与决定导出**，不放业务逻辑。
- 一个文件只做一件事：一页 / 一个组件 / 一组功能函数 / 一个状态集合；测试与实现同文件。
- 前后端共用的代码（Money、数据模型、xlsx 导出）**默认放在后端 `server/` 下**，客户端直接 `use crate::server::...`。
- 文件夹子模块用 `parent.rs` + `parent/child.rs`，禁止 `mod.rs`。

## 架构与数据流

- 客户端启动 → 连接页输入地址并选择 HTTP 版本 → `PUT /api/live` 探测 → 拉取对象/货币/版本 → 主界面
- 主界面每 5 分钟自动探测存活；底部状态栏显示存活/失联，点击展开详情
- 界面提示统一写入 `AccountApp.toast`，底部状态栏显示，3 秒后消失
- 连接页「连接」兼任进入/返回；地址或协议变化时先断开再重连
- 桌面端关闭窗口时（`AccountApp::drop`）读取会话中的地址与协议，在独立线程发 `POST /api/close`（1.5s 超时）
- HTTP 请求统一在 `ui/http.rs` 中通过专用 Tokio 运行时执行，只从 `cx.background_spawn` 调用

## UI 约定（GPUI）

- 三个操作页（记录 / 对象货币 / 统计）内容**左上对齐**，不上下/左右居中。
- 滚动使用 `gpui_kit::component::scroll::ScrollableElement` 的
  `overflow_y_scrollbar()`；内容容器需要 `h_full + min_h_0` 才能正确滚动。
- 日期/时间一律使用 `ui/date.rs` 封装的 gpui-kit `DatePicker`（日期 + 分钟时间；
  统计月份选 `%Y-%m`），不要退回纯文本输入。
- 交易对象选择使用 `records.rs` 的搜索面板（输入框 + 过滤列表 + 滚动），
  交易对象多时也能快速定位。
- 列表内所有按钮/元素 ID 必须全局唯一（例如 `del-grp-{gid}-{record_id}`），
  否则 debug 构建会因 `Duplicate a11y node id` panic。
- 图标资产：
  - Windows 的窗口/任务栏图标由 `build.rs` + `winresource` 把 `assets/icon.ico`
    编入 exe（GPUI 读取资源 ID=1）；
  - Linux/X11 通过 `WindowOptions.icon`（`ui/assets.rs::window_icon`）设置；
  - `ui/assets.rs` 的 `Assets` 合并项目 `assets/` 与 gpui-kit 默认资产。

## Money（`server/money.rs`）

```rust
pub struct Money { pub value: i128, pub divisor: i128 }
```

- 实际值 = `value / divisor`；`divisor` 恒为正整数。
- `new(value, divisor)`：校验分母为正，**不约分**；`Money::from_string` 默认按十进制（`"1.23"` → `123/100`）。
- 删除 `from_int` / `to_decimal_string`；用 `Money::from(i128)`、`Money::ZERO`、`Display`、`FromStr`。
- `Display` 支持任意分母长除法，最多 12 位小数并去尾零（`5/2` → `2.5`，`1/3` → `0.333333333333`）。
- 判等/排序按**数值**而非内部表示：手写 `PartialEq/Eq/Ord/PartialOrd`（交叉相乘；极端溢出退回 f64）。
- `Deserialize` 复用 `new` 校验分母，拒绝 0/负数。
- 四则运算统一 `checked_add / checked_sub / checked_mul / checked_div`，结果自动 `normalized()`。
- 约分规则（成本受控）：
  1. 分子、分母同为偶数 → 位运算判断，各除以 2（**一次 `if`**）；
  2. 分母 > `REDUCE_DIVISOR_THRESHOLD`（1e9）→ 做一次完整 gcd。
- 数据库以 TEXT 保存分子/分母（SQLite INTEGER 只有 64 位，不能存 i128）。

## 数据库（`DB_VERSION = 3`）

| 表 | 说明 |
|---|---|
| `currencies` | 货币（`UNIQUE(name)`） |
| `counterparties` | 交易对象（余额 TEXT、`created_month`、`UNIQUE(name,currency_id)`） |
| `transactions_YYYY_MM` | 月表（组与记录同表，金额 TEXT） |
| `counterparty_monthly_YYYY` | 按年分表：对象 × 货币 × **有交易的月份**：`out_*` / `in_*`（TEXT） |

- 删除旧的 `total_stats`（每月按货币汇总交易量，无业务意义）；`/api/stats*` 改为按需从月表计算。
- 月度明细**按年分表**（`counterparty_monthly_2026` 等），按需惰性创建。
- **没有交易的月份不建行**；增删后 `out == 0 && in == 0` 时删除该行。
- 迁移 `2 → 3`：重建月表/交易对象表把金额 `CAST` 为 TEXT，回填真实交易月份到年表。
- 旧版单表 `counterparty_monthly` 不再兼容：本地调试库直接删除重建（未发布过该结构）。
- **所有账务修改必须开启事务**：`add_group` / `add_record` / `delete_group` / `delete_record` / `add_counterparty` / `delete_counterparty` / `delete_currency` / `rename_*`。
- `adjust_counterparty_balance` 必须在事务内调用；失败时月表、余额、月度明细一起回滚。

## API

非 RESTful：**PUT = 读，POST = 其余**。

```text
PUT  /api/live                       存活检查
PUT  /api/help                       接口定义
PUT  /api/version                    API 与数据库版本号
POST /api/close                      关闭服务器（HTTP/2 与 HTTP/3 共用）
POST /api/currencies                 添加货币
POST /api/counterparties             添加交易对象
POST /api/groups                     添加组（含多条记录）
POST /api/groups/{group_id}/records  往组内添加记录
POST /api/currencies/rename          重命名货币
POST /api/counterparties/rename      重命名交易对象
POST /api/currencies/delete          删除货币（须未被引用）
POST /api/counterparties/delete      删除交易对象（须未被引用）
POST /api/groups/delete              删除组（按 group_months 定位组实例）
POST /api/records/delete             删除记录
PUT  /api/stats                      所有月份统计（按需计算）
PUT  /api/stats/month                指定月份范围统计
PUT  /api/stats/objects              按对象统计月范围收支
PUT  /api/counterparties/monthly     对象月度明细
PUT  /api/records                    时间范围记录
PUT  /api/records/object             指定对象的时间范围记录
PUT  /api/counterparties             交易对象列表
PUT  /api/currencies                 货币列表
```

`API_VERSION = 2`。

## HTTP/2 与 HTTP/3

- 启动参数 `--http-version <2|3>`，默认 2。
  - `2`：TCP + TLS + HTTP/2（axum-server）。
  - `3`：UDP + QUIC（quinn），ALPN `h3`，`h3` + `h3-quinn` + `h3-axum` 复用同一 Router。
- 启动时安装统一的 rustls crypto provider（ring）。
- `AppState.shutdown` 是协议无关的 `Shutdown`（Notify + 可选 axum handle）。
- 客户端连接页选择 HTTP/2 或 HTTP/3；HTTP/3 使用 `http3_prior_knowledge()`，不依赖 Alt-Svc。
- `reqwest` 的 HTTP/3 需要 `--cfg reqwest_unstable`：本地由 `.cargo/config.toml` 提供，CI 通过 `RUSTFLAGS` 提供。

## 技能（skills）

使用 `npx skills` 维护：

- 全局：`coding-standards`（`affaan-m/ecc`）— 通用编码规范 + 测试规范
- 项目：`rust-best-practices`（`apollographql/skills`）— Rust 编码/clippy/错误处理/测试
- 项目：`sqlite-best-practices`（`erayack/sqlite-best-practices`）— SQLite 查询/schema/事务
- 项目：`gpui-kit` + `gpui-kit-design-guides` — GPUI 组件与设计规范

常用命令：

```bash
npx skills list
npx skills update
npx skills add <owner/repo> --skill <name> -a cline -y
npx skills remove <name> -y
```

旧的自定义 `acount` skill 已删除，其中编码/测试规范已迁入本文件。

## 代码规范

### 注释

- 每个模块顶部说明职责以及与其他模块的关系。
- 所有公开函数/类型/字段写文档注释（功能、参数、返回值、错误）。
- 关键逻辑、边界条件、设计取舍写中文注释，说明**为什么**而不是重复代码。
- 不使用 lint 抑制掩盖问题；废弃代码直接删除。

### 风格与结构

- `cargo fmt` 统一格式；`cargo clippy --all-targets --features client` 无 warning。
- 单一职责：一个文件只做一件事；重复逻辑提取到公用模块。
- 数据与操作分离：纯函数放独立模块，UI/DB 只负责编排。
- 优先引用/借用，避免无意义 clone。
- 日志分级：`error`（不可恢复）、`warn`（可降级）、`info`（关键节点）、`debug`（调试细节）。
- 平台差异用 `#[cfg(...)]` 封装；禁止在业务代码里散落平台判断。

### 测试规范

- 测试与实现同文件，使用 `#[cfg(test)] mod tests`。
- 每个测试前写中文文档注释，说明**验证了什么**；命名 `xxx_should_yyy`。
- 覆盖正常路径、边界条件、错误路径。
- **有边界必须测边界**（长度 64/65、256/257、金额正负号等）；**无明确边界测集合**，并随机化与测试无关的字段（固定种子 LCG，不引入 rand）。
- **相等断言必须配套至少一个不相等断言**，防止假阳性。
- 数值类必须覆盖负数与零（`-0.05`、`-1.23`、`-4.1`）。
- 长度测试用 ASCII 字符（SQLite `length()` 按字节、Rust `chars()` 按标量，二者不同）。
- 数据库测试使用 `Connection::open_in_memory()`，禁止在项目目录产生临时 db 文件。
- 修改功能后同步审查旧测试：行为已变的测试要更新，不能保留误导性测试名。

## 常用命令

```bash
cd account

cargo test --features client        # 全部单元测试（含 UI 纯逻辑/后端 DB）
cargo clippy --all-targets --features client
cargo fmt

cargo run --bin account_server
cargo run --bin account_server -- --http-version 3
cargo run --bin account_client --features client

cargo build --release --bin account_client --features client
```

## CI（`.github/workflows/release.yml`）

- push `main` 自动构建；tag 取 `account/Cargo.toml` 的 `version`（已存在则只构建不发版）。
- 计算端：Windows x64、Linux x64（musl 静态）、macOS arm64（可选）。
- 桌面客户端：Windows x64、Linux x64、macOS arm64（可选）；Linux 安装 GPUI 所需 Wayland/X11/fontconfig/Vulkan 依赖。
- 所有作业设置 `RUSTFLAGS=--cfg reqwest_unstable`（Linux 计算端同时带 `-C target-feature=+crt-static`）。
- 不再安装 dioxus-cli、不再构建 WebAssembly。
- `Cargo.toml` 的 `description` 必须为 ASCII（打包器 code page 限制）。

## 版本

- 当前 `0.5.0`；`Cargo.toml`、`Cargo.lock`、README、本文件、更新日志需同步。


