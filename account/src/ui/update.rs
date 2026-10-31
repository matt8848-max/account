//! 启动时更新检测
//!
//! 应用启动时只请求一次 GitHub 最新 release，与本地 `CARGO_PKG_VERSION` 比较：
//! 有新版则写入全局状态（侧边栏提示 + Toast）；断网、限流、尚无 release 均静默跳过。
//!
//! 仓库地址取自 `Cargo.toml` 的 `repository` 字段（远端配置好后自动生效，
//! 留空则本功能自动关闭）。

use super::{GlobalState, http_get, push_toast};
use dioxus::prelude::*;

/// GitHub API：最新发布（404 表示尚无 release，属正常情况）
const LATEST_RELEASE_API: &str = "https://api.github.com/repos/{slug}/releases/latest";

/// 启动时检测一次：有新版则写入全局状态并提示用户。
pub(crate) fn spawn_check(mut st: GlobalState) {
    // 远端未配置（repository 留空/非法）时直接跳过，不影响启动流程
    let Some(slug) = repo_slug(env!("CARGO_PKG_REPOSITORY")) else {
        return;
    };
    spawn(async move {
        let url = LATEST_RELEASE_API.replace("{slug}", &slug);
        let release = match http_get(&url).await {
            Ok(value) => parse_release(&value),
            Err(e) => {
                // 断网 / 限流 / 还没有 release 都不打扰用户，只留调试日志
                log::debug!("更新检测失败（忽略）: {e}");
                None
            }
        };
        let Some((tag, page)) = release else {
            log::debug!("更新检测：未找到已发布的版本，跳过");
            return;
        };
        let current = env!("CARGO_PKG_VERSION");
        if !is_newer(&tag, current) {
            log::debug!("更新检测：已是最新版本 {current}（远端 {tag}）");
            return;
        }
        st.release_url.set(Some(page));
        st.update_hint
            .set(Some(format!("发现新版本 {tag}（当前 {current}）")));
        push_toast(&st, &format!("发现新版本 {tag}，当前版本 {current}"));
    });
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
