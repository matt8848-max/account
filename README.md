# 记账软件

个人记账软件：记录每一笔转入/转出，按月份查看收支与结算余额，并可将指定交易对象、指定月份区间的流水导出为 Excel。

- **单用户、无账号体系**：数据只保存在自己电脑上的 SQLite 文件里
- **客户端-服务器（C/S）架构**：计算端（`account_server`）负责存储与统计，客户端（`account_client`）负责展示
- **桌面客户端使用 GPUI + GPUI Kit**：原生渲染，不再依赖 WebView / 浏览器内核

当前版本 **0.5.1**，开源协议 **MIT**。

## 支持平台

| 平台 | 客户端（account_client） | 计算端（account_server） |
|---|---|---|
| Windows 10/11（x64） | ✅ GPUI 桌面应用 | ✅ |
| Linux（x64） | ✅ GPUI 桌面应用（需 Vulkan / Wayland 或 X11） | ✅ **静态二进制，不依赖系统动态库** |
| macOS（Apple Silicon / arm64） | ✅ GPUI 桌面应用 | ✅ |
| 浏览器（WebAssembly） | ❌ 0.5.0 起不再提供 | — |
| Android / iOS | ❌ **不支持手机端** | ❌ |

> 0.5.0 移除了 Dioxus 与 Web 版：GPUI 的 wasm 支持仍属实验性，不作为正式分发路径。

## 功能

### 交易记录
- 以「交易组」为单位记账：一个组可包含多条转入/转出记录，便于记录一次拆分消费
- 支持跨月组：同一组的记录可以分布在不同月份
- 按时间范围、交易对象筛选；进入页面自动刷新
- 日期/时间使用 gpui-kit 日历选择器，无需手输
- 交易对象选择支持搜索（对象多时按名称/货币过滤），并以弹出窗口选择转入/转出对象
- 转出与转入都选中后自动校验货币：一致自动填入，不一致提示并清空转入方
- 「复制上一行」可快速复用上一行的转出/转入/货币类型/日期
- 记录排序切换（从新到旧 / 从旧到新）；每条记录显示所属组名
- 三个操作页均支持纵横双向滚动条、内容左上对齐
- 删除单条记录 / 整个组前弹出确认框，余额与对象月度明细自动回滚
- 创建失败时保留已录入内容，只有创建成功后才清空表单

### 交易对象与货币
- 管理交易对象（如「支付宝」「微信钱包」「现金」）与货币（如 RMB、USD）
- **支持重命名交易对象与货币**：后端提供 `rename` 接口，前端同步更新对象列表、记录与统计缓存
- 新增记录时自动联动余额：转出方减少、转入方增加；删除时自动回滚
- 禁止跨币种交易；被引用的货币与交易对象不可删除

### 数据统计
- 选择多个交易对象与月份区间，查看每个对象的逐月收入、支出
- 横向卡片内为表格：月份、收入、收入环比、收入同比、支出、支出环比、支出同比、结算余额（斑马纹）
- 所选范围内没有交易的月份记为 0；环比/同比只在基准月份早于账号开始记账月份时留空，基准值为 0 时显示 N.A.
- **逐月结算余额**：从当前余额出发按月份倒推，得到每月月末余额
- 双栏选择：左侧可搜索列表点「添加」，右侧已选列表可「踢出」
- **📈 画图**：按当前对象与月份生成桑基图/流向图，主要对象以「组」框展示，
  左侧净流入、右侧净流出；次要对象只保留与其相关的流向，双向往来按净额合并

### 导出 Excel（.xlsx）
在「数据统计」页选择交易对象与月份后点击 **📤 导出**：

1. 弹出确认框，列出导出的交易对象与时间区间，并可选（默认不勾选，勾选后必定生成对应 sheet）：
   - 交易对象卡片（独立 sheet，带自动筛选）
   - 桑基图/流向图（独立 sheet）
2. 选择保存位置（桌面原生「另存为」对话框）
3. 后台生成并写盘，进度显示在底部状态栏

每个交易对象各自生成一个 sheet：标题行合并 A~I、表头自动筛选并冻结前两行、流水按交易时间排序；金额列由原始分子/分母计算。卡片与流向 sheet 同样保留原始分子/分母。

### 版本更新提醒
客户端**启动时检测一次** GitHub 最新发布版本，结果显示在底部状态栏：

- 有新版本（红色）：点击用浏览器打开发布页
- 已是最新（绿色）：点击重新检测
- 检测失败（灰色）：点击重试

### 底部状态栏
- **后端连接**：存活 / 失联（绿色 / 红色），点击展开详情（地址、版本、上次探测时间、间隔）
- **检测更新**：见上
- **提示信息与进度**：操作结果、导出进度都在这里显示

### 连接页与后端开关
- 连接页为五行布局：标题 / 地址 + HTTP 版本 / 默认地址提示 / 连接·断开·关闭服务器 / 信息提示
- **HTTP 版本可选**：`HTTP/2`（服务端默认）或 `HTTP/3`（服务端以 `--http-version 3` 启动）
- 「连接」兼任进入/返回主页面；「断开」只断开网络；「关闭服务器」需二次确认
- 桌面端关闭窗口时会联动关闭后端（若仍处于连接状态）

## 快速开始

### 方式一：使用发布包

1. 在 [Releases](https://github.com/matt8848-max/account/releases) 下载：
   - `account_server-<平台>.zip` / `.tar.gz`：计算端（Linux 版为静态二进制）
   - `account_client-<平台>.zip` / `.tar.gz`：桌面客户端
2. 先启动计算端 `account_server`（首次运行会在同目录创建 `account.db` 与自签名证书 `certs/`）
3. 再启动客户端 `account_client`，输入后端地址 `https://127.0.0.2:3000` 并点击「连接」

**HTTP/3 示例**：

```bash
# 计算端用 QUIC 启动（UDP 3000）
account_server --http-version 3
# 客户端连接页选择「HTTP/3」后再连接
```

> 未签名可执行文件可能触发系统提示：Windows SmartScreen 选择“仍要运行”；macOS 首次需右键「打开」。

### 方式二：从源码构建

环境要求：`cargo/rustc 1.92+`（edition 2024）。**不再需要 dioxus-cli 或 wasm32 目标**。

**Linux 额外系统依赖**（仅桌面客户端；计算端不启用 `client` 特性，不需要 GUI 库）：

```bash
sudo apt-get install -y pkg-config gcc g++ clang \
  libfontconfig-dev libwayland-dev libxkbcommon-x11-dev \
  libx11-xcb-dev libssl-dev libzstd-dev \
  libwebkit2gtk-4.1-dev libasound2-dev \
  vulkan-validationlayers libvulkan1
```

```bash
cd account

# 计算端（默认 127.0.0.2:3000，数据库与 account_server 同目录）
cargo run --bin account_server
cargo run --bin account_server -- --ip 127.0.0.1 --port 3001 --db data/custom.db
cargo run --bin account_server -- --http-version 3
cargo run --bin account_server -- --help

# 桌面客户端（开发运行，需要 client 特性）
cargo run --bin account_client --features client
```

## 使用说明

1. **准备基础数据**：「对象/货币」页先建货币，再建交易对象并填写初始余额
2. **记账**：在「交易记录」页新建交易组，填写组名与若干条转入/转出记录
3. **查看**：按时间范围与交易对象筛选记录；余额随记录自动联动
4. **改名**：对象/货币列表点击「✎」内联重命名
5. **统计**：在「数据统计」页选择对象与月份，点「🔍 查对象统计」
6. **导出**：同一页面选择对象与月份后点「📤 导出交易记录」
7. **关闭后端**：终端 `Ctrl+C`，或连接页「⛔ 关闭服务器」；桌面端直接关窗也会关闭后端

## 数据与安全

- **数据存储**：SQLite 单文件、交易记录按月分表；对象月度明细按年分表
  `counterparty_monthly_YYYY`，**没有交易的月份不建行**，默认 `account.db`
- **金额精度**：`i128 / i128` 定点数，禁止浮点；数据库以 TEXT 保存分子/分母
- **传输安全**：HTTPS；HTTP/2（TCP）或 HTTP/3（QUIC）由启动参数选择；首次启动自动生成自签名证书
- 桌面客户端与后端之间跳过自签证书校验（本地自用场景）
- **建议定期备份** `account.db`；备份前先停止 account_server

## 版本与发布（CI）

- 推送到 `main` 分支触发 `.github/workflows/release.yml`
- 版本号取自 `account/Cargo.toml`，工作流自动以其作为 tag 创建 Release（tag 已存在则只构建不发版）
- 产物：计算端（Windows / **Linux 静态 musl** / macOS arm64 可选）、桌面客户端（Windows / Linux / macOS arm64 可选）
- 不再构建 WebAssembly 产物；CI 不再安装 dioxus-cli

## 开发与测试

```bash
cd account
cargo test --features client      # 单元测试（SQLite 内存库，不产生临时文件）
cargo clippy --all-targets --features client
cargo fmt
```

代码结构（单包双 bin）：

```text
account/
├── bin/account_server.rs   # 计算端（TLS HTTP/2 或 QUIC HTTP/3 + API + CLI）
├── bin/account_client.rs   # GPUI 桌面客户端启动器
├── build.rs                # Windows：把 assets/icon.ico 编入 exe 资源（ID=1）
├── assets/                 # 全部图片资产（窗口/安装包图标）
└── src/
    ├── lib.rs              # 只注册 server / ui
    ├── server.rs           # 后端父模块（只注册/导出）
    ├── server/
    │   ├── money.rs        # i128 定点数、Display/FromStr、判等/排序、四则运算
    │   ├── models.rs       # 数据模型（含对象月度明细）
    │   ├── export.rs       # xlsx 纯逻辑
    │   ├── api.rs          # 路由、AppState、Shutdown
    │   └── db.rs + db/     # schema/迁移、月表、counterparty_monthly_YYYY
    └── ui.rs + ui/         # GPUI 客户端（父模块只注册/导出）
        ├── app.rs          # 根视图 AccountApp
        ├── app_state.rs    # 纯状态与函数（含测试）
        ├── date.rs         # DatePicker 日期/月份选择封装
        ├── assets.rs       # 项目 assets + gpui-kit 资产源、窗口图标
        ├── http.rs         # HTTP/2 与 HTTP/3、连接会话
        ├── connect.rs / records.rs / master.rs / stats_page.rs
        ├── sankey.rs       # 桑基图/流向图（纯 div 绘制）
        ├── export.rs / status_bar.rs / update.rs / confirm.rs
        └── main_layout.rs
```

## 许可证

本项目以 **MIT** 协议开源，详见 [LICENSE](LICENSE)。
