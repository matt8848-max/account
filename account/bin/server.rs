//! 计算端（后端服务器）
//!
//! 启动 HTTP/2.0 TLS 服务器，默认监听 `127.0.0.2:3000`。
//! 使用 rcgen 生成自签名证书，保证 HTTP/2 协商（h2 over TLS）。
//!
//! 该 bin 仅用于 native 目标；wasm 目标下编译为空壳（由 client bin 承担）。
//!
//! 启动参数：
//! - `--ip <IP>`   监听 IP 地址（默认 `127.0.0.2`）
//! - `--db <路径>` 数据库文件路径（默认与 server 可执行文件同目录的 `account.db`）
//! - `-h/--help`   显示帮助

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use account::api::{self, AppState};
    use account::db;
    use anyhow::{Context, Result};
    use axum_server::tls_rustls::RustlsConfig;
    use rcgen::{CertificateParams, KeyPair};
    use std::{
        net::SocketAddr,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    /// 默认监听 IP
    const DEFAULT_IP: &str = "127.0.0.2";
    /// 默认端口
    const PORT: u16 = 3000;
    /// 证书文件路径
    const CERT_PEM: &str = "certs/cert.pem";
    const KEY_PEM: &str = "certs/key.pem";
    /// 数据库文件名（默认与 server 同目录）
    const DB_FILE_NAME: &str = "account.db";

    /// CLI 参数解析结果
    struct Cli {
        ip: String,
        db: PathBuf,
    }

    /// 解析命令行参数。
    ///
    /// 支持 `--ip <值>` / `--ip=<值>`、`--db <值>` / `--db=<值>`、`-h/--help`。
    /// 参数缺失或不合法时返回错误；`--help` 打印帮助后返回 `Ok(None)` 表示正常退出。
    fn parse_args() -> Result<Option<Cli>> {
        let args: Vec<String> = std::env::args().skip(1).collect();

        if args.iter().any(|a| a == "-h" || a == "--help") {
            print_help();
            return Ok(None);
        }

        let mut ip = DEFAULT_IP.to_string();
        let mut db = default_db_path();

        let mut it = args.iter();
        while let Some(arg) = it.next() {
            let (key, inline_val) = match arg.split_once('=') {
                Some((k, v)) => (k, Some(v.to_string())),
                None => (arg.as_str(), None),
            };
            let mut take_value = |name: &str| -> Result<String> {
                if let Some(v) = &inline_val {
                    Ok(v.clone())
                } else {
                    it.next()
                        .cloned()
                        .with_context(|| format!("参数 {name} 缺少值"))
                }
            };
            match key {
                "--ip" => {
                    let v = take_value("--ip")?;
                    if v.trim().is_empty() {
                        return Err(anyhow::anyhow!("--ip 不能为空"));
                    }
                    ip = v;
                }
                "--db" => {
                    let v = take_value("--db")?;
                    if v.trim().is_empty() {
                        return Err(anyhow::anyhow!("--db 不能为空"));
                    }
                    db = PathBuf::from(v);
                }
                other => {
                    return Err(anyhow::anyhow!("未知参数: {other}（使用 --help 查看帮助）"));
                }
            }
        }

        Ok(Some(Cli { ip, db }))
    }

    /// 打印 CLI 帮助
    fn print_help() {
        println!("记账软件计算端（server）");
        println!();
        println!("用法: server [选项]");
        println!();
        println!("选项:");
        println!("    --ip <地址>      监听 IP 地址（默认 {DEFAULT_IP}）");
        println!("    --db <路径>      数据库文件路径（默认与 server 同目录下的 {DB_FILE_NAME}）");
        println!("    -h, --help       显示帮助");
    }

    /// 默认数据库路径：与 server 可执行文件同目录下的 account.db
    fn default_db_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join(DB_FILE_NAME)))
            .unwrap_or_else(|| PathBuf::from(DB_FILE_NAME))
    }

    /// 当前本地时间，形如 `UTC+8 2026-10-31 12:34:56`。
    ///
    /// 时区取自**系统时区**而非固定东八区：server 只编译为 native 目标，
    /// 因此可以直接使用 `chrono::Local`（wasm 无系统时区概念，不在此列）。
    fn now_local() -> String {
        use chrono::Offset;

        let now = chrono::Local::now();
        // local_minus_utc() 给出相对 UTC 的秒数（含夏令时），据此拼出 UTC±H[:MM] 标签
        let offset_secs = now.offset().fix().local_minus_utc();
        let sign = if offset_secs < 0 { '-' } else { '+' };
        let abs = offset_secs.unsigned_abs();
        let hours = abs / 3600;
        let minutes = (abs % 3600) / 60;
        // 整点时区省略分钟（UTC+8），半小时/45 分钟时区保留两位分钟（UTC+5:30）
        let zone = if minutes == 0 {
            format!("UTC{sign}{hours}")
        } else {
            format!("UTC{sign}{hours}:{minutes:02}")
        };
        format!("{zone} {}", now.format("%Y-%m-%d %H:%M:%S"))
    }

    pub async fn run() -> Result<()> {
        let Some(cli) = parse_args()? else {
            return Ok(());
        };

        // 打开数据库（默认与 server 同目录）
        let conn =
            db::open(&cli.db).with_context(|| format!("初始化数据库失败: {}", cli.db.display()))?;
        let db_ver = db::db_version(&conn)?;
        log::info!("数据库已就绪: {}", cli.db.display());

        // 准备 TLS 证书（不存在则自动生成自签名证书）
        let tls_config = ensure_tls_config().await.context("准备 TLS 证书失败")?;

        // 预留关闭句柄，供 POST /api/close 优雅关闭
        let shutdown_handle = axum_server::Handle::new();
        let state = AppState {
            conn: Arc::new(Mutex::new(conn)),
            shutdown: shutdown_handle.clone(),
        };
        let app = api::router(state);

        let addr: SocketAddr = format!("{}:{PORT}", cli.ip)
            .parse()
            .with_context(|| format!("无效监听 IP 地址: {}", cli.ip))?;

        log::info!("记账计算端启动: https://{addr} (HTTP/2.0)");
        log::info!("API 版本: {}", api::API_VERSION);
        log::info!("数据库版本: {db_ver}");
        log::info!("当前时间: {}", now_local());

        axum_server::bind_rustls(addr, tls_config)
            .handle(shutdown_handle)
            .serve(app.into_make_service())
            .await
            .context("服务器运行失败")?;
        log::info!("服务器已关闭");
        Ok(())
    }

    /// 准备 TLS 配置：加载或生成自签名证书
    async fn ensure_tls_config() -> Result<RustlsConfig> {
        if Path::new(CERT_PEM).exists() && Path::new(KEY_PEM).exists() {
            log::info!("加载已有证书: {CERT_PEM}");
            return RustlsConfig::from_pem_file(CERT_PEM, KEY_PEM)
                .await
                .context("读取证书失败");
        }
        log::info!("生成自签名证书用于 HTTPS/HTTP2...");

        // 生成自签名证书
        let key_pair = KeyPair::generate().context("生成密钥失败")?;
        let mut params =
            CertificateParams::new(vec!["127.0.0.2".to_string(), "localhost".to_string()])
                .context("创建证书参数失败")?;
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let cert = params.self_signed(&key_pair).context("生成自签证书失败")?;

        // 写入文件
        let cert_pem = cert.pem();
        let key_pem = key_pair.serialize_pem();
        std::fs::create_dir_all("certs").context("创建 certs 目录失败")?;
        std::fs::write(CERT_PEM, cert_pem).context("写入证书文件失败")?;
        std::fs::write(KEY_PEM, key_pem).context("写入密钥文件失败")?;
        log::info!("自签名证书生成: {CERT_PEM}（客户端需信任该证书）");

        RustlsConfig::from_pem_file(CERT_PEM, KEY_PEM)
            .await
            .context("加载生成的证书失败")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// 验证默认数据库路径：应为 server 可执行文件同目录的绝对路径 account.db
        #[test]
        fn default_db_path_is_exe_dir() {
            let p = default_db_path();
            assert!(p.is_absolute(), "默认 db 应为绝对路径（exe 同目录）");
            assert_eq!(
                p.file_name().unwrap().to_str().unwrap(),
                DB_FILE_NAME,
                "默认 db 文件名应为 account.db"
            );
        }

        /// 验证本地时间格式：`UTC±H[:MM] YYYY-MM-DD HH:MM:SS`。
        ///
        /// 时区标签取自系统时区（不硬编码东八区，非东八区/夏令时机器同样应通过），
        /// 时间部分固定 19 个字符且分隔符位置正确。
        #[test]
        fn now_local_format() {
            let s = now_local();
            let (zone, time) = s.split_once(' ').expect("应包含时区标签与时间两部分");
            assert!(
                zone.starts_with("UTC+") || zone.starts_with("UTC-"),
                "时区标签应以 UTC± 开头，实际 {zone}"
            );
            // 符号之后：`8` 或 `5:30` 形式（整点时区省略分钟）
            let rest = &zone[3..];
            let digits = &rest[1..];
            let (hours, minutes) = match digits.split_once(':') {
                Some((h, m)) => (h, m),
                None => (digits, "00"),
            };
            assert!(
                !hours.is_empty() && hours.chars().all(|c| c.is_ascii_digit()),
                "小时部分非法: {zone}"
            );
            assert_eq!(minutes.len(), 2, "分钟部分应为两位: {zone}");
            assert!(
                minutes.chars().all(|c| c.is_ascii_digit()),
                "分钟部分非法: {zone}"
            );

            assert_eq!(time.len(), 19, "时间字符串长度应为 19，实际 {time}");
            assert_eq!(&time[4..5], "-");
            assert_eq!(&time[7..8], "-");
            assert_eq!(&time[10..11], " ");
            assert_eq!(&time[13..14], ":");
            assert_eq!(&time[16..17], ":");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 日志级别：debug 构建默认 debug 级；release 构建默认 info 级。
    // 仍可通过 RUST_LOG 覆盖。确保 release 至少能看到 info，debug 至少能看到 debug。
    #[cfg(debug_assertions)]
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
    #[cfg(not(debug_assertions))]
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    imp::run().await
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // wasm 目标下 server bin 为空壳
}
