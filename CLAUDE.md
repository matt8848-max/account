# CLAUDE.md

本文件为 Claude Code 提供本项目（记账软件）的开发上下文。**必须保持与代码实现一致**。

## 项目概述

记账软件，功能为账单管理（个人记账）。采用客户端-服务器（C/S）架构：

- **计算端（后端 `server/`）**：负责存储数据，提供 HTTP API
- **客户端（前端 `client/`）**：负责请求合理的数据，并以正确方式展示（Dioxus Web 页面）

无账户系统，单用户使用。C/S 架构仅为保证客户端多样性。

## 技术栈

| 层 | 技术 | 说明 |
|---|---|---|
| 后端 | Rust + axum（HTTP/2.0） | HTTP 框架，通信协议至少 HTTP/2.0 确保安全性 |
| 数据库 | SQLite + rusqlite (bundled) | 同步轻量库，按月分表；bundled 编译内置 SQLite，开发环境无需安装系统 SQLite |
| 前端 | Dioxus 0.7 + dioxus-cli (v0.7.9) | 跨平台 Web UI |
| 日志 | log + env_logger | 前后端均有日志；异步场景使用 tokio |

## 项目结构

单包双 bin 结构（`account/`）：

```
记账软件/
├── CLAUDE.md
├── README.md            ← 软件说明（功能、支持平台、安装与使用）
├── 更新日志.md          ← CHANGELOG（0.3.0 → 0.4.0）
├── LICENSE              ← MIT
├── .github/workflows/
│   └── release.yml      ← push main 自动构建 + 以 Cargo 版本为 tag 创建 Release
└── account/             # 单包双 bin（server + client 共享代码）
    ├── Cargo.toml
    ├── bin/
    │   ├── server.rs    # 计算端（native）— CLI 参数 + TLS + HTTP/2 + API 路由 + 优雅关闭
    │   └── client.rs    # 客户端启动器（native 桌面 / wasm Web）
    └── src/
        ├── lib.rs       ← 统一模块导出（export/models/api/db/ui）
        ├── export.rs    ← xlsx 导出（纯逻辑：月份边界、汇总、sheet 名清洗、工作簿生成）
        ├── models.rs    ← 数据模型（金额定点数、组、记录、货币、交易对象）
        ├── api.rs       ← HTTP API 实现（PUT=读，POST=其余所有 + close + CORS）
        ├── db.rs        ← 数据库连接、迁移（DB_VERSION=2）、总表/货币/对象操作
        ├── db/month.rs  ← 月表操作（组/记录 CRUD、group_months、月度统计）
        └── ui.rs        ← 客户端 UI 公共父模块（全局状态、HTTP 封装、Toast、确认弹窗、主布局）
        └── ui/
            ├── connect.rs  ← 连接页（含「关闭服务器」按钮）
            ├── records.rs  ← 交易记录浏览器（删除记录/组有确认弹窗；进入页面自动刷新）
            ├── master.rs   ← 交易对象/货币管理（删除有确认弹窗）
            ├── stats.rs    ← 数据统计（含「📤 导出交易记录」入口）
            ├── export.rs   ← 导出保存对话框（Web 浏览器下载 / 桌面 rfd 另存为）
            └── update.rs   ← 启动时检测 GitHub 新版本（仅一次）
```

**依赖 target 分离**：
- `[target.'cfg(not(target_arch = "wasm32"))']` — axum/axum-server/rusqlite(bundled)/tokio/reqwest/rcgen/rust_xlsxwriter；
  `dioxus(desktop)` 与 `rfd` 为 **optional**，仅 `client` 特性启用
- `[target.wasm32-unknown-unknown]` — rust_xlsxwriter(wasm feature)；`dioxus(web)/gloo-net/gloo-timers/web-sys/wasm-bindgen/js-sys` 均为 **optional**，仅 `client` 特性启用
- **`client` 特性**：开启后才编译 `src/ui.rs`（`#[cfg(feature = "client")]`）与 `[[bin]] client`
  （`required-features = ["client"]`），因此 server 的依赖图里**没有** dioxus/GTK/WebKit

## 架构与数据流

- 客户端启动 → 连接页输入后端地址 → `live` 探测连通 → 进入主界面
- 主界面每 5 分钟自动通过 `live` 接口探测存活；状态栏显示上次探测时间与结果
- 界面提示统一走 Toast 浮层（`push_toast`）：Web 端额外弹 `window.alert`，桌面端在 UI 内展示——两端均可见
- 连接页提供「关闭服务器」按钮：未连接时灰色禁用；连接后红色可点，点击后弹确认框，调用 `POST /api/close` 优雅关闭后端
- **防呆设计**：删除记录/删除组/删除货币/删除交易对象/关闭服务器，均先弹确认框并列出将删除的内容（`ConfirmDialog`），确认后才执行
- 前端启动时拉取全部交易对象与货币存入内存缓存；增删成功后才更新缓存
- 所有增删改查均以后端成功响应为准再更新前端显示，未指定处理方式的失败弹 Toast 警告
- 交易对象/货币在增删成功后自动更新内存缓存
- **余额联动**：新增组/记录时转出方余额减、转入方余额加（事务内）；删除组/记录时反转（转出方加回、转入方减回）。前端在创建组/删除记录/删除组成功后自动重新拉取交易对象列表，实时展示最新余额
- **组包含月份**：组创建时计算其涉及的月份集合，以逗号分隔字符串冗余写入该组每条记录的 `group_months` 列；`add_record`/`delete_record` 后自动重算回写（`refresh_group_months`，按组实例定位，禁止全局扫描）
- **跨币种禁止**：转出对象与转入对象的货币必须相同，且金额货币必须与对象货币一致（`month::validate_no_cross_currency`）
- **交易记录页进入时自动刷新**：从其他页切回交易记录页时，挂载即主动拉取记录，无需手动操作
- **默认查询范围近 7 天**：应用启动时交易记录页 `query_from` 默认为当前时间往前 7 天、`query_to` 默认为当前时间（避免默认拉取全部记录）
- 统计必须选择至少一个交易对象；月份范围用原生 `month` 型输入框
- **统计页双栏选择**：左侧为可搜索的交易对象列表（「添加」按钮始终可点，重复添加去重，条目不删除），
  右侧为已选列表（「✕ 踢出」移除）。搜索框为空时显示全部
- **逐月结算余额由前端计算**：从当前余额出发，按月份从新到旧倒推
  （某月月末余额 = 当前余额 − 该月之后所有月份的净流入）。因此查询时 `to` 固定传 `null`，
  拿到起始月之后的全部月份，展示时再按用户选择的结束月过滤
- **全局 CSS 重置**：根组件 `App` 通过 `style` 元素注入 `GLOBAL_CSS`（连接页与主布局两个分支均注入），清除浏览器默认的 `html, body` 8px `margin`，并统一 `box-sizing: border-box`，避免错误的高度占用导致页面四周空白与多余滚动条
- 最外层容器用 `height:100dvh; overflow:hidden`，内容区由 `main` 自行滚动，避免桌面/网页出现多余滚动条
- **导出 xlsx（数据统计页）**：点「📤 导出交易记录」→ `ConfirmDialog` 展示范围（对象 + 月份区间）→
  确认后按对象逐个 `PUT /api/records`（`from`=起始月 1 号 0 点、`to`=结束月最后一天 23:59:59、`counterparty_id`）→
  `export::build_xlsx` 生成字节 → 保存对话框（Web 走 `Blob`+`<a download>`；桌面走 `rfd` 另存为 + `std::fs::write`）
- **导出格式**：每对象一个 sheet；第 1 行交易对象/时间区间/流水总额（转入、转出、净额）；第 2 行表头（`autofilter` 默认开启、冻结前两行）；
  第 3 行起明细按 `traded_at` 升序；「金额」列为公式 `=H{r}/I{r}`（原始金额分子/分母）并写入缓存值
- **启动更新检测**：根组件 `App` 挂载时（`use_effect`，仅一次）请求
  `https://api.github.com/repos/{owner}/{repo}/releases/latest`，`owner/repo` 由 `env!("CARGO_PKG_REPOSITORY")` 解析；
  版本比较为逐段数字比较（`ui/update.rs::is_newer`），失败/无 release 静默跳过

## 数据库设计

SQLite 单文件数据库，按月分表。核心规则：

### 版本控制

- **数据库版本**：使用 SQLite `PRAGMA user_version` 记录 schema 版本，当前版本 `DB_VERSION = 2`（见 `db.rs`）。
- 版本历史：
  - 1：初始版本（无结构变更，早期库视为版本 0 与之同构）
  - 2：月表新增 `group_months` 列，记录每个组包含哪些月份
- **升级规则**：只向下兼容一个版本——最新为 `N` 时仅允许从 `N-1` 升级到 `N`；版本 0 与 1 结构相同，均允许升级到 2。更旧的版本拒绝升级。
- **新建数据库**：直接写入当前版本号（`DB_VERSION=2`），不再是版本 0。
- **API 版本**：`api::API_VERSION = 1`；`PUT /api/version` 返回 `{ "api": 1, "db": 2 }` 供前端展示。
- **0.3.0 兼容性**：未修改数据库 schema，`DB_VERSION` 保持 2、`API_VERSION` 保持 1；0.2.x 的数据库与旧客户端均可直接使用。

### 总表（meta，如 `total_stats`）

记录每月汇总与月表名：

| 字段 | 说明 |
|---|---|
| month | 月份（如 `2026-08`） |
| table_name | 月表名 |
| stats | 该月统计 JSON（收入/支出，**按货币分别统计**） |

### 月表

每月一个表，命名如 `transactions_2026_08`。**组和交易记录存储在同一个表中**（不分表）：

| 字段 | 说明 |
|---|---|
| id | 主键 |
| group_id | 组 ID（从该组涉及的月份表中取最小未使用正整数；不同月份的组可复用同号） |
| group_name | 组名称（最多 64 字符，可为空） |
| group_months | 组包含的所有月份（逗号分隔，如 `2026-08,2026-09`） |
| from_id | 转出交易对象（FK → counterparties） |
| to_id | 转入交易对象（FK → counterparties） |
| amount_value | 金额数值（整数） |
| amount_divisor | 金额除数（整数） |
| currency_id | 货币（FK → currencies） |
| remark | 备注（最多 256 个字符的字符串） |
| traded_at | 交易时间（精确到秒） |

**组的概念**：一条交易可能被细分为多条具体操作。例如微信钱包提现 300 元收手续费，会产生微信钱包→微信交易、微信钱包→银行卡两条记录，同属一个组。组内每条记录遵循会计约定：**转入、转出、金额**三要素 + 可选的备注。

**组 ID 分配**：从该组涉及的月份表中取**最小未使用的正整数**（如新组跨 9/10 月，则取 9、10 月表中都未出现的最小正整数）。不同月份的组可复用相同 group_id——只要 group_months 不重叠，删除组/加记录时按月份定位即可互不影响，避免跨月组导致组 ID 虚高。

**时间语义**：月表存储的是**交易时间**（`traded_at`）而非创建时间——所有交易均为手动记录，不存在自动录入场景；时间精确到秒。

**备注限制**：`remark` 为可选字符串，最长 256 个字符。

**金额使用定点数两个整数 `(value, divisor)` 表示**，实际值 = value / divisor：

- `(1000, 10)` → 100.0
- `(1000, 100)` → 10.00

避免浮点数精度误差。不同货币分别统计，不做汇率换算合并。

**计算后自动去零**：`Money` 提供 `normalized()`（约去 value/divisor 的公共 10 因子）、
`checked_add()` / `checked_sub()`（按最小公倍数对齐精度，结果自动去零）。余额联动、月度统计、对象统计、
统计页结算余额汇总等所有计算结果必须使用这些方法（如 `1.3 + 2.7` 得到 `(4,1)` 而不是 `(40,10)`）。
**数据库存储不强制去零**：`adjust_counterparty_balance` 写回时仍沿用原最大 divisor，保持历史数据精度。

### 货币表（currencies）

| 字段 | 说明 |
|---|---|
| id | 主键 |
| name | 货币名称（如 RMB、HK） |

月表通过 `currency_id` 外键依赖。**被使用的货币不允许删除**。不允许直接修改名称。

### 交易对象表（counterparties）

| 字段 | 说明 |
|---|---|
| id | 主键 |
| name | 交易对象名称（如 微信钱包、银行卡） |
| currency_id | 当前资金货币（FK → currencies） |
| balance_value | 当前资金数值（整数） |
| balance_divisor | 当前资金除数（整数） |

**唯一约束 `UNIQUE(name, currency_id)`**：同一对象不同货币视为不同对象（如 RMB 微信、HK 微信 可并存）。**被使用的交易对象不允许删除**。不允许直接修改名称。

### 约束

- 所有交易记录的交易对象必须存在于交易对象表（外键约束）
- **禁止跨币种转账**：转出与转入对象的货币必须相同，金额货币需与对象货币一致（`month::validate_no_cross_currency`）
- 金额一律为两个整数组成的定点数，禁止直接使用浮点数
- 删除货币/交易对象前必须校验未被引用
- 名称不可修改，防止引用不一致
- 交易对象 `UNIQUE(name, currency_id)` 复合唯一
- 组名为可选字符串（≤64 字符），空名存 NULL；记录列表回退显示 `组#ID`
- **同 group_id 可在不同月份的组间复用**（group_months 不重叠）；删除组/加记录时按 `group_months` 定位组实例，避免误删同号其他月的组
- 组包含月份 `group_months` 冗余存储，记录变更后必须重新聚合回写（按组实例定位，禁止全局扫描），保证与真实记录一致

## API 规范

通信协议使用 **HTTP/2.0 及以上**，确保安全性。

非 RESTful：**PUT 负责读类操作，POST 负责其余所有操作（增/删/改/其他，含 close）**。不使用 DELETE/PATCH/GET。

### 基础接口

| 方法 | 路径 | 说明 |
|---|---|---|
| PUT | `/api/live` | 存活检查 |
| PUT | `/api/help` | 返回所有接口定义 |
| POST | `/api/close` | 关闭服务器（优雅关闭，先响应后 100ms 触发 shutdown） |

### 增（POST）

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/api/currencies` | 添加货币 |
| POST | `/api/counterparties` | 添加交易对象 |
| POST | `/api/groups` | 添加一个组 |
| POST | `/api/groups/{group_id}/records` | 往组内添加一条交易记录（请求体为 RecordSubmit 字段 + 可选 `group_months` 定位组实例） |

### 删（POST）

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/api/currencies/delete` | 删除货币（须未被使用） |
| POST | `/api/counterparties/delete` | 删除交易对象（须未被使用） |
| POST | `/api/groups/delete` | 删除一个组（请求体 `{ "id": group_id, "group_months": "2026-08,2026-09" }`，按月份定位组实例，避免误删同号其他月组） |
| POST | `/api/records/delete` | 删除一条交易记录 |

### 查（PUT）

| 方法 | 路径 | 说明 |
|---|---|---|
| PUT | `/api/version` | 返回 API 与数据库版本号 `{ "api": 1, "db": 2 }` |
| PUT | `/api/stats` | 查总表的收支统计 |
| PUT | `/api/stats/month` | 查某月范围的收支统计（from/to 为 `YYYY-MM`，可省略） |
| PUT | `/api/stats/objects` | 按指定一个/多个交易对象统计月范围收支；响应含 `balance` 条目（当前余额）。统计页查询时 `to` 传 `null`，由前端按返回的全部月份倒推每月结算余额 |
| PUT | `/api/records` | 查一定时间范围的所有记录（含 `group_months`） |
| PUT | `/api/records/object` | 查一定时间范围内指定交易对象的所有记录 |
| PUT | `/api/counterparties` | 查有哪些交易对象 |
| PUT | `/api/currencies` | 查有哪些货币类型 |

## 服务器 CLI 参数

`server` 支持启动参数（`bin/server.rs`）：

| 参数 | 说明 | 默认值 |
|---|---|---|
| `--ip <地址>` | 监听 IP 地址 | `127.0.0.2`（端口固定 3000） |
| `--db <路径>` | 数据库文件路径 | 与 server 可执行文件同目录下的 `account.db` |
| `-h, --help` | 显示帮助 | — |

启动成功后输出 info 日志：监听地址（`https://IP:3000`）、API 版本、数据库版本、当前时间（**系统时区**，形如 `UTC+8 2026-10-31 12:34:56`；负偏移 `UTC-8`、半小时时区 `UTC+5:30`）。

日志级别：debug 构建默认 `debug`，release 构建默认 `info`（仍可被 `RUST_LOG` 覆盖）。

## 前端 UI 组织（重要）

跨页公用的代码保留在 `ui.rs`（父模块）；**每页 UI 分别存于 `ui/` 子目录**（类似 `db.rs` + `db/month.rs`）：

- `ui.rs`：全局状态（`GlobalState`）、HTTP 封装（`http_put/http_post` 及绝对 URL 的 `http_get`）、平台差异封装（sleep/now_str/提示）、全局 CSS 重置（`GLOBAL_CSS`，`App` 注入 `style` 元素；清除浏览器默认的 `html/body` 8px `margin` 并统一 `box-sizing: border-box`）、`DayTimeInput`、`SearchSelect`（搜索+候选选择组件：输入框 `value` 只绑定用户输入、已选项用 `placeholder` 呈现；`Enter` 确认、`Escape`/失焦收起候选、候选用 `mousedown`+`prevent_default` 选择、`empty_label` 插入「全部」项）、`ConfirmDialog`、`MainLayout`、根 `App`
- `ui/connect.rs`：连接页（含关闭服务器按钮）
- `ui/records.rs`：交易记录浏览器（挂载时自动刷新；行格式：时间 转出→转入 金额 货币 备注 删除条目按钮 组名 删除组按钮；新建组行的转出/转入、顶部「对象」筛选均用 `SearchSelect` 搜索选择）
- `ui/master.rs`：交易对象/货币管理（货币、交易对象均有搜索栏，空搜索显示全部；灰白相间列表——交易对象为「名称左 / 金额右 / 货币类型右」，货币名称居中）
- `ui/stats.rs`：数据统计（双栏交易对象选择：左可搜索列表点「添加」自动去重、右已选列表可「踢出」；结果卡片显示当前余额与逐月收入/支出/前端计算的结算余额；顶部提供「📤 导出交易记录」）
- `ui/export.rs`：导出保存对话框（`ExportSaveDialog` + `PendingExport`；`sheet_from_views` 把记录视图转成导出表；Web 用 `Blob`+`<a download>` 下载，桌面用 `rfd::FileDialog` 选目录并 `std::fs::write`）
- `ui/update.rs`：启动时更新检测（`spawn_check`/`repo_slug`/`parse_release`/`is_newer`）

**平台差异条件编译**（`#[cfg(target_arch = "wasm32")]`）：
- HTTP：wasm 用 gloo-net、native 用 reqwest（跳过自签名证书验证）
- 定时：wasm 用 gloo-timers、native 用 tokio
- 时间/时区：wasm 用 JS Date、native 用 chrono::Local
- 提示：Web 端额外 `window.alert`，桌面端走 UI 内 Toast 浮层

**界面提示**：统一用 `push_toast`（Toast 浮层，两端可见）；Web 端额外弹原生 alert。

**确认弹窗**：删除记录/组/货币/交易对象、关闭服务器均使用 `ConfirmDialog`，弹窗中列出将删除的内容。

**Dioxus 0.7 注意**：
- Signal 读取用 `signal()` 调用、写入用 `signal.set()`/`signal.write()`
- 列表行数据抽成独立组件（`RecordRow`/`CurrencyRow`/`CounterpartyRow`）避免借用生命周期问题
- 事件闭包需 `FnMut`：move 闭包捕获 String 时先在闭包体内 clone，或预先 clone 副本供捕获
- hooks（`use_signal`/`use_context`）必须在组件函数体顶层调用，禁止在闭包内调用

## 已废弃

**更新总表统计功能**（前端 UI 已注释、后端 `POST /api/stats/update` 已删除）——因增删记录/组时交易对象余额已自动联动更新（见 `db/month.rs` 测试）。

## 开发命令

```bash
cd account

# 后端（native）带自定义参数
cargo run --bin server                          # 默认 127.0.0.2:3000，db 与 exe 同目录
cargo run --bin server -- --ip 127.0.0.1        # 指定监听 IP
cargo run --bin server -- --db data/custom.db   # 指定数据库路径
cargo run --bin server -- --help                # 查看帮助

cargo test --features client         # 运行单元测试（69 个，含 UI/导出/更新检测/时区）
cargo clippy --all-targets --features client  # 静态检查

# 打开调试日志（PowerShell：$env:RUST_LOG="debug"; cargo run --bin server）
$env:RUST_LOG="debug"; cargo run --bin server

# 前端（必须带 --features client）
cargo check --target wasm32-unknown-unknown --bin client --features client   # wasm 类型检查
cargo build --target wasm32-unknown-unknown --bin client --features client   # Web 构建
dx build --release --platform web --bin client --features bundle,client   # Dioxus 打包（必须指明 bin=client）
cargo run --bin client --features client             # 桌面客户端（native）
dx bundle --release --platform desktop --bin client --features bundle,client  # 桌面安装包

# 安装 dioxus-cli（指定版本）
cargo install dioxus-cli --version 0.7.10
```

## 发布流程（CI）

- 推送 `main` 分支 → `.github/workflows/release.yml` 自动构建：
  计算端与桌面客户端（Windows x64 / Linux x64 / **macOS arm64**，Intel macOS 已停发）、Web 版（wasm）；
  桌面客户端为最小化包（`Dioxus.toml` 的 `webview_install_mode = "Skip"` 表示不自带 WebView2）
- Linux **客户端**作业需装 GTK/WebKit/xdo 开发库并断言 `pkg-config --modversion glib-2.0 / webkit2gtk-4.1 / libxdo`；
  缺 `libxdo-dev` 会在链接期报 `rust-lld: unable to find library -lxdo`（`muda`/`wry` 依赖）
- Linux **server** 作业无需任何 GUI 库：server 不启用 `client` 特性，依赖图中不含 dioxus/GTK/WebKit
- 工作流使用 Node 24 版 action（`checkout@v7` / `cache@v6` / `upload-artifact@v7` / `download-artifact@v8`），
  避免 Node 20 弃用告警
- `meta` 作业从 `account/Cargo.toml` 读取 `version` 作为 **tag（不带 v 前缀）**；
  用 `git ls-remote --tags` 判断 tag 是否已存在，已存在则只构建不发版（幂等）
- 发版由 `softprops/action-gh-release` 完成（需要 `permissions: contents: write`）
- 客户端「启动更新检测」读取 `releases/latest`，因此**首个 release 发布后该功能才会给出提示**
- **`Cargo.toml` 的 `description` 必须为 ASCII**：`dx bundle` 生成的 NSIS / WiX 脚本使用 code page 1252，
  中文会导致 `makensis`（Bad text encoding）与 `light.exe`（LGHT0311）失败；中文说明写在 README 中

## 代码规范

- 前后端均使用日志库（log + env_logger），通过 `RUST_LOG` 控制级别；异步场景使用 tokio
- 核心逻辑必须编写单元测试：金额定点数运算、月度统计聚合、API 参数校验、货币/对象删除约束、组 ID 分配（含跨月复用与按月份定位删除）、group_months 维护、数据库迁移
- **测试必须使用 SQLite 内存库**（`Connection::open_in_memory()` + 手动建 schema），不得在项目目录产生临时 db 文件；且天然规避并行测试锁冲突
- 数据库访问统一收敛到 `db/` 数据访问层，禁止在 API 层直接写 SQL
- 金额一律用定点数 `(value, divisor)` 两个整数，禁止 f32/f64
- 客户端 UI 组件与平台无关；HTTP 层通过 cfg 分支（native reqwest / wasm gloo-net），定时用 `sleep_ms` 适配
- **平台差异一律用 `#[cfg(target_arch = "wasm32")]` 封装**（见 skill `docs/rust.md`「跨平台条件编译」）
- Dioxus 0.7：Signal 读取用 `signal()` 调用、写入用 `signal.set()`/`signal.write()`；列表行数据抽成独立组件避免借用生命周期问题
- 按 skill 规范：`lib.rs` 统一导出模块，`bin/*.rs` 保持薄入口；文件夹子模块用 `parent.rs` + `parent/child.rs`（如 `db.rs` + `db/month.rs`、`ui.rs` + `ui/`），禁止 `mod.rs`
- 测试注释：每个测试前写文档注释说明验证点；有边界的测边界；无边界测集合且随机化无关内容；长度类测试用 ASCII 字符