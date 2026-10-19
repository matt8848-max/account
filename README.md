# 记账软件

个人账单管理系统。客户端-服务器（C/S）架构：

- **计算端（server）**：Rust + axum（HTTP/2.0 TLS），SQLite 按月分表存储
- **客户端（client）**：Dioxus 0.7（同一份 UI 组件，支持 Web / 桌面）

后端地址：`https://127.0.0.2:3000`（自签证书），当前版本 **0.3.0**。

## 环境要求

| 工具 | 版本 |
|---|---|
| cargo / rustc | 1.96+（edition 2024） |
| dioxus-cli | 0.7.9（`cargo install dioxus-cli --version 0.7.9`） |
| wasm32 target | `rustup target add wasm32-unknown-unknown` |

## 编译

项目位于 `account/`，单包双 bin。

### 后端（计算端）

```bash
cd account
cargo build --bin server

# 运行（默认 127.0.0.2:3000，数据库与 server 可执行文件同目录）
cargo run --bin server

# 指定监听 IP
cargo run --bin server -- --ip 127.0.0.1

# 指定数据库路径
cargo run --bin server -- --db data/custom.db

# 查看帮助
cargo run --bin server -- --help
```

监听 `127.0.0.2:3000`。若 `certs/` 下无证书，首次启动会自动生成自签名证书并创建数据库（默认与 server 同目录的 `account.db`）。
启动成功后终端输出：监听地址、API 版本、数据库版本、当前时间（UTC+8）。

关闭后端：终端 `Ctrl+C`，或由客户端连接页的「关闭服务器」按钮（`POST /api/close`）优雅关闭。

### 证书（手动生成，推荐）

证书由 **mkcert** 手动生成，放置于 `account/certs/`（文件名固定为 `cert.pem` / `key.pem`）。

```bash
# 1. 安装 mkcert（一次即可）：
#    Windows: choco install mkcert 或 winget install mkcert
#    macOS:   brew install mkcert
#    Linux:   sudo apt install mkcert

# 2. 安装本地 CA（一次即可）
mkcert -install

# 3. 在 account/certs/ 目录生成证书
#    域名列表按需填写：account.com、localhost、127.0.0.2、IPv6 地址（如 ::3000）等
cd certs
mkcert -cert-file cert.pem -key-file key.pem account.com account.com localhost 127.0.0.2 ::3000
```

生成后 `server` 启动时自动加载 `certs/cert.pem` 与 `certs/key.pem`；mkcert 的本地 CA 已被系统信任，浏览器/客户端访问 `https://127.0.0.2:3000` 不再有证书警告。

### 前端（客户端）

**Web（wasm）**：

```bash
cd account
cargo build --target wasm32-unknown-unknown --bin client

# 或使用 dioxus-cli 构建（必须指定 bin=client，默认找包名 account 会报错）
dx build --release --platform web --bin client
```

**桌面（native）**：

```bash
cd account
cargo run --bin client
```

桌面客户端启动后，在连接页输入后端地址（默认 `https://127.0.0.2:3000`），点击连接即可。

## 构建打包（发布）

打包使用 dioxus-cli（Dioxus.toml 已配置）。注意**必须同时满足**：

1. 指定 `--bin client`（单包双 bin，默认找包名 account 会报错）
2. 显式启用 `--features bundle`（Cargo.toml 中定义了 `bundle` 特性，`dx bundle` 需要）

```bash
cd account

# 桌面应用安装包（NSIS）
dx bundle --release --platform desktop --bin client --features bundle
```

打包后的 exe 安装程序位于 `target/dx/account/bundle/windows/nsis/` 目录下。

其他平台/格式：

```bash
# Web 静态站点（部署到任意静态服务器即可，需先信任 https://127.0.0.2:3000 自签证书）
dx bundle --release --platform web --bin client --features bundle

# 开发运行（Desktop）
dx serve --platform desktop --bin client

# 开发运行（Web 调试）
dx serve --platform web --bin client
```

> 提示：`dx bundle` 时若报 "Failed to find target account"，确认是否带 `--bin client --features bundle`。

## 运行

C/S 架构，需先启动**计算端（后端）**，再启动**客户端（前端）**。

### 1. 启动后端

```bash
cd account
cargo run --bin server
```

首次启动时若 `certs/` 下无证书会自动生成自签名证书，并创建数据库（`account.db`），监听 `https://127.0.0.2:3000`。保持此进程运行。（证书也可手动生成，见上文「证书（手动生成，推荐）」）

### 2. 启动前端

**桌面客户端（native，推荐即时验证）**：

```bash
cd account

# 方式一：dioxus-cli 运行（自动构建 + 打开桌面窗口）
dx serve --platform desktop --bin client

# 方式二：cargo 直接运行
cargo run --bin client
```

**Web 客户端（wasm）**：

```bash
cd account

# 方式一：dioxus-cli serve（开发模式，热重载 + 自动打开浏览器）
dx serve --platform web --bin client

# 方式二：构建 + serve（生产构建）
dx build --release --platform web --bin client && dx serve --platform web --bin client
```

启动后进入连接页，输入 `https://127.0.0.2:3000` 点击连接即可使用。

> 提示：Windows cmd 下请使用 `cd /d account` 或 PowerShell 的 `Set-Location account` 保证目录切换生效（skill 记录的问题 B）；PowerShell 下 `RUST_LOG=debug` 需写作 `$env:RUST_LOG="debug"; cargo run --bin server`。

## 测试

```bash
cd account
cargo test
```

覆盖内容：

- 定点数金额去零与加减精度对齐（1.3 + 2.7 = 4.0 → `(4, 1)`）
- 统计页逐月结算余额倒推（含结束月过滤、汇总去零）
- 货币增删查与删除约束（被交易对象/记录引用的货币不可删）
- 交易对象删除约束
- 交易组插入（跨月分表）与记录查询
- 月度统计聚合（转入=收入、转出=支出，按货币分别统计）
- 备注 256 字符长度校验、交易时间月份解析
- 组 ID 按涉及月份分配 + 全局查重
- 组包含月份（`group_months`）的写入与增删记录后的重算回写
- 数据库迁移（版本 0/1 → 2 补列回填、升级规则、过旧版本拒绝）
- 服务器 CLI 参数（默认 db 路径、东八区时间格式）

**测试使用 SQLite 内存库**（`open_in_memory`），不会在项目目录产生任何临时文件。

## 代码检查

```bash
cd account
cargo check        # 快速类型检查（不生成二进制）
cargo clippy       # 静态检查

# 后端调试日志
# PowerShell 7.3+ 或 bash：
RUST_LOG=debug cargo run --bin server
# PowerShell（所有版本）：
$env:RUST_LOG="debug"; cargo run --bin server
```

## 项目结构

```
account/
├── bin/server.rs    # 计算端（native，TLS + HTTP/2）
├── bin/client.rs    # 客户端启动器（桌面 / Web）
└── src/
    ├── lib.rs       # 模块导出
    ├── models.rs    # 数据模型（金额定点数等）
    ├── api.rs       # HTTP API（PUT 读 / POST 其余）
    ├── db.rs        # 数据库层（总表/货币/对象）
    ├── db/month.rs  # 月表层（组/记录、月度统计）
    └── ui.rs        # 客户端 UI 公共父模块（全局状态、HTTP、Toast、确认弹窗、主布局）
        └── ui/      # 页面组件
            ├── connect.rs  # 连接页
            ├── records.rs  # 交易记录
            ├── master.rs   # 交易对象/货币管理（搜索 + 灰白相间列表）
            └── stats.rs    # 数据统计（双栏选择 + 逐月结算余额）
```

## 常见问题

| 问题 | 解决 |
|---|---|
| `database is locked` | 测试用内存库；多进程勿共享同一 SQLite 文件 |
| 浏览器证书警告 | 自签证书，首次访问需手动信任 |
| edition 2024 workspace 报 resolver | 根 Cargo.toml 显式写 `resolver = "3"` |
| Windows cmd 中包建到错误目录 | 避免 `cd dir && cargo xxx`，用 `--manifest-path` |
| PowerShell 中 `RUST_LOG=debug cargo` 报错 | Unix 语法不适用；用 `$env:RUST_LOG="debug"; cargo ...` |
| `dx build` 报 "Failed to find target account" | 单包双 bin 项目，需加 `--bin client` |
| `dx bundle` 报 "Failed to find target account" | 单包双 bin 项目，需加 `--bin client --features bundle` |
