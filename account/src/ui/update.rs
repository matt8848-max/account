//! 更新检测（启动时一次 + 底部状态栏手动重试）
//!
//! 状态存放在 `GlobalState.update_state`，由 `ui/status_bar.rs` 展示：
//! - 有新版本：红色，点击打开发布页（桌面用系统浏览器，wasm 开新标签页）
//! - 已是最新：绿色，点击重新检测
//! - 检测失败：灰色，点击重新检测
//!
//! 应用第一次启动时自动检测一次（`App` 挂载时调用 `spawn_check`，切换页面不会重试）。
//! 仓库地址取自 `Cargo.toml` 的 `repository` 字段（留空则本功能自动关闭）。

use super::{GlobalState, http_get};
use dioxus::prelude::*;

/// GitHub API：最新发布（404 表示尚无 release，属正常情况）
const LATEST_RELEASE_API: &str = "https://api.github.com/repos/{slug}/releases/latest";

/// 更新检测状态（底部状态栏据此决定文案、颜色与点击行为）
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UpdateState {
    /// 尚未检测，或仓库地址未配置（状态栏不展示该组件）
    Idle,
    /// 正在检测（不可点击，避免重复请求）
    Checking,
    /// 发现新版本：点击打开发布页
    Available { tag: String, page: String },
    /// 已是最新：点击重新检测
    Latest { tag: String },
    /// 检测失败：点击重新检测
    Failed { reason: String },
}

impl UpdateState {
    /// 状态栏文案
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Idle => String::new(),
            Self::Checking => "⟳ 检测更新中…".to_string(),
            Self::Available { tag, .. } => format!("⬆ 有新版本 {tag}"),
            Self::Latest { tag } => format!("✓ 已是最新 {tag}"),
            Self::Failed { .. } => "⟳ 检测更新失败，点击重试".to_string(),
        }
    }

    /// 状态栏颜色：有新版红色、最新绿色、检测中/失败灰色
    pub(crate) fn color(&self) -> &'static str {
        match self {
            Self::Available { .. } => "#c0392b",
            Self::Latest { .. } => "#2e7d32",
            Self::Checking | Self::Failed { .. } => "#888888",
            Self::Idle => "#888888",
        }
    }

    /// 是否可点击（检测中与未启用时不可点）
    pub(crate) fn is_clickable(&self) -> bool {
        matches!(
            self,
            Self::Available { .. } | Self::Latest { .. } | Self::Failed { .. }
        )
    }

    /// 鼠标悬停提示（失败时给出原因，便于排查）
    pub(crate) fn title(&self) -> String {
        match self {
            Self::Available { page, .. } => format!("打开发布页: {page}"),
            Self::Latest { .. } => "点击重新检测更新".to_string(),
            Self::Failed { reason } => format!("检测失败: {reason}（点击重试）"),
            Self::Checking => "正在检测更新".to_string(),
            Self::Idle => String::new(),
        }
    }
}

/// 发起一次更新检测（有新版/最新/失败都会写入状态，供状态栏展示）。
///
/// 已在检测中时直接返回，避免启动检测与手动重试撞车造成重复请求。
pub(crate) fn spawn_check(mut st: GlobalState) {
    // 远端未配置（repository 留空/非法）时直接跳过，不影响启动流程
    let Some(slug) = repo_slug(env!("CARGO_PKG_REPOSITORY")) else {
        return;
    };
    spawn(async move {
        if (st.update_state)() == UpdateState::Checking {
            return;
        }
        st.update_state.set(UpdateState::Checking);
        let url = LATEST_RELEASE_API.replace("{slug}", &slug);
        let current = env!("CARGO_PKG_VERSION");
        let next = match http_get(&url).await {
            Ok(value) => match parse_release(&value) {
                Some((tag, page)) => resolve_state(&tag, &page, current),
                None => {
                    // 还没有 release：对用户来说就是「已是最新」
                    log::debug!("更新检测：尚无已发布的版本，视为已最新");
                    UpdateState::Latest {
                        tag: current.to_string(),
                    }
                }
            },
            Err(e) => {
                log::debug!("更新检测失败: {e}");
                UpdateState::Failed { reason: e }
            }
        };
        st.update_state.set(next);
    });
}

/// 依据远端标签与当前版本得出检测结果（纯函数，便于单测）。
pub(crate) fn resolve_state(tag: &str, page: &str, current: &str) -> UpdateState {
    if is_newer(tag, current) {
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

/// 从仓库地址解析 `owner/repo`（仅支持 github.com；无法解析返回 None）。
pub(crate) fn repo_slug(repository: &str) -> Option<String> {
    let url = repository.trim().trim_end_matches('/');
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let mut parts = rest.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    // 只接受 owner/repo 两段，多余的路径段视为不合法
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// 从 release JSON 提取 `(版本标签, 发布页地址)`；缺少 `tag_name` 时返回 None。
pub(crate) fn parse_release(value: &serde_json::Value) -> Option<(String, String)> {
    let tag = value.get("tag_name")?.as_str()?.trim().to_string();
    if tag.is_empty() {
        return None;
    }
    let page = value
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Some((tag, page))
}

/// 解析版本号为主体数字序列（忽略 `v` 前缀与 `-beta`/`+build` 后缀），非数字段按 0 计。
fn parse_version(version: &str) -> Vec<u64> {
    let version = version.trim().trim_start_matches(['v', 'V']);
    let core = version.split(['-', '+']).next().unwrap_or(version);
    core.split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

/// 判断远端版本是否比本地新（逐段比较，缺省段按 0 计）。
///
/// 例：`0.4.0 > 0.3.9`、`0.10.0 > 0.9.9`、`v0.4.0 == 0.4.0`（不提示）。
pub(crate) fn is_newer(latest: &str, current: &str) -> bool {
    let latest = parse_version(latest);
    let current = parse_version(current);
    if latest.is_empty() || current.is_empty() {
        return false;
    }
    let len = latest.len().max(current.len());
    for idx in 0..len {
        let a = latest.get(idx).copied().unwrap_or(0);
        let b = current.get(idx).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证检测结果：远端更新 → 可点打的开发布页（红色）；同版本/更旧 → 已是最新（绿色）
    #[test]
    fn resolve_state_should_map_version_compare() {
        let newer = resolve_state(
            "0.5.0",
            "https://github.com/a/b/releases/tag/0.5.0",
            "0.4.0",
        );
        assert_eq!(
            newer,
            UpdateState::Available {
                tag: "0.5.0".to_string(),
                page: "https://github.com/a/b/releases/tag/0.5.0".to_string(),
            }
        );
        assert!(newer.is_clickable());
        assert_eq!(newer.color(), "#c0392b");
        assert!(newer.label().contains("0.5.0"));

        // 同版本与更旧版本都算「已是最新」，点击重新检测
        for tag in ["0.4.0", "0.3.9", "v0.4.0"] {
            let state = resolve_state(tag, "", "0.4.0");
            assert_eq!(
                state,
                UpdateState::Latest {
                    tag: tag.to_string()
                }
            );
            assert!(state.is_clickable());
            assert_eq!(state.color(), "#2e7d32");
        }
    }

    /// 验证状态展示：检测中不可点击；失败为灰色且文案提示可重试；未启用不展示文案
    #[test]
    fn update_state_should_expose_label_and_clickability() {
        assert!(!UpdateState::Checking.is_clickable());
        assert!(!UpdateState::Idle.is_clickable());
        assert_eq!(UpdateState::Idle.label(), "");

        let failed = UpdateState::Failed {
            reason: "HTTP 403".to_string(),
        };
        assert!(failed.is_clickable());
        assert_eq!(failed.color(), "#888888");
        assert!(failed.label().contains("重试"));
        // 悬停提示带上失败原因，便于排查
        assert!(failed.title().contains("HTTP 403"));
    }

    /// 验证仓库地址解析：https/http 均可，末尾斜杠与 `.git` 后缀被忽略
    #[test]
    fn repo_slug_should_parse_github_repository() {
        assert_eq!(
            repo_slug("https://github.com/matt8848-max/account"),
            Some("matt8848-max/account".to_string())
        );
        assert_eq!(
            repo_slug("https://github.com/matt8848-max/account/"),
            Some("matt8848-max/account".to_string())
        );
        assert_eq!(
            repo_slug("https://github.com/matt8848-max/account.git"),
            Some("matt8848-max/account".to_string())
        );
        assert_eq!(repo_slug("http://github.com/a/b"), Some("a/b".to_string()));
    }

    /// 验证仓库地址解析的非法输入：空、非 GitHub、路径段数量不对均返回 None
    #[test]
    fn repo_slug_should_reject_invalid_repository() {
        assert_eq!(repo_slug(""), None);
        assert_eq!(repo_slug("https://gitlab.com/a/b"), None);
        assert_eq!(repo_slug("https://github.com/only-owner"), None);
        assert_eq!(repo_slug("https://github.com/a/b/tree/main"), None);
        assert_eq!(repo_slug("https://github.com//b"), None);
    }

    /// 验证版本比较：主/次/修订号任一段更大即视为更新，位数不同（0.10 > 0.9）也正确
    #[test]
    fn is_newer_should_compare_semver_segments() {
        assert!(is_newer("0.4.0", "0.3.0"));
        assert!(is_newer("0.3.1", "0.3.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v0.4.0", "0.3.0"));
        assert!(is_newer("1", "0.9.9"));
        // 相同版本 / 更旧版本不提示
        assert!(!is_newer("0.4.0", "0.4.0"));
        assert!(!is_newer("v0.4.0", "0.4.0"));
        assert!(!is_newer("0.4", "0.4.0"));
        assert!(!is_newer("0.3.9", "0.4.0"));
    }

    /// 验证版本比较的容错：预发布后缀忽略，非数字标签不会误判为新版本
    #[test]
    fn is_newer_should_tolerate_non_numeric_tags() {
        assert!(is_newer("0.5.0-beta.1", "0.4.0"));
        assert!(!is_newer("latest", "0.4.0"));
        assert!(!is_newer("", "0.4.0"));
    }

    /// 验证 release 解析：取 tag_name 与 html_url；缺 tag_name 返回 None
    #[test]
    fn parse_release_should_extract_tag_and_page() {
        let value = serde_json::json!({
            "tag_name": " 0.4.0 ",
            "html_url": "https://github.com/matt8848-max/account/releases/tag/0.4.0",
        });
        assert_eq!(
            parse_release(&value),
            Some((
                "0.4.0".to_string(),
                "https://github.com/matt8848-max/account/releases/tag/0.4.0".to_string()
            ))
        );

        // 无 tag_name / 空标签 → None（如 404 返回的错误体）
        assert_eq!(
            parse_release(&serde_json::json!({ "message": "Not Found" })),
            None
        );
        assert_eq!(
            parse_release(&serde_json::json!({ "tag_name": "  " })),
            None
        );
        // 缺少 html_url 时仍返回标签，页面地址为空串
        assert_eq!(
            parse_release(&serde_json::json!({ "tag_name": "0.4.0" })),
            Some(("0.4.0".to_string(), String::new()))
        );
    }
}
