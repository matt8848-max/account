//! 构建脚本：把 `assets/icon.ico` 编入 Windows 可执行文件。
//!
//! GPUI 的 Windows 后端通过 `LoadImageW(module, ID=1, ...)` 读取窗口图标；
//! 不注册资源时任务栏/标题栏图标为空。

fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            panic!("嵌入 Windows 图标失败: {e}");
        }
    }
}
