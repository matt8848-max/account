# 记账软件

个人记账软件：记录每一笔转入/转出，按月份查看收支与结算余额，并可将指定交易对象、指定月份区间的流水导出为 Excel。

- **单用户、无账号体系**：数据只保存在自己电脑上的 SQLite 文件里
- **客户端-服务器（C/S）架构**：计算端（server）负责存储与统计，客户端（client）负责展示
- **同一份界面代码**同时编译为桌面程序与网页（WebAssembly）

当前版本 **0.4.0**，开源协议 **MIT**。

## 支持平台

| 平台 | 客户端 | 计算端（server） |
|---|---|---|
| Windows 10/11（x64） | ✅ 桌面应用（使用系统自带 WebView2） | ✅ |
| Linux（x64） | ✅ 桌面应用（使用系统自带 WebKitGTK） | ✅ |
| macOS（Apple Silicon / Intel） | ✅ 桌面应用（使用系统自带 WKWebView） | ✅ |
| 浏览器（现代浏览器） | ✅ Web 版（wasm） | — |
| Android / iOS | ❌ **不支持手机端** | ❌ |

> 桌面客户端**不自带浏览器内核**（安装包最小化）：Windows 用系统 WebView2、Linux 用 WebKitGTK、macOS 用 WKWebView。
> 若系统缺少对应 WebView 运行时，请先安装（Windows 10/11 一般已自带 WebView2）。

## 功能

### 交易记录
- 以「交易组」为单位记账：一个组可包含多条转入/转出记录，便于记录一次拆分消费
- 支持跨月组：同一组的记录可以分布在不同月份
- 按时间范围、交易对象筛选；进入页面自动刷新
- 删除记录/整组前弹确认框并列出内容（防误删）

### 交易对象与货币
- 管理交易对象（如「支付宝」「微信钱包」「现金」）与货币（如 RMB、USD）
- 新增记录时自动联动余额：转出方减少、转入方增加；删除时自动回滚
- 禁止跨币种交易，避免金额被错误相加
- 交易对象/货币均支持搜索；被引用的货币与交易对象不可删除

### 数据统计
- 选择多个交易对象与月份区间，查看每个对象的逐月收入、支出
- **逐月结算余额**：从当前余额出发按月份倒推，得到每月月末余额
- 双栏选择：左侧可搜索列表点「添加」，右侧已选列表可「踢出」

### 导出 Excel（.xlsx）
在「数据统计」页选择交易对象与月份后点击 **📤 导出交易记录**：

1. 弹出确认框，列出导出的交易对象与时间区间（取消则中止导出）
2. 确认后选择保存位置与文件名（Web 版由浏览器下载，只需填写文件名）
3. 每个交易对象各自生成一个 sheet：
   - 第 1 行：交易对象、时间区间、流水总额（转入合计 / 转出合计 / 净额）
   - 第 2 行：表头，**默认开启自动筛选**，并冻结前两行
   - 第 3 行起：流水明细（转出/转入、金额、货币类型、交易对象、交易日期、备注、交易分组、原始金额分子/分母）

其中「金额」列由原始金额的分子/分母自动计算公式得出；交易日期为 Excel 日期时间格式，可直接按时间排序。

### 版本更新提醒
客户端**启动时检测一次** GitHub 最新发布版本，若比当前版本新则在侧边栏与提示浮层中告知（不会自动下载或跳转）。

## 快速开始

### 方式一：使用发布包（推荐）

1. 在 [Releases](https://github.com/matt8848-max/account/releases) 下载对应平台的文件：
   - `account-server-<平台>.zip` / `.tar.gz`：计算端
   - `account-client-<平台>.zip` / `.tar.gz`：桌面客户端（内含安装包或可执行文件）
   - `account-web-wasm.zip`：Web 版静态站点（可选）
2. 先启动计算端 `server`（首次运行会在同目录创建 `account.db` 与自签名证书 `certs/`）
3. 再启动客户端，输入后端地址 `https://127.0.0.2:3000` 并点击「连接」

> 未签名的可执行文件可能触发系统提示：Windows 会出现 SmartScreen 提示（选择"仍要运行"）；
> macOS 首次需右键「打开」，或执行 `xattr -dr com.apple.quarantine <应用路径>`。

### 方式二：从源码构建

环境要求：`cargo/rustc 1.88+`（edition 2024）、`dioxus-cli 0.7.10`、`wasm32-unknown-unknown` 目标（仅 Web 版需要）。

```bash
cd account

# 计算端（默认 127.0.0.2:3000，数据库与 server 可执行文件同目录）
cargo run --bin server
cargo run --bin server -- --ip 127.0.0.1 --db data/custom.db
cargo run --bin server -- --help

# 桌面客户端（开发运行）
cargo run --bin client

# Web 版（开发 / 打包）
cargo build --target wasm32-unknown-unknown --bin client
dx build --release --platform web --bin client --features bundle

# 桌面安装包（Windows/macOS/Linux 各自在本机打包）
dx bundle --release --platform desktop --bin client --features bundle
```

## 使用说明

1. **准备基础数据**：「对象/货币」页先建货币（如 RMB），再建交易对象并填写初始余额
2. **记账**：在「交易记录」页新建交易组，填写组名与若干条转入/转出记录
3. **查看**：按时间范围与交易对象筛选记录；交易对象余额随记录自动联动
4. **统计**：在「数据统计」页选择对象与月份，点「🔍 查对象统计」
5. **导出**：同一页面选择对象与月份后点「📤 导出交易记录」
6. **关闭后端**：终端 `Ctrl+C`，或在连接页点「⛔ 关闭服务器」（需确认）

## 数据与安全

- **数据存储**：SQLite 单文件、按月分表，默认是 server 同目录下的 `account.db`（可用 `--db` 指定）
- **传输安全**：HTTPS + HTTP/2；首次启动自动生成自签名证书到 `certs/`，也可用 mkcert 手动生成
  （`cert.pem` / `key.pem`），此时浏览器访问不再有证书警告
- 桌面客户端与后端之间会跳过自签证书校验（本地自用场景）
- **建议定期备份** `account.db`；备份前先停止 server，避免遗漏 WAL 中的数据

## 版本与发布（CI）

- 推送到 `main` 分支会触发 `.github/workflows/release.yml`
- 版本号取自 `account/Cargo.toml`，工作流**自动以其作为 tag 创建 Release**（tag 已存在则只构建不发版）
- 发布产物：计算端（Windows / Linux / macOS）、桌面客户端（Windows / Linux / macOS，macOS 含 Apple Silicon 与 Intel）、Web 版静态站点

## 开发与测试

```bash
cd account
cargo test                 # 单元测试（SQLite 内存库，不产生临时文件）
cargo clippy --all-targets # 静态检查
cargo fmt                  # 统一格式
```

代码结构（单包双 bin）：

```
account/
├── bin/server.rs   # 计算端（TLS + HTTP/2 + API 路由）
├── bin/client.rs   # 客户端启动器（桌面 / Web）
└── src/
    ├── models.rs   # 数据模型（金额定点数等）
    ├── api.rs      # HTTP API（PUT 读 / POST 其余）
    ├── db.rs       # 数据库层（总表 / 货币 / 交易对象）
    ├── db/month.rs # 月表（组 / 记录 / 统计 / 迁移）
    ├── export.rs   # xlsx 导出（纯逻辑，可单测）
    └── ui.rs + ui/ # 客户端界面（连接页 / 记录 / 对象 / 统计 / 导出 / 更新检测）
```

## 许可证

本项目以 **MIT** 协议开源，详见 [LICENSE](LICENSE)。
