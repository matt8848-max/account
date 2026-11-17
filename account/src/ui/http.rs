//! 客户端 HTTP 层与会话
//!
//! GPUI 的事件循环不是 Tokio，因此所有 `reqwest` 请求都在专用的 Tokio
//! 运行时上执行，并且只从 `cx.background_spawn` 的后台任务里调用，
//! 避免阻塞 UI 线程。
//!
//! 支持两种协议：
//! - [`HttpVersion::Http2`]：HTTPS + HTTP/2（服务端默认）
//! - [`HttpVersion::Http3`]：QUIC + HTTP/3（服务端 `--http-version 3`）
//!
//! HTTP/3 使用 `http3_prior_knowledge()` 直接对 UDP 端口发起请求，
//! 不依赖 Alt-Svc 发现（本地自签名证书场景更稳定）。

use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;

/// 连接页可选的 HTTP 版本。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HttpVersion {
    #[default]
    Http2,
    Http3,
}

impl HttpVersion {
    /// 按钮展示文本。
    pub fn label(self) -> &'static str {
        match self {
            Self::Http2 => "HTTP/2",
            Self::Http3 => "HTTP/3",
        }
    }

    /// 启动参数值（与服务端 `--http-version` 一致）。
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Http2 => 2,
            Self::Http3 => 3,
        }
    }

    /// 所有可选项（连接页按顺序渲染）。
    pub fn all() -> [Self; 2] {
        [Self::Http2, Self::Http3]
    }
}

/// 请求超时：本地后端足够；避免 UI 长时间等待。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// 专用 Tokio 运行时（仅用于 reqwest）。
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("创建 HTTP 运行时失败")
    })
}

/// HTTP/2 客户端（跳过自签名证书校验，本地自用场景）。
fn http2_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("创建 HTTP/2 客户端失败")
    })
}

/// HTTP/3 客户端：`http3_prior_knowledge` 直接使用 QUIC。
fn http3_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(REQUEST_TIMEOUT)
            .http3_prior_knowledge()
            .build()
            .expect("创建 HTTP/3 客户端失败")
    })
}

fn client(version: HttpVersion) -> &'static reqwest::Client {
    match version {
        HttpVersion::Http2 => http2_client(),
        HttpVersion::Http3 => http3_client(),
    }
}

/// 发送 PUT/POST 请求并解析 JSON。
///
/// 非 2xx 时优先读取后端统一错误体 `{"error": "..."}`，否则返回状态码文本。
pub fn request(
    method: &str,
    base: &str,
    path: &str,
    body: Option<Value>,
    version: HttpVersion,
) -> Result<Value, String> {
    let url = format!("{base}{path}");
    let client = client(version);
    runtime().block_on(async move {
        let mut req = match method {
            "PUT" => client.put(&url),
            _ => client.post(&url),
        };
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if status.as_u16() >= 400 {
            let err: Value = serde_json::from_str(&text)
                .unwrap_or_else(|_| serde_json::json!({ "error": text }));
            return Err(err["error"].as_str().unwrap_or("请求失败").to_string());
        }
        serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {e}"))
    })
}

/// PUT 请求（读类操作）。
pub fn put(
    base: &str,
    path: &str,
    body: Option<Value>,
    version: HttpVersion,
) -> Result<Value, String> {
    request("PUT", base, path, body, version)
}

/// POST 请求（写类操作）。
pub fn post(
    base: &str,
    path: &str,
    body: Option<Value>,
    version: HttpVersion,
) -> Result<Value, String> {
    request("POST", base, path, body, version)
}

/// 存活探测。
pub fn ping(base: &str, version: HttpVersion) -> bool {
    matches!(
        put(base, "/api/live", Some(serde_json::json!({})), version),
        Ok(v) if v["status"].as_str() == Some("ok")
    )
}

/// GET 绝对 URL（GitHub Releases 等外部接口）。
pub fn get_json(url: &str) -> Result<Value, String> {
    runtime().block_on(async move {
        let resp = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?
            .get(url)
            .header(
                "User-Agent",
                concat!("account-client/", env!("CARGO_PKG_VERSION")),
            )
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if status.as_u16() >= 400 {
            return Err(format!("HTTP {status}: {text}"));
        }
        serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {e}"))
    })
}

/// 已连接会话：供桌面端关窗时按相同协议关闭后端。
pub mod session {
    use super::HttpVersion;
    use std::sync::Mutex;

    /// 当前连接信息；`None` 表示未连接。
    static ACTIVE: Mutex<Option<(String, HttpVersion)>> = Mutex::new(None);

    fn lock() -> std::sync::MutexGuard<'static, Option<(String, HttpVersion)>> {
        ACTIVE.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 记录当前连接。
    pub fn set(base: &str, version: HttpVersion) {
        *lock() = Some((base.to_string(), version));
    }

    /// 读取当前连接（关窗联动使用）。
    pub fn active() -> Option<(String, HttpVersion)> {
        lock().clone()
    }

    /// 清除会话（断开 / 关闭服务器 / 退出）。
    pub fn clear() {
        *lock() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证版本标签与启动参数值一一对应，且两个版本可区分。
    #[test]
    fn http_version_should_map_labels_and_numbers() {
        assert_eq!(HttpVersion::Http2.as_u8(), 2);
        assert_eq!(HttpVersion::Http3.as_u8(), 3);
        assert_ne!(HttpVersion::Http2.label(), HttpVersion::Http3.label());
        assert_eq!(HttpVersion::all().len(), 2);
    }

    /// 验证会话读写与清除。
    #[test]
    fn session_should_store_and_clear() {
        session::set("https://127.0.0.2:3000", HttpVersion::Http3);
        let active = session::active().expect("应有会话");
        assert_eq!(active.0, "https://127.0.0.2:3000");
        assert_eq!(active.1, HttpVersion::Http3);
        session::clear();
        assert!(session::active().is_none());
        // 不相等断言：清除后不能仍保留旧值
        assert_ne!(
            session::active().map(|s| s.0),
            Some("https://127.0.0.2:3000".into())
        );
    }
}
