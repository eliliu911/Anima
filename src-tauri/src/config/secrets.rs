//! API Key 存储。
//!
//! 按 DECISIONS.md 的要求，正式存储走 macOS Keychain，配置文件里不落明文。
//! 前端永远拿不到 key 本身，只能问"某个后端有没有配 key"（[`has_secret`]）。
//!
//! ## 为什么还有环境变量这条路
//!
//! Keychain 条目的访问控制（ACL）绑定的是**读取它的那个可执行文件**。
//! `cargo tauri dev` 每次改代码都会产出一个新的、未签名的二进制，系统认为
//! 那是"另一个程序"，于是每次启动都弹授权框要登录密码 —— 点"始终允许"也没用，
//! 下次重新编译又是新身份。这是 Tauri 本地开发的已知痛点。
//!
//! 所以查找顺序是：**环境变量 → Keychain**。开发时用环境变量喂 key，一次都不弹框；
//! 打包成签名 `.app` 之后二进制身份稳定，Keychain 才是那条正路。
//!
//! ## 第三条路：应用数据目录里的安全文件（2026-09-18 加，**打包版的正路**）
//!
//! 上面那段的结论「打包成签名 .app 之后用 Keychain」**前提已经不成立**：
//! 用户是免费 Apple 账号，拿不到 Developer ID 证书，只能 ad-hoc 签名，
//! 而 ad-hoc 签名的身份基于 cdhash —— **每次重编都变**，于是 Keychain
//! 每次都当「另一个程序」，每个 provider 弹一次框。和 `tauri dev` 一模一样。
//!
//! 同时**打包版双击启动没有任何环境变量**（不经过 shell），
//! 所以开发期那条路在打包版里直接断掉，表现是「模型列表拉不出来」。
//!
//! 所以存到 `<应用数据目录>/secrets.env`，权限 600，格式和 `.env.local` 一样。
//! 查找顺序变成：**进程环境变量 → 安全文件 → Keychain**。
//!
//! - 环境变量还在最前面：`with-env.sh` 注入的开发期 key 继续优先，行为不变
//! - 安全文件是**写入的默认去处**，所以设置面板存 key 不再弹任何框
//! - Keychain 只剩**读取回退**，给已经存过的老数据用
//!
//! 权限比 600 宽的话会**自动 chmod 回去**再读 —— 只警告不修的护栏等于没有。
//!
//! ## 为什么要缓存
//!
//! 即使 ACL 正确，每次读 Keychain 都是一次系统调用。而 `get_config` 会为每个
//! 后端查一次"有没有 key"，一次刷新就是 6 次读 —— ACL 一旦不匹配，就是 6 个弹框。
//! 因此进程内缓存结果，每个 provider 每个进程最多读一次。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use crate::error::{Error, Result};

const SERVICE: &str = "com.anima.desktop";

/// 环境变量前缀。provider id 转大写、`-` 换成 `_`，
/// 例如 `azure` → `ANIMA_API_KEY_AZURE`。
const ENV_PREFIX: &str = "ANIMA_API_KEY_";

/// 安全文件的文件名。放在应用数据目录里，和 `config.json` 并排。
pub const SECRET_FILE: &str = "secrets.env";

/// 一个 key 是从哪儿来的。**界面要靠它决定「这条能不能在设置里改」。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretSource {
    /// 进程环境变量（`with-env.sh` / shell）。界面改不掉
    Env,
    /// 应用数据目录里的 `secrets.env`。**写入的默认去处**
    File,
    /// macOS Keychain。只剩读取回退，给老数据用
    Keychain,
}

struct SecretFile {
    path: PathBuf,
    /// 环境变量名 -> 值。键用**全名**（`ANIMA_API_KEY_OPENAI`），
    /// 这样和 `.env.local` / `.env.example` 是同一套命名，不用再映射一次
    vars: HashMap<String, String>,
}

fn secret_file() -> &'static RwLock<Option<SecretFile>> {
    static F: OnceLock<RwLock<Option<SecretFile>>> = OnceLock::new();
    F.get_or_init(|| RwLock::new(None))
}

/// 解析 `.env` 那种格式。**纯函数，方便测。**
///
/// 认 `KEY=VALUE`、`export KEY=VALUE`、`#` 注释、空行，值两端的引号剥掉。
/// **不认行内注释** —— key 里出现 `#` 是完全合法的，把它当注释砍掉会
/// 得到一个被截短的 key，然后表现成「认证失败」，谁也想不到是这儿。
pub fn parse_env_file(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((k, v)) = line.split_once('=') else { continue };
        let k = k.trim();
        if k.is_empty() {
            continue;
        }
        let v = v.trim();
        let v = v
            .strip_prefix('"')
            .and_then(|x| x.strip_suffix('"'))
            .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
            .unwrap_or(v);
        if !v.is_empty() {
            out.insert(k.to_string(), v.to_string());
        }
    }
    out
}

/// 把一份变量写回 `.env` 文本。排序输出 —— 每次写的顺序一样，diff 才有意义。
pub fn render_env_file(vars: &HashMap<String, String>) -> String {
    let mut keys: Vec<&String> = vars.keys().collect();
    keys.sort();
    let mut s = String::from(
        "# Anima 的 API Key。**这个文件权限是 600，不要放进版本库、不要发给别人。**\n\
         # 由设置面板或 `anima key set` 写入；手工改完重启应用生效。\n\
         # 变量名规则：ANIMA_API_KEY_ + provider id 大写（- 和 . 换成 _）。\n\n",
    );
    for k in keys {
        s.push_str(&format!("{k}={}\n", vars[k]));
    }
    s
}

/// 权限收紧到 600。**比 600 宽就直接改掉**，不是只警告 ——
/// 一个只会抱怨的护栏等于没有护栏，而这里护的是明文 key。
#[cfg(unix)]
fn tighten(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).map_err(|e| Error::config(format!("读不到 {}: {e}", path.display())))?;
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        tracing::warn!(path = %path.display(), current = format!("{mode:o}"), "密钥文件权限过宽，已收紧到 600");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| Error::config(format!("收紧 {} 的权限失败：{e}", path.display())))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn tighten(_path: &Path) -> Result<()> {
    Ok(())
}

/// 启动时调一次，把安全文件读进来。**必须在任何 `get_secret` 之前。**
///
/// 文件不存在不算错 —— 全新安装、或者只用环境变量的开发机都是这样。
pub fn init(config_dir: &Path) -> Result<()> {
    let path = config_dir.join(SECRET_FILE);
    let vars = if path.is_file() {
        tighten(&path)?;
        let text = std::fs::read_to_string(&path).map_err(|e| Error::config(format!("读不了密钥文件：{e}")))?;
        parse_env_file(&text)
    } else {
        HashMap::new()
    };
    tracing::info!(count = vars.len(), path = %path.display(), "密钥文件已加载");
    *secret_file().write().unwrap() = Some(SecretFile { path, vars });
    Ok(())
}

/// 安全文件里配的 key。
fn from_file(provider_id: &str) -> Option<String> {
    let name = env_var_name(provider_id);
    secret_file()
        .read()
        .unwrap()
        .as_ref()?
        .vars
        .get(&name)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// 往安全文件里写一条（空值 = 删掉这一条）。文件权限**创建时就是 600**。
fn write_file(provider_id: &str, secret: &str) -> Result<()> {
    let mut guard = secret_file().write().unwrap();
    let f = guard
        .as_mut()
        .ok_or_else(|| Error::config("密钥文件还没初始化 —— secrets::init 没被调用"))?;
    let name = env_var_name(provider_id);
    if secret.is_empty() {
        f.vars.remove(&name);
    } else {
        f.vars.insert(name, secret.to_string());
    }
    if let Some(dir) = f.path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::config(format!("建不了配置目录：{e}")))?;
    }
    let text = render_env_file(&f.vars);
    // **先建成 600 再写**，不能先写完再 chmod —— 中间那一瞬文件是 644 的
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut h = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&f.path)
            .map_err(|e| Error::config(format!("写不了密钥文件：{e}")))?;
        h.write_all(text.as_bytes()).map_err(|e| Error::config(format!("写不了密钥文件：{e}")))?;
    }
    #[cfg(not(unix))]
    std::fs::write(&f.path, text.as_bytes()).map_err(|e| Error::config(format!("写不了密钥文件：{e}")))?;
    tighten(&f.path)?;
    Ok(())
}

/// provider id -> key（None 表示确认过"没有"，避免反复去问 Keychain）
fn cache() -> &'static RwLock<HashMap<String, Option<String>>> {
    static CACHE: OnceLock<RwLock<HashMap<String, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn env_var_name(provider_id: &str) -> String {
    format!(
        "{ENV_PREFIX}{}",
        provider_id.to_uppercase().replace(['-', '.'], "_")
    )
}

/// 环境变量里配的 key。命中时完全不碰 Keychain。
fn from_env(provider_id: &str) -> Option<String> {
    std::env::var(env_var_name(provider_id))
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn entry(provider_id: &str) -> Result<keyring::Entry> {
    // account 用 provider id，一个后端一条记录
    Ok(keyring::Entry::new(SERVICE, provider_id)?)
}

fn read_keychain(provider_id: &str) -> Result<Option<String>> {
    match entry(provider_id)?.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

pub fn set_secret(provider_id: &str, secret: &str) -> Result<()> {
    if secret.is_empty() {
        // 空字符串当作"清除"，避免 Keychain 里留一条空记录
        return delete_secret(provider_id);
    }

    // **写安全文件，不写 Keychain。** 理由见模块头：ad-hoc 签名每次重编
    // 身份都变，Keychain 会每个 provider 弹一次框，和开发期一样难用。
    write_file(provider_id, secret)?;
    cache()
        .write()
        .unwrap()
        .insert(provider_id.to_string(), Some(secret.to_string()));
    Ok(())
}

pub fn get_secret(provider_id: &str) -> Result<Option<String>> {
    // 环境变量优先，且不进缓存 —— 它本来就是零成本读取，
    // 也让"改完环境变量重启即生效"这件事保持直观。
    if let Some(key) = from_env(provider_id) {
        return Ok(Some(key));
    }

    // 安全文件第二 —— 也不进缓存，改完重启即生效，和环境变量一致
    if let Some(key) = from_file(provider_id) {
        return Ok(Some(key));
    }

    if let Some(hit) = cache().read().unwrap().get(provider_id) {
        return Ok(hit.clone());
    }

    let fetched = read_keychain(provider_id)?;
    cache()
        .write()
        .unwrap()
        .insert(provider_id.to_string(), fetched.clone());
    Ok(fetched)
}

pub fn delete_secret(provider_id: &str) -> Result<()> {
    // **两处都要删。** 只删一处的话另一处还在，界面显示「已清除」而请求照常带着 key ——
    // 这正是这个仓库反复栽的「一份事实存两处」。
    let file_result = write_file(provider_id, "");
    let result = match entry(provider_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err.into()),
    };
    // 无论删除是否成功都要让缓存失效，否则内存里还留着已删掉的 key
    cache().write().unwrap().remove(provider_id);
    result.and(file_result)
}

/// 只回答"有没有"，不返回内容 —— 给设置面板显示状态用。
pub fn has_secret(provider_id: &str) -> bool {
    matches!(get_secret(provider_id), Ok(Some(s)) if !s.is_empty())
}

/// 该后端的 key 是不是来自环境变量。
///
/// UI 用它提示"这条是环境变量注入的，在界面里改不掉"，
/// 免得用户在设置里存了 Keychain 却发现不生效。
pub fn is_from_env(provider_id: &str) -> bool {
    from_env(provider_id).is_some()
}

/// 这条 key 是从哪儿来的。**只有 [`SecretSource::Env`] 那条在界面里改不掉。**
pub fn secret_source(provider_id: &str) -> Option<SecretSource> {
    if from_env(provider_id).is_some() {
        return Some(SecretSource::Env);
    }
    if from_file(provider_id).is_some() {
        return Some(SecretSource::File);
    }
    // **走 `get_secret` 而不是直接读 Keychain** —— 它有进程内缓存。
    // 直接读的话每刷新一次设置页就是每个 provider 一次系统调用，
    // 而 ad-hoc 签名下每次调用都可能弹一个授权框。
    matches!(get_secret(provider_id), Ok(Some(s)) if !s.is_empty()).then_some(SecretSource::Keychain)
}

/// 安全文件在哪、存了几条、权限对不对。诊断页和 `anima doctor` 用。
pub fn secret_file_status() -> (String, usize, bool) {
    let guard = secret_file().read().unwrap();
    let Some(f) = guard.as_ref() else {
        return (String::new(), 0, false);
    };
    let ok = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(&f.path).map(|m| m.permissions().mode() & 0o077 == 0).unwrap_or(true)
        }
        #[cfg(not(unix))]
        {
            true
        }
    };
    (f.path.to_string_lossy().to_string(), f.vars.len(), ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **碰安全文件的测试必须串行。** `secret_file()` 是进程级静态，
    /// 并行跑时各自 `init` 到不同临时目录，后一个会把前一个的覆盖掉 ——
    /// 表现是「单跑全过、一起跑就挂」。
    fn 独占() -> std::sync::MutexGuard<'static, ()> {
        static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
        L.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn env_var_name_normalises_provider_id() {
        assert_eq!(env_var_name("azure"), "ANIMA_API_KEY_AZURE");
        assert_eq!(env_var_name("lm-studio"), "ANIMA_API_KEY_LM_STUDIO");
        assert_eq!(env_var_name("gpt.4o"), "ANIMA_API_KEY_GPT_4O");
    }

    #[test]
    fn env_var_takes_priority_and_avoids_keychain() {
        // 用一个不可能存在于 Keychain 的 id，证明值确实来自环境变量
        let id = "anima-test-provider-env-only";
        let name = env_var_name(id);

        // SAFETY: 测试进程内自己设自己读；这个变量名不与其他测试重叠
        unsafe { std::env::set_var(&name, "  from-env  ") };

        assert_eq!(get_secret(id).unwrap().as_deref(), Some("from-env"));
        assert!(has_secret(id));
        assert!(is_from_env(id));

        unsafe { std::env::remove_var(&name) };
        assert!(!is_from_env(id));
    }

    /// `.env` 解析：注释、`export`、引号、空行都要认。
    #[test]
    fn 解析env格式() {
        let v = parse_env_file(
            "# 注释\n\nANIMA_API_KEY_OPENAI=sk-abc\nexport ANIMA_API_KEY_AZURE=\"sk-def\"\n  ANIMA_API_KEY_X = sk-ghi  \n坏行没有等号\n=没有名字\nANIMA_API_KEY_EMPTY=\n",
        );
        assert_eq!(v.get("ANIMA_API_KEY_OPENAI").unwrap(), "sk-abc");
        assert_eq!(v.get("ANIMA_API_KEY_AZURE").unwrap(), "sk-def", "引号要剥掉");
        assert_eq!(v.get("ANIMA_API_KEY_X").unwrap(), "sk-ghi", "两边空格要去掉");
        assert!(!v.contains_key("ANIMA_API_KEY_EMPTY"), "空值等于没配");
        assert_eq!(v.len(), 3);
    }

    /// **不认行内注释。** key 里出现 `#` 完全合法，当注释砍掉会得到一个
    /// 被截短的 key，然后表现成「认证失败」—— 谁也想不到是这儿。
    #[test]
    fn 不把井号当行内注释() {
        let v = parse_env_file("ANIMA_API_KEY_OPENAI=sk-ab#cd\n");
        assert_eq!(v.get("ANIMA_API_KEY_OPENAI").unwrap(), "sk-ab#cd");
    }

    /// 写出去再读回来要一模一样，而且顺序稳定（diff 才有意义）。
    #[test]
    fn 写出再读回不变() {
        let mut m = HashMap::new();
        m.insert("ANIMA_API_KEY_B".to_string(), "sk-2".to_string());
        m.insert("ANIMA_API_KEY_A".to_string(), "sk-1".to_string());
        let text = render_env_file(&m);
        assert_eq!(parse_env_file(&text), m);
        assert!(text.find("ANIMA_API_KEY_A").unwrap() < text.find("ANIMA_API_KEY_B").unwrap(), "要排序输出");
    }

    /// **文件一创建就是 600**，不能先写完再 chmod —— 中间那一瞬是 644 的。
    #[cfg(unix)]
    #[test]
    fn 安全文件权限是600() {
        let _g = 独占();
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        init(d.path()).unwrap();
        write_file("openai", "sk-secret").unwrap();
        let p = d.path().join(SECRET_FILE);
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        // 权限被人改宽了要自动收回去
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        init(d.path()).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600, "宽权限没被收紧");
    }

    /// 写进安全文件的 key 要读得回来，删掉要真的没了。
    #[cfg(unix)]
    #[test]
    fn 安全文件读写一轮() {
        let _g = 独占();
        let d = tempfile::tempdir().unwrap();
        init(d.path()).unwrap();
        let id = "anima-test-file-provider";
        assert!(from_file(id).is_none());
        write_file(id, "sk-from-file").unwrap();
        assert_eq!(from_file(id).as_deref(), Some("sk-from-file"));
        assert_eq!(secret_source(id), Some(SecretSource::File));
        // 重新 init（模拟重启）要还在
        init(d.path()).unwrap();
        assert_eq!(from_file(id).as_deref(), Some("sk-from-file"));
        write_file(id, "").unwrap();
        assert!(from_file(id).is_none(), "清空应当等于删除");
    }

    /// **环境变量排在安全文件前面** —— 开发期 `with-env.sh` 注入的必须继续优先。
    #[cfg(unix)]
    #[test]
    fn 环境变量压过安全文件() {
        let _g = 独占();
        let d = tempfile::tempdir().unwrap();
        init(d.path()).unwrap();
        let id = "anima-test-priority";
        write_file(id, "sk-file").unwrap();
        let name = env_var_name(id);
        unsafe { std::env::set_var(&name, "sk-env") };
        assert_eq!(get_secret(id).unwrap().as_deref(), Some("sk-env"));
        assert_eq!(secret_source(id), Some(SecretSource::Env));
        unsafe { std::env::remove_var(&name) };
        assert_eq!(get_secret(id).unwrap().as_deref(), Some("sk-file"), "环境变量撤掉后该回到安全文件");
    }

    #[test]
    fn blank_env_var_is_treated_as_unset() {
        let id = "anima-test-provider-blank-env";
        let name = env_var_name(id);

        unsafe { std::env::set_var(&name, "   ") };
        assert!(!is_from_env(id), "全空白应当视为没配");
        unsafe { std::env::remove_var(&name) };
    }
}
