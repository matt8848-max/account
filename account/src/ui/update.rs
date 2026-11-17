//! 更新检测（启动一次 + 状态栏手动重试）
//!
//! 仓库地址取自 `Cargo.toml` 的 `repository` 字段（留空则功能自动关闭）。
//! 网络请求在后台线程执行，结果写入 `AccountApp.update_state`。

use super::http;

/// 更新检测状态（状态栏据此决定文案、颜色与点击行为）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    /// 尚未检测，或仓库地址未配置（状态栏不展示）
    Idle,
    /// 正在检测（不可点击）
    Checking,
    /// 发现新版本：点击打开发布页
    Available { tag: String, page: String },
    /// 已是最新：点击重新检测
    Latest { tag: String },
    /// 检测失败：点击重试
    Failed { reason: String },
}

impl UpdateState {
    pub fn label(&self) -> String {
        match self {
            Self::Idle => String::new(),
            Self::Checking => "⟳ 检测更新中…".to_string(),
            Self::Available { tag, .. } => format!("⬆ 有新版本 {tag}"),
            Self::Latest { tag } => format!("✓ 已是最新 {tag}"),
            Self::Failed { .. } => "⟳ 检测更新失败，点击重试".to_string(),
        }
    }

    pub fn is_clickable(&self) -> bool {
        matches!(
            self,
            Self::Available { .. } | Self::Latest { .. } | Self::Failed { .. }
        )
    }

    pub fn title(&self) -> String {
        match self {
            Self::Available { page, .. } => format!("打开发布页: {page}"),
            Self::Latest { .. } => "点击重新检测更新".to_string(),
            Self::Failed { reason } => format!("检测失败: {reason}（点击重试）"),
            Self::Checking => "正在检测更新".to_string(),
            Self::Idle => String::new(),
        }
    }
}

const LATEST_RELEASE_API: &str = "https://api.github.com/repos/{slug}/releases/latest";

/// 从 `repository` 字段解析 GitHub `owner/repo`；非 GitHub 返回 None。
pub fn repo_slug(repository: &str) -> Option<String> {
    let rest = repository
        .trim()
        .trim_end_matches('/')
        .strip_prefix("https://github.com/")
        .or_else(|| repository.trim().strip_prefix("http://github.com/"))?;
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let mut parts = rest.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// 解析 release：返回 `(tag, page)`。
pub fn parse_release(value: &serde_json::Value) -> Option<(String, String)> {
    let tag = value["tag_name"].as_str()?.trim();
    if tag.is_empty() {
        return None;
    }
    let page = value["html_url"].as_str().unwrap_or("").to_string();
    Some((tag.to_string(), page))
}

/// 版本比较：按数值段比较，忽略 `v` 前缀与预发布后缀。
pub fn is_newer(latest: &str, current: &str) -> bool {
    fn parse(v: &str) -> Vec<u64> {
        v.trim()
            .trim_start_matches('v')
            .split('.')
            .map(|s| {
                s.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
            })
            .map(|s| s.parse().unwrap_or(0))
            .collect()
    }
    let a = parse(latest);
    let b = parse(current);
    let len = a.len().max(b.len());
    for i in 0..len {
        let x = *a.get(i).unwrap_or(&0);
        let y = *b.get(i).unwrap_or(&0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// 根据 release 与当前版本计算状态。
pub fn resolve_state(tag: &str, page: &str, current: &str) -> UpdateState {
    if is_newer(tag, current) && !page.is_empty() {
        UpdateState::Available {
            tag: tag.to_string(),
            page: page.to_string(),
        }
    } else {
        UpdateState::Latest {
            tag: tag.to_string(),
        }
    }
}

/// 打开外部链接：仅允许 https；桌面调用系统浏览器。
pub fn is_safe_external_url(url: &str) -> bool {
    let url = url.trim();
    url.len() > "https://".len()
        && url.starts_with("https://")
        && !url.contains(char::is_whitespace)
}

pub fn open_external(url: &str) -> Result<(), String> {
    if !is_safe_external_url(url) {
        return Err("链接不合法，仅支持 https".to_string());
    }
    #[cfg(target_os = "windows")]
    let spawned = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
    };
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(url).spawn();
    spawned
        .map(|_| ())
        .map_err(|e| format!("打开浏览器失败: {e}"))
}

/// 发起一次更新检测（阻塞；由调用方放到后台线程）。
pub fn check_now(current: &str) -> UpdateState {
    let Some(slug) = repo_slug(env!("CARGO_PKG_REPOSITORY")) else {
        return UpdateState::Idle;
    };
    let url = LATEST_RELEASE_API.replace("{slug}", &slug);
    match http::get_json(&url) {
        Ok(value) => match parse_release(&value) {
            Some((tag, page)) => resolve_state(&tag, &page, current),
            None => UpdateState::Latest {
                tag: current.to_string(),
            },
        },
        Err(e) => UpdateState::Failed { reason: e },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证仓库地址解析：https/http 均可，末尾斜杠与 .git 后缀被忽略。
    #[test]
    fn repo_slug_should_parse_github_repository() {
        assert_eq!(
            repo_slug("https://github.com/matt8848-max/account"),
            Some("matt8848-max/account".into())
        );
        assert_eq!(repo_slug("https://github.com/a/b/"), Some("a/b".into()));
        assert_eq!(repo_slug("https://github.com/a/b.git"), Some("a/b".into()));
        assert_eq!(repo_slug("http://github.com/a/b"), Some("a/b".into()));
        assert_eq!(repo_slug(""), None);
        assert_eq!(repo_slug("https://gitlab.com/a/b"), None);
        assert_eq!(repo_slug("https://github.com/only-owner"), None);
    }

    /// 验证版本比较：主/次/修订号任一段更大即视为更新。
    #[test]
    fn is_newer_should_compare_segments() {
        assert!(is_newer("0.5.0", "0.4.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v0.4.1", "0.4.0"));
        assert!(!is_newer("0.5.0", "0.5.0"));
        assert!(!is_newer("latest", "0.5.0"));
        // 不相等断言：相同版本与新版本必须得到不同的判定
        assert_ne!(is_newer("0.5.0", "0.5.0"), is_newer("0.5.1", "0.5.0"));
    }

    /// 验证 release 解析与状态映射。
    #[test]
    fn parse_release_and_state_should_work() {
        let v = serde_json::json!({
            "tag_name": " 0.5.0 ",
            "html_url": "https://github.com/a/b/releases/tag/0.5.0",
        });
        let (tag, page) = parse_release(&v).unwrap();
        assert_eq!(tag, "0.5.0");
        assert!(matches!(
            resolve_state(&tag, &page, "0.4.0"),
            UpdateState::Available { .. }
        ));
        assert!(matches!(
            resolve_state("0.4.0", "https://x", "0.4.0"),
            UpdateState::Latest { .. }
        ));
        assert_eq!(parse_release(&serde_json::json!({})), None);
    }

    /// 验证外部链接白名单：仅 https，拒绝 http/javascript/空白。
    #[test]
    fn external_url_should_only_allow_https() {
        assert!(is_safe_external_url("https://example.com/x"));
        assert!(!is_safe_external_url("http://example.com"));
        assert!(!is_safe_external_url("javascript:alert(1)"));
        assert!(!is_safe_external_url("https://example.com/a b"));
        assert!(!is_safe_external_url(""));
    }
}
