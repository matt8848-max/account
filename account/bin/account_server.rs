//! 计算端（后端服务器）
//!
//! 启动 HTTP/2.0 TLS 服务器，默认监听 `127.0.0.2:3000`。
//! 使用 rcgen 生成自签名证书，保证 HTTP/2 协商（h2 over TLS）。
//!
//! 该 bin 仅用于 native 目标；wasm 目标下编译为空壳（由客户端 bin 承担）。
//!
//! 启动参数：
//! - `--ip <IP>`            监听 IP 地址（默认 `127.0.0.2`）
//! - `--port <端口>`         监听端口（默认 `3000`）
//! - `--db <路径>`           数据库文件路径（默认与程序同目录的 `account.db`）
//! - `--http-version <2|3>` HTTP 版本（**预留参数**，当前无作用，仅校验并记录）
//! - `-h/--help`            显示帮助

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use account::server::api::{self, AppState};
    use account::server::db;
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
    const DEFAULT_PORT: u16 = 3000;
    /// 支持的 HTTP 版本：2 = TCP + TLS（axum-server），3 = UDP + QUIC（h3）
    const HTTP_VERSIONS: [u8; 2] = [2, 3];
    /// HTTP/3 的 TLS ALPN 标识
    const H3_ALPN: &[u8] = b"h3";
    /// 证书文件路径
    const CERT_PEM: &str = "certs/cert.pem";
    const KEY_PEM: &str = "certs/key.pem";
    /// 数据库文件名（默认与程序同目录）
    const DB_FILE_NAME: &str = "account.db";

    /// CLI 参数解析结果
    struct Cli {
        ip: String,
        port: u16,
        db: PathBuf,
        /// HTTP 版本（预留参数：当前无实际作用，仅校验并记录，便于后续接入 HTTP/3）
        http_version: u8,
    }

    /// 解析命令行参数（从进程参数读取）
    fn parse_args() -> Result<Option<Cli>> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        parse_args_from(&args)
    }

    /// 解析命令行参数（与进程参数解耦，便于单元测试直接传入参数列表）
    ///
    /// 支持 `--ip` / `--port` / `--db` / `--http-version` 的 `<值>` 与 `=<值>` 两种写法，
    /// 以及 `-h/--help`。参数缺失或不合法时返回错误；
    /// `--help` 打印帮助后返回 `Ok(None)` 表示正常退出。
    fn parse_args_from(args: &[String]) -> Result<Option<Cli>> {
        if args.iter().any(|a| a == "-h" || a == "--help") {
            print_help();
            return Ok(None);
        }

        let mut ip = DEFAULT_IP.to_string();
        let mut port = DEFAULT_PORT;
        let mut db = default_db_path();
        let mut http_version: u8 = 2;

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
                "--port" => {
                    let v = take_value("--port")?;
                    // 端口 0 会让系统随机分配，客户端无法预知，故与解析失败同视为非法
                    port = v
                        .trim()
                        .parse::<u16>()
                        .with_context(|| format!("--port 应为 1~65535 的整数: {v}"))?;
                    if port == 0 {
                        return Err(anyhow::anyhow!("--port 不能为 0"));
                    }
                }
                "--db" => {
                    let v = take_value("--db")?;
                    if v.trim().is_empty() {
                        return Err(anyhow::anyhow!("--db 不能为空"));
                    }
                    db = PathBuf::from(v);
                }
                "--http-version" => {
                    let v = take_value("--http-version")?;
                    let parsed = v
                        .trim()
                        .parse::<u8>()
                        .with_context(|| format!("--http-version 应为 2 或 3: {v}"))?;
                    if !HTTP_VERSIONS.contains(&parsed) {
                        return Err(anyhow::anyhow!("--http-version 仅支持 2 或 3: {v}"));
                    }
                    http_version = parsed;
                }
                other => {
                    return Err(anyhow::anyhow!("未知参数: {other}（使用 --help 查看帮助）"));
                }
            }
        }

        Ok(Some(Cli {
            ip,
            port,
            db,
            http_version,
        }))
    }

    /// 打印 CLI 帮助
    fn print_help() {
        println!("记账软件计算端（account_server）");
        println!();
        println!("用法: account_server [选项]");
        println!();
        println!("选项:");
        println!("    --ip <地址>             监听 IP 地址（默认 {DEFAULT_IP}）");
        println!("    --port <端口>           监听端口（默认 {DEFAULT_PORT}）");
        println!(
            "    --db <路径>             数据库文件路径（默认与程序同目录下的 {DB_FILE_NAME}）"
        );
        println!("    --http-version <2|3>    HTTP 版本（2=HTTPS/HTTP2，3=QUIC/HTTP3，默认 2）");
        println!("    -h, --help              显示帮助");
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
        // 统一 rustls 加密提供者（ring）：HTTP/2（axum-server）与 HTTP/3（quinn）
        // 必须使用同一个进程级默认提供者，否则 `ServerConfig::builder()` 会 panic。
        // 已安装过时返回 Err，忽略即可。
        let _ = rustls::crypto::ring::default_provider().install_default();

        let Some(cli) = parse_args()? else {
            return Ok(());
        };

        // 打开数据库（默认与 server 同目录）
        let conn =
            db::open(&cli.db).with_context(|| format!("初始化数据库失败: {}", cli.db.display()))?;
        let db_ver = db::db_version(&conn)?;
        log::info!("数据库已就绪: {}", cli.db.display());

        // 准备 TLS 证书（不存在则自动生成自签名证书），返回 PEM 字节：
        // HTTP/2 交给 axum-server，HTTP/3 交给 quinn/rustls。
        let (cert_pem, key_pem) = ensure_cert_files().await.context("准备 TLS 证书失败")?;

        let addr: SocketAddr = format!("{}:{}", cli.ip, cli.port)
            .parse()
            .with_context(|| format!("无效监听地址: {}:{}", cli.ip, cli.port))?;

        log::info!("记账计算端启动: {} (HTTP/{})", addr, cli.http_version);
        log::info!("API 版本: {}", api::API_VERSION);
        log::info!("数据库版本: {db_ver}");
        log::info!("当前时间: {}", now_local());

        match cli.http_version {
            3 => serve_http3(conn, addr, &cert_pem, &key_pem).await,
            _ => serve_http2(conn, addr, &cert_pem, &key_pem).await,
        }
    }

    /// HTTP/2（TLS + axum-server）
    async fn serve_http2(
        conn: rusqlite::Connection,
        addr: SocketAddr,
        cert_pem: &[u8],
        key_pem: &[u8],
    ) -> Result<()> {
        let tls_config = RustlsConfig::from_pem(cert_pem.to_vec(), key_pem.to_vec())
            .await
            .context("构建 HTTP/2 TLS 配置失败")?;

        let shutdown_handle = axum_server::Handle::new();
        let state = AppState {
            conn: Arc::new(Mutex::new(conn)),
            shutdown: api::Shutdown::for_http2(shutdown_handle.clone()),
        };
        let app = api::router(state);

        log::info!("监听 https://{addr}（HTTP/2）");
        axum_server::bind_rustls(addr, tls_config)
            .handle(shutdown_handle)
            .serve(app.into_make_service())
            .await
            .context("HTTP/2 服务器运行失败")?;
        log::info!("服务器已关闭");
        Ok(())
    }

    /// HTTP/3（QUIC + h3 + h3-axum）
    async fn serve_http3(
        conn: rusqlite::Connection,
        addr: SocketAddr,
        cert_pem: &[u8],
        key_pem: &[u8],
    ) -> Result<()> {
        let state = AppState {
            conn: Arc::new(Mutex::new(conn)),
            shutdown: api::Shutdown::for_http3(),
        };
        let shutdown = state.shutdown.clone();
        let app = api::router(state);

        let server_config = build_quic_server_config(cert_pem, key_pem)?;
        let endpoint = quinn::Endpoint::server(server_config, addr)
            .with_context(|| format!("QUIC 监听失败: {addr}"))?;

        log::info!("监听 udp://{addr}（HTTP/3 / QUIC）");
        loop {
            tokio::select! {
                _ = shutdown.wait() => {
                    log::info!("收到关闭请求，停止 QUIC 监听");
                    endpoint.close(0u32.into(), b"shutdown");
                    break;
                }
                incoming = endpoint.accept() => {
                    let Some(incoming) = incoming else { break };
                    let app = app.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_h3_connection(incoming, app).await {
                            log::warn!("HTTP/3 连接异常: {e:#}");
                        }
                    });
                }
            }
        }
        log::info!("服务器已关闭");
        Ok(())
    }

    /// 处理一条 QUIC 连接：逐条接受 H3 请求并交给 axum Router。
    async fn handle_h3_connection(incoming: quinn::Incoming, app: axum::Router) -> Result<()> {
        let conn = incoming.await.context("QUIC 握手失败")?;
        let h3_conn = h3_quinn::Connection::new(conn);
        let mut h3 = h3::server::builder()
            .build::<_, bytes::Bytes>(h3_conn)
            .await
            .context("建立 H3 连接失败")?;

        loop {
            match h3.accept().await {
                Ok(Some(resolver)) => {
                    if let Err(e) = h3_axum::serve_h3_with_axum(app.clone(), resolver).await {
                        log::warn!("H3 请求处理失败: {e}");
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    if h3_axum::is_graceful_h3_close(&e) {
                        break;
                    }
                    return Err(anyhow::anyhow!("H3 连接错误: {e}"));
                }
            }
        }
        Ok(())
    }

    /// 构建 QUIC 的 rustls 配置（ALPN 固定为 `h3`）。
    fn build_quic_server_config(cert_pem: &[u8], key_pem: &[u8]) -> Result<quinn::ServerConfig> {
        use std::io::Cursor;

        let certs = rustls_pemfile::certs(&mut Cursor::new(cert_pem))
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("解析证书失败")?;
        let key = rustls_pemfile::private_key(&mut Cursor::new(key_pem))
            .context("解析私钥失败")?
            .ok_or_else(|| anyhow::anyhow!("证书文件缺少私钥"))?;

        let mut tls = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .context("构建 H3 TLS 配置失败")?;
        tls.alpn_protocols = vec![H3_ALPN.to_vec()];

        let quic = quinn::crypto::rustls::QuicServerConfig::try_from(tls)
            .context("构建 QUIC 加密配置失败")?;
        Ok(quinn::ServerConfig::with_crypto(Arc::new(quic)))
    }

    /// 准备 TLS 证书文件：存在则读取，否则生成自签名证书并写盘。
    ///
    /// 返回 `(cert_pem, key_pem)` 字节，供 HTTP/2 与 HTTP/3 共用。
    async fn ensure_cert_files() -> Result<(Vec<u8>, Vec<u8>)> {
        if Path::new(CERT_PEM).exists() && Path::new(KEY_PEM).exists() {
            log::info!("加载已有证书: {CERT_PEM}");
            let cert = std::fs::read(CERT_PEM).context("读取证书失败")?;
            let key = std::fs::read(KEY_PEM).context("读取私钥失败")?;
            return Ok((cert, key));
        }
        log::info!("生成自签名证书（HTTPS / QUIC 共用）...");

        let key_pair = KeyPair::generate().context("生成密钥失败")?;
        let mut params =
            CertificateParams::new(vec!["127.0.0.2".to_string(), "localhost".to_string()])
                .context("创建证书参数失败")?;
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let cert = params.self_signed(&key_pair).context("生成自签证书失败")?;

        let cert_pem = cert.pem().into_bytes();
        let key_pem = key_pair.serialize_pem().into_bytes();
        std::fs::create_dir_all("certs").context("创建 certs 目录失败")?;
        std::fs::write(CERT_PEM, &cert_pem).context("写入证书文件失败")?;
        std::fs::write(KEY_PEM, &key_pem).context("写入密钥文件失败")?;
        log::info!("自签名证书生成: {CERT_PEM}（客户端需信任该证书）");
        Ok((cert_pem, key_pem))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// 验证默认数据库路径：应为程序可执行文件同目录的绝对路径 account.db
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

        /// 把 `&[&str]` 参数列表转成 `parse_args_from` 需要的 `Vec<String>`
        fn argv(list: &[&str]) -> Vec<String> {
            list.iter().map(|s| s.to_string()).collect()
        }

        /// 验证默认参数：未指定时使用默认 IP / 端口 3000 / HTTP 版本 2，
        /// 且数据库默认落在程序同目录
        #[test]
        fn parse_args_should_use_defaults() {
            let cli = parse_args_from(&argv(&[]))
                .expect("空参数应解析成功")
                .expect("非 --help 应返回 Some");
            assert_eq!(cli.ip, DEFAULT_IP);
            assert_eq!(cli.port, DEFAULT_PORT);
            assert_eq!(cli.http_version, 2);
            assert_eq!(
                cli.db.file_name().and_then(|n| n.to_str()),
                Some(DB_FILE_NAME)
            );
            // 不相等断言：端口不能被解析成 0（0 会让系统随机分配端口）
            assert_ne!(cli.port, 0);
        }

        /// 验证 `--port` 与 `--http-version` 的两种写法（空格分隔 / `=` 内联）都能解析
        #[test]
        fn parse_args_should_accept_port_and_http_version() {
            let cli = parse_args_from(&argv(&["--port", "8080", "--http-version", "3"]))
                .expect("解析应成功")
                .expect("非 --help 应返回 Some");
            assert_eq!(cli.port, 8080);
            assert_eq!(cli.http_version, 3);

            let cli = parse_args_from(&argv(&["--port=65535", "--http-version=2"]))
                .expect("解析应成功")
                .expect("非 --help 应返回 Some");
            assert_eq!(cli.port, 65535);
            assert_eq!(cli.http_version, 2);
            // 不相等断言：端口参数不能被忽略而回退到默认值
            assert_ne!(cli.port, DEFAULT_PORT);
        }

        /// 验证非法取值一律报错（不静默回退默认值），合法取值不误判
        #[test]
        fn parse_args_should_reject_invalid_values() {
            // 端口：0、越界、非数字、缺值
            for bad in [
                vec!["--port", "0"],
                vec!["--port", "65536"],
                vec!["--port", "abc"],
                vec!["--port"],
            ] {
                assert!(parse_args_from(&argv(&bad)).is_err(), "应拒绝: {bad:?}");
            }
            // HTTP 版本：只允许 2 或 3
            for bad in [
                vec!["--http-version", "1"],
                vec!["--http-version", "4"],
                vec!["--http-version", "http2"],
            ] {
                assert!(parse_args_from(&argv(&bad)).is_err(), "应拒绝: {bad:?}");
            }
            // 未知参数
            assert!(parse_args_from(&argv(&["--unknown", "1"])).is_err());
            // 空 IP / 空 db
            assert!(parse_args_from(&argv(&["--ip", "  "])).is_err());
            assert!(parse_args_from(&argv(&["--db", ""])).is_err());

            // 不相等断言：合法端口必须被判为合法（防止校验过严把正常参数也拒了）
            assert!(parse_args_from(&argv(&["--port", "3000"])).is_ok());
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
