//! 独立窗口的开关与位置记忆。
//!
//! ## 为什么是独立窗口，不是一个窗口里的标签页
//!
//! 桌宠的形态要求「关掉所有面板她还在桌面上」。做成一个大窗口里的标签页
//! 的话，那个窗口本身就是应用 —— 关了她就没了。所以每一块都是独立窗口，
//! 托盘是唯一入口。
//!
//! ## 位置要记住
//!
//! 每次开都在屏幕正中弹出来，用起来很烦：你把对话窗摆在右下角，
//! 关掉再开又跳回中间。所以位置和尺寸落进 `config.json`，
//! **按窗口分别记**。

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::error::{Error, Result};

/// 一个窗口记住的几何信息
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// 一种面板窗口的定义
pub struct Panel {
    label: &'static str,
    title: &'static str,
    width: f64,
    height: f64,
    min_width: f64,
    min_height: f64,
}

/// 独立面板定义。尺寸取自宽屏工作室设计
const PANELS: &[Panel] = &[
    Panel {
        label: "chat",
        title: "Chat",
        width: 780.0,
        height: 640.0,
        min_width: 580.0,
        min_height: 460.0,
    },
    Panel {
        label: "persona",
        title: "Souls / Avatar",
        width: 760.0,
        height: 680.0,
        min_width: 520.0,
        min_height: 480.0,
    },
    Panel {
        label: "anima_studio",
        title: "Anima Studio",
        width: 1440.0,
        height: 900.0,
        min_width: 1100.0,
        min_height: 700.0,
    },
    Panel {
        label: "settings",
        title: "Settings",
        width: 760.0,
        height: 580.0,
        min_width: 620.0,
        min_height: 440.0,
    },
    Panel {
        label: "memory",
        title: "Memory",
        width: 560.0,
        height: 620.0,
        min_width: 420.0,
        min_height: 420.0,
    },
    Panel {
        label: "diag",
        title: "Diagnostics",
        width: 640.0,
        height: 480.0,
        min_width: 480.0,
        min_height: 380.0,
    },
];

pub fn panel(label: &str) -> Option<&'static Panel> {
    PANELS.iter().find(|p| p.label == label)
}

/// 所有面板的 label。托盘菜单按它生成
pub fn labels() -> Vec<(&'static str, &'static str)> {
    PANELS.iter().map(|p| (p.label, p.title)).collect()
}

/// 托盘里两个固定项的文案。
///
/// **和启动时建托盘的那份必须是同一个来源。** 之前 `lib.rs` 启动时硬编码英文、
/// `update_tray_menu` 另写一份 —— 于是不管配置里选的什么语言，**每次启动
/// 托盘都是英文**，要手动去设置里再切一次才对。
pub fn tray_titles(lang: &str) -> (&'static str, &'static str) {
    if lang.starts_with("zh") {
        ("隐藏她 / 显示她", "退出")
    } else {
        ("Hide Her / Show Her", "Quit")
    }
}

/// 根据语言获取面板在托盘中的显示名称
pub fn labels_for_lang(lang: &str) -> Vec<(&'static str, &'static str)> {
    let is_zh = lang.starts_with("zh");
    PANELS
        .iter()
        .map(|p| {
            let title = match p.label {
                "chat" => if is_zh { "对话" } else { "Chat" },
                "persona" => if is_zh { "形态" } else { "Souls / Avatar" },
                "anima_studio" => if is_zh { "Anima Studio" } else { "Anima Studio" },
                "settings" => if is_zh { "设置" } else { "Settings" },
                "memory" => if is_zh { "记忆" } else { "Memory" },
                "diag" => if is_zh { "诊断" } else { "Diagnostics" },
                _ => p.title,
            };
            (p.label, title)
        })
        .collect()
}

/// 打开（或聚焦）一个面板窗口。
///
/// **已经开着就只聚焦，不重建** —— 重建会丢掉窗口里的状态，
/// 而且用户点两次托盘就会看到内容闪一下
pub fn open(app: &AppHandle, label: &str, saved: Option<Geometry>) -> Result<()> {
    let Some(def) = panel(label) else {
        return Err(Error::config(format!("没有叫「{label}」的面板")));
    };

    if let Some(existing) = app.get_webview_window(label) {
        if label != "chat" {
            if let Some(chat_win) = app.get_webview_window("chat") {
                if chat_win.is_visible().unwrap_or(false) && !chat_win.is_minimized().unwrap_or(false) {
                    if let (Ok(chat_pos), Ok(chat_size), Ok(cur_size)) = (chat_win.outer_position(), chat_win.inner_size(), existing.inner_size()) {
                        let chat_center_x = chat_pos.x as f64 + (chat_size.width as f64 / 2.0);
                        let chat_center_y = chat_pos.y as f64 + (chat_size.height as f64 / 2.0);
                        let new_x = (chat_center_x - (cur_size.width as f64 / 2.0)) as i32;
                        let new_y = (chat_center_y - (cur_size.height as f64 / 2.0)) as i32;
                        let _ = existing.set_position(tauri::Position::Physical(tauri::PhysicalPosition { x: new_x, y: new_y }));
                    }
                }
            }
        }
        let _ = existing.show();
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(());
    }

    let url = WebviewUrl::App(format!("index.html?window={label}").into());
    let mut builder = WebviewWindowBuilder::new(app, label, url)
        .title(def.title)
        .min_inner_size(def.min_width, def.min_height)
        .resizable(true)
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);

    // 确定新打开窗口的位置与尺寸：
    // 若当前 Chat 窗口开着，则以 Chat 窗口当前位置的「正中心」居中叠放打开
    let target_size = saved
        .map(|g| (g.width as f64, g.height as f64))
        .unwrap_or((def.width, def.height));

    let mut centered_on_chat = false;
    if label != "chat" {
        if let Some(chat_win) = app.get_webview_window("chat") {
            if chat_win.is_visible().unwrap_or(false) && !chat_win.is_minimized().unwrap_or(false) {
                if let (Ok(chat_pos), Ok(chat_size)) = (chat_win.outer_position(), chat_win.inner_size()) {
                    let chat_center_x = chat_pos.x as f64 + (chat_size.width as f64 / 2.0);
                    let chat_center_y = chat_pos.y as f64 + (chat_size.height as f64 / 2.0);
                    let new_x = chat_center_x - (target_size.0 / 2.0);
                    let new_y = chat_center_y - (target_size.1 / 2.0);
                    builder = builder
                        .inner_size(target_size.0, target_size.1)
                        .position(new_x, new_y);
                    centered_on_chat = true;
                }
            }
        }
    }

    if !centered_on_chat {
        match saved {
            Some(g) => {
                builder = builder
                    .inner_size(g.width as f64, g.height as f64)
                    .position(g.x as f64, g.y as f64);
            }
            None => builder = builder.inner_size(def.width, def.height).center(),
        }
    }

    let win = builder
        .build()
        .map_err(|e| Error::Io(format!("打开「{}」窗口失败：{e}", def.title)))?;
    let _ = win.set_focus();
    tracing::info!(window = label, "打开面板窗口");
    Ok(())
}

/// 读出一个窗口当前的几何信息，用来落盘。
///
/// **最小化 / 隐藏时不要记**：那时候系统给出的位置是无意义的
/// （某些平台会返回 -32000 这种），记下来之后窗口下次会开到屏幕外面
pub fn geometry_of(app: &AppHandle, label: &str) -> Option<Geometry> {
    let w = app.get_webview_window(label)?;
    if !w.is_visible().unwrap_or(false) || w.is_minimized().unwrap_or(false) {
        return None;
    }
    let pos = w.outer_position().ok()?;
    let size = w.inner_size().ok()?;
    if size.width == 0 || size.height == 0 {
        return None;
    }
    Some(Geometry {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_panel_has_sane_defaults() {
        for p in PANELS {
            assert!(!p.label.is_empty() && !p.title.is_empty());
            // 默认尺寸不能小于最小尺寸，否则窗口一开就被顶大，
            // 用户看到的是"我设的默认值没生效"
            assert!(p.width >= p.min_width, "{} 默认宽度小于最小宽度", p.label);
            assert!(p.height >= p.min_height, "{} 默认高度小于最小高度", p.label);
        }
    }

    /// label 是窗口的唯一键，也是前端路由的参数 —— 重了会互相顶掉
    #[test]
    fn labels_are_unique_and_url_safe() {
        let mut seen = std::collections::HashSet::new();
        for (label, _) in labels() {
            assert!(seen.insert(label), "重复的 label：{label}");
            assert!(
                label.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "label 会进 URL，只能用小写字母和下划线：{label}"
            );
        }
    }

    /// 旧短剧面板 2026-09-16 物理删除了（流程早已搬进 Anima Studio）。
    /// 钉一下，别哪天又从别的分支合回来。
    #[test]
    fn 旧短剧面板已删除() {
        assert!(panel("studio").is_none());
        for lang in ["zh-CN", "en"] {
            assert!(labels_for_lang(lang).iter().all(|(l, _)| *l != "studio"));
        }
    }

    /// 角色层不在这张表里 —— 它是常驻窗口，由 tauri.conf.json 定义，
    /// 生命周期和这五个面板完全不同
    #[test]
    fn the_avatar_layer_is_not_a_panel() {
        assert!(panel("avatar").is_none());
        assert!(panel("main").is_none());
    }

    #[test]
    fn unknown_labels_are_rejected() {
        assert!(panel("nope").is_none());
        assert_eq!(labels().len(), PANELS.len());
    }


}
#[cfg(test)]
mod tray_lang_tests {
    use super::*;

    /// **托盘文案只能有一份来源。**
    ///
    /// 出过事：`lib.rs` 启动时硬编码英文、`update_tray_menu` 另写一份，
    /// 而语言只存在前端的 localStorage 里 —— 于是不管配置里选的什么语言，
    /// 每次启动托盘都是英文，用户要进设置里再切一次才生效。
    #[test]
    fn 托盘文案随语言变() {
        let (zh_toggle, zh_quit) = tray_titles("zh-CN");
        let (en_toggle, en_quit) = tray_titles("en-US");
        assert_ne!(zh_toggle, en_toggle, "中英文的切换项文案一样，等于没本地化");
        assert_ne!(zh_quit, en_quit);
        assert_eq!(zh_quit, "退出");
        assert_eq!(en_quit, "Quit");
        // 认不出来的语言回落英文，而不是 panic 或空串
        let (fb, _) = tray_titles("fr-FR");
        assert_eq!(fb, en_toggle);
    }

    /// 面板名也要跟着变 —— 只翻译两个固定项、面板还是英文，比全英文更怪。
    #[test]
    fn 面板名也随语言变() {
        let zh = labels_for_lang("zh-CN");
        let en = labels_for_lang("en-US");
        assert_eq!(zh.len(), en.len());
        let zh_chat = zh.iter().find(|(l, _)| *l == "chat").unwrap().1;
        let en_chat = en.iter().find(|(l, _)| *l == "chat").unwrap().1;
        assert_eq!(zh_chat, "对话");
        assert_eq!(en_chat, "Chat");
    }
}
