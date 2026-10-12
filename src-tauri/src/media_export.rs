//! 把画布上生成的图 / 视频存到「下载」文件夹。
//!
//! # 为什么不用网页那套 `<a download>`
//!
//! 前端原来的 `downloadMedia` 把本机文件读成 base64、造一个 `<a download>` 点一下。
//! 在桌面应用的 WebView 里这不一定生效，而它不报错就直接返回 —— 用户点了没反应，
//! 也走不到后面「在访达中显示」那一步。超分、补帧节点的视频几十 MB，读成 base64 也慢。
//!
//! 这里直接复制文件：确定能用、不经过内存里的 base64。重名自动加序号，不覆盖已有文件。
//! 只认媒体文件，前端传什么路径都不会被拿来复制别的东西。

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

const MEDIA_EXTS: &[&str] = &["mp4", "mov", "webm", "m4v", "png", "jpg", "jpeg", "webp", "gif", "wav", "mp3", "m4a", "flac"];

/// 复制到 `dest_dir`，返回新文件的路径。
pub fn save_copy(src: &Path, dest_dir: &Path) -> Result<PathBuf> {
    let ext = src.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default();
    if !MEDIA_EXTS.contains(&ext.as_str()) {
        return Err(Error::config(format!("只能存图片、视频、音频文件：{}", src.display())));
    }
    if !src.is_file() {
        return Err(Error::config(format!("文件不在了：{}", src.display())));
    }
    std::fs::create_dir_all(dest_dir).map_err(|e| Error::config(format!("建不出下载文件夹：{e}")))?;
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("anima");
    let mut dest = dest_dir.join(format!("{stem}.{ext}"));
    let mut n = 1;
    while dest.exists() {
        n += 1;
        dest = dest_dir.join(format!("{stem} ({n}).{ext}"));
    }
    std::fs::copy(src, &dest).map_err(|e| Error::config(format!("复制失败：{e}")))?;
    Ok(dest)
}

/// 存到当前用户的「下载」文件夹，返回存好的路径（前端接着在访达里选中它）。
#[tauri::command]
pub fn media_save_to_downloads(path: String) -> Result<String> {
    let dir = dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .ok_or_else(|| Error::config("找不到下载文件夹"))?;
    let src = path.strip_prefix("file://").unwrap_or(&path);
    Ok(save_copy(Path::new(src), &dir)?.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 复制到下载文件夹_重名加序号_不覆盖() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("canvas_upscale_1.mp4");
        std::fs::write(&src, b"video").unwrap();
        let out = dir.path().join("Downloads");
        let a = save_copy(&src, &out).unwrap();
        let b = save_copy(&src, &out).unwrap();
        assert_eq!(a.file_name().unwrap(), "canvas_upscale_1.mp4");
        assert_eq!(b.file_name().unwrap(), "canvas_upscale_1 (2).mp4");
        assert_eq!(std::fs::read(&b).unwrap(), b"video");
    }

    #[test]
    fn 只认媒体文件_文件不在就报错() {
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("secrets.env");
        std::fs::write(&secret, b"KEY=1").unwrap();
        assert!(save_copy(&secret, dir.path()).is_err());
        assert!(save_copy(&dir.path().join("gone.mp4"), dir.path()).is_err());
    }
}
