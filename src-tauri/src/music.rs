//! 放音乐：网络电台 + 控制本机 Music.app。
//!
//! ## 为什么不是"上网找一首歌来播"
//!
//! 用户想要的是「帮我上网播放音乐」。直觉的做法是"搜索 → 拿到 URL → 播"，
//! 但那条路**在真实世界里走不通**：
//!
//! - `search_web` 返回的是文字，不是音频链接
//! - 网易云 / QQ音乐 / YouTube 的页面是 HTML，下载下来是一堆标签
//! - 那些平台的音频都有鉴权和加密，直链是拿不到的
//!
//! 这不是"MCP 不够强"能解决的 —— 是版权平台本来就不给直链。
//! 所以改成两条**真的能走通**的路。
//!
//! ## 一、网络电台：ffplay
//!
//! 电台是**无尽的流**，而现有的 `play_audio` 是"整个下载到内存再播" ——
//! 对无尽流那是永远下不完。改用 `ffplay`（ffmpeg 自带，本机已有），
//! 它能吃 mp3 / aac / m3u8 和各种跳转。
//!
//! ## 二、本机音乐：AppleScript
//!
//! Music.app 的播放/暂停/下一首可以脚本控制。
//!
//! **但实测这台机器的曲库是 0 首**，所以"播放周杰伦"这种按名字点歌
//! 没法可靠实现 —— 工具说明里如实写明，让模型别去尝试然后编一句
//! "已经为你播放"。
//!
//! ## 已知的一个副作用：外放时电台会喂给麦克风
//!
//! 播放闸门管的是 Anima **自己**的输出音量，它不知道 ffplay 的存在。
//! 用外放听电台时，音乐会进麦克风 —— 纯器乐一般会被幻听过滤挡掉
//! （音乐不是语音，Whisper 的 avg logprob 很低），但**带人声的台**
//! （`dream`）有一定概率被当成用户在说话。戴耳机就没这个问题。

use std::process::{Child, Command};
use std::sync::{Mutex, OnceLock};

use crate::llm::Tool;

/// 正在放电台的那个 ffplay 进程。
///
/// 用 static 而不是挂在 AppState 上：**扬声器只有一套**，电台是系统级的
/// 单例。挂进语音会话的依赖里会暗示它是"每个会话一个"，那是错的语义。
fn radio_process() -> &'static Mutex<Option<Child>> {
    static P: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(None))
}

/// 内置电台表。
///
/// 只收**公开的直链流**（SomaFM 这类），不碰版权平台。
/// 每条都实测能解（`ffprobe` 出 mp3/44100）。
const STATIONS: &[(&str, &str, &str)] = &[
    (
        "lofi",
        "https://ice1.somafm.com/groovesalad-128-mp3",
        "chill 电子，适合工作时垫底",
    ),
    (
        "chill",
        "https://ice1.somafm.com/groovesalad-128-mp3",
        "同 lofi",
    ),
    (
        "ambient",
        "https://ice2.somafm.com/dronezone-128-mp3",
        "环境音，几乎没有旋律，最不打扰",
    ),
    (
        "dream",
        "https://ice1.somafm.com/lush-128-mp3",
        "梦幻人声流行，有女声",
    ),
    (
        "jazz",
        "https://ice2.somafm.com/sonicuniverse-128-mp3",
        "现代爵士",
    ),
    (
        "classical",
        "https://stream.wqxr.org/wqxr",
        "古典，纽约 WQXR 电台",
    ),
    (
        "indie",
        "https://ice1.somafm.com/indiepop-128-mp3",
        "独立流行",
    ),
    (
        "lounge",
        "https://ice2.somafm.com/secretagent-128-mp3",
        "慵懒 lounge / downtempo",
    ),
    (
        "space",
        "https://ice2.somafm.com/deepspaceone-128-mp3",
        "深空氛围，比 ambient 更冷",
    ),
];

/// 中文说法 → 电台名。
///
/// 工具说明里给的是英文名，但**小模型经常直接把用户的原话塞进参数** ——
/// 用户说"来点爵士"，参数就是"爵士"。没有这张表的话它会掉到默认台，
/// 用户听到的是"我说要爵士，它放了个电子乐"
const ALIASES: &[(&str, &str)] = &[
    ("爵士", "jazz"),
    ("古典", "classical"),
    ("钢琴", "classical"),
    ("轻音乐", "ambient"),
    ("纯音乐", "ambient"),
    ("环境", "ambient"),
    ("白噪", "ambient"),
    ("安静", "ambient"),
    ("舒缓", "lounge"),
    ("放松", "lounge"),
    ("独立", "indie"),
    ("民谣", "indie"),
    ("电子", "lofi"),
    ("摇滚", "indie"),
    ("流行", "dream"),
];

/// 按名字或风格找一个电台。找不到就退回第一个 ——
/// **宁可放一个差不多的，也不要因为名字对不上就什么都不放**
fn find_station(query: &str) -> (&'static str, &'static str) {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (STATIONS[0].0, STATIONS[0].1);
    }
    // 中文先查，因为中文说法更具体（"古典"要的是古典，不该被英文子串抢走）
    let resolved = ALIASES
        .iter()
        .find(|(cn, _)| q.contains(cn))
        .map(|(_, en)| *en)
        .unwrap_or(q.as_str());
    for (name, url, _) in STATIONS {
        if resolved.contains(name) || name.contains(resolved) {
            return (name, url);
        }
    }
    (STATIONS[0].0, STATIONS[0].1)
}

/// 给联调测试用的清单（名字 + 地址）。
///
/// 电台会关、地址会变，而失败是**静默的**：ffplay 照样启动，只是没声音。
/// `tests/live.rs` 拿这张表逐条 probe 一遍
pub fn station_table() -> Vec<(&'static str, &'static str)> {
    STATIONS.iter().map(|(n, u, _)| (*n, *u)).collect()
}

pub fn declarations() -> Vec<Tool> {
    let list = STATIONS
        .iter()
        .map(|(n, _, d)| format!("{n}（{d}）"))
        .collect::<Vec<_>>()
        .join("、");
    vec![
        Tool {
            name: "play_radio".to_string(),
            description: format!(
                "放一个网络电台当背景音乐。用户说「放点音乐」「来点轻音乐」时用这个。\
                 可选风格：{list}。说不清就留空，会给一个安静的默认台。"
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "station": { "type": "string", "description": "风格关键词，如 lofi / jazz / ambient" }
                }
            }),
        },
        Tool {
            name: "stop_radio".to_string(),
            description: "关掉正在放的网络电台。".to_string(),
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
        },
        Tool {
            name: "music_control".to_string(),
            description: format!(
                "控制本机正在开着的音乐播放器（{}，以及 Apple 的 Music.app）。\
                 会自动认出哪个开着，不用问用户是哪个。\
                 **只能控制播放/暂停/上一首/下一首，不能按歌名点歌** —— \
                 这些播放器不提供搜歌的接口，点歌要让用户自己在 App 里选。",
                MENU_PLAYERS
                    .iter()
                    .map(|p| p.label)
                    .collect::<Vec<_>>()
                    .join("、")
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["play", "pause", "next", "previous", "status"]
                    }
                },
                "required": ["action"]
            }),
        },
    ]
}

pub fn is_music_call(name: &str) -> bool {
    matches!(name, "play_radio" | "stop_radio" | "music_control")
}

/// 关掉正在放的电台。返回是否真的关了一个
pub fn stop_radio() -> bool {
    let mut guard = radio_process().lock().unwrap();
    if let Some(mut child) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
        return true;
    }
    false
}

fn start_radio(query: &str) -> String {
    // 先停掉上一个，否则两个台一起响
    stop_radio();

    let (name, url) = find_station(query);
    // ffplay 是 ffmpeg 自带的。**它不在的话要说清楚怎么装** ——
    // 只说"播放失败"用户无从下手
    let spawned = Command::new("ffplay")
        .args(["-nodisp", "-loglevel", "quiet", "-autoexit", url])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    match spawned {
        Ok(mut child) => {
            // **spawn 成功 ≠ 有声音。** 台挂了的话 ffplay 照样起得来，
            // 连不上再退出。不等这一下，模型会说"正在放 jazz 台"而用户什么都没听见 ——
            // 又是一次静默失败
            std::thread::sleep(std::time::Duration::from_millis(1200));
            if let Ok(Some(status)) = child.try_wait() {
                tracing::warn!(station = name, url, ?status, "电台起来就退了");
                return format!(
                    "「{name}」台连不上，没放成。可以换一个风格试试，\
                     或者如实告诉用户这个台现在不可用 —— 不要说已经放了。"
                );
            }
            *radio_process().lock().unwrap() = Some(child);
            tracing::info!(station = name, url, "开始放电台");
            format!("正在放「{name}」台。简短说一句就好，不用报电台地址。")
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            "放不了：这台机器上找不到 ffplay。它是 ffmpeg 自带的，\
             装一下就行：`brew install ffmpeg`。请把这句话转告用户。"
                .to_string()
        }
        Err(e) => format!("启动播放器失败：{e}"),
    }
}

/// 跑一段 AppleScript，返回它的输出
fn osascript(script: &str) -> Result<String, String> {
    let out = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("执行 AppleScript 失败：{e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// 一个靠**点菜单栏**来控制的播放器。
///
/// ## 为什么是菜单栏
///
/// 网易云音乐没有 AppleScript 字典，`orpheus://` 只有内部 webview 路由，
/// 窗口内容也不给辅助功能读（整个窗口只有三个无名按钮 = 红黄绿交通灯）。
/// 三条常规路都断了。
///
/// 媒体键那条也断了，而且断得很隐蔽：`MRMediaRemoteSendCommand` 返回 `true`，
/// 但什么都没发生 —— 那个 `true` 只表示"消息发出去了"。用音量键交叉验证过，
/// 合成的系统事件连系统音量都推不动，**所以不是某个 App 不理，是这条路本身封了**。
/// 而普通按键（空格）是能合成的，说明封的只是 system-defined 那一类。
///
/// 但**菜单栏是能读能点的** —— 哪怕窗口内容一片空白。实测网易云的「控制」菜单里
/// 有：播放、下一个、上一个、赞……而且第一项的**名字本身就是状态读数**
/// （显示"播放"=没在放，显示"暂停"=正在放）。
///
/// ## 加新播放器要注意
///
/// **只加你在真机上验过的。** 菜单名会随 App 版本和语言变，照着别人的
/// 文档抄一行进来，坏掉的时候是静默的 —— 点不着就什么都不发生。
struct MenuPlayer {
    /// 辅助功能里的进程名
    process: &'static str,
    /// 用户会怎么称呼它
    label: &'static str,
    /// 「控制」菜单叫什么。系统语言不同名字不同，都试一遍
    menu: &'static [&'static str],
    /// 播放/暂停项。它永远是第 1 项，名字在"播放"/"暂停"之间切
    next_item: &'static [&'static str],
    prev_item: &'static [&'static str],
    /// 第一项显示这个词时，表示**正在放**（点下去会暂停）
    playing_word: &'static str,
}

/// 只有实测验过的才在这里。网易云是在 macOS 26 / 客户端 3.x 上验的
const MENU_PLAYERS: &[MenuPlayer] = &[MenuPlayer {
    process: "NeteaseMusic",
    label: "网易云音乐",
    menu: &["控制", "Controls"],
    next_item: &["下一个", "Next"],
    prev_item: &["上一个", "Previous"],
    playing_word: "暂停",
}];

/// 某个进程开着吗
fn process_running(name: &str) -> bool {
    osascript(&format!(
        r#"tell application "System Events" to return (exists (processes where name is "{name}"))"#
    ))
    .map(|s| s.trim() == "true")
    .unwrap_or(false)
}

/// 辅助功能权限没给的话，AppleScript 会以特定的错误码失败。
/// **要认出这一类并说人话** —— 原始错误是 "osascript is not allowed assistive
/// access. (-1719)"，直接抛给用户等于没说
fn is_permission_error(err: &str) -> bool {
    err.contains("-1719")
        || err.contains("-25211")
        || err.contains("assistive access")
        || err.contains("辅助访问")
        || err.contains("辅助功能")
}

const GRANT_HINT: &str = "缺辅助功能权限，控制不了播放器。\
    请转告用户去「系统设置 → 隐私与安全性 → 辅助功能」里把 Anima 打开。\
    （开发期每次重新编译二进制都会变，可能要重新勾一次。）";

impl MenuPlayer {
    /// 在这个播放器的「控制」菜单里点一项。
    ///
    /// `item` 是 `None` 时点第 1 项（播放/暂停那个，它的名字是会变的，
    /// 所以只能按位置点）
    fn click(&self, item: Option<&[&str]>) -> Result<String, String> {
        // 菜单名和菜单项名都可能因语言而异，逐个试 —— 试不着才算失败
        let mut last = String::from("没找到可点的菜单项");
        for menu in self.menu {
            let target = match item {
                None => "menu item 1 of m".to_string(),
                Some(names) => {
                    let mut found = None;
                    for n in names {
                        let probe = format!(
                            r#"tell application "System Events" to tell process "{}" to return (exists menu item "{n}" of menu 1 of menu bar item "{menu}" of menu bar 1)"#,
                            self.process
                        );
                        match osascript(&probe) {
                            Ok(s) if s.trim() == "true" => {
                                found = Some(format!("menu item \"{n}\" of m"));
                                break;
                            }
                            Ok(_) => {}
                            Err(e) => last = e,
                        }
                    }
                    match found {
                        Some(t) => t,
                        None => continue,
                    }
                }
            };
            // `after` 之类是 AppleScript 保留字，变量名别踩
            let script = format!(
                "tell application \"System Events\"\n\
                 tell process \"{proc}\"\n\
                 set m to menu 1 of menu bar item \"{menu}\" of menu bar 1\n\
                 click {target}\n\
                 delay 1.5\n\
                 return name of menu item 1 of m\n\
                 end tell\n\
                 end tell",
                proc = self.process,
            );
            match osascript(&script) {
                Ok(s) => return Ok(s),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// 不点，只读第一项的名字 —— 那就是播放状态
    fn state_word(&self) -> Result<String, String> {
        for menu in self.menu {
            let script = format!(
                r#"tell application "System Events" to tell process "{}" to return name of menu item 1 of menu 1 of menu bar item "{menu}" of menu bar 1"#,
                self.process
            );
            if let Ok(s) = osascript(&script) {
                return Ok(s);
            }
        }
        Err("读不到播放器的菜单".to_string())
    }

    fn is_playing(&self) -> bool {
        self.state_word()
            .map(|w| w == self.playing_word)
            .unwrap_or(false)
    }
}

/// 现在开着的、能控制的第三方播放器
fn running_menu_player() -> Option<&'static MenuPlayer> {
    MENU_PLAYERS.iter().find(|p| process_running(p.process))
}

fn control_menu_player(p: &MenuPlayer, action: &str) -> String {
    let playing = p.is_playing();
    let result = match action {
        "status" => {
            return if playing {
                format!(
                    "{} 正在放。（读不到具体歌名 —— 它的窗口不给辅助功能读，只有菜单栏能读）",
                    p.label
                )
            } else {
                format!("{} 开着，但没在放", p.label)
            };
        }
        // 播放/暂停是同一项，所以先读状态，已经是想要的就别点
        "play" if playing => return format!("{} 本来就在放，没动它", p.label),
        "pause" if !playing => return format!("{} 本来就没在放，没动它", p.label),
        "play" | "pause" => p.click(None),
        "next" => p.click(Some(p.next_item)),
        "previous" => p.click(Some(p.prev_item)),
        _ => unreachable!(),
    };
    match result {
        Ok(_) => format!(
            "已在 {} 上{}",
            p.label,
            match action {
                "play" => "开始播放",
                "pause" => "暂停",
                "next" => "切到下一首",
                _ => "切到上一首",
            }
        ),
        Err(e) if is_permission_error(&e) => GRANT_HINT.to_string(),
        Err(e) => format!("{} 没能执行：{e}", p.label),
    }
}

/// Music.app 现在开着吗。
///
/// **必须先问这一句**：`tell application "Music" to ...` 会把它**启动起来**。
/// 用户问"现在在放什么"结果弹出一个播放器，这是不对的 ——
/// 只有明确要 play 的时候才该让它启动
fn music_is_running() -> bool {
    osascript(
        r#"tell application "System Events" to return (exists (processes where name is "Music"))"#,
    )
    .map(|s| s.trim() == "true")
    .unwrap_or(false)
}

fn control_music(action: &str) -> String {
    // 先认参数，再碰系统 —— 顺序反过来的话，写错的 action 会先得到
    // "Music.app 没开着"，模型永远看不到自己参数传错了
    if !matches!(action, "play" | "pause" | "next" | "previous" | "status") {
        return format!("不支持的操作「{action}」。可用：play / pause / next / previous / status");
    }
    // **谁开着就控制谁**，不让模型选。多一个 app 参数就多一处它能选错的地方，
    // 而用户说"暂停一下"时心里想的就是眼前正在响的那个
    if let Some(p) = running_menu_player() {
        return control_menu_player(p, action);
    }
    // play 之外的操作都建立在"它已经在放"的前提上，不该由我们把它拉起来
    if action != "play" && !music_is_running() {
        return "Music.app 现在没开着。如实告诉用户，可以问他要不要打开。".to_string();
    }
    let script = match action {
        "play" => "tell application \"Music\" to play",
        "pause" => "tell application \"Music\" to pause",
        "next" => "tell application \"Music\" to next track",
        "previous" => "tell application \"Music\" to previous track",
        "status" => {
            return match osascript(
                "tell application \"Music\"\n\
                 if player state is playing then\n\
                 return \"正在放：\" & (name of current track) & \" —— \" & (artist of current track)\n\
                 else\n\
                 return \"没在放\"\n\
                 end if\n\
                 end tell",
            ) {
                Ok(s) => s,
                // 曲库空 / 没有当前曲目时 AppleScript 会报错，
                // 如实说而不是让模型以为出了大问题
                Err(_) => "Music.app 里现在没有正在播放的曲目".to_string(),
            };
        }
        // 上面已经把动作认全了，走不到这里
        _ => unreachable!(),
    };
    match osascript(script) {
        Ok(_) => format!(
            "已{}",
            match action {
                "play" => "继续播放",
                "pause" => "暂停",
                "next" => "切到下一首",
                _ => "切到上一首",
            }
        ),
        Err(e) => format!("Music.app 没能执行：{e}。可能是曲库为空或没有正在播放的曲目"),
    }
}

/// 执行音乐相关的工具调用
pub fn execute(name: &str, arguments: &str) -> String {
    let args: serde_json::Value = serde_json::from_str(arguments).unwrap_or_default();
    match name {
        "play_radio" => {
            let q = args.get("station").and_then(|v| v.as_str()).unwrap_or("");
            start_radio(q)
        }
        "stop_radio" => {
            if stop_radio() {
                "已关掉电台".to_string()
            } else {
                "本来就没在放电台".to_string()
            }
        }
        "music_control" => {
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("status");
            control_music(action)
        }
        other => format!("不认识的音乐工具：{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_station_has_a_real_stream_url() {
        for (name, url, desc) in STATIONS {
            assert!(url.starts_with("https://"), "{name} 的地址不是 https");
            assert!(!desc.is_empty(), "{name} 没有说明，模型没法判断什么时候用");
        }
    }

    #[test]
    fn matches_a_station_by_keyword() {
        assert_eq!(find_station("jazz").0, "jazz");
        assert_eq!(find_station("来点 lofi").0, "lofi");
        assert_eq!(find_station("AMBIENT").0, "ambient");
    }

    /// **名字对不上也要放点什么。** 因为找不到台就什么都不放，
    /// 用户看到的是"它说放了但没声音"
    #[test]
    fn an_unknown_style_still_plays_something() {
        let (name, url) = find_station("赛博朋克电子摇滚");
        assert!(!name.is_empty());
        assert!(url.starts_with("https://"));
    }

    /// 小模型会把用户的原话直接塞进参数。「来点爵士」不能放成电子乐
    #[test]
    fn chinese_style_words_find_the_right_station() {
        assert_eq!(find_station("爵士").0, "jazz");
        assert_eq!(find_station("来点古典的").0, "classical");
        assert_eq!(find_station("放点轻音乐").0, "ambient");
        assert_eq!(find_station("独立音乐").0, "indie");
    }

    /// 每个别名都要指向表里真实存在的台，否则它等于没写
    #[test]
    fn every_alias_points_at_a_real_station() {
        for (cn, en) in ALIASES {
            assert!(
                STATIONS.iter().any(|(n, _, _)| n == en),
                "别名「{cn}」指向了不存在的台 {en}"
            );
            assert_eq!(find_station(cn).0, *en, "别名「{cn}」没解析到 {en}");
        }
    }

    #[test]
    fn an_empty_query_gets_the_default() {
        assert_eq!(find_station("").0, STATIONS[0].0);
        assert_eq!(find_station("   ").0, STATIONS[0].0);
    }

    /// 工具说明里必须写明"不能点歌" —— 否则模型会尝试，
    /// 然后编一句"已经为你播放周杰伦"
    #[test]
    fn the_tool_admits_it_cannot_pick_songs_by_name() {
        let d = declarations();
        let music = d.iter().find(|t| t.name == "music_control").unwrap();
        assert!(music.description.contains("不能按歌名点歌"));
    }

    /// 电台清单要进工具说明，否则模型不知道有哪些风格可选
    #[test]
    fn the_station_list_is_visible_to_the_model() {
        let d = declarations();
        let radio = d.iter().find(|t| t.name == "play_radio").unwrap();
        for (name, _, _) in STATIONS {
            assert!(radio.description.contains(name), "清单里缺 {name}");
        }
    }

    #[test]
    fn unknown_action_explains_the_options() {
        let r = control_music("dance");
        assert!(r.contains("play") && r.contains("pause"));
    }

    /// 权限没给时 AppleScript 报的是 "not allowed assistive access. (-1719)"。
    /// **认不出这一类就等于把原始错误抛给用户**，他不知道要去哪儿点
    #[test]
    fn permission_errors_are_recognised() {
        for e in [
            "execution error: osascript is not allowed assistive access. (-1719)",
            "错误：不允许辅助访问。 (-25211)",
        ] {
            assert!(is_permission_error(e), "没认出来：{e}");
        }
        assert!(!is_permission_error(
            "不能获得“menu bar item 控制”。 (-1728)"
        ));
    }

    /// 权限提示要**说清楚去哪儿点**，还要提醒开发期会掉 ——
    /// 这和 Keychain 那个坑是同一类：重新编译二进制就变了
    #[test]
    fn the_permission_hint_says_where_to_click() {
        assert!(GRANT_HINT.contains("辅助功能"));
        assert!(GRANT_HINT.contains("系统设置"));
        assert!(GRANT_HINT.contains("重新编译"));
    }

    /// 表里每一行都得填全。少一个菜单名，点不着的时候是**静默的**
    #[test]
    fn every_menu_player_entry_is_complete() {
        for p in MENU_PLAYERS {
            assert!(!p.process.is_empty() && !p.label.is_empty());
            assert!(!p.menu.is_empty(), "{} 没写「控制」菜单叫什么", p.label);
            assert!(
                !p.next_item.is_empty() && !p.prev_item.is_empty(),
                "{}",
                p.label
            );
            assert!(
                !p.playing_word.is_empty(),
                "{} 没写状态词，读不出在不在放",
                p.label
            );
        }
    }

    /// 模型得知道它能控制哪些播放器，否则用户说"暂停网易云"它会说做不到
    #[test]
    fn the_tool_names_the_players_it_can_control() {
        let d = declarations();
        let m = d.iter().find(|t| t.name == "music_control").unwrap();
        for p in MENU_PLAYERS {
            assert!(m.description.contains(p.label), "说明里没提 {}", p.label);
        }
    }

    #[test]
    fn stopping_when_nothing_plays_is_not_an_error() {
        // 没在放的时候关一下不该炸
        assert!(!execute("stop_radio", "{}").is_empty());
    }
}
