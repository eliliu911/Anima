use serde::{Serialize, Serializer};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("网络请求失败：{0}")]
    Network(String),

    /// 上游返回了非 2xx。带上 body，因为本地推理服务的关键报错都在 body 里。
    #[error("上游服务返回 {status}：{body}")]
    Upstream { status: u16, body: String },

    #[error("解析上游响应失败：{0}")]
    Decode(String),

    #[error("配置错误：{0}")]
    Config(String),

    /// 找不到 API Key，或 Keychain 读写失败
    #[error("凭据错误：{0}")]
    Credential(String),

    #[error("未找到后端 \"{0}\"，请先在设置里配置")]
    UnknownProvider(String),

    #[error("IO 错误：{0}")]
    Io(String),

    /// 运行时错误 —— 业务上没走通，和磁盘/网络无关。
    ///
    /// **单独一个变体是必要的。** 早先 [`Error::runtime`] 指向 `Io`，
    /// 于是所有业务错误在界面上都显示成「IO 错误：…」——
    /// 用户看到「IO 错误」会去查磁盘和网络，而真正的原因写在后面那句里。
    #[error("{0}")]
    Runtime(String),
}

impl Error {
    pub fn upstream(status: u16, body: impl Into<String>) -> Self {
        let body = body.into();
        // 上游报错体有时是整篇 HTML 错误页，截断避免污染日志和 UI
        let body = if body.chars().count() > 600 {
            body.chars().take(600).collect::<String>() + "…"
        } else {
            body
        };
        Self::Upstream { status, body }
    }

    pub fn decode(msg: impl Into<String>) -> Self {
        Self::Decode(msg.into())
    }

    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    pub fn runtime(msg: impl Into<String>) -> Self {
        Self::Runtime(msg.into())
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        // reqwest 顶层的 Display 只说"error sending request for url"，真正的原因
        // （连接被拒、DNS 失败、TLS 握手失败、连接被对端提前关闭）在 source 链里。
        // 排查本地推理服务时那一层才是关键信息，所以整条链都拼进来。
        let mut msg = err.to_string();
        let mut source = std::error::Error::source(&err);
        while let Some(cause) = source {
            let text = cause.to_string();
            // 有些层的 Display 会把下层原文再重复一遍，避免拼出重复内容
            if !msg.contains(&text) {
                msg.push('：');
                msg.push_str(&text);
            }
            source = cause.source();
        }
        Self::Network(msg)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Self::Decode(err.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

impl From<keyring::Error> for Error {
    fn from(err: keyring::Error) -> Self {
        Self::Credential(err.to_string())
    }
}

/// Tauri command 的返回值需要 Serialize；序列化成人类可读的字符串，
/// 前端拿到的直接就是可以显示的错误文案。
impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
