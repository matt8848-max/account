//! 计算端（后端服务器）
//!
//! 启动 HTTP/2.0 TLS 服务器，默认监听 `127.0.0.2:3000`。
//! 使用 rcgen 生成自签名证书，保证 HTTP/2 协商（h2 over TLS）。
//!
//! 该 bin 仅用于 native 目标；wasm 目标下编译为空壳（由 client bin 承担）。

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use account::api::{self, AppState};
    use account::db;
    use anyhow::{Context, Result};
    use axum_server::tls_rustls::RustlsConfig;
    use rcgen::{CertificateParams, KeyPair};
    use std::{
        net::SocketAddr,
        path::Path,
        sync::{Arc, Mutex},
    };

    /// 监听地址（后端服务器地址）
    const BIND_ADDR: &str = "127.0.0.2:3000";
    /// 证书文件路径
    const CERT_PEM: &str = "certs/cert.pem";
    const KEY_PEM: &str = "certs/key.pem";

    pub async fn run() -> Result<()> {
        // 打开数据库
        let conn = db::open(Path::new(db::DB_FILE)).context("初始化数据库失败")?;
        log::info!("数据库已就绪: {}", db::DB_FILE);

        // 准备 TLS 证书（不存在则自动生成自签名证书）
        let tls_config = ensure_tls_config().await.context("准备 TLS 证书失败")?;

        let state = AppState {
            conn: Arc::new(Mutex::new(conn)),
        };
        let app = api::router(state);

        let addr: SocketAddr = BIND_ADDR
            .parse()
            .context(format!("无效监听地址 {BIND_ADDR}"))?;
        log::info!("记账计算端启动: https://{addr} (HTTP/2.0)");

        axum_server::bind_rustls(addr, tls_config)
            .serve(app.into_make_service())
            .await
            .context("服务器运行失败")?;
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
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    imp::run().await
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // wasm 目标下 server bin 为空壳
}