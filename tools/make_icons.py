# /// script
# requires-python = ">=3.10"
# dependencies = ["pillow>=11.0"]
# ///
"""从个人照片生成网页与桌面端图标资产。

用法（在仓库根目录）：

    uv run tools/make_icons.py 个人照片.jpg --delete-source

说明：

- 源图按**网页标准**输出一套 favicon / 图标（`account/public/`，dx 会把该目录
  原样拷贝到 Web 构建产物根目录），以及桌面窗口与安装包用的图标
  （`account/assets/`，编译期 `include_bytes!` 嵌入）。
- 缩放一律使用**近邻插值**（`Image.Resampling.NEAREST`），不使用双线性/三线性，
  这样缩小后保留一点像素画质感。
- 照片是正方形，因此等比缩放即等比；若传入非正方形图片，先居中裁成正方形再缩放
  （仍然等比，不做拉伸变形）。
- `.ico` 由脚本直接按 ICO 容器格式拼装各尺寸的 PNG 帧，
  避免 Pillow 写 ICO 时改用 LANCZOS 重采样。
- 生成完成后可用 `--delete-source` 删除源照片（默认不删）。
"""

from __future__ import annotations

import argparse
import io
import struct
import sys
from pathlib import Path

from PIL import Image

# 网页图标尺寸：浏览器标签页、PWA/Android、iOS 主屏
PNG_SIZES: dict[str, int] = {
    "icon-32.png": 32,
    "icon-64.png": 64,
    "icon-128.png": 128,
    "apple-touch-icon-180.png": 180,
    "icon-192.png": 192,
    "icon-512.png": 512,
}
# favicon.ico 内含的尺寸（16/32 浏览器标签页，48 桌面快捷方式）
ICO_SIZES: tuple[int, ...] = (16, 32, 48)
# 桌面窗口图标（编译期嵌入客户端二进制，同时作为安装包图标源）
DESKTOP_ICON: tuple[str, int] = ("icon-256.png", 256)


def square(img: Image.Image) -> Image.Image:
    """把图片裁成正方形（居中裁剪，保持等比不变形）"""
    img = img.convert("RGBA")
    side = min(img.width, img.height)
    left = (img.width - side) // 2
    top = (img.height - side) // 2
    return img.crop((left, top, left + side, top + side))


def resize_nearest(img: Image.Image, size: int) -> Image.Image:
    """近邻等比缩放（刻意不做抗锯齿，保留像素画质感）"""
    return img.resize((size, size), Image.Resampling.NEAREST)


def png_bytes(img: Image.Image, size: int) -> bytes:
    """把图片缩放到指定边长并编码为 PNG 字节"""
    buf = io.BytesIO()
    resize_nearest(img, size).save(buf, format="PNG", optimize=True)
    return buf.getvalue()


def write_ico(path: Path, img: Image.Image, sizes: tuple[int, ...]) -> None:
    """按 ICO 容器格式写入多尺寸图标（每帧为 PNG 数据）

    Pillow 写 ICO 时会自行重采样（LANCZOS），无法保证近邻；这里手工拼装，
    保证每一帧都是 `resize_nearest` 的结果。
    """
    frames = [(size, png_bytes(img, size)) for size in sizes]
    # 头部：保留位 0、类型 1（图标）、帧数
    header = struct.pack("<HHH", 0, 1, len(frames))
    entries = b""
    offset = 6 + 16 * len(frames)
    payload = b""
    for size, data in frames:
        # 宽/高为 1 字节，256 用 0 表示；planes=1、bitcount=32
        dim = 0 if size >= 256 else size
        entries += struct.pack(
            "<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset
        )
        offset += len(data)
        payload += data
    path.write_bytes(header + entries + payload)


def main() -> int:
    parser = argparse.ArgumentParser(description="从源图生成网页与桌面端图标资产")
    parser.add_argument("source", type=Path, help="源图片路径（正方形照片）")
    parser.add_argument(
        "--public-dir",
        type=Path,
        default=Path("account/public"),
        help="Web 静态资源目录（默认 account/public）",
    )
    parser.add_argument(
        "--assets-dir",
        type=Path,
        default=Path("account/assets"),
        help="编译期嵌入的资源目录（默认 account/assets）",
    )
    parser.add_argument(
        "--delete-source",
        action="store_true",
        help="生成完成后删除源图片",
    )
    args = parser.parse_args()

    if not args.source.is_file():
        print(f"源图片不存在: {args.source}", file=sys.stderr)
        return 1

    with Image.open(args.source) as raw:
        img = square(raw)
    args.public_dir.mkdir(parents=True, exist_ok=True)
    args.assets_dir.mkdir(parents=True, exist_ok=True)

    for name, size in PNG_SIZES.items():
        resize_nearest(img, size).save(
            args.public_dir / name, format="PNG", optimize=True
        )
        print(f"生成 {args.public_dir / name} ({size}x{size})")

    ico_path = args.public_dir / "favicon.ico"
    write_ico(ico_path, img, ICO_SIZES)
    print(f"生成 {ico_path} {ICO_SIZES}")

    name, size = DESKTOP_ICON
    resize_nearest(img, size).save(args.assets_dir / name, format="PNG", optimize=True)
    print(f"生成 {args.assets_dir / name} ({size}x{size})")

    # 自检：ICO 能被重新读出且尺寸齐全，避免容器拼装错误流入发布产物
    with Image.open(ico_path) as ico:
        written = set(ico.ico.sizes())
    expected = {(s, s) for s in ICO_SIZES}
    if written != expected:
        print(f"ICO 自检失败: 期望 {expected}，实际 {written}", file=sys.stderr)
        return 1

    if args.delete_source:
        args.source.unlink()
        print(f"已删除源图片 {args.source}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
