import { useEffect, useState, useCallback } from "react";
import { useProject } from "../store/project";
import {
  workspaceChooseFolder,
  workspaceCheckStatus,
  workspaceInit,
  workspaceSaveFiles,
  workspaceOpenInFinder,
  workspaceListKnowledgeDocs,
  workspaceDeleteKnowledgeDoc,
  workspaceGetPermissions,
  workspaceSavePermissions,
  workspaceReindexMemory,
  workspaceKnowledgeCount,
  type WorkspaceStatus,
  type KnowledgeDocInfo,
  type ProjectPermissions,
} from "../api";

const PROMPT_TEMPLATES = {
  general: (pName: string) => `# ${pName} —— AI 专属协同智能体指令规范 (Agent Instructions)

<identity>
你是项目「${pName}」的专属核心 AI 智能体与全能工作伙伴。你深刻理解本项目的工作流、架构设计与业务目标，能够调用本项目的专属工具集、本地知识库与执行沙箱，为用户提供端到端的专业协助。
</identity>

## 🎯 核心工作流与行为原则 (Core Directives)
1. **先探测后行动 (Observe Before Action)**：
   - 在进行任何修改或生成前，优先检索工作区已有文件与 .anima/knowledge/ 知识库，避免无依据凭空假设；
2. **保持架构一致性 (Architectural Integrity)**：
   - 严格遵守现有的工程规范、命名约定与设计模式，非必要不引入额外冗余依赖；
3. **最小侵入与安全可控 (Safe & Non-Intrusive)**：
   - 修改现有内容时，保持原有逻辑与注释的完整性，确保每次变更可被快速验证与回滚；
4. **多分支决策主动提供选项 (Structured Options)**：
   - 当遇到架构选型、创作分支或关键参数调优时，主动列出 2~3 个结构化方案供用户快速抉择。

## 📚 领域知识与记忆调阅 (Knowledge & Context)
- 项目本地知识文档位于 .anima/knowledge/，包含业务背景、接口定义与参考物料；
- 项目行为准则位于 .anima/rules.md，执行任务时必须严格遵从其中的约束与限制。`,

  drama: (pName: string) => `# ${pName} —— AI 影视短剧项目人设与创作指令规范

<identity>
你是短剧项目「${pName}」的【执行导演与首席金牌编剧】。你深刻理解微短剧（竖屏 9:16 与横屏 16:9）的节奏节拍、爆款爽点结构、人物张力设计与影视工业化生产流程。
</identity>

## 🎬 核心创作准则与工作流
1. **黄金 3 秒抓人法则**：
   - 每集开头 3 秒必须抛出强烈悬念、冲突或反差，迅速锁定受众注意力；
2. **反转与情绪拉扯**：
   - 每 30~45 秒设置一次情绪反转或信息增量，单集结尾必须埋下致命 Hook（留钩子）；
3. **专业电影分镜拆解**：
   - 剧本需精准转化为标准化分镜：标明【景别】（特写/中景/全景）、【机位运镜】（推镜头/摇镜头/环绕）、【音效配乐】与【ComfyUI 画面 Prompt】；
4. **角色与美术一致性**：
   - 严格维护主要角色的面容形态、服化道色彩基调与光影氛围，确保全集视觉风格高度统一。

## 📚 知识库与规则调阅
- 角色设定表与剧情大纲存放于 .anima/knowledge/，请在创作时主动参考；
- 剧情禁忌与审核红线遵循 .anima/rules.md。`,

  developer: (pName: string) => `# ${pName} —— 软件工程架构与结对编程助手指令

<identity>
你是项目「${pName}」的【Principal 级别软件架构师与高阶结对编程助手】。你精通现代软件工程范式、洁净架构（Clean Architecture）与自动化测试，致力于协助用户构建高可靠、高可维护的代码系统。
</identity>

## 🎯 核心工作原则 (Core Directives)
1. **探测优先于行动 (Observe Before Action)**：
   - 在修改代码前，先通读相关模块源码，理清依赖关系与调用链，严禁凭空假设；
2. **测试驱动与可验证性 (Test & Verify)**：
   - 每次功能迭代或 Bug 修复，必须提供或执行单元测试验证，确保改动不引入回归；
3. **保持文档与实现一致 (Documentation Integrity)**：
   - 保持现有注释与 API 文档的完整性，重大改动主动同步更新 .anima/knowledge/ 中的架构说明；
4. **决策透明与方案对比 (Structured Options)**：
   - 在面对复杂重构或方案选型时，主动提供 2~3 个结构化方案并分析优劣利弊。`,
};

const DEFAULT_RULES = `# 项目行为准则与业务约束规范 (Project Rules & Constraints)

## 1. 交互与沟通风格 (Communication Tone)
- **结论先行**：优先以精炼、结构化的语言呈现核心方案与关键结论；
- **可点击引用**：涉及项目内的具体文件、类名与函数时，使用 Markdown 格式精准标定；
- **透明反馈**：在执行多步骤操作时，清晰列出当前所处阶段与验证结果。

## 2. 安全与执行约束 (Security & Execution Safety)
- **禁止静默破坏**：未经用户明确授权，严禁静默执行破坏性文件删除（如 rm -rf）、全局代码重置或格式化覆盖；
- **沙箱隔离**：所有终端命令优先在标准安全沙箱中运行，隔离未授权的网络与外部系统访问；
- **敏感信息脱敏**：严禁在日志、提示词或输出中明文泄露 .env 密钥、Token 与私钥内容。

## 3. 产物与持久化规范 (Artifacts & File Standards)
- **统一存储**：中间临时分析产物、向量记忆与缓存索引统一存放在 .anima/ 目录下；
- **文档即代码**：保持技术文档与实际代码实现严格同步，每次重大变更主动同步更新对应的知识库说明。`;

export function ProjectSettings({
  projectId: propProjectId,
  onClose,
}: {
  projectId?: string;
  onClose?: () => void;
}) {
  const { projects, activeProjectId, updateProject, deleteProject } = useProject();

  const currentProjectId = propProjectId || activeProjectId || projects[0]?.id;
  const project = projects.find((p) => p.id === currentProjectId);

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [systemPrompt, setSystemPrompt] = useState("");
  const [characterId, setCharacterId] = useState("");
  const [workspacePath, setWorkspacePath] = useState("");

  const [isRenaming, setIsRenaming] = useState(false);
  const [workspaceStatus, setWorkspaceStatus] = useState<WorkspaceStatus | null>(null);
  const [promptFileContent, setPromptFileContent] = useState("");
  const [rulesContent, setRulesContent] = useState("");

  // 展开抽屉状态
  const [editingSection, setEditingSection] = useState<"prompt" | "rules" | null>(null);

  // 权限与安全策略实体状态
  const [permissions, setPermissions] = useState<ProjectPermissions>({
    security_preset: "inherit",
    file_access: {
      allow_reads: true,
      allow_writes: true,
      allowed_paths: [],
      denied_paths: [".git/", ".env*", "node_modules/"],
    },
    terminal_policy: {
      auto_approve_commands: ["ls", "cat", "git status", "git diff", "npm test", "cargo check"],
      blocked_commands: ["rm -rf /", "sudo", "mkfs", "dd"],
      sandbox_mode: "standard",
    },
  });

  // 3 个子功能模态弹窗状态
  const [activeModal, setActiveModal] = useState<"knowledge" | "file_access" | "terminal" | null>(null);
  const [knowledgeDocs, setKnowledgeDocs] = useState<KnowledgeDocInfo[]>([]);
  const [reindexing, setReindexing] = useState(false);
  // 已索引的块数。null = 还没查过。**这个数字必须来自向量库**，
  // 「目录里有几个文件」在索引失败时依然好看，而那正是最该被看见的时候。
  const [indexedChunks, setIndexedChunks] = useState<number | null>(null);

  // 新增白名单/黑名单输入框
  const [newAllowedPath, setNewAllowedPath] = useState("");
  const [newDeniedPath, setNewDeniedPath] = useState("");
  const [newAutoCmd, setNewAutoCmd] = useState("");
  const [newBlockedCmd, setNewBlockedCmd] = useState("");

  const [loading, setLoading] = useState(false);
  const [statusMsg, setStatusMsg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // 同步项目信息
  useEffect(() => {
    if (project) {
      setName(project.name);
      setDescription(project.description || "");
      setSystemPrompt(project.system_prompt || "");
      setCharacterId(project.character_id || "");
      setWorkspacePath(project.workspace_path || "");
    }
  }, [project]);

  // 探测工作区状态与权限策略
  const refreshWorkspace = useCallback(async () => {
    if (!workspacePath) {
      setWorkspaceStatus(null);
      return;
    }
    try {
      const st = await workspaceCheckStatus(workspacePath);
      setWorkspaceStatus(st);
      if (st.prompt_content) setPromptFileContent(st.prompt_content);
      if (st.rules_content) setRulesContent(st.rules_content);

      if (st.is_initialized) {
        const perms = await workspaceGetPermissions(workspacePath);
        setPermissions(perms);
        const docs = await workspaceListKnowledgeDocs(workspacePath);
        setKnowledgeDocs(docs);
        // 索引块数单独拉。**失败不能影响这一屏的其它内容** ——
        // 记忆系统是惰性初始化的，嵌入后端没起来时这一个调用会失败，
        // 而项目设置的其余部分和它无关。
        try {
          setIndexedChunks(await workspaceKnowledgeCount(workspacePath));
        } catch {
          setIndexedChunks(null);
        }
      }
    } catch (err) {
      console.error("检查工作区失败:", err);
    }
  }, [workspacePath]);

  useEffect(() => {
    void refreshWorkspace();
  }, [refreshWorkspace]);

  if (!project) {
    return (
      <div className="flex flex-col items-center justify-center p-12 text-center text-[var(--ink-3)]">
        <span className="text-4xl mb-3 opacity-50">📁</span>
        <div className="font-semibold text-[14px] text-[var(--ink-2)]">暂无选中的项目</div>
        <div className="text-[12px] mt-1">请在左侧新建或选择一个项目进行管理。</div>
      </div>
    );
  }

  // 1. 选择本地目录
  const handleChooseFolder = async () => {
    try {
      const folder = await workspaceChooseFolder();
      if (folder) {
        setWorkspacePath(folder);
        const st = await workspaceCheckStatus(folder);
        setWorkspaceStatus(st);
        await updateProject(
          project.id,
          name || project.name,
          description,
          systemPrompt,
          characterId,
          folder
        );
        setStatusMsg("已绑定工作区目录");
        setTimeout(() => setStatusMsg(null), 2500);
      }
    } catch (err) {
      setError(`选择目录失败: ${err}`);
    }
  };

  // 2. 一键初始化工作区脚手架
  const handleInitWorkspace = async (templateType = "general") => {
    if (!workspacePath) return;
    setLoading(true);
    setError(null);
    try {
      const st = await workspaceInit({
        path: workspacePath,
        project_name: name || project.name,
        template_type: templateType,
      });
      setWorkspaceStatus(st);
      if (st.prompt_content) setPromptFileContent(st.prompt_content);
      if (st.rules_content) setRulesContent(st.rules_content);
      await refreshWorkspace();
        // 说清**这一次**新建了什么。「初始化完成」在已经齐了的情况下也会显示，
        // 而那时其实什么都没发生 —— 用户看不出区别。
        setStatusMsg(
          st.created.length > 0
            ? `已新建 ${st.created.length} 个文件：${st.created.join("、")}`
            : "脚手架已经是齐的，本次没有新建文件（已有内容未改动）",
        );
        setTimeout(() => setStatusMsg(null), 5000);
    } catch (err) {
      setError(`初始化工作区失败: ${err}`);
    } finally {
      setLoading(false);
    }
  };

  // 3. 修改 Security Preset
  const handleSecurityPresetChange = async (val: string) => {
    const updated: ProjectPermissions = {
      ...permissions,
      security_preset: val,
    };
    setPermissions(updated);
    if (workspacePath && workspaceStatus?.is_initialized) {
      await workspaceSavePermissions(workspacePath, updated);
      setStatusMsg(`已应用安全预设: ${val}`);
      setTimeout(() => setStatusMsg(null), 2000);
    }
  };

  // 4. 保存全部设置
  const handleSaveAll = async () => {
    setLoading(true);
    setError(null);
    try {
      await updateProject(
        project.id,
        name.trim() || project.name,
        description.trim(),
        systemPrompt.trim(),
        characterId.trim(),
        workspacePath.trim()
      );

      if (workspacePath && workspaceStatus?.is_initialized) {
        await workspaceSaveFiles({
          path: workspacePath,
          promptContent: promptFileContent,
          rulesContent: rulesContent,
        });
        await workspaceSavePermissions(workspacePath, permissions);
      }

      setStatusMsg("✅ 全部设置与权限策略已保存");
      setEditingSection(null);
      setTimeout(() => setStatusMsg(null), 2500);
    } catch (err) {
      setError(`保存失败: ${err}`);
    } finally {
      setLoading(false);
    }
  };

  // 5. 打开目录
  const handleOpenFinder = (subpath = "") => {
    if (workspacePath) {
      const target = subpath ? `${workspacePath}/${subpath}` : workspacePath;
      workspaceOpenInFinder(target).catch(() => {});
    }
  };

  // 6. 重新建立向量索引
  const handleReindexMemory = async () => {
    if (!workspacePath) return;
    setReindexing(true);
    try {
      const count = await workspaceReindexMemory(workspacePath);
      setStatusMsg(`⚡ 向量知识库重构完成！已建立 ${count} 篇文档语义索引。`);
      setTimeout(() => setStatusMsg(null), 3000);
    } catch (err) {
      setError(`索引失败: ${err}`);
    } finally {
      setReindexing(false);
    }
  };

  // 7. 删除知识库文档
  const handleDeleteDoc = async (filename: string) => {
    if (!workspacePath) return;
    try {
      await workspaceDeleteKnowledgeDoc(workspacePath, filename);
      const docs = await workspaceListKnowledgeDocs(workspacePath);
      setKnowledgeDocs(docs);
      setStatusMsg(`已删除文档: ${filename}`);
      setTimeout(() => setStatusMsg(null), 2000);
    } catch (err) {
      setError(`删除文档失败: ${err}`);
    }
  };

  return (
    <div className="flex flex-col gap-6 max-w-3xl w-full mx-auto pb-8 text-[var(--ink)]">
      {/* 顶部 Header：标题与副标题 */}
      <div className="flex items-start justify-between pb-4 border-b border-[var(--line)]">
        <div className="flex flex-col gap-1">
          <div className="flex items-center gap-2">
            {isRenaming ? (
              <input
                autoFocus
                value={name}
                onChange={(e) => setName(e.target.value)}
                onBlur={() => setIsRenaming(false)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") setIsRenaming(false);
                  if (e.key === "Escape") setIsRenaming(false);
                }}
                className="text-[20px] font-bold tracking-tight bg-[var(--surface-2)] border border-[var(--celadon)] rounded-lg px-2 py-0.5 outline-none text-[var(--ink)] min-w-[200px]"
              />
            ) : (
              <h2
                onClick={() => setIsRenaming(true)}
                className="text-[20px] font-bold tracking-tight text-[var(--ink)] m-0 flex items-center gap-2 group cursor-pointer hover:opacity-80 transition-opacity"
              >
                <span>{name || project.name}</span>
                <span className="text-[13px] text-[var(--ink-3)] group-hover:text-[var(--ink)] opacity-60 group-hover:opacity-100 transition-all">
                  ✏️
                </span>
              </h2>
            )}
          </div>
          <p className="text-[12.5px] text-[var(--ink-3)] m-0">
            Manage project folders, agent settings, and permissions.
          </p>
        </div>

        {onClose && (
          <button
            type="button"
            onClick={onClose}
            className="text-[var(--ink-3)] hover:text-[var(--ink)] p-1.5 rounded-lg hover:bg-[var(--surface-2)] cursor-pointer transition-colors"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        )}
      </div>

      {/* 状态与通知条 */}
      {(statusMsg || error) && (
        <div
          className={`p-3 rounded-xl text-[12px] font-medium flex items-center justify-between transition-all ${
            error
              ? "bg-red-500/10 text-red-500 border border-red-500/20"
              : "bg-emerald-500/10 text-emerald-600 border border-emerald-500/20"
          }`}
        >
          <span>{error ? `⚠️ ${error}` : statusMsg}</span>
          <button
            type="button"
            onClick={() => {
              setError(null);
              setStatusMsg(null);
            }}
            className="text-[11px] underline opacity-70 cursor-pointer"
          >
            关闭
          </button>
        </div>
      )}

      {/* ============================================================== */}
      {/* 1. Folders 模块                                                */}
      {/* ============================================================== */}
      <div className="flex flex-col gap-2.5">
        <h3 className="text-[12.5px] font-bold text-[var(--ink-2)] uppercase tracking-wider m-0">
          Folders
        </h3>

        <div className="bg-[var(--paper)] border border-[var(--line)] rounded-2xl p-2 flex flex-col gap-2 shadow-2xs">
          {workspacePath ? (
            <div className="flex items-center justify-between px-3 py-2 bg-[var(--surface)] border border-[var(--line)] rounded-xl">
              <div className="flex items-center gap-2.5 min-w-0">
                <span className="text-[14px] opacity-70">📁</span>
                <span className="font-semibold text-[13px] text-[var(--ink)] truncate">
                  {workspacePath.split("/").filter(Boolean).pop()}/
                </span>
                <span className="text-[11px] font-mono text-[var(--ink-3)] truncate hidden sm:inline">
                  {workspacePath}
                </span>
                {workspaceStatus?.is_initialized && (
                  <span className="text-[9.5px] px-2 py-0.5 rounded-full bg-emerald-500/15 text-emerald-600 font-medium shrink-0">
                    .anima Ready
                  </span>
                )}
              </div>

              <div className="flex items-center gap-1.5 shrink-0">
                <button
                  type="button"
                  onClick={() => handleOpenFinder()}
                  className="text-[11px] px-2.5 py-1 rounded-lg bg-[var(--surface-2)] hover:bg-[var(--surface-3)] text-[var(--ink-2)] hover:text-[var(--ink)] font-medium border border-[var(--line)] transition-colors cursor-pointer"
                >
                  Reveal
                </button>
                <button
                  type="button"
                  onClick={() => setWorkspacePath("")}
                  title="解绑目录"
                  className="text-[var(--ink-3)] hover:text-red-500 p-1 rounded hover:bg-red-500/10 transition-colors cursor-pointer"
                >
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                    <path d="M18 6L6 18M6 6l12 12" />
                  </svg>
                </button>
              </div>
            </div>
          ) : null}

          <button
            type="button"
            onClick={handleChooseFolder}
            className="w-full py-2.5 rounded-xl border border-dashed border-[var(--line)] hover:border-[var(--celadon)] text-[12.5px] font-medium text-[var(--ink-2)] hover:text-[var(--celadon)] bg-[var(--surface)] hover:bg-[var(--surface-3)] flex items-center justify-center gap-1.5 transition-all cursor-pointer"
          >
            <span>+</span> Add Folder
          </button>
        </div>

        {/* 未初始化警告卡片 */}
        {workspacePath && !workspaceStatus?.is_initialized && (
          <div className="flex flex-wrap items-center justify-between gap-3 p-3 px-4 rounded-xl bg-amber-500/10 border border-amber-500/25 text-[12px]">
            <div className="flex items-center gap-2 text-amber-600 font-medium">
              <span>⚠️</span> 目录缺少 .anima 配置文件与知识库脚手架
            </div>
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => handleInitWorkspace("general")}
                disabled={loading}
                className="px-3 py-1 rounded-lg bg-amber-500 text-white font-semibold text-[11.5px] hover:opacity-90 cursor-pointer shadow-2xs"
              >
                一键初始化 (.anima)
              </button>
              {/* developer 模板原先从界面上够不着 —— 后端有三套
                  （general / drama / developer），这里只给了前两个，
                  而 coding 项目恰好最该用第三个。 */}
              <button
                type="button"
                onClick={() => handleInitWorkspace("developer")}
                disabled={loading}
                className="px-3 py-1 rounded-lg bg-[var(--surface)] border border-amber-500/30 text-amber-600 font-semibold text-[11.5px] hover:bg-amber-500/15 cursor-pointer"
              >
                软件开发脚手架
              </button>
              <button
                type="button"
                onClick={() => handleInitWorkspace("drama")}
                disabled={loading}
                className="px-3 py-1 rounded-lg bg-[var(--surface)] border border-amber-500/30 text-amber-600 font-semibold text-[11.5px] hover:bg-amber-500/15 cursor-pointer"
              >
                短剧工坊脚手架
              </button>
            </div>
          </div>
        )}
      </div>

      {/* ============================================================== */}
      {/* 2. Agent Settings                                              */}
      {/* ============================================================== */}
      <div className="flex flex-col gap-2.5">
        <h3 className="text-[12.5px] font-bold text-[var(--ink-2)] uppercase tracking-wider m-0">
          Agent Settings
        </h3>

        <div className="bg-[var(--surface-2)] border border-[var(--line)] rounded-2xl p-4 flex flex-col gap-3.5">
          <div className="flex items-center justify-between gap-4">
            <div className="flex flex-col gap-0.5">
              <span className="text-[13px] font-bold text-[var(--ink)]">
                Security Preset
              </span>
              <span className="text-[11.5px] text-[var(--ink-3)] leading-normal">
                Choose a predefined security preset for the agent. This controls terminal auto-execution policy, and file access policy.
              </span>
            </div>
            <select
              value={permissions.security_preset}
              onChange={(e) => handleSecurityPresetChange(e.target.value)}
              className="px-3 py-1.5 text-[12px] font-medium bg-[var(--surface)] border border-[var(--line)] rounded-xl outline-none text-[var(--ink)] cursor-pointer shrink-0"
            >
              <option value="inherit">Inherit General</option>
              <option value="auto_read">Auto-Approve Read</option>
              <option value="strict">Strict Confirm</option>
            </select>
          </div>
        </div>
      </div>

      {/* ============================================================== */}
      {/* 3. Agent Behavior (提示词与准则)                                */}
      {/* ============================================================== */}
      <div className="flex flex-col gap-2.5">
        <h3 className="text-[12.5px] font-bold text-[var(--ink-2)] uppercase tracking-wider m-0">
          Agent Behavior
        </h3>

        <div className="bg-[var(--surface-2)] border border-[var(--line)] rounded-2xl p-4 flex flex-col gap-4">
          {/* PROMPT.md Card */}
          <div className="flex flex-col gap-2.5">
            <div className="flex items-center justify-between gap-4">
              <div className="flex flex-col gap-0.5">
                <span className="text-[13px] font-bold text-[var(--ink)]">
                  System Prompt (PROMPT.md)
                </span>
                <span className="text-[11.5px] text-[var(--ink-3)] leading-normal">
                  Specifies Agent&apos;s role instructions, domain persona, and knowledge bounds for this project.
                </span>
              </div>
              <button
                type="button"
                onClick={() => setEditingSection(editingSection === "prompt" ? null : "prompt")}
                className="px-3.5 py-1 text-[11.5px] font-semibold bg-[var(--surface)] hover:bg-[var(--surface-3)] border border-[var(--line)] rounded-xl text-[var(--ink)] cursor-pointer transition-all shrink-0"
              >
                {editingSection === "prompt" ? "Collapse" : "Open"}
              </button>
            </div>

            {editingSection === "prompt" && (
              <div className="mt-2 flex flex-col gap-2.5 animate-in fade-in duration-150">
                <div className="flex items-center justify-between">
                  <span className="text-[11px] text-[var(--ink-3)] font-medium">快捷填入行业级预设模版：</span>
                  <div className="flex items-center gap-1.5">
                    <button
                      type="button"
                      onClick={() => {
                        const t = PROMPT_TEMPLATES.drama(name || project.name);
                        setPromptFileContent(t);
                        setSystemPrompt(t);
                      }}
                      className="text-[10.5px] px-2 py-0.5 rounded-lg bg-[var(--surface)] hover:bg-[var(--surface-3)] text-[var(--ink-2)] border border-[var(--line)] cursor-pointer"
                    >
                      🎬 短剧编导模版
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        const t = PROMPT_TEMPLATES.developer(name || project.name);
                        setPromptFileContent(t);
                        setSystemPrompt(t);
                      }}
                      className="text-[10.5px] px-2 py-0.5 rounded-lg bg-[var(--surface)] hover:bg-[var(--surface-3)] text-[var(--ink-2)] border border-[var(--line)] cursor-pointer"
                    >
                      💻 架构师编程模版
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        const t = PROMPT_TEMPLATES.general(name || project.name);
                        setPromptFileContent(t);
                        setSystemPrompt(t);
                      }}
                      className="text-[10.5px] px-2 py-0.5 rounded-lg bg-[var(--surface)] hover:bg-[var(--surface-3)] text-[var(--ink-2)] border border-[var(--line)] cursor-pointer"
                    >
                      🌟 全能助手模版
                    </button>
                  </div>
                </div>

                <textarea
                  rows={9}
                  value={promptFileContent || systemPrompt || PROMPT_TEMPLATES.general(name || project.name)}
                  onChange={(e) => {
                    setPromptFileContent(e.target.value);
                    setSystemPrompt(e.target.value);
                  }}
                  placeholder="输入项目专属系统指令（例如角色设定、专业领域背景、回答规范）..."
                  className="w-full p-3 text-[12px] font-mono leading-relaxed rounded-xl bg-[var(--surface)] border border-[var(--celadon)]/70 outline-none resize-none text-[var(--ink)] shadow-inner"
                />
              </div>
            )}
          </div>

          <div className="border-t border-[var(--line)]/60 pt-3">
            {/* rules.md Card */}
            <div className="flex items-center justify-between gap-4">
              <div className="flex flex-col gap-0.5">
                <span className="text-[13px] font-bold text-[var(--ink)]">
                  Project Rules (.anima/rules.md)
                </span>
                <span className="text-[11.5px] text-[var(--ink-3)] leading-normal">
                  Specifies Agent&apos;s behavior when asking for review, code constraints, and safety guidelines.
                </span>
              </div>
              <button
                type="button"
                onClick={() => setEditingSection(editingSection === "rules" ? null : "rules")}
                className="px-3.5 py-1 text-[11.5px] font-semibold bg-[var(--surface)] hover:bg-[var(--surface-3)] border border-[var(--line)] rounded-xl text-[var(--ink)] cursor-pointer transition-all shrink-0"
              >
                {editingSection === "rules" ? "Collapse" : "Open"}
              </button>
            </div>

            {editingSection === "rules" && (
              <div className="mt-3 flex flex-col gap-2.5 animate-in fade-in duration-150">
                <div className="flex items-center justify-between">
                  <span className="text-[11px] text-[var(--ink-3)] font-medium">标准化行为与安全约束：</span>
                  <button
                    type="button"
                    onClick={() => setRulesContent(DEFAULT_RULES)}
                    className="text-[10.5px] px-2 py-0.5 rounded-lg bg-[var(--surface)] hover:bg-[var(--surface-3)] text-[var(--ink-2)] border border-[var(--line)] cursor-pointer"
                  >
                    ✨ 载入标准规则模板
                  </button>
                </div>

                <textarea
                  rows={9}
                  value={rulesContent || DEFAULT_RULES}
                  onChange={(e) => setRulesContent(e.target.value)}
                  placeholder="定义本项目的回答规则、禁止事项、代码规范、短剧剧情禁忌..."
                  className="w-full p-3 text-[12px] font-mono leading-relaxed rounded-xl bg-[var(--surface)] border border-[var(--celadon)]/70 outline-none resize-none text-[var(--ink)] shadow-inner"
                />
              </div>
            )}
          </div>
        </div>
      </div>

      {/* ============================================================== */}
      {/* 4. Local Permissions & Knowledge                               */}
      {/* ============================================================== */}
      <div className="flex flex-col gap-2.5">
        <h3 className="text-[12.5px] font-bold text-[var(--ink-2)] uppercase tracking-wider m-0">
          Local Permissions
        </h3>

        <div className="bg-[var(--surface-2)] border border-[var(--line)] rounded-2xl divide-y divide-[var(--line)] overflow-hidden">
          {/* Knowledge Base */}
          <div className="p-3.5 px-4 flex items-center justify-between gap-4 hover:bg-[var(--surface-3)]/40 transition-colors">
            <div className="flex flex-col gap-0.5">
              <span className="text-[13px] font-bold text-[var(--ink)]">
                Knowledge Base (RAG Docs)
              </span>
              <span className="text-[11.5px] text-[var(--ink-3)]">
                {workspaceStatus?.is_initialized
                  ? `Indexed local reference documents in .anima/knowledge (${knowledgeDocs.length} files).`
                  : "Put reference documents in .anima/knowledge for automatic semantic retrieval."}
              </span>
            </div>
            <button
              type="button"
              onClick={() => setActiveModal("knowledge")}
              className="px-3.5 py-1 text-[11.5px] font-semibold bg-[var(--surface)] hover:bg-[var(--surface-3)] border border-[var(--line)] rounded-xl text-[var(--ink)] cursor-pointer transition-all shrink-0"
            >
              Open
            </button>
          </div>

          {/* File Access Rules */}
          <div className="p-3.5 px-4 flex items-center justify-between gap-4 hover:bg-[var(--surface-3)]/40 transition-colors">
            <div className="flex flex-col gap-0.5">
              <span className="text-[13px] font-bold text-[var(--ink)]">
                File Access Rules
              </span>
              <span className="text-[11.5px] text-[var(--ink-3)]">
                Configure allowed and denied paths for file reads and writes in workspace.
              </span>
            </div>
            <button
              type="button"
              onClick={() => setActiveModal("file_access")}
              className="px-3.5 py-1 text-[11.5px] font-semibold bg-[var(--surface)] hover:bg-[var(--surface-3)] border border-[var(--line)] rounded-xl text-[var(--ink)] cursor-pointer transition-all shrink-0"
            >
              Open
            </button>
          </div>

          {/* Terminal Commands */}
          <div className="p-3.5 px-4 flex items-center justify-between gap-4 hover:bg-[var(--surface-3)]/40 transition-colors">
            <div className="flex flex-col gap-0.5">
              <span className="text-[13px] font-bold text-[var(--ink)]">
                Terminal Commands
              </span>
              <span className="text-[11.5px] text-[var(--ink-3)]">
                Configure allowed terminal commands and execution sandbox rules.
              </span>
            </div>
            <button
              type="button"
              onClick={() => setActiveModal("terminal")}
              className="px-3.5 py-1 text-[11.5px] font-semibold bg-[var(--surface)] hover:bg-[var(--surface-3)] border border-[var(--line)] rounded-xl text-[var(--ink)] cursor-pointer transition-all shrink-0"
            >
              Open
            </button>
          </div>
        </div>
      </div>

      {/* ============================================================== */}
      {/* 5. 底部操作栏                                                 */}
      {/* ============================================================== */}
      <div className="flex items-center justify-between pt-4 border-t border-[var(--line)]">
        <button
          type="button"
          onClick={() => {
            if (confirm(`确定要删除项目「${project.name}」吗？`)) {
              deleteProject(project.id);
              onClose?.();
            }
          }}
          className="text-[12px] text-red-500/80 hover:text-red-500 font-medium cursor-pointer transition-colors flex items-center gap-1"
        >
          <span>🗑️</span> 删除项目
        </button>

        <button
          type="button"
          onClick={handleSaveAll}
          disabled={loading}
          className="px-5 py-2 bg-[var(--celadon)] text-white font-bold text-[12.5px] rounded-xl hover:opacity-90 shadow-sm cursor-pointer transition-all disabled:opacity-50"
        >
          {loading ? "Saving..." : "Save Changes"}
        </button>
      </div>

      {/* ============================================================== */}
      {/* 6. 功能弹窗 1: Knowledge Base 管理弹窗                          */}
      {/* ============================================================== */}
      {activeModal === "knowledge" && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs animate-in fade-in duration-200">
          <div className="w-full max-w-xl bg-[var(--surface)] border border-[var(--line)] rounded-3xl shadow-2xl p-6 flex flex-col gap-4">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <span className="text-xl">📚</span>
                <h3 className="text-[16px] font-bold text-[var(--ink)] m-0">
                  Knowledge Base (RAG 知识库)
                </h3>
              </div>
              <button
                type="button"
                onClick={() => setActiveModal(null)}
                className="w-7 h-7 rounded-full bg-[var(--surface-2)] text-[var(--ink-3)] hover:text-[var(--ink)] flex items-center justify-center cursor-pointer"
              >
                ✕
              </button>
            </div>

            <p className="text-[12px] text-[var(--ink-3)] m-0">
              放入 .anima/knowledge 目录的 Markdown、TXT 与参考资料会被 Anima 自动向量化并在任务中进行语义召回。
            </p>

            {/* 文档列表 */}
            <div className="max-h-60 overflow-y-auto bg-[var(--surface-2)] border border-[var(--line)] rounded-2xl divide-y divide-[var(--line)]">
              {knowledgeDocs.length > 0 ? (
                knowledgeDocs.map((doc) => (
                  <div key={doc.name} className="p-2.5 px-3.5 flex items-center justify-between gap-3 text-[12px]">
                    <div className="flex items-center gap-2 min-w-0">
                      <span className="opacity-70">📄</span>
                      <span className="font-medium text-[var(--ink)] truncate">{doc.name}</span>
                      <span className="text-[10px] text-[var(--ink-3)] font-mono">
                        ({(doc.size_bytes / 1024).toFixed(1)} KB)
                      </span>
                    </div>
                    <button
                      type="button"
                      onClick={() => handleDeleteDoc(doc.name)}
                      className="text-red-500 hover:bg-red-500/10 p-1 rounded cursor-pointer"
                    >
                      🗑️
                    </button>
                  </div>
                ))
              ) : (
                <div className="p-6 text-center text-[12px] text-[var(--ink-3)]">
                  知识库暂无文档，请点击下方按钮打开目录并放入文件。
                </div>
              )}
            </div>

            {/* 动作按键 */}
            <div className="flex items-center justify-between pt-2 border-t border-[var(--line)]">
              <button
                type="button"
                onClick={() => handleOpenFinder(".anima/knowledge")}
                className="px-3.5 py-1.5 bg-[var(--surface-2)] hover:bg-[var(--surface-3)] border border-[var(--line)] rounded-xl text-[12px] font-medium text-[var(--ink)] cursor-pointer flex items-center gap-1.5"
              >
                <span>📂</span> 在访达中打开文件夹
              </button>
              <div className="flex items-center gap-3">
              {/* 已索引块数与「目录里有几个文件」是两个不同的数字。
                  文档改了但没重建索引时，只有这个数字能看出不一致。 */}
              {/* 「有文档但没索引」必须显式提示。
                  文档数来自目录，块数来自向量库 —— 两者不一致时
                  search_knowledge 一条都查不到，而**不报错**：模型只会当成
                  「这个项目没有相关文档」，然后按自己的假设往下做。
                  两种常见成因：刚放进文档还没重建；换机器 clone 下来
                  （文档跟着仓库走，索引不跟）。 */}
              {indexedChunks === 0 && knowledgeDocs.length > 0 ? (
                <span className="text-[11px] text-amber-700 bg-amber-50 border border-amber-200/70 rounded-lg px-2 py-0.5">
                  有 {knowledgeDocs.length} 篇文档但索引是空的 —— 点右边重建
                </span>
              ) : (
                <span className="text-[11px] text-[var(--ink-3)] tabular-nums">
                  {indexedChunks === null
                    ? ""
                    : indexedChunks === 0
                      ? "尚未索引"
                      : `已索引 ${indexedChunks} 块`}
                </span>
              )}
              <button
                type="button"
                onClick={handleReindexMemory}
                disabled={reindexing}
                className="px-4 py-1.5 bg-[var(--celadon)] text-white font-bold text-[12px] rounded-xl hover:opacity-90 cursor-pointer shadow-sm disabled:opacity-50 flex items-center gap-1.5"
              >
                {reindexing ? "正在索引..." : "⚡ 立即重建向量索引"}
              </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* ============================================================== */}
      {/* 7. 功能弹窗 2: File Access Rules                               */}
      {/* ============================================================== */}
      {activeModal === "file_access" && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs animate-in fade-in duration-200">
          <div className="w-full max-w-xl bg-[var(--surface)] border border-[var(--line)] rounded-3xl shadow-2xl p-6 flex flex-col gap-4">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <span className="text-xl">📁</span>
                <h3 className="text-[16px] font-bold text-[var(--ink)] m-0">
                  File Access Rules（文件读写权限）
                </h3>
              </div>
              <button
                type="button"
                onClick={() => setActiveModal(null)}
                className="w-7 h-7 rounded-full bg-[var(--surface-2)] text-[var(--ink-3)] hover:text-[var(--ink)] flex items-center justify-center cursor-pointer"
              >
                ✕
              </button>
            </div>

            {/* 读写开关 */}
            <div className="grid grid-cols-2 gap-3">
              <label className="p-3 rounded-2xl bg-[var(--surface-2)] border border-[var(--line)] flex items-center justify-between cursor-pointer">
                <span className="text-[12.5px] font-medium text-[var(--ink)]">允许读取工作区文件</span>
                <input
                  type="checkbox"
                  checked={permissions.file_access.allow_reads}
                  onChange={(e) =>
                    setPermissions({
                      ...permissions,
                      file_access: { ...permissions.file_access, allow_reads: e.target.checked },
                    })
                  }
                  className="w-4 h-4 accent-[var(--celadon)]"
                />
              </label>
              <label className="p-3 rounded-2xl bg-[var(--surface-2)] border border-[var(--line)] flex items-center justify-between cursor-pointer">
                <span className="text-[12.5px] font-medium text-[var(--ink)]">允许修改/新建文件</span>
                <input
                  type="checkbox"
                  checked={permissions.file_access.allow_writes}
                  onChange={(e) =>
                    setPermissions({
                      ...permissions,
                      file_access: { ...permissions.file_access, allow_writes: e.target.checked },
                    })
                  }
                  className="w-4 h-4 accent-[var(--celadon)]"
                />
              </label>
            </div>

            {/* 允许白名单目录 */}
            <div className="flex flex-col gap-1.5">
              <span className="text-[12px] font-bold text-[var(--ink)]">允许访问的工作区目录 (Allowed Paths)</span>
              <div className="flex flex-wrap gap-1.5 p-2 bg-[var(--surface-2)] border border-[var(--line)] rounded-xl min-h-[42px] items-center">
                {permissions.file_access.allowed_paths.map((p, idx) => (
                  <span
                    key={idx}
                    className="text-[11px] px-2 py-0.5 rounded-lg bg-emerald-500/15 text-emerald-600 font-mono flex items-center gap-1"
                  >
                    {p}
                    <button
                      type="button"
                      onClick={() =>
                        setPermissions({
                          ...permissions,
                          file_access: {
                            ...permissions.file_access,
                            allowed_paths: permissions.file_access.allowed_paths.filter((_, i) => i !== idx),
                          },
                        })
                      }
                      className="hover:font-bold cursor-pointer"
                    >
                      ×
                    </button>
                  </span>
                ))}
              </div>
              <div className="flex gap-2 mt-1">
                <input
                  type="text"
                  value={newAllowedPath}
                  onChange={(e) => setNewAllowedPath(e.target.value)}
                  placeholder="添加允许目录路径（例如：/Users/.../docs/）"
                  className="flex-1 p-2 text-[12px] rounded-xl bg-[var(--surface-2)] border border-[var(--line)] outline-none text-[var(--ink)]"
                />
                <button
                  type="button"
                  onClick={() => {
                    if (newAllowedPath.trim()) {
                      setPermissions({
                        ...permissions,
                        file_access: {
                          ...permissions.file_access,
                          allowed_paths: [...permissions.file_access.allowed_paths, newAllowedPath.trim()],
                        },
                      });
                      setNewAllowedPath("");
                    }
                  }}
                  className="px-3 py-1 text-[12px] font-bold bg-[var(--surface-3)] border border-[var(--line)] rounded-xl text-[var(--ink)] cursor-pointer"
                >
                  添加
                </button>
              </div>
            </div>

            {/* 拒绝黑名单 */}
            <div className="flex flex-col gap-1.5">
              <span className="text-[12px] font-bold text-[var(--ink)]">禁止访问的敏感路径 (Denied Paths)</span>
              <div className="flex flex-wrap gap-1.5 p-2 bg-[var(--surface-2)] border border-[var(--line)] rounded-xl min-h-[42px] items-center">
                {permissions.file_access.denied_paths.map((p, idx) => (
                  <span
                    key={idx}
                    className="text-[11px] px-2 py-0.5 rounded-lg bg-red-500/15 text-red-500 font-mono flex items-center gap-1"
                  >
                    {p}
                    <button
                      type="button"
                      onClick={() =>
                        setPermissions({
                          ...permissions,
                          file_access: {
                            ...permissions.file_access,
                            denied_paths: permissions.file_access.denied_paths.filter((_, i) => i !== idx),
                          },
                        })
                      }
                      className="hover:font-bold cursor-pointer"
                    >
                      ×
                    </button>
                  </span>
                ))}
              </div>
              <div className="flex gap-2 mt-1">
                <input
                  type="text"
                  value={newDeniedPath}
                  onChange={(e) => setNewDeniedPath(e.target.value)}
                  placeholder="添加敏感路径（例如：.env*, keys/）"
                  className="flex-1 p-2 text-[12px] rounded-xl bg-[var(--surface-2)] border border-[var(--line)] outline-none text-[var(--ink)]"
                />
                <button
                  type="button"
                  onClick={() => {
                    if (newDeniedPath.trim()) {
                      setPermissions({
                        ...permissions,
                        file_access: {
                          ...permissions.file_access,
                          denied_paths: [...permissions.file_access.denied_paths, newDeniedPath.trim()],
                        },
                      });
                      setNewDeniedPath("");
                    }
                  }}
                  className="px-3 py-1 text-[12px] font-bold bg-[var(--surface-3)] border border-[var(--line)] rounded-xl text-[var(--ink)] cursor-pointer"
                >
                  添加
                </button>
              </div>
            </div>

            <div className="flex justify-end pt-2 border-t border-[var(--line)]">
              <button
                type="button"
                onClick={async () => {
                  if (workspacePath && workspaceStatus?.is_initialized) {
                    await workspaceSavePermissions(workspacePath, permissions);
                  }
                  setActiveModal(null);
                  setStatusMsg("✅ 文件读写规则已保存");
                }}
                className="px-5 py-2 bg-[var(--celadon)] text-white font-bold text-[12.5px] rounded-xl cursor-pointer"
              >
                保存规则
              </button>
            </div>
          </div>
        </div>
      )}

      {/* ============================================================== */}
      {/* 8. 功能弹窗 3: Terminal Commands                               */}
      {/* ============================================================== */}
      {activeModal === "terminal" && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs animate-in fade-in duration-200">
          <div className="w-full max-w-xl bg-[var(--surface)] border border-[var(--line)] rounded-3xl shadow-2xl p-6 flex flex-col gap-4">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <span className="text-xl">💻</span>
                <h3 className="text-[16px] font-bold text-[var(--ink)] m-0">
                  Terminal Commands（终端命令执行策略）
                </h3>
              </div>
              <button
                type="button"
                onClick={() => setActiveModal(null)}
                className="w-7 h-7 rounded-full bg-[var(--surface-2)] text-[var(--ink-3)] hover:text-[var(--ink)] flex items-center justify-center cursor-pointer"
              >
                ✕
              </button>
            </div>

            {/* 沙箱模式 */}
            <div className="flex flex-col gap-1.5">
              <span className="text-[12px] font-bold text-[var(--ink)]">沙箱隔离模式 (Sandbox Mode)</span>
              <select
                value={permissions.terminal_policy.sandbox_mode}
                onChange={(e) =>
                  setPermissions({
                    ...permissions,
                    terminal_policy: { ...permissions.terminal_policy, sandbox_mode: e.target.value },
                  })
                }
                className="p-2.5 text-[12.5px] font-medium bg-[var(--surface-2)] border border-[var(--line)] rounded-xl outline-none text-[var(--ink)]"
              >
                <option value="standard">Standard Sandbox (标准沙箱，隔离未授权网络与外部目录)</option>
                <option value="unsandboxed">Unsandboxed (完全宿主权限，执行需手动审批)</option>
              </select>
            </div>

            {/* 常用免确认白名单命令 */}
            <div className="flex flex-col gap-1.5">
              <span className="text-[12px] font-bold text-[var(--ink)]">免确认常用命令 (Auto-approved Commands)</span>
              <div className="flex flex-wrap gap-1.5 p-2 bg-[var(--surface-2)] border border-[var(--line)] rounded-xl min-h-[42px] items-center">
                {permissions.terminal_policy.auto_approve_commands.map((cmd, idx) => (
                  <span
                    key={idx}
                    className="text-[11px] px-2 py-0.5 rounded-lg bg-emerald-500/15 text-emerald-600 font-mono flex items-center gap-1"
                  >
                    {cmd}
                    <button
                      type="button"
                      onClick={() =>
                        setPermissions({
                          ...permissions,
                          terminal_policy: {
                            ...permissions.terminal_policy,
                            auto_approve_commands: permissions.terminal_policy.auto_approve_commands.filter((_, i) => i !== idx),
                          },
                        })
                      }
                      className="hover:font-bold cursor-pointer"
                    >
                      ×
                    </button>
                  </span>
                ))}
              </div>
              <div className="flex gap-2 mt-1">
                <input
                  type="text"
                  value={newAutoCmd}
                  onChange={(e) => setNewAutoCmd(e.target.value)}
                  placeholder="添加免确认命令前缀（例如：git log, pytest）"
                  className="flex-1 p-2 text-[12px] rounded-xl bg-[var(--surface-2)] border border-[var(--line)] outline-none text-[var(--ink)]"
                />
                <button
                  type="button"
                  onClick={() => {
                    if (newAutoCmd.trim()) {
                      setPermissions({
                        ...permissions,
                        terminal_policy: {
                          ...permissions.terminal_policy,
                          auto_approve_commands: [...permissions.terminal_policy.auto_approve_commands, newAutoCmd.trim()],
                        },
                      });
                      setNewAutoCmd("");
                    }
                  }}
                  className="px-3 py-1 text-[12px] font-bold bg-[var(--surface-3)] border border-[var(--line)] rounded-xl text-[var(--ink)] cursor-pointer"
                >
                  添加
                </button>
              </div>
            </div>

            {/* 高危拦截黑名单命令 */}
            <div className="flex flex-col gap-1.5">
              <span className="text-[12px] font-bold text-[var(--ink)]">禁止/高危拦截命令 (Blocked Commands)</span>
              <div className="flex flex-wrap gap-1.5 p-2 bg-[var(--surface-2)] border border-[var(--line)] rounded-xl min-h-[42px] items-center">
                {permissions.terminal_policy.blocked_commands.map((cmd, idx) => (
                  <span
                    key={idx}
                    className="text-[11px] px-2 py-0.5 rounded-lg bg-red-500/15 text-red-500 font-mono flex items-center gap-1"
                  >
                    {cmd}
                    <button
                      type="button"
                      onClick={() =>
                        setPermissions({
                          ...permissions,
                          terminal_policy: {
                            ...permissions.terminal_policy,
                            blocked_commands: permissions.terminal_policy.blocked_commands.filter((_, i) => i !== idx),
                          },
                        })
                      }
                      className="hover:font-bold cursor-pointer"
                    >
                      ×
                    </button>
                  </span>
                ))}
              </div>
              <div className="flex gap-2 mt-1">
                <input
                  type="text"
                  value={newBlockedCmd}
                  onChange={(e) => setNewBlockedCmd(e.target.value)}
                  placeholder="添加禁止命令（例如：rm -rf /, mkfs）"
                  className="flex-1 p-2 text-[12px] rounded-xl bg-[var(--surface-2)] border border-[var(--line)] outline-none text-[var(--ink)]"
                />
                <button
                  type="button"
                  onClick={() => {
                    if (newBlockedCmd.trim()) {
                      setPermissions({
                        ...permissions,
                        terminal_policy: {
                          ...permissions.terminal_policy,
                          blocked_commands: [...permissions.terminal_policy.blocked_commands, newBlockedCmd.trim()],
                        },
                      });
                      setNewBlockedCmd("");
                    }
                  }}
                  className="px-3 py-1 text-[12px] font-bold bg-[var(--surface-3)] border border-[var(--line)] rounded-xl text-[var(--ink)] cursor-pointer"
                >
                  添加
                </button>
              </div>
            </div>

            <div className="flex justify-end pt-2 border-t border-[var(--line)]">
              <button
                type="button"
                onClick={async () => {
                  if (workspacePath && workspaceStatus?.is_initialized) {
                    await workspaceSavePermissions(workspacePath, permissions);
                  }
                  setActiveModal(null);
                  setStatusMsg("✅ 终端命令执行策略已保存");
                }}
                className="px-5 py-2 bg-[var(--celadon)] text-white font-bold text-[12.5px] rounded-xl cursor-pointer"
              >
                保存策略
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
