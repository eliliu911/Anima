//! SQLite 存储：会话、消息、以及性格/情感状态。
//!
//! 分工（ARCHITECTURE.md 3.4 的双轨制）：
//!   - **这里**存结构化、需要精确查询的东西：会话元数据、逐条消息、情感数值
//!   - LanceDB 存需要**语义检索**的长期记忆事实（见 [`super::vector`]）
//!
//! 按坑点 4 开 WAL 模式。写入频率本身很低（每轮对话两条消息，不是每个 token
//! 一次），所以不需要额外的批量攒写层 —— 真正要避免的是「每个 token 触发一次
//! 磁盘 IO」，而管线里根本不存在那条路径。
//!
//! rusqlite 是同步 API，所有操作都包在 `spawn_blocking` 里，避免堵住 tokio。

use std::path::Path;
use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::{Error, Result};
use crate::llm::Role;

/// 一条落库的消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: String,
    /// Unix 秒
    pub created_at: i64,
    /// 这一轮的工具调用与结果，JSON。**没有它，重启后生成的图就没了。**
    ///
    /// 图片卡是从工具结果渲染的，而这张表长期只存 `content`（模型说的话）。
    /// 于是重启回到对话里，图片凭空消失 —— 文件其实一直在磁盘上，
    /// 丢的只是记录里对它的引用。本机 79 条 assistant 消息里只有 2 条
    /// 还看得见图，那 2 条是模型碰巧自己在正文里回显了链接，是运气不是设计。
    #[serde(default)]
    pub tool_json: Option<String>,
}

/// 一个提交出去、还没收回结果的 ComfyUI 任务。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComfyJob {
    pub prompt_id: String,
    pub session_id: String,
    /// `image` / `video`，决定回收时怎么渲染
    pub kind: String,
    /// 出图节点 id，回收结果时要从 `/history` 的这个键下面取
    pub save_node: String,
    /// 给用户看的一句话，例如"生成图片：图书馆里的少女"
    pub summary: String,
    /// **产物该落到哪个文件。** 空 = 没有固定归属（对话里那条路就是这样，
    /// 结果进 `generated/` 再贴进消息）。分镜和创作台一定要给 ——
    /// 不给的话，收回来的文件谁也不知道该放回哪一镜。
    pub dest: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub character_id: Option<String>,
    pub workspace_path: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub title: Option<String>,
    pub project_id: Option<String>,
    pub archived: bool,
}

/// 情感轴。ARCHITECTURE.md 3.3 的性格养成系统用它驱动 System Prompt 与微表情。
///
/// 阶段三才会真正开始更新这些数值，表结构先立在这里，免得到时再改库。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EmotionState {
    /// 好感度 0-100
    pub affection: f32,
    /// 心情 -100（沮丧）..100（兴奋）
    pub mood: f32,
    /// 活跃度 0-100
    pub energy: f32,
}

impl Default for EmotionState {
    fn default() -> Self {
        // 中性起点：不冷不热，情绪平稳
        Self {
            affection: 50.0,
            mood: 0.0,
            energy: 60.0,
        }
    }
}

/// Clone 得到的是**同一个连接**的句柄（内部 Arc 共享），
/// 不是新开一个连接 —— 后台提取任务需要持有它，但不该各自开连接。
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    /// 打开（或创建）数据库并建表。
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path).map_err(sqlite_err)?;
        Self::configure(&conn)?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 内存库，仅测试用
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(sqlite_err)?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn configure(conn: &Connection) -> Result<()> {
        // WAL：读写不互相阻塞。桌面单进程场景下这是最划算的一档
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(sqlite_err)?;
        // NORMAL 而非 FULL：崩溃时最多丢最后一个事务，换来显著更少的 fsync。
        // 对话记录丢最后一条是可接受的，卡顿不可接受。
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(sqlite_err)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(sqlite_err)?;
        Ok(())
    }

    fn migrate(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS projects (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                description TEXT,
                system_prompt TEXT,
                character_id TEXT,
                workspace_path TEXT,
                created_at  INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS sessions (
                id          TEXT PRIMARY KEY,
                started_at  INTEGER NOT NULL,
                ended_at    INTEGER,
                title       TEXT,
                project_id  TEXT REFERENCES projects(id),
                archived    INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS messages (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role        TEXT NOT NULL,
                content     TEXT NOT NULL,
                created_at  INTEGER NOT NULL
            );

            -- 按会话取最近若干条是最高频的查询
            CREATE INDEX IF NOT EXISTS idx_messages_session
                ON messages(session_id, id);

            -- 已提取过事实的消息，避免重复提取（提取要花一次模型调用）
            CREATE TABLE IF NOT EXISTS extracted (
                message_id  INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
                extracted_at INTEGER NOT NULL,
                fact_count  INTEGER NOT NULL
            );

            -- 情感状态，**按作用域分行**。
            --
            -- scope = 'global'：全局那一份。好感度只存在这里。
            -- scope = 'skill:<id>'：某个角色的心情与活跃度。
            --
            -- 为什么这样分（见 docs/PERSONALITY_MEMORY_SKILLS.md 3.2）：
            -- 好感度本质是"我们之间的关系"，不该因为换了身份而归零；
            -- 而"刚处理完一个急诊"和"刚聊完游戏"的即时状态本来就不同。
            CREATE TABLE IF NOT EXISTS emotion_scoped (
                scope      TEXT PRIMARY KEY,
                affection  REAL NOT NULL,
                mood       REAL NOT NULL,
                energy     REAL NOT NULL,
                updated_at INTEGER NOT NULL
            );
            "#,
        )
        .map_err(sqlite_err)?;

        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN project_id TEXT REFERENCES projects(id)",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN archived INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute("ALTER TABLE projects ADD COLUMN character_id TEXT", []);
        let _ = conn.execute("ALTER TABLE projects ADD COLUMN workspace_path TEXT", []);

        // 工具调用/结果。旧行为 NULL，还原时当"没有工具"处理，不影响老对话。
        let _ = conn.execute("ALTER TABLE messages ADD COLUMN tool_json TEXT", []);

        // ── 提交给 ComfyUI 但还没收回结果的任务 ──
        //
        // `prompt_id` 以前只活在内存里。关掉 Anima 再打开，ComfyUI 那边任务
        // 还在跑，而这边完全不知道它存在 —— 于是用户再说一次，就又排一个新的，
        // 而先跑完的那个结果永远回不来。
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS comfy_jobs (
                prompt_id   TEXT PRIMARY KEY,
                session_id  TEXT NOT NULL,
                kind        TEXT NOT NULL,
                save_node   TEXT NOT NULL,
                summary     TEXT NOT NULL,
                created_at  INTEGER NOT NULL,
                status      TEXT NOT NULL DEFAULT 'running'
            );
            CREATE INDEX IF NOT EXISTS idx_comfy_jobs_status ON comfy_jobs(status);
            "#,
        )
        .map_err(sqlite_err)?;

        // **产物该放哪。** 没有它的时候，回收唯一会的交付动作是「往聊天会话里
        // 追加一条消息」—— 而分镜/创作台没有会话，`session_id` 是空串，
        // `append_message` 直接撞外键。真出过事：两段 r2v 视频在 ComfyUI 那边
        // 出好了、`download_outputs` 也下载成功了，最后一米死在这儿，
        // 而且每次开机重试都失败同一次，任务永远销不掉。
        let _ = conn.execute("ALTER TABLE comfy_jobs ADD COLUMN dest TEXT", []);

        // ── 把旧的单行 emotion 表搬到 emotion_scoped 的 global 行 ──
        //
        // 老表是 `CHECK (id = 1)` 的单行表，SQLite 删不掉 CHECK 约束，所以只能
        // 建新表再搬。**不能跳过这一步**：那一行里是用户攒了很久的好感度，
        // 丢了等于把关系清零 —— 而那正是分档设计要避免的事。
        //
        // 只在新表还是空的时候搬，避免重复运行时覆盖已有数据。
        let has_old: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='emotion'",
                [],
                |_| Ok(true),
            )
            .optional()
            .map_err(sqlite_err)?
            .unwrap_or(false);
        if has_old {
            let new_empty: i64 = conn
                .query_row("SELECT COUNT(*) FROM emotion_scoped", [], |r| r.get(0))
                .map_err(sqlite_err)?;
            if new_empty == 0 {
                let moved = conn
                    .execute(
                        "INSERT INTO emotion_scoped (scope, affection, mood, energy, updated_at)
                         SELECT 'global', affection, mood, energy, updated_at FROM emotion WHERE id = 1",
                        [],
                    )
                    .map_err(sqlite_err)?;
                if moved > 0 {
                    tracing::info!("已把旧的情感状态迁移到 global 作用域");
                }
            }
            // 老表留着不删：迁移只跑一次，而留着它意味着出问题时还能对账。
            // 一行数据的空间成本可以忽略
        }
        Ok(())
    }

    /// 开一个新会话，返回 id
    pub async fn create_session(&self) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let conn = Arc::clone(&self.conn);
        let session_id = id.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO sessions (id, started_at) VALUES (?1, ?2)",
                params![session_id, now()],
            )
            .map_err(sqlite_err)?;
            Ok(())
        })
        .await
        .map_err(join_err)??;

        Ok(id)
    }

    /// 追加一条消息，返回它的 id
    pub async fn append_message(&self, session_id: &str, role: Role, content: &str) -> Result<i64> {
        self.append_message_with_tools(session_id, role, content, None)
            .await
    }

    /// 追加一条消息，连同这一轮的工具调用/结果一起落库。
    pub async fn append_message_with_tools(
        &self,
        session_id: &str,
        role: Role,
        content: &str,
        tool_json: Option<&str>,
    ) -> Result<i64> {
        let tool_json = tool_json.map(|s| s.to_string());
        let conn = Arc::clone(&self.conn);
        let session_id = session_id.to_string();
        let role = role_to_str(role).to_string();
        let content = content.to_string();

        // 第一句用户消息顺手当标题。没有标题的话历史列表里每一条都叫"新对话"，
        // 等于没有列表 —— 本机 122 条会话里 0 条有标题
        let is_user = role == role_to_str(Role::User);
        let first_line = content.clone();
        let for_title = session_id.clone();

        let row = tokio::task::spawn_blocking(move || -> Result<i64> {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO messages (session_id, role, content, created_at, tool_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![session_id, role, content, now(), tool_json],
            )
            .map_err(sqlite_err)?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .map_err(join_err)?;

        if is_user {
            // 失败不能影响消息本身已经落库这件事
            if let Err(err) = self.title_from_first_message(&for_title, &first_line).await {
                tracing::warn!(%err, "写会话标题失败");
            }
        }
        row
    }

    /// 取某个会话最近 `limit` 条消息，按时间正序返回。
    pub async fn recent_messages(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<StoredMessage>> {
        let conn = Arc::clone(&self.conn);
        let session_id = session_id.to_string();

        tokio::task::spawn_blocking(move || -> Result<Vec<StoredMessage>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn
                .prepare(
                    "SELECT id, session_id, role, content, created_at
                     FROM messages WHERE session_id = ?1
                     ORDER BY id DESC LIMIT ?2",
                )
                .map_err(sqlite_err)?;

            let rows = stmt
                .query_map(params![session_id, limit as i64], |row| {
                    Ok(StoredMessage {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        role: row.get(2)?,
                        content: row.get(3)?,
                        created_at: row.get(4)?,
                        // 这条查询喂的是模型上下文，工具结果另有预算分档处理
                        tool_json: None,
                    })
                })
                .map_err(sqlite_err)?;

            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(sqlite_err)?);
            }
            // 查询是倒序取的（为了用上索引），返回前翻正
            out.reverse();
            Ok(out)
        })
        .await
        .map_err(join_err)?
    }

    /// 标记某条消息已提取过事实
    pub async fn mark_extracted(&self, message_id: i64, fact_count: usize) -> Result<()> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT OR REPLACE INTO extracted (message_id, extracted_at, fact_count)
                 VALUES (?1, ?2, ?3)",
                params![message_id, now(), fact_count as i64],
            )
            .map_err(sqlite_err)?;
            Ok(())
        })
        .await
        .map_err(join_err)?
    }

    pub async fn is_extracted(&self, message_id: i64) -> Result<bool> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || -> Result<bool> {
            let conn = conn.blocking_lock();
            let found: Option<i64> = conn
                .query_row(
                    "SELECT message_id FROM extracted WHERE message_id = ?1",
                    params![message_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(sqlite_err)?;
            Ok(found.is_some())
        })
        .await
        .map_err(join_err)?
    }

    /// 全局作用域的名字。好感度只存在这一行
    pub const GLOBAL_SCOPE: &'static str = "global";

    /// 某个技能的作用域名。None = 没扮演角色，就用全局那一份
    pub fn skill_scope(skill_id: Option<&str>) -> String {
        match skill_id {
            Some(id) if !id.trim().is_empty() => format!("skill:{}", id.trim()),
            _ => Self::GLOBAL_SCOPE.to_string(),
        }
    }

    /// 读某个作用域的原始行。没有就返回 None
    async fn raw_row(&self, scope: String) -> Result<Option<(EmotionState, i64)>> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || -> Result<Option<(EmotionState, i64)>> {
            let conn = conn.blocking_lock();
            conn.query_row(
                "SELECT affection, mood, energy, updated_at FROM emotion_scoped WHERE scope = ?1",
                params![scope],
                |row| {
                    Ok((
                        EmotionState {
                            affection: row.get(0)?,
                            mood: row.get(1)?,
                            energy: row.get(2)?,
                        },
                        row.get(3)?,
                    ))
                },
            )
            .optional()
            .map_err(sqlite_err)
        })
        .await
        .map_err(join_err)?
    }

    /// 读情感轴：**好感度取全局，心情与活跃度取给定作用域**。
    ///
    /// 这个组合是分档设计的全部内容（见 PERSONALITY_MEMORY_SKILLS 3.2）：
    /// 换角色时关系连续（养成感不断），但各角色有各自的脾气（角色感成立）。
    ///
    /// 某个技能第一次被用到时它还没有行 —— 那时**继承全局的心情与活跃度**，
    /// 而不是从中性重新开始：切个角色就情绪突然归零会显得很怪。
    pub async fn emotion_for(&self, scope: &str) -> Result<EmotionState> {
        let global = self.raw_row(Self::GLOBAL_SCOPE.to_string()).await?;
        let affection = global
            .as_ref()
            .map(|(s, _)| s.affection)
            .unwrap_or(EmotionState::default().affection);

        if scope == Self::GLOBAL_SCOPE {
            let mut state = global.map(|(s, _)| s).unwrap_or_default();
            state.affection = affection;
            return Ok(state);
        }

        let own = self.raw_row(scope.to_string()).await?;
        let (mood, energy) = match own {
            Some((s, _)) => (s.mood, s.energy),
            // 第一次用这个技能：继承全局的即时状态
            None => match &global {
                Some((s, _)) => (s.mood, s.energy),
                None => {
                    let d = EmotionState::default();
                    (d.mood, d.energy)
                }
            },
        };
        Ok(EmotionState {
            affection,
            mood,
            energy,
        })
    }

    /// 读全局那一份。等价于 `emotion_for(GLOBAL_SCOPE)`
    pub async fn emotion(&self) -> Result<EmotionState> {
        self.emotion_for(Self::GLOBAL_SCOPE).await
    }

    /// 读情感轴，同时返回**距上次更新过了多少小时**。
    ///
    /// 与 [`Self::emotion`] 分开而不是替换它：调用方大多只关心当前值，
    /// 而空闲衰减只有会话启动那一次需要知道时间差。
    pub async fn emotion_with_age(&self) -> Result<(EmotionState, f32)> {
        self.emotion_with_age_for(Self::GLOBAL_SCOPE).await
    }

    /// 按作用域读，并给出该作用域距上次更新的小时数。
    ///
    /// 用**这个作用域自己的**时间戳而不是全局的：一个一周没用的角色，
    /// 它的心情就该衰减一周，而不是跟着别的角色的活跃度走。
    pub async fn emotion_with_age_for(&self, scope: &str) -> Result<(EmotionState, f32)> {
        let state = self.emotion_for(scope).await?;
        let own = self.raw_row(scope.to_string()).await?;
        // 这个作用域还没有行时退回全局的时间戳 —— 那时它继承的正是全局状态
        let updated_at = match own {
            Some((_, at)) => at,
            None => self
                .raw_row(Self::GLOBAL_SCOPE.to_string())
                .await?
                .map(|(_, at)| at)
                .unwrap_or(0),
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        // updated_at 为 0 表示从没写过（首次运行），此时不该按"1970 年至今"衰减
        let hours = if updated_at <= 0 {
            0.0
        } else {
            ((now - updated_at).max(0) as f32) / 3600.0
        };
        Ok((state, hours))
    }

    pub async fn set_emotion(&self, state: EmotionState) -> Result<()> {
        self.set_emotion_for(Self::GLOBAL_SCOPE, state).await
    }

    /// 写情感轴：**好感度总是写进 global，心情与活跃度写进给定作用域**。
    ///
    /// 这一条是分档的关键。如果好感度也按技能存，那"关系"就会随身份分裂 ——
    /// 而它本该是跨角色累积的唯一一条轴。
    pub async fn set_emotion_for(&self, scope: &str, state: EmotionState) -> Result<()> {
        let conn = Arc::clone(&self.conn);
        let scope = scope.to_string();
        let is_global = scope == Self::GLOBAL_SCOPE;
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            let ts = now();

            // 好感度：只更新 global 那一行的这一列，其余列保持不动。
            // 用 UPSERT 而不是 INSERT OR REPLACE —— 后者会把 global 行里的
            // 心情与活跃度一起覆盖掉，那是默认状态下的即时情绪，不该被技能里的
            // 一轮对话改写
            conn.execute(
                "INSERT INTO emotion_scoped (scope, affection, mood, energy, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(scope) DO UPDATE SET affection = ?2, updated_at = ?5",
                params![
                    Store::GLOBAL_SCOPE,
                    state.affection,
                    state.mood,
                    state.energy,
                    ts
                ],
            )
            .map_err(sqlite_err)?;

            if !is_global {
                // 技能行：心情与活跃度是它自己的；affection 一列也写进去，
                // 但读的时候一律取 global 那一份，所以它只是个副本
                conn.execute(
                    "INSERT OR REPLACE INTO emotion_scoped
                     (scope, affection, mood, energy, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![scope, state.affection, state.mood, state.energy, ts],
                )
                .map_err(sqlite_err)?;
            } else {
                // 全局作用域：心情与活跃度也要真的落到 global 行
                conn.execute(
                    "UPDATE emotion_scoped SET mood = ?2, energy = ?3, updated_at = ?4
                     WHERE scope = ?1",
                    params![Store::GLOBAL_SCOPE, state.mood, state.energy, ts],
                )
                .map_err(sqlite_err)?;
            }
            Ok(())
        })
        .await
        .map_err(join_err)?
    }

    /// 统计信息，给设置面板显示「记了多少东西」
    pub async fn counts(&self) -> Result<(usize, usize)> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || -> Result<(usize, usize)> {
            let conn = conn.blocking_lock();
            let sessions: i64 = conn
                .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
                .map_err(sqlite_err)?;
            let messages: i64 = conn
                .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
                .map_err(sqlite_err)?;
            Ok((sessions as usize, messages as usize))
        })
        .await
        .map_err(join_err)?
    }

    // --- Projects CRUD ---
    // ── 下面这一组是项目/会话的 CRUD ──
    //
    // **必须是 async 且用 `lock().await`。** 原先写的是同步 `pub fn` +
    // `conn.blocking_lock()`，而调用方是 async 的 Tauri command ——
    // tokio 的 `blocking_lock` 在运行时里会直接 panic
    // （"Cannot block the current thread from within a runtime"）。
    //
    // panic 发生在 command 里，表现是 invoke 被拒、前端 catch 到一个错，
    // 而那个错只进了 console —— 用户看到的是"点了完全没反应"。
    //
    // 这个文件里别处的 blocking_lock 都在 `spawn_blocking` 的闭包里，
    // 那是对的：那是专门的阻塞线程，不是异步运行时。
    pub async fn get_projects(&self) -> Result<Vec<Project>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare("SELECT id, name, description, system_prompt, character_id, workspace_path, created_at FROM projects ORDER BY created_at DESC")
            .map_err(sqlite_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    system_prompt: row.get(3)?,
                    character_id: row.get(4)?,
                    workspace_path: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(sqlite_err)?;

        let mut projects = Vec::new();
        for p in rows.flatten() {
            projects.push(p);
        }
        Ok(projects)
    }

    pub async fn create_project(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
        system_prompt: Option<&str>,
        character_id: Option<&str>,
        workspace_path: Option<&str>,
        created_at: i64,
    ) -> Result<()> {
        self.conn.lock().await.execute(
            "INSERT INTO projects (id, name, description, system_prompt, character_id, workspace_path, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, name, description, system_prompt, character_id, workspace_path, created_at]
        ).map_err(sqlite_err)?;
        Ok(())
    }

    pub async fn update_project(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
        system_prompt: Option<&str>,
        character_id: Option<&str>,
        workspace_path: Option<&str>,
    ) -> Result<()> {
        self.conn.lock().await.execute(
            "UPDATE projects SET name = ?1, description = ?2, system_prompt = ?3, character_id = ?4, workspace_path = ?5 WHERE id = ?6",
            rusqlite::params![name, description, system_prompt, character_id, workspace_path, id]
        ).map_err(sqlite_err)?;
        Ok(())
    }

    pub async fn delete_project(&self, id: &str) -> Result<()> {
        self.conn
            .lock()
            .await
            .execute("DELETE FROM projects WHERE id = ?1", rusqlite::params![id])
            .map_err(sqlite_err)?;
        Ok(())
    }

    // --- Sessions CRUD ---
    /// 删掉一条消息都没有的会话，返回删了几条。
    ///
    /// 空会话里没有任何信息 —— 它唯一的作用是把历史列表撑成一堵
    /// 一模一样的"新对话"墙。
    pub async fn purge_empty_sessions(&self) -> Result<usize> {
        let conn = std::sync::Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || -> Result<usize> {
            let conn = conn.blocking_lock();
            let n = conn
                .execute(
                    "DELETE FROM sessions WHERE NOT EXISTS \
                     (SELECT 1 FROM messages WHERE messages.session_id = sessions.id)",
                    [],
                )
                .map_err(sqlite_err)?;
            Ok(n)
        })
        .await
        .map_err(join_err)
        .and_then(|r| r)
        .inspect(|n| {
            if *n > 0 {
                tracing::info!(count = n, "扫掉空会话");
            }
        })
    }

    fn extract_clean_user_prompt(content: &str) -> String {
        let trimmed = content.trim();
        if trimmed.starts_with("[上传图片]") || trimmed.starts_with("[附件文档") {
            let mut non_bracket_lines = Vec::new();
            let mut in_code_block = false;
            for line in trimmed.lines() {
                let l = line.trim();
                if l.starts_with("```") {
                    in_code_block = !in_code_block;
                    continue;
                }
                if in_code_block {
                    continue;
                }
                if l.starts_with("[上传图片]")
                    || l.starts_with("以下图片已经保存")
                    || l.starts_with("- Image ")
                    || l.starts_with("file_url:")
                    || l.starts_with("local_path:")
                    || l.starts_with("[注意：")
                    || l.starts_with("[附件文档")
                {
                    continue;
                }
                if !l.is_empty() {
                    non_bracket_lines.push(l);
                }
            }
            if !non_bracket_lines.is_empty() {
                return non_bracket_lines.join(" ");
            }
            return "上传图片".to_string();
        }
        trimmed.to_string()
    }

    /// 没有标题的会话，拿第一句用户消息当标题。
    ///
    /// 不叫模型：一句话的开头已经足够把它和别的会话区分开，而这是**同步、免费、
    /// 不会失败**的。真要更好的摘要标题可以之后再叠一层后台模型，
    /// 但那不该是"列表能不能读"的前提。
    async fn title_from_first_message(&self, session_id: &str, content: &str) -> Result<()> {
        const MAX_CHARS: usize = 18;
        let clean = Self::extract_clean_user_prompt(content);
        let mut title: String = clean.trim().chars().take(MAX_CHARS).collect();
        if title.is_empty() {
            return Ok(());
        }
        if clean.trim().chars().count() > MAX_CHARS {
            title.push('…');
        }
        let conn = std::sync::Arc::clone(&self.conn);
        let id = session_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            // 只在还没有标题时写 —— 后面改过名的不能被覆盖
            conn.execute(
                "UPDATE sessions SET title = ?1 WHERE id = ?2 AND (title IS NULL OR title = '')",
                rusqlite::params![title, id],
            )
            .map_err(sqlite_err)?;
            Ok(())
        })
        .await
        .map_err(join_err)?
    }

    pub async fn create_session_for_project(&self, project_id: Option<&str>) -> Result<String> {
        // 开新会话前先扫掉空壳。
        //
        // 每次开语音都会建一行，**哪怕一个字都没说**；点"新对话"也一样。
        // 攒下来的结果是历史列表里一长串一模一样的"新对话"，
        // 本机实测 122 条里有 48 条是这种（一条消息都没有）。
        //
        // 在这里扫而不是定时扫：这一刻正好是"上一个会话已经不会再被写入"的时点。
        if let Err(err) = self.purge_empty_sessions().await {
            // 清理失败不能挡住开新会话
            tracing::warn!(%err, "清理空会话失败");
        }
        let id = uuid::Uuid::new_v4().to_string();
        let conn = std::sync::Arc::clone(&self.conn);
        let session_id = id.clone();
        let pid = project_id.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO sessions (id, started_at, project_id) VALUES (?1, ?2, ?3)",
                rusqlite::params![session_id, now(), pid],
            )
            .map_err(sqlite_err)?;
            Ok(())
        })
        .await
        .map_err(join_err)??;

        // **谁建的会话要能查。** "每说一句话就多一个新对话"这类报告，
        // 没有这行日志就只能靠猜 —— 会话是在 Rust 侧凭空出现的
        tracing::info!(session = %id, project = ?project_id, "新建会话");

        Ok(id)
    }

    pub async fn get_sessions(
        &self,
        project_id: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<SessionInfo>> {
        let conn = self.conn.lock().await;
        let query = match (project_id, include_archived) {
            (Some(_), true) => "SELECT id, started_at, ended_at, title, project_id, archived FROM sessions WHERE project_id = ?1 ORDER BY started_at DESC",
            (Some(_), false) => "SELECT id, started_at, ended_at, title, project_id, archived FROM sessions WHERE project_id = ?1 AND archived = 0 ORDER BY started_at DESC",
            (None, true) => "SELECT id, started_at, ended_at, title, project_id, archived FROM sessions WHERE project_id IS NULL ORDER BY started_at DESC",
            (None, false) => "SELECT id, started_at, ended_at, title, project_id, archived FROM sessions WHERE project_id IS NULL AND archived = 0 ORDER BY started_at DESC",
        };

        let mut stmt = conn.prepare(query).map_err(sqlite_err)?;

        let mut sessions = Vec::new();
        if let Some(pid) = project_id {
            let rows = stmt
                .query_map(rusqlite::params![pid], |row| {
                    Ok(SessionInfo {
                        id: row.get(0)?,
                        started_at: row.get(1)?,
                        ended_at: row.get(2)?,
                        title: row.get(3)?,
                        project_id: row.get(4)?,
                        archived: row.get::<_, i32>(5)? != 0,
                    })
                })
                .map_err(sqlite_err)?;
            for s in rows.flatten() {
                sessions.push(s);
            }
        } else {
            let rows = stmt
                .query_map([], |row| {
                    Ok(SessionInfo {
                        id: row.get(0)?,
                        started_at: row.get(1)?,
                        ended_at: row.get(2)?,
                        title: row.get(3)?,
                        project_id: row.get(4)?,
                        archived: row.get::<_, i32>(5)? != 0,
                    })
                })
                .map_err(sqlite_err)?;
            for s in rows.flatten() {
                sessions.push(s);
            }
        }

        Ok(sessions)
    }

    /// 全部会话，不分项目。
    ///
    /// `get_sessions(None, ..)` 的语义是"**没挂项目**的那些"，不是"全部" ——
    /// 侧栏的「对话」分区要的是后者（跟各家 AI 助手一致：项目是分组，
    /// 而对话列表本身是全量的最近对话）。两个语义混用会让人以为对话丢了。
    pub async fn get_all_sessions(&self, include_archived: bool) -> Result<Vec<SessionInfo>> {
        let conn = self.conn.lock().await;
        let query = if include_archived {
            "SELECT id, started_at, ended_at, title, project_id, archived FROM sessions ORDER BY started_at DESC"
        } else {
            "SELECT id, started_at, ended_at, title, project_id, archived FROM sessions WHERE archived = 0 ORDER BY started_at DESC"
        };
        let mut stmt = conn.prepare(query).map_err(sqlite_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SessionInfo {
                    id: row.get(0)?,
                    started_at: row.get(1)?,
                    ended_at: row.get(2)?,
                    title: row.get(3)?,
                    project_id: row.get(4)?,
                    archived: row.get::<_, i32>(5)? != 0,
                })
            })
            .map_err(sqlite_err)?;
        // 这里**不吞行**：反序列化失败要报出来。静默丢行的表现是
        // "历史列表少了几条"，而且没有任何线索
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(sqlite_err)?);
        }
        Ok(out)
    }

    /// 记下一个刚提交给 ComfyUI、还没收回结果的任务。
    pub async fn record_comfy_job(
        &self,
        prompt_id: &str,
        session_id: &str,
        kind: &str,
        save_node: &str,
        summary: &str,
        dest: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO comfy_jobs
             (prompt_id, session_id, kind, save_node, summary, dest, created_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running')",
            params![prompt_id, session_id, kind, save_node, summary, dest, now()],
        )
        .map_err(sqlite_err)?;
        Ok(())
    }

    /// 任务已经收回结果（或确认失败），不再需要开机回收。
    pub async fn finish_comfy_job(&self, prompt_id: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE comfy_jobs SET status = 'done' WHERE prompt_id = ?1",
            params![prompt_id],
        )
        .map_err(sqlite_err)?;
        Ok(())
    }

    /// 还挂着的任务，开机时用来和 ComfyUI 对账。
    pub async fn pending_comfy_jobs(&self) -> Result<Vec<ComfyJob>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT prompt_id, session_id, kind, save_node, summary, dest, created_at
                 FROM comfy_jobs WHERE status = 'running' ORDER BY created_at ASC",
            )
            .map_err(sqlite_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ComfyJob {
                    prompt_id: row.get(0)?,
                    session_id: row.get(1)?,
                    kind: row.get(2)?,
                    save_node: row.get(3)?,
                    summary: row.get(4)?,
                    dest: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(sqlite_err)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(sqlite_err)?);
        }
        Ok(out)
    }

    pub async fn get_session_messages(&self, session_id: &str) -> Result<Vec<StoredMessage>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, created_at, tool_json FROM messages WHERE session_id = ?1 ORDER BY id ASC"
        ).map_err(sqlite_err)?;
        let rows = stmt
            .query_map(rusqlite::params![session_id], |row| {
                Ok(StoredMessage {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    created_at: row.get(4)?,
                    tool_json: row.get(5)?,
                })
            })
            .map_err(sqlite_err)?;

        let mut msgs = Vec::new();
        for m in rows.flatten() {
            msgs.push(m);
        }
        Ok(msgs)
    }

    pub async fn delete_session_message(&self, session_id: &str, message_id: i64) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "DELETE FROM messages WHERE session_id = ?1 AND id = ?2",
            rusqlite::params![session_id, message_id],
        )
        .map_err(sqlite_err)?;
        Ok(())
    }

    pub async fn rollback_session_to_message(
        &self,
        session_id: &str,
        message_id: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "DELETE FROM messages WHERE session_id = ?1 AND id >= ?2",
            rusqlite::params![session_id, message_id],
        )
        .map_err(sqlite_err)?;
        Ok(())
    }

    pub async fn update_session_title(&self, id: &str, title: &str) -> Result<()> {
        self.conn
            .lock()
            .await
            .execute(
                "UPDATE sessions SET title = ?1 WHERE id = ?2",
                rusqlite::params![title, id],
            )
            .map_err(sqlite_err)?;
        Ok(())
    }

    pub async fn archive_session(&self, id: &str) -> Result<()> {
        self.conn
            .lock()
            .await
            .execute(
                "UPDATE sessions SET archived = 1 WHERE id = ?1",
                rusqlite::params![id],
            )
            .map_err(sqlite_err)?;
        Ok(())
    }

    pub async fn delete_session(&self, id: &str) -> Result<()> {
        tracing::info!(session = %id, "删除会话");
        self.conn
            .lock()
            .await
            .execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![id])
            .map_err(sqlite_err)?;
        Ok(())
    }
}

fn role_to_str(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn sqlite_err(err: rusqlite::Error) -> Error {
    Error::Io(format!("SQLite 错误：{err}"))
}

fn join_err(err: tokio::task::JoinError) -> Error {
    Error::Io(format!("数据库任务异常退出：{err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> Store {
        Store::open_in_memory().expect("建库失败")
    }

    /// **老库能升级。** 这一列是后加的，真机上那个库里已经攒了几十条消息。
    ///
    /// 建一个没有 `tool_json` 的旧表，再用 `Store::open` 打开：列要被补上，
    /// 老消息要照常读得出来（这一列为 NULL）。加列的那句写的是 `let _ = `，
    /// 失败会被吞掉 —— 所以必须有测试真的走一遍，否则老用户升上来
    /// 每次读消息都报错，而错误在别处才浮出来。
    #[tokio::test]
    async fn 老库补得上这一列且老消息照常读() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE messages (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     session_id TEXT NOT NULL,
                     role TEXT NOT NULL,
                     content TEXT NOT NULL,
                     created_at INTEGER NOT NULL);
                 INSERT INTO messages (session_id, role, content, created_at)
                 VALUES ('s-old', 'assistant', '升级前就存在的一条', 1);",
            )
            .unwrap();
        }

        let store = Store::open(&path).expect("打开老库失败");
        let msgs = store.get_session_messages("s-old").await.unwrap();
        assert_eq!(msgs.len(), 1, "老消息不能在升级中丢");
        assert_eq!(msgs[0].content, "升级前就存在的一条");
        assert!(msgs[0].tool_json.is_none());

        // 新写入的带工具，和老行共存
        store
            .append_message_with_tools("s-old", Role::Assistant, "新的", Some("{}"))
            .await
            .unwrap();
        let msgs = store.get_session_messages("s-old").await.unwrap();
        assert_eq!(msgs[1].tool_json.as_deref(), Some("{}"));
    }

    /// **重启后图片消失**的库这一层。
    ///
    /// 这张表长期只有 `role`/`content`，工具调用与结果一个字没存 ——
    /// 而图片卡是从工具结果渲染的。真机上 79 条 assistant 消息里只有 2 条
    /// 还看得见图，那 2 条是模型碰巧自己在正文里回显了链接。
    #[tokio::test]
    async fn 工具结果跟着消息一起存下来() {
        let store = store().await;
        let s1 = store.create_session().await.unwrap();
        let payload = r#"{"toolCalls":[{"id":"c1","name":"generate_image"}]}"#;
        store
            .append_message_with_tools(&s1, Role::Assistant, "给你生成好了", Some(payload))
            .await
            .unwrap();

        let msgs = store.get_session_messages(&s1).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].tool_json.as_deref(), Some(payload));
    }

    #[tokio::test]
    async fn 没有工具的消息这一列是空的() {
        let store = store().await;
        let s1 = store.create_session().await.unwrap();
        store.append_message(&s1, Role::User, "你好").await.unwrap();
        assert!(store.get_session_messages(&s1).await.unwrap()[0]
            .tool_json
            .is_none());
    }

    /// ComfyUI 挂账：记上 → 查得到 → 销账后不再出现。
    ///
    /// `prompt_id` 以前只活在内存里，Anima 一关就没了，而 ComfyUI 那边还在跑。
    /// 用户回到对话什么都看不到，只好再说一次 —— 又排一个新任务。
    #[tokio::test]
    async fn 挂账的任务查得到销账后就不查了() {
        let store = store().await;
        let s1 = store.create_session().await.unwrap();
        store
            .record_comfy_job("p-1", &s1, "video", "10,9,outputs", "图书馆里的少女", None)
            .await
            .unwrap();

        let pending = store.pending_comfy_jobs().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].session_id, s1, "得记住是哪条对话，不然补不回原处");
        assert_eq!(pending[0].save_node, "10,9,outputs");
        assert_eq!(pending[0].summary, "图书馆里的少女");

        store.finish_comfy_job("p-1").await.unwrap();
        assert!(store.pending_comfy_jobs().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn 同一个任务记两次不会变成两条() {
        // 重试/回退路径可能对同一个 prompt_id 记两次，翻倍的话结果会被补两遍
        let store = store().await;
        let s1 = store.create_session().await.unwrap();
        for _ in 0..2 {
            store
                .record_comfy_job("p-1", &s1, "image", "9", "同一个", None)
                .await
                .unwrap();
        }
        assert_eq!(store.pending_comfy_jobs().await.unwrap().len(), 1);
    }

    /// 空会话（一条消息都没有）会在开新会话时被扫掉。
    ///
    /// 每开一次语音就建一行，哪怕一个字没说 —— 本机实测 122 条里 48 条是空的，
    /// 历史列表变成一堵一模一样的"新对话"墙
    #[tokio::test]
    async fn empty_sessions_are_swept_when_a_new_one_starts() {
        let store = store().await;
        let empty = store.create_session_for_project(None).await.unwrap();
        let used = store.create_session_for_project(None).await.unwrap();
        store
            .append_message(&used, Role::User, "有内容")
            .await
            .unwrap();

        // 再开一个 —— 这一刻扫
        let fresh = store.create_session_for_project(None).await.unwrap();
        let ids: Vec<String> = store
            .get_sessions(None, false)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();

        assert!(!ids.contains(&empty), "空会话没被扫掉");
        assert!(ids.contains(&used), "有内容的会话被误删了 —— 这是数据丢失");
        assert!(ids.contains(&fresh), "刚建的这个不该被自己扫掉");
    }

    /// 第一句用户消息当标题；改过名之后不能被后面的消息覆盖
    #[tokio::test]
    async fn first_user_message_becomes_the_title() {
        let store = store().await;
        let s1 = store.create_session_for_project(None).await.unwrap();
        store
            .append_message(&s1, Role::User, "帮我看看这段代码为什么跑不起来")
            .await
            .unwrap();
        store
            .append_message(&s1, Role::User, "第二句不该覆盖标题")
            .await
            .unwrap();

        let title = store.get_sessions(None, false).await.unwrap()[0]
            .title
            .clone()
            .expect("应当自动有标题");
        assert!(title.starts_with("帮我看看这段代码"), "实际：{title}");
        assert!(title.chars().count() <= 19, "标题太长会撑破侧栏：{title}");

        // 手工改名之后不该被覆盖
        store
            .update_session_title(&s1, "我自己起的名")
            .await
            .unwrap();
        store
            .append_message(&s1, Role::User, "又说了一句")
            .await
            .unwrap();
        assert_eq!(
            store.get_sessions(None, false).await.unwrap()[0]
                .title
                .as_deref(),
            Some("我自己起的名")
        );
    }

    /// 项目 → 项目下的会话 → 列表 → 归档 → 删除，一条龙。
    ///
    /// 这条盖的是 `get_sessions` 那四个查询分支：有/无 project_id ×
    /// 含/不含归档。四条 SQL 是手写的，参数绑定分两个分支，
    /// 绑错了不会编译报错，只会**少列几条会话**。
    #[tokio::test]
    async fn projects_and_sessions_round_trip() {
        let store = store().await;

        store
            .create_project("p1", "P1", None, None, None, None, 1000)
            .await
            .unwrap();
        store
            .create_project("p2", "英语老师", None, None, None, None, 200)
            .await
            .unwrap();
        assert_eq!(store.get_projects().await.unwrap().len(), 2);

        // 每个都写一条消息 —— 开新会话会顺手扫掉空会话
        // （见 empty_sessions_are_swept_when_a_new_one_starts），
        // 不写的话它们会在下一次 create 时消失
        let a = store.create_session_for_project(Some("p1")).await.unwrap();
        store.append_message(&a, Role::User, "甲").await.unwrap();
        let b = store.create_session_for_project(Some("p1")).await.unwrap();
        store.append_message(&b, Role::User, "乙").await.unwrap();
        let c = store.create_session_for_project(None).await.unwrap();
        store.append_message(&c, Role::User, "丙").await.unwrap();

        // 项目内只看得到自己的
        assert_eq!(
            store.get_sessions(Some("p1"), false).await.unwrap().len(),
            2
        );
        assert_eq!(
            store.get_sessions(Some("p2"), false).await.unwrap().len(),
            0
        );

        // 不选项目时看到的是**没挂项目**的那些 —— 而不是全部。
        // 这一条很容易写反，写反的表现是历史列表串台
        let loose = store.get_sessions(None, false).await.unwrap();
        assert_eq!(loose.len(), 1);
        assert_eq!(loose[0].id, c);

        // 归档后默认不列，include_archived 才出现
        store.archive_session(&a).await.unwrap();
        assert_eq!(
            store.get_sessions(Some("p1"), false).await.unwrap().len(),
            1
        );
        assert_eq!(store.get_sessions(Some("p1"), true).await.unwrap().len(), 2);

        // 自动标题已经写进去了，这里验的是手工改名能盖过它
        store.update_session_title(&c, "起个名字").await.unwrap();
        assert_eq!(
            store.get_sessions(None, false).await.unwrap()[0]
                .title
                .as_deref(),
            Some("起个名字")
        );

        store.delete_session(&c).await.unwrap();
        assert!(store.get_sessions(None, false).await.unwrap().is_empty());
    }

    /// 老会话（迁移前建的，project_id 为 NULL）必须还能被列出来。
    ///
    /// 本机真实库里有 124 条这样的会话 —— 列不出来等于历史全没了，
    /// 而且不会报任何错
    #[tokio::test]
    async fn sessions_from_before_projects_existed_still_list() {
        let store = store().await;
        let old = store.create_session_for_project(None).await.unwrap();
        let listed = store.get_sessions(None, false).await.unwrap();
        assert_eq!(listed.len(), 1, "迁移前建的会话消失了");
        assert_eq!(listed[0].id, old);
        assert!(listed[0].project_id.is_none());
    }

    #[tokio::test]
    async fn appends_and_reads_messages_in_chronological_order() {
        let store = store().await;
        let session = store.create_session_for_project(None).await.unwrap();

        store
            .create_project("proj-2", "Project 2", None, None, None, None, 2000)
            .await
            .unwrap();
        store
            .create_project("proj-1", "Project 1", None, None, None, None, 1000)
            .await
            .unwrap();

        store
            .append_message(&session, Role::User, "第一句")
            .await
            .unwrap();
        store
            .append_message(&session, Role::Assistant, "第二句")
            .await
            .unwrap();
        store
            .append_message(&session, Role::User, "第三句")
            .await
            .unwrap();

        let messages = store.recent_messages(&session, 10).await.unwrap();
        let contents: Vec<_> = messages.iter().map(|m| m.content.as_str()).collect();
        // 必须是时间正序，即便内部是倒序查的
        assert_eq!(contents, vec!["第一句", "第二句", "第三句"]);
    }

    #[tokio::test]
    async fn recent_messages_returns_the_latest_not_the_earliest() {
        let store = store().await;
        let session = store.create_session().await.unwrap();
        for i in 0..10 {
            store
                .append_message(&session, Role::User, &format!("msg{i}"))
                .await
                .unwrap();
        }

        let messages = store.recent_messages(&session, 3).await.unwrap();
        let contents: Vec<_> = messages.iter().map(|m| m.content.as_str()).collect();
        // 取最近 3 条，且仍按正序
        assert_eq!(contents, vec!["msg7", "msg8", "msg9"]);
    }

    #[tokio::test]
    async fn messages_are_scoped_to_their_session() {
        let store = store().await;
        let a = store.create_session().await.unwrap();
        let b = store.create_session().await.unwrap();

        store.append_message(&a, Role::User, "属于A").await.unwrap();
        store.append_message(&b, Role::User, "属于B").await.unwrap();

        let from_a = store.recent_messages(&a, 10).await.unwrap();
        assert_eq!(from_a.len(), 1);
        assert_eq!(from_a[0].content, "属于A");
    }

    #[tokio::test]
    async fn extraction_marker_is_idempotent() {
        let store = store().await;
        let session = store.create_session().await.unwrap();
        let id = store
            .append_message(&session, Role::User, "养了只猫")
            .await
            .unwrap();

        assert!(!store.is_extracted(id).await.unwrap());
        store.mark_extracted(id, 2).await.unwrap();
        assert!(store.is_extracted(id).await.unwrap());
        // 重复标记不该报错（INSERT OR REPLACE）
        store.mark_extracted(id, 3).await.unwrap();
        assert!(store.is_extracted(id).await.unwrap());
    }

    #[tokio::test]
    async fn emotion_age_is_zero_before_first_write() {
        // updated_at 为 0 表示从没写过。不特判的话会按"1970 年至今"衰减，
        // 首次运行就把状态冲到中性 —— 而它本来就是中性，看不出错，
        // 等有了非中性初值才会暴露
        let store = Store::open_in_memory().unwrap();
        let (_, hours) = store.emotion_with_age().await.unwrap();
        assert_eq!(hours, 0.0, "首次运行不该算出巨大的空闲时长");
    }

    #[tokio::test]
    async fn emotion_age_is_small_right_after_a_write() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_emotion(EmotionState {
                affection: 70.0,
                mood: 20.0,
                energy: 60.0,
            })
            .await
            .unwrap();
        let (state, hours) = store.emotion_with_age().await.unwrap();
        assert_eq!(state.affection, 70.0);
        assert!(hours < 0.01, "刚写完的空闲时长应当接近 0，实际 {hours}");
    }

    #[tokio::test]
    async fn emotion_defaults_to_neutral_then_persists() {
        let store = store().await;
        let initial = store.emotion().await.unwrap();
        assert_eq!(initial.affection, 50.0);
        assert_eq!(initial.mood, 0.0);

        store
            .set_emotion(EmotionState {
                affection: 88.0,
                mood: 42.0,
                energy: 70.0,
            })
            .await
            .unwrap();

        let loaded = store.emotion().await.unwrap();
        assert_eq!(loaded.affection, 88.0);
        assert_eq!(loaded.mood, 42.0);

        // 单行表：再写一次是更新而非插入第二行
        store
            .set_emotion(EmotionState {
                affection: 10.0,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(store.emotion().await.unwrap().affection, 10.0);
    }

    #[tokio::test]
    async fn counts_reflect_what_was_written() {
        let store = store().await;
        let session = store.create_session().await.unwrap();
        store
            .append_message(&session, Role::User, "一")
            .await
            .unwrap();
        store
            .append_message(&session, Role::User, "二")
            .await
            .unwrap();

        let (sessions, messages) = store.counts().await.unwrap();
        assert_eq!(sessions, 1);
        assert_eq!(messages, 2);
    }

    #[tokio::test]
    async fn wal_and_schema_survive_reopen() {
        let dir = std::env::temp_dir().join(format!("anima-store-{}", uuid::Uuid::new_v4()));
        let path = dir.join("memory.db");

        let session = {
            let store = Store::open(&path).expect("首次打开失败");
            let session = store.create_session().await.unwrap();
            store
                .append_message(&session, Role::User, "重开后还在吗")
                .await
                .unwrap();
            session
        };

        // 重新打开：数据和表结构都应当还在
        let store = Store::open(&path).expect("重新打开失败");
        let messages = store.recent_messages(&session, 10).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "重开后还在吗");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 提取纯净用户输入作为会话标题() {
        let raw = "[上传图片]\n以下图片已经保存为本地文件。\n- Image 1: 450x_auto.webp\n  file_url: file:///path/450x.webp\n  local_path: /path/450x.webp\n\n请帮根据这个图片生成视频";
        let clean = Store::extract_clean_user_prompt(raw);
        assert_eq!(clean, "请帮根据这个图片生成视频");
    }
}
