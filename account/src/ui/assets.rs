//! 客户端资产（项目 `assets/` + gpui-kit 图标）
//!
//! `gpui_kit::assets::Assets` 只包含组件自带的 SVG 图标；项目自己的
//! PNG/ICO 资产需要单独嵌入，否则通过路径引用时会加载失败。
//!
//! 本模块实现一个合并的 [`AssetSource`]：
//! 1. 先查 `account/assets/`（rust-embed）；
//! 2. 再回退到 gpui-kit 默认资产。

use gpui_kit::{Result, SharedString};
use rust_embed::RustEmbed;
use std::borrow::Cow;
use std::sync::Arc;

/// 项目 `account/assets/` 目录。
#[derive(RustEmbed)]
#[folder = "assets"]
struct ProjectAssets;

/// 应用资产源：项目资产优先，gpui-kit 资产兜底。
pub struct Assets;

impl gpui_kit::AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(file) = ProjectAssets::get(path) {
            return Ok(Some(file.data));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut out = gpui_kit::assets::Assets.list(path)?;
        for file in ProjectAssets::iter() {
            let file = file.as_ref();
            if path.is_empty() || file.starts_with(path) {
                out.push(SharedString::from(file.to_string()));
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }
}

/// 窗口图标（Linux/X11 使用；Windows 由 build.rs 编入 exe 资源）。
pub fn window_icon() -> Option<Arc<image::RgbaImage>> {
    let bytes = include_bytes!("../../assets/icon-256.png");
    image::load_from_memory(bytes)
        .ok()
        .map(|image| Arc::new(image.to_rgba8()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::AssetSource as _;

    /// 验证项目资产可以加载，且缺失路径会回退到 gpui-kit 默认资产。
    #[test]
    fn project_assets_should_load_and_fallback() {
        let assets = Assets;
        let icon = assets
            .load("icon-256.png")
            .expect("加载项目资产")
            .expect("icon-256.png 应存在");
        assert!(!icon.is_empty());

        // gpui-kit 图标路径应能通过回退加载
        use gpui_kit::assets::IconNamed as _;
        use gpui_kit::component::IconName;
        let icon_path = IconName::Search.path();
        let fallback = assets.load(&icon_path).expect("加载兜底资产");
        assert!(fallback.is_some(), "缺少兜底图标: {icon_path}");

        // 不存在的路径返回 Err（gpui-kit 默认资产源的约定）
        assert!(assets.load("not-exists.bin").is_err());
    }

    /// 验证窗口图标可解码。
    #[test]
    fn window_icon_should_decode() {
        let icon = window_icon().expect("icon-256.png 应能解码");
        assert_eq!(icon.dimensions(), (256, 256));
        // 不相等断言：不能是空图
        assert_ne!(icon.width(), 0);
    }
}
