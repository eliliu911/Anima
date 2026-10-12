/**
 * 技能与拓展（角色扮演/任务规程）管理：
 * 支持直接在页面添加、修改和删除技能，支持编辑 Prompt、工具白名单、参数偏好与分层作用域
 */
import { useEffect, useState } from "react";
import {
  listSkills,
  selectSkills,
  saveSkill,
  deleteSkill,
  type Skill,
  type SaveSkillArgs,
} from "../api/voice";
import { useTranslation } from "react-i18next";
import { useVoice } from "../store/voice";

/**
 * 按当前界面语言挑技能的显示文案。
 *
 * 中文界面下用 `*_zh`，缺了就回落到英文那三个 —— 用户自己加的技能不写中文
 * 字段时仍然正常显示，而不是空白。文案放在 SKILL.md 的 frontmatter 里而不是
 * 应用的语言包里，因为技能是用户可新增可编辑的文件。
 */
function skillText(s: Skill, zh: boolean) {
  return {
    name: (zh && s.name_zh) || s.name,
    marker: (zh && s.marker_zh) || s.marker,
    description: (zh && s.description_zh) || s.description,
  };
}
import { useProject } from "../store/project";

export function SkillSettings() {
  const { i18n } = useTranslation();
  const zh = (i18n.language || "").startsWith("zh");
  const [skills, setSkills] = useState<Skill[]>([]);
  const [active, setActive] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);

  // 编辑 / 新建模态弹窗状态
  const [editingSkill, setEditingSkill] = useState<Skill | null>(null);
  const [isCreating, setIsCreating] = useState(false);
  const [formData, setFormData] = useState<SaveSkillArgs>({
    id: "",
    name: "",
    marker: "",
    description: "",
    prefer_tier: null,
    thinking: null,
    tools: [],
    source: "",
    kind: "persona",
    surface: "both",
    prompt: "",
    is_project_level: false,
  });
  const [toolsInput, setToolsInput] = useState("");

  // 删除确认模态状态
  const [deletingSkill, setDeletingSkill] = useState<Skill | null>(null);

  const marker = useVoice((s) => s.skillMarker);
  const workspacePath = useProject(
    (s) => s.projects.find((p) => p.id === s.activeProjectId)?.workspace_path ?? null,
  );

  const loadAllSkills = async () => {
    try {
      const view = await listSkills(workspacePath);
      setSkills(view.skills);
      setActive(view.active);
    } catch (err) {
      setMessage(`${err}`);
    }
  };

  useEffect(() => {
    void loadAllSkills();
  }, [marker, workspacePath]);

  async function toggleSkill(id: string) {
    if (busy) return;
    const newActive = active.includes(id) ? active.filter((i) => i !== id) : [...active, id];
    setBusy(true);
    setMessage(null);
    try {
      await selectSkills(newActive, workspacePath);
      setActive(newActive);
      setMessage("已应用技能组合");
      setTimeout(() => setMessage(null), 2000);
    } catch (err) {
      setMessage(`${err}`);
    } finally {
      setBusy(false);
    }
  }

  async function clearSkills() {
    if (busy) return;
    setBusy(true);
    setMessage(null);
    try {
      await selectSkills([], workspacePath);
      setActive([]);
      setMessage("已切回默认角色");
      setTimeout(() => setMessage(null), 2000);
    } catch (err) {
      setMessage(`${err}`);
    } finally {
      setBusy(false);
    }
  }

  const openCreateModal = () => {
    setIsCreating(true);
    setEditingSkill(null);
    setFormData({
      id: "",
      name: "",
      marker: "",
      description: "",
      prefer_tier: null,
      thinking: null,
      tools: [],
      source: "",
      kind: "persona",
      surface: "both",
      prompt: "你是一个专业的智能体助手。\n\n## 职责与工作规范\n- 严谨、专注、实事求是\n- 遵守安全原则与工具边界\n",
      is_project_level: !!workspacePath,
    });
    setToolsInput("");
  };

  const openEditModal = (skill: Skill) => {
    setIsCreating(false);
    setEditingSkill(skill);
    setFormData({
      id: skill.id,
      name: skill.name,
      marker: skill.marker || "",
      description: skill.description || "",
      prefer_tier: skill.prefer_tier,
      thinking: skill.thinking,
      tools: skill.tools || [],
      source: skill.source || "",
      kind: skill.kind || "persona",
      surface: skill.surface || "both",
      prompt: skill.prompt || "",
      is_project_level: skill.scope === "project",
    });
    setToolsInput((skill.tools || []).join(", "));
  };

  const handleSave = async () => {
    if (!formData.name.trim() || !formData.id.trim()) {
      setMessage("请填写技能名称与英文唯一 ID");
      return;
    }

    const parsedTools = toolsInput
      .split(/[,，\n]/)
      .map((t) => t.trim())
      .filter(Boolean);

    setBusy(true);
    try {
      await saveSkill({
        ...formData,
        id: formData.id.trim(),
        name: formData.name.trim(),
        marker: formData.marker?.trim() || "",
        description: formData.description?.trim() || "",
        tools: parsedTools,
        workspace_path: workspacePath,
      });
      setIsCreating(false);
      setEditingSkill(null);
      setMessage(isCreating ? "技能添加成功" : "技能已更新");
      setTimeout(() => setMessage(null), 2500);
      await loadAllSkills();
    } catch (err) {
      setMessage(`保存失败: ${err}`);
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async () => {
    if (!deletingSkill) return;
    setBusy(true);
    try {
      await deleteSkill(deletingSkill.id, workspacePath);
      setDeletingSkill(null);
      setMessage(`技能「${deletingSkill.name}」已删除`);
      setTimeout(() => setMessage(null), 2500);
      await loadAllSkills();
    } catch (err) {
      setMessage(`删除失败: ${err}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-4 max-w-4xl pb-8">
      {/* 顶部标题与新建按钮 */}
      <div className="flex items-center justify-between pb-1">
        <div className="flex flex-col">
          <span className="text-[14px] font-bold text-[var(--ink)] tracking-tight">技能与拓展 · 角色扮演</span>
          <span className="text-[12px] text-[var(--ink-2)] mt-0.5 leading-relaxed">
            管理与激活专业技能与指令规程，可动态切换角色身份、提示词准则与工具白名单。
          </span>
        </div>

        <button
          type="button"
          onClick={openCreateModal}
          className="flex items-center gap-1.5 px-3 py-1.5 bg-[var(--surface-3)] hover:bg-[var(--surface-2)] border border-[var(--line)] rounded-xl text-[12.5px] font-semibold text-[var(--ink)] cursor-pointer transition-colors shadow-2xs shrink-0"
        >
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
            <path d="M12 5v14M5 12h14" />
          </svg>
          <span>添加技能</span>
        </button>
      </div>

      <div className="p-3.5 px-4 rounded-2xl bg-[var(--paper)] border border-[var(--line)] shadow-2xs text-[12px] leading-relaxed text-[var(--ink-2)] flex items-center justify-between">
        <div>
          每个技能包含独立的身份设定、工作流、可用工具与思考偏好。内容存放于{" "}
          <code className="bg-[var(--surface)] px-1.5 py-0.5 rounded-md text-[11px] font-mono border border-[var(--line)] text-[var(--ink)]">
            skills/&lt;id&gt;/SKILL.md
          </code>
          ，支持即改即用。
        </div>
      </div>

      <div className="flex flex-col gap-3 mt-1">
        {/* 默认角色卡片 */}
        <button
          type="button"
          className={`flex flex-col gap-1 border rounded-2xl p-4 text-left transition-all cursor-pointer shadow-2xs ${
            active.length === 0
              ? "border-[var(--celadon)] ring-1 ring-[var(--celadon)]/25 bg-[var(--paper)]"
              : "bg-[var(--paper)] border-[var(--line)] hover:border-[var(--ink-3)]/50"
          } disabled:opacity-50`}
          disabled={busy}
          onClick={() => void clearSkills()}
        >
          <div className="flex items-center justify-between">
            <span className="text-[13.5px] font-bold text-[var(--ink)] flex items-center gap-2">
              <span>默认</span>
              {active.length === 0 && (
                <span className="text-[10.5px] bg-[var(--celadon-2)] text-[var(--celadon)] font-medium px-2 py-0.5 rounded-full">
                  当前使用
                </span>
              )}
            </span>
          </div>
          <span className="text-[12px] text-[var(--ink-2)] leading-relaxed mt-0.5">
            标准形态，不扮演任何特定角色，使用 Anima 原生多模态能力与知识。
          </span>
        </button>

        {/* 技能卡片列表 */}
        {skills.map((s) => {
          const isSelected = active.includes(s.id);
          return (
            <div key={s.id} className="relative flex flex-col group">
              <button
                type="button"
                className={`flex flex-col gap-1.5 border rounded-2xl p-4 text-left transition-all cursor-pointer pr-36 shadow-2xs ${
                  isSelected
                    ? "border-[var(--celadon)] ring-1 ring-[var(--celadon)]/25 bg-[var(--paper)]"
                    : "bg-[var(--paper)] border-[var(--line)] hover:border-[var(--ink-3)]/50"
                } disabled:opacity-50`}
                disabled={busy}
                onClick={() => void toggleSkill(s.id)}
              >
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="text-[13.5px] font-bold text-[var(--ink)]">
                    {skillText(s, zh).name}
                  </span>
                  {skillText(s, zh).marker && (
                    <span className="text-[10.5px] bg-sky-50 text-sky-600 border border-sky-200/60 font-medium px-2 py-0.5 rounded-full">
                      {skillText(s, zh).marker}
                    </span>
                  )}
                  {s.scope === "project" && (
                    <span className="text-[10.5px] bg-amber-50 text-amber-700 border border-amber-200/70 font-medium px-2 py-0.5 rounded-full">
                      本项目
                    </span>
                  )}
                  {isSelected && (
                    <span className="text-[10.5px] bg-[var(--celadon-2)] text-[var(--celadon)] font-medium px-2 py-0.5 rounded-full">
                      已激活
                    </span>
                  )}
                </div>

                <span className="text-[12px] text-[var(--ink-2)] leading-relaxed">
                  {skillText(s, zh).description}
                </span>

                <div className="flex items-center gap-1.5 mt-1 flex-wrap">
                  {s.prefer_tier && (
                    <span className="text-[11px] text-[var(--ink-2)] bg-[var(--surface)] px-2 py-0.5 rounded-lg border border-[var(--line)] shadow-2xs">
                      {s.prefer_tier === "background" ? "后台链路" : "实时链路"}
                    </span>
                  )}
                  {s.thinking !== null && (
                    <span className="text-[11px] text-[var(--ink-2)] bg-[var(--surface)] px-2 py-0.5 rounded-lg border border-[var(--line)] shadow-2xs">
                      思考{s.thinking ? "开启" : "关闭"}
                    </span>
                  )}
                  <span className="text-[11px] text-[var(--ink-2)] bg-[var(--surface)] px-2 py-0.5 rounded-lg border border-[var(--line)] shadow-2xs">
                    {s.tools.length === 0 ? "全部工具" : `工具 ${s.tools.length} 类`}
                  </span>
                </div>
              </button>

              {/* 卡片右上角快捷操作栏：详情、修改、删除 */}
              <div className="absolute top-4 right-4 flex items-center gap-1.5">
                <button
                  type="button"
                  title="查看详情"
                  className="text-[11.5px] font-medium text-[var(--ink-2)] hover:text-[var(--ink)] px-2.5 py-1 bg-[var(--surface)] hover:bg-[var(--surface-2)] border border-[var(--line)] rounded-xl shadow-2xs transition-colors cursor-pointer"
                  onClick={(e) => {
                    e.stopPropagation();
                    setExpanded(expanded === s.id ? null : s.id);
                  }}
                >
                  {expanded === s.id ? "收起" : "详情"}
                </button>

                <button
                  type="button"
                  title="编辑技能"
                  className="text-[11.5px] font-medium text-[var(--ink-2)] hover:text-[var(--ink)] px-2.5 py-1 bg-[var(--surface)] hover:bg-[var(--surface-2)] border border-[var(--line)] rounded-xl shadow-2xs transition-colors cursor-pointer"
                  onClick={(e) => {
                    e.stopPropagation();
                    openEditModal(s);
                  }}
                >
                  编辑
                </button>

                <button
                  type="button"
                  title="删除技能"
                  className="text-[11.5px] font-medium text-red-500 hover:text-red-600 hover:bg-red-500/10 px-2 py-1 bg-[var(--surface)] border border-[var(--line)] rounded-xl shadow-2xs transition-colors cursor-pointer"
                  onClick={(e) => {
                    e.stopPropagation();
                    setDeletingSkill(s);
                  }}
                >
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                    <path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                  </svg>
                </button>
              </div>

              {expanded === s.id && (
                <div className="text-[12px] text-[var(--ink-2)] bg-[var(--paper)] p-4 rounded-2xl border border-[var(--line)] mt-2 flex flex-col gap-2 shadow-2xs">
                  <div>
                    <strong className="font-semibold text-[var(--ink)]">可用工具集：</strong>
                    <span className="text-[var(--ink-2)] ml-1 font-mono">
                      {s.tools.length === 0 ? "跟随全局配置（无限制）" : s.tools.join("、")}
                    </span>
                  </div>
                  {s.prompt && (
                    <div className="flex flex-col gap-1 pt-1 border-t border-[var(--line)]/50">
                      <strong className="font-semibold text-[var(--ink)]">提示词与设定：</strong>
                      <pre className="text-[11px] font-mono bg-[var(--surface)] p-2.5 rounded-xl border border-[var(--line)] whitespace-pre-wrap max-h-40 overflow-y-auto">
                        {s.prompt}
                      </pre>
                    </div>
                  )}
                  {s.source && (
                    <div className="text-[11px] text-[var(--ink-3)] mt-1 pt-2 border-t border-[var(--line)]/50">
                      来源归属：{s.source}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>

      {skills.length === 0 && (
        <div className="text-[12.5px] text-[var(--ink-3)] text-center py-6 border border-dashed border-[var(--line)] rounded-2xl bg-[var(--paper)] shadow-2xs">
          未发现自定义技能。点击右上角「添加技能」即可创建。
        </div>
      )}

      {/* 新建 / 编辑技能模态弹窗 */}
      {(isCreating || editingSkill) && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-6 bg-black/60 backdrop-blur-xs animate-in fade-in duration-150">
          <div className="w-full max-w-2xl max-h-[90vh] overflow-y-auto bg-[var(--surface)] border border-[var(--line)] rounded-3xl p-6 shadow-2xl flex flex-col gap-4">
            <div className="flex items-center justify-between pb-2 border-b border-[var(--line)]">
              <h3 className="text-[16px] font-bold">
                {isCreating ? "添加新技能 (New Skill)" : `编辑技能 - ${editingSkill?.name}`}
              </h3>
              <button
                type="button"
                onClick={() => {
                  setIsCreating(false);
                  setEditingSkill(null);
                }}
                className="p-1 rounded-lg text-[var(--ink-3)] hover:text-[var(--ink)] cursor-pointer"
              >
                ✕
              </button>
            </div>

            <div className="flex flex-col gap-3.5">
              <div className="grid grid-cols-2 gap-3">
                <div className="flex flex-col gap-1.5">
                  <label className="text-[12px] font-bold text-[var(--ink-2)]">技能唯一标识 ID (英文/小写)</label>
                  <input
                    type="text"
                    disabled={!isCreating}
                    placeholder="例如: family-doctor, code-reviewer"
                    value={formData.id}
                    onChange={(e) => setFormData({ ...formData, id: e.target.value })}
                    className="w-full bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-1.5 text-[12.5px] outline-none text-[var(--ink)] focus:border-[var(--celadon)] disabled:opacity-60"
                  />
                </div>

                <div className="flex flex-col gap-1.5">
                  <label className="text-[12px] font-bold text-[var(--ink-2)]">技能显示名称</label>
                  <input
                    type="text"
                    placeholder="例如: 家庭医生, 代码审查专家"
                    value={formData.name}
                    onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                    className="w-full bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-1.5 text-[12.5px] outline-none text-[var(--ink)] focus:border-[var(--celadon)]"
                  />
                </div>
              </div>

              <div className="grid grid-cols-2 gap-3">
                <div className="flex flex-col gap-1.5">
                  <label className="text-[12px] font-bold text-[var(--ink-2)]">顶部状态标识 Badge (2~4字)</label>
                  <input
                    type="text"
                    placeholder="例如: 医生, 审查"
                    value={formData.marker || ""}
                    onChange={(e) => setFormData({ ...formData, marker: e.target.value })}
                    className="w-full bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-1.5 text-[12.5px] outline-none text-[var(--ink)] focus:border-[var(--celadon)]"
                  />
                </div>

                <div className="flex flex-col gap-1.5">
                  <label className="text-[12px] font-bold text-[var(--ink-2)]">技能类型</label>
                  <select
                    value={formData.kind || "persona"}
                    onChange={(e) => setFormData({ ...formData, kind: e.target.value as any })}
                    className="w-full bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-1.5 text-[12.5px] outline-none text-[var(--ink)] cursor-pointer"
                  >
                    <option value="persona">身份型技能 (Persona - 勾选后全局持续生效)</option>
                    <option value="task">任务型技能 (Task - 由 /斜杠指令触发单次执行)</option>
                  </select>
                </div>
              </div>

              <div className="flex flex-col gap-1.5">
                <label className="text-[12px] font-bold text-[var(--ink-2)]">一句话简要描述</label>
                <input
                  type="text"
                  placeholder="说明该技能的专注领域与专长"
                  value={formData.description || ""}
                  onChange={(e) => setFormData({ ...formData, description: e.target.value })}
                  className="w-full bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-1.5 text-[12.5px] outline-none text-[var(--ink)] focus:border-[var(--celadon)]"
                />
              </div>

              <div className="grid grid-cols-2 gap-3">
                <div className="flex flex-col gap-1.5">
                  <label className="text-[12px] font-bold text-[var(--ink-2)]">模型偏好链路</label>
                  <select
                    value={formData.prefer_tier || "default"}
                    onChange={(e) =>
                      setFormData({
                        ...formData,
                        prefer_tier: e.target.value === "default" ? null : (e.target.value as any),
                      })
                    }
                    className="w-full bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-1.5 text-[12.5px] outline-none text-[var(--ink)] cursor-pointer"
                  >
                    <option value="default">跟随系统默认配置</option>
                    <option value="realtime">实时链路 (高响应速度)</option>
                    <option value="background">后台链路 (深度大模型推理)</option>
                  </select>
                </div>

                <div className="flex flex-col gap-1.5">
                  <label className="text-[12px] font-bold text-[var(--ink-2)]">保存位置 / 作用域</label>
                  <select
                    value={formData.is_project_level ? "project" : "global"}
                    onChange={(e) =>
                      setFormData({ ...formData, is_project_level: e.target.value === "project" })
                    }
                    className="w-full bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-1.5 text-[12.5px] outline-none text-[var(--ink)] cursor-pointer"
                  >
                    <option value="global">全局通用技能 (skills/)</option>
                    {workspacePath && (
                      <option value="project">当前项目专有技能 (.anima/skills/)</option>
                    )}
                  </select>
                </div>
              </div>

              <div className="flex flex-col gap-1.5">
                <label className="text-[12px] font-bold text-[var(--ink-2)]">
                  可用工具白名单 (英文逗号分隔，留空表示全量开放)
                </label>
                <input
                  type="text"
                  placeholder="例如: filesystem__*, search_knowledge, web_search"
                  value={toolsInput}
                  onChange={(e) => setToolsInput(e.target.value)}
                  className="w-full bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-1.5 text-[12.5px] outline-none text-[var(--ink)] focus:border-[var(--celadon)] font-mono"
                />
              </div>

              <div className="flex flex-col gap-1.5">
                <label className="text-[12px] font-bold text-[var(--ink-2)]">
                  System Prompt 提示词与专业规程 (Markdown 正文)
                </label>
                <textarea
                  rows={8}
                  placeholder="填写该技能注入给模型的完整身份准则、思维规程与回复风格..."
                  value={formData.prompt || ""}
                  onChange={(e) => setFormData({ ...formData, prompt: e.target.value })}
                  className="w-full bg-[var(--surface-2)] border border-[var(--line)] rounded-xl p-3 text-[12px] outline-none text-[var(--ink)] font-mono focus:border-[var(--celadon)] resize-none leading-relaxed"
                />
              </div>
            </div>

            <div className="flex items-center justify-end gap-2 pt-2 border-t border-[var(--line)]">
              <button
                type="button"
                onClick={() => {
                  setIsCreating(false);
                  setEditingSkill(null);
                }}
                className="px-4 py-1.5 text-[12.5px] font-medium text-[var(--ink-3)] hover:text-[var(--ink)] rounded-xl cursor-pointer"
              >
                取消
              </button>
              <button
                type="button"
                onClick={handleSave}
                disabled={!formData.name.trim() || !formData.id.trim() || busy}
                className="px-5 py-1.5 text-[12.5px] font-semibold bg-[var(--celadon)] text-white rounded-xl cursor-pointer disabled:opacity-50 transition-all shadow-2xs"
              >
                保存技能
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 删除技能确认弹窗 */}
      {deletingSkill && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-6 bg-black/60 backdrop-blur-xs animate-in fade-in duration-150">
          <div className="w-full max-w-md bg-[var(--surface)] border border-[var(--line)] rounded-3xl p-6 shadow-2xl flex flex-col gap-4">
            <h3 className="text-[16px] font-bold text-red-500">确认删除技能</h3>
            <p className="text-[12.5px] text-[var(--ink-2)] leading-relaxed">
              确定要删除技能「<strong>{deletingSkill.name}</strong>」({deletingSkill.id}) 吗？
              这将从磁盘移除对应的 <code className="font-mono bg-[var(--surface-2)] px-1 py-0.5 rounded">SKILL.md</code> 文件。
            </p>
            <div className="flex items-center justify-end gap-2 pt-2 border-t border-[var(--line)]">
              <button
                type="button"
                onClick={() => setDeletingSkill(null)}
                className="px-4 py-1.5 text-[12.5px] font-medium text-[var(--ink-3)] hover:text-[var(--ink)] rounded-xl cursor-pointer"
              >
                取消
              </button>
              <button
                type="button"
                onClick={handleDelete}
                disabled={busy}
                className="px-4 py-1.5 text-[12.5px] font-semibold bg-red-500 hover:bg-red-600 text-white rounded-xl cursor-pointer transition-all shadow-2xs"
              >
                确认删除
              </button>
            </div>
          </div>
        </div>
      )}

      {message && (
        <div className="fixed bottom-6 right-6 z-50 text-[12.5px] text-[var(--ink)] bg-[var(--paper)] border border-[var(--celadon)] shadow-lg px-4 py-2.5 rounded-2xl flex items-center gap-2 animate-in fade-in slide-in-from-bottom-2">
          <span className="w-2 h-2 rounded-full bg-[var(--celadon)] animate-pulse" />
          <span>{message}</span>
        </div>
      )}
    </div>
  );
}
