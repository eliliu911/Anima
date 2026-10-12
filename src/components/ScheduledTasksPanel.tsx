/**
 * 任务安排（Scheduled Tasks）主管理面板：
 * 对标 Codex / Antigravity，支持创建、管理、状态查看、执行日志与产物结果
 */
import { useEffect, useState } from "react";
import {
  type RuntimeJob,
  type RuntimeJobKind,
  runtimeListJobs,
  runtimeCreateJob,
  runtimeStartJobRunner,
  runtimePauseJob,
  runtimeResumeJob,
  runtimeCancelJob,
  runtimeExportJobLog,
} from "../api";
import { useProject } from "../store/project";

export function ScheduledTasksPanel() {
  const { projects, activeProjectId } = useProject();

  const [jobs, setJobs] = useState<RuntimeJob[]>([]);
  const [loading, setLoading] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedJobId, setSelectedJobId] = useState<string | null>(null);

  // 新建任务模态弹窗状态
  const [isCreating, setIsCreating] = useState(false);
  const [newTitle, setNewTitle] = useState("");
  const [newKind, setNewKind] = useState<RuntimeJobKind>("generic");
  const [newSteps, setNewSteps] = useState<string>("分析项目上下文与文件依赖\n运行诊断与自动化测试\n生成总结报告与优化建议");

  // 查看任务执行日志状态
  const [logModalJobId, setLogModalJobId] = useState<string | null>(null);
  const [jobLogContent, setJobLogContent] = useState<string>("");
  const [logLoading, setLogLoading] = useState(false);

  const fetchJobs = async () => {
    setLoading(true);
    try {
      const list = await runtimeListJobs();
      setJobs(list);
      if (list.length > 0 && !selectedJobId) {
        setSelectedJobId(list[0].id);
      }
    } catch (err) {
      console.error("Failed to load runtime jobs:", err);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void fetchJobs();
  }, []);

  const handleCreateTask = async () => {
    if (!newTitle.trim()) return;
    const stepList = newSteps
      .split("\n")
      .map((s) => s.trim())
      .filter(Boolean);

    try {
      const job = await runtimeCreateJob({
        kind: newKind,
        title: newTitle.trim(),
        steps: stepList.length > 0 ? stepList : ["执行自动化分析流程"],
      });
      setIsCreating(false);
      setNewTitle("");
      await fetchJobs();
      setSelectedJobId(job.id);
    } catch (err) {
      console.error("Failed to create task:", err);
    }
  };

  const handleRunJob = async (jobId: string) => {
    const activeProject = projects.find((p) => p.id === activeProjectId);
    try {
      await runtimeStartJobRunner({
        job_id: jobId,
        workspace_path: activeProject?.workspace_path || null,
        network_enabled: true,
      });
      await fetchJobs();
    } catch (err) {
      console.error("Failed to start job:", err);
    }
  };

  const handlePauseJob = async (jobId: string) => {
    try {
      await runtimePauseJob(jobId);
      await fetchJobs();
    } catch (err) {
      console.error("Failed to pause job:", err);
    }
  };

  const handleResumeJob = async (jobId: string) => {
    try {
      await runtimeResumeJob(jobId);
      await fetchJobs();
    } catch (err) {
      console.error("Failed to resume job:", err);
    }
  };

  const handleCancelJob = async (jobId: string) => {
    try {
      await runtimeCancelJob(jobId);
      await fetchJobs();
    } catch (err) {
      console.error("Failed to cancel job:", err);
    }
  };

  const handleViewLog = async (jobId: string) => {
    setLogModalJobId(jobId);
    setLogLoading(true);
    try {
      const rawLog = await runtimeExportJobLog(jobId);
      setJobLogContent(rawLog);
    } catch (err) {
      setJobLogContent(`获取日志失败：${err}`);
    } finally {
      setLogLoading(false);
    }
  };

  const filteredJobs = jobs.filter((j) =>
    j.title.toLowerCase().includes(searchQuery.toLowerCase()) ||
    j.id.toLowerCase().includes(searchQuery.toLowerCase())
  );

  const selectedJob = jobs.find((j) => j.id === selectedJobId) || jobs[0];

  const renderStatusBadge = (status: string) => {
    switch (status) {
      case "running":
        return (
          <span className="px-2 py-0.5 rounded-full text-[11px] font-semibold bg-blue-500/10 text-blue-500 border border-blue-500/20 flex items-center gap-1.5">
            <span className="w-1.5 h-1.5 rounded-full bg-blue-500 animate-pulse" />
            运行中 (Running)
          </span>
        );
      case "succeeded":
        return (
          <span className="px-2 py-0.5 rounded-full text-[11px] font-semibold bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20 flex items-center gap-1.5">
            <span className="w-1.5 h-1.5 rounded-full bg-emerald-500" />
            已完成 (Succeeded)
          </span>
        );
      case "failed":
        return (
          <span className="px-2 py-0.5 rounded-full text-[11px] font-semibold bg-red-500/10 text-red-500 border border-red-500/20 flex items-center gap-1.5">
            <span className="w-1.5 h-1.5 rounded-full bg-red-500" />
            已失败 (Failed)
          </span>
        );
      case "paused":
        return (
          <span className="px-2 py-0.5 rounded-full text-[11px] font-semibold bg-amber-500/10 text-amber-500 border border-amber-500/20 flex items-center gap-1.5">
            <span className="w-1.5 h-1.5 rounded-full bg-amber-500" />
            已暂停 (Paused)
          </span>
        );
      case "cancelled":
        return (
          <span className="px-2 py-0.5 rounded-full text-[11px] font-semibold bg-[var(--surface-3)] text-[var(--ink-3)] border border-[var(--line)]">
            已取消 (Cancelled)
          </span>
        );
      default:
        return (
          <span className="px-2 py-0.5 rounded-full text-[11px] font-semibold bg-[var(--surface-3)] text-[var(--ink-2)] border border-[var(--line)]">
            就绪 (Pending)
          </span>
        );
    }
  };

  return (
    <div className="flex-1 flex flex-col h-full min-h-0 min-w-0 bg-[var(--surface)] text-[var(--ink)] overflow-hidden">
      {/* 顶部标题栏与新建操作 */}
      <div className="pt-4 pb-3 px-6 flex items-center justify-between border-b border-[var(--line)] shrink-0 select-none">
        <div className="flex flex-col">
          <div className="flex items-center gap-2">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--celadon)]">
              <circle cx="12" cy="12" r="10" />
              <polyline points="12 6 12 12 16 14" />
            </svg>
            <h1 className="text-[17px] font-bold tracking-tight">已安排的任务 (Scheduled Tasks)</h1>
          </div>
          <span className="text-[12px] text-[var(--ink-3)] mt-0.5">
            配置和查看后台定时任务、智能体管线执行进度、日志与产物
          </span>
        </div>

        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => void fetchJobs()}
            className="p-1.5 text-[var(--ink-3)] hover:text-[var(--ink)] hover:bg-[var(--surface-2)] rounded-xl transition-colors cursor-pointer"
            title="刷新列表"
          >
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M21.5 2v6h-6M21.34 15.57a10 10 0 1 1-.57-8.38l5.67-5.67" />
            </svg>
          </button>
          <button
            type="button"
            onClick={() => setIsCreating(true)}
            className="flex items-center gap-1.5 px-3.5 py-1.5 bg-[var(--surface-3)] hover:bg-[var(--surface-2)] border border-[var(--line)] rounded-xl text-[12.5px] font-semibold text-[var(--ink)] cursor-pointer transition-all shadow-2xs"
          >
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
              <path d="M12 5v14M5 12h14" />
            </svg>
            <span>新建任务</span>
          </button>
        </div>
      </div>

      {/* 搜索过滤条 */}
      <div className="px-6 py-2.5 border-b border-[var(--line)]/60 flex items-center gap-2 bg-[var(--surface-2)]/30 shrink-0">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-[var(--ink-3)]">
          <circle cx="11" cy="11" r="8" />
          <line x1="21" y1="21" x2="16.65" y2="16.65" />
        </svg>
        <input
          type="text"
          placeholder="搜索已安排任务或 ID..."
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          className="flex-1 bg-transparent border-none outline-none text-[12.5px] text-[var(--ink)] placeholder-[var(--ink-3)]"
        />
      </div>

      {/* 主体双栏视图：左侧任务列表 + 右侧详情 */}
      <div className="flex-1 flex min-h-0 min-w-0 overflow-hidden">
        {/* 左侧任务卡片列表 */}
        <div className="w-[340px] border-r border-[var(--line)] flex flex-col min-h-0 bg-[var(--surface-2)]/40 shrink-0 overflow-y-auto p-3 gap-2 select-none">
          {loading && jobs.length === 0 && (
            <div className="py-8 text-center text-[12.5px] text-[var(--ink-3)]">
              加载任务中...
            </div>
          )}

          {filteredJobs.length === 0 && !loading && (
            <div className="py-12 px-4 text-center flex flex-col items-center gap-2 text-[var(--ink-3)]">
              <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" className="opacity-40">
                <circle cx="12" cy="12" r="10" />
                <polyline points="12 6 12 12 16 14" />
              </svg>
              <span className="text-[13px] font-medium">暂无安排的任务</span>
              <p className="text-[11.5px] leading-relaxed max-w-[220px]">
                点击右上角「+ 新建任务」创建代码重构、测试审查或定时报告任务
              </p>
            </div>
          )}

          {filteredJobs.map((job) => {
            const isSelected = selectedJob?.id === job.id;
            return (
              <div
                key={job.id}
                onClick={() => setSelectedJobId(job.id)}
                className={`p-3.5 rounded-2xl border transition-all cursor-pointer flex flex-col gap-2 relative ${
                  isSelected
                    ? "bg-[var(--surface)] border-[var(--celadon)] ring-2 ring-[var(--celadon)]/15 shadow-xs"
                    : "bg-[var(--surface)]/70 border-[var(--line)] hover:border-[var(--line-2)] hover:bg-[var(--surface)]"
                }`}
              >
                <div className="flex items-start justify-between gap-2">
                  <span className="text-[13.5px] font-bold text-[var(--ink)] leading-snug line-clamp-1">
                    {job.title}
                  </span>
                  <div className="shrink-0">{renderStatusBadge(job.status)}</div>
                </div>

                <div className="flex items-center justify-between text-[11px] text-[var(--ink-3)] pt-1 border-t border-[var(--line)]/40 font-mono">
                  <span>{job.steps.length} 个步骤</span>
                  <span>{new Date(job.created_at_ms).toLocaleDateString()}</span>
                </div>
              </div>
            );
          })}
        </div>

        {/* 右侧任务详情、步骤状态与执行输出 */}
        {selectedJob ? (
          <div className="flex-1 flex flex-col min-h-0 min-w-0 overflow-y-auto p-6 gap-6 bg-[var(--surface)]">
            {/* 任务头部 */}
            <div className="flex items-start justify-between gap-4 pb-4 border-b border-[var(--line)]">
              <div className="flex flex-col gap-1.5 min-w-0">
                <div className="flex items-center gap-2.5">
                  <h2 className="text-[18px] font-bold tracking-tight">{selectedJob.title}</h2>
                  {renderStatusBadge(selectedJob.status)}
                </div>
                <div className="flex items-center gap-3 text-[11.5px] text-[var(--ink-3)] font-mono">
                  <span>ID: {selectedJob.id}</span>
                  <span>类型: {selectedJob.kind}</span>
                  <span>创建时间: {new Date(selectedJob.created_at_ms).toLocaleString()}</span>
                </div>
              </div>

              {/* 任务操作按钮群 */}
              <div className="flex items-center gap-2 shrink-0">
                {selectedJob.status === "paused" && (
                  <button
                    type="button"
                    onClick={() => handleResumeJob(selectedJob.id)}
                    className="flex items-center gap-1.5 px-3.5 py-1.5 rounded-xl bg-emerald-500 hover:bg-emerald-600 text-white text-[12.5px] font-semibold cursor-pointer transition-colors shadow-2xs"
                  >
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor">
                      <polygon points="5 3 19 12 5 21 5 3" />
                    </svg>
                    <span>继续执行</span>
                  </button>
                )}

                {selectedJob.status !== "running" && selectedJob.status !== "paused" && (
                  <button
                    type="button"
                    onClick={() => handleRunJob(selectedJob.id)}
                    className="flex items-center gap-1.5 px-3.5 py-1.5 rounded-xl bg-emerald-500 hover:bg-emerald-600 text-white text-[12.5px] font-semibold cursor-pointer transition-colors shadow-2xs"
                  >
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor">
                      <polygon points="5 3 19 12 5 21 5 3" />
                    </svg>
                    <span>开始执行</span>
                  </button>
                )}

                {selectedJob.status === "running" && (
                  <button
                    type="button"
                    onClick={() => handlePauseJob(selectedJob.id)}
                    className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl bg-amber-500/15 hover:bg-amber-500/25 border border-amber-500/30 text-amber-600 text-[12px] font-medium cursor-pointer transition-colors"
                  >
                    <span>暂停</span>
                  </button>
                )}

                {selectedJob.status !== "cancelled" && selectedJob.status !== "succeeded" && (
                  <button
                    type="button"
                    onClick={() => handleCancelJob(selectedJob.id)}
                    className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl bg-red-500/10 hover:bg-red-500/20 border border-red-500/20 text-red-500 text-[12px] font-medium cursor-pointer transition-colors"
                  >
                    <span>取消</span>
                  </button>
                )}

                <button
                  type="button"
                  onClick={() => handleViewLog(selectedJob.id)}
                  className="flex items-center gap-1.5 px-3 py-1.5 rounded-xl bg-[var(--surface-2)] hover:bg-[var(--surface-3)] border border-[var(--line)] text-[var(--ink)] text-[12px] font-medium cursor-pointer transition-colors"
                >
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                    <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                    <polyline points="14 2 14 8 20 8" />
                  </svg>
                  <span>查看日志与事件</span>
                </button>
              </div>
            </div>

            {/* 错误提示（若存在） */}
            {selectedJob.error && (
              <div className="p-3.5 rounded-2xl bg-red-500/10 border border-red-500/20 text-red-500 text-[12.5px] flex items-center gap-2">
                <span>⚠️ 错误原因: {selectedJob.error}</span>
              </div>
            )}

            {/* 步骤列表 (Step Breakdown) */}
            <div className="flex flex-col gap-3">
              <h3 className="text-[13.5px] font-bold uppercase tracking-wider text-[var(--ink-2)]">
                执行步骤与进度 ({selectedJob.steps.length})
              </h3>

              <div className="flex flex-col gap-2.5">
                {selectedJob.steps.map((step, idx) => {
                  return (
                    <div
                      key={step.id}
                      className="p-4 rounded-2xl border border-[var(--line)] bg-[var(--surface-2)]/50 flex flex-col gap-2.5"
                    >
                      <div className="flex items-center justify-between">
                        <div className="flex items-center gap-2.5">
                          <span className="w-5 h-5 rounded-full bg-[var(--surface-3)] flex items-center justify-center text-[11px] font-mono font-bold text-[var(--ink-2)]">
                            {idx + 1}
                          </span>
                          <span className="text-[13.5px] font-bold text-[var(--ink)]">{step.title}</span>
                        </div>
                        <span className="text-[11.5px] font-medium font-mono text-[var(--ink-3)]">
                          状态: {step.status}
                        </span>
                      </div>

                      {step.message && (
                        <p className="text-[12px] text-[var(--ink-3)] bg-[var(--surface)] p-2.5 rounded-xl font-mono">
                          {step.message}
                        </p>
                      )}

                      {/* 产物 / Artifacts */}
                      {step.artifacts.length > 0 && (
                        <div className="flex flex-wrap gap-2 pt-1 border-t border-[var(--line)]/40">
                          {step.artifacts.map((art, aIdx) => {
                            return (
                              <span key={aIdx} className="px-2.5 py-1 rounded-lg bg-[var(--surface-3)] text-[11px] font-mono text-[var(--celadon)] border border-[var(--line)]">
                                📄 {JSON.stringify(art)}
                              </span>
                            );
                          })}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          </div>
        ) : (
          <div className="flex-1 flex items-center justify-center text-[var(--ink-3)] text-[13px]">
            请在左侧选择一个任务查看详情
          </div>
        )}
      </div>

      {/* 新建任务模态弹窗 */}
      {isCreating && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-6 bg-black/60 backdrop-blur-xs animate-in fade-in duration-150">
          <div className="w-full max-w-lg bg-[var(--surface)] border border-[var(--line)] rounded-3xl p-6 shadow-2xl flex flex-col gap-4">
            <div className="flex items-center justify-between">
              <h3 className="text-[16px] font-bold">新建安排任务 (New Scheduled Task)</h3>
              <button
                type="button"
                onClick={() => setIsCreating(false)}
                className="p-1 rounded-lg text-[var(--ink-3)] hover:text-[var(--ink)] cursor-pointer"
              >
                ✕
              </button>
            </div>

            <div className="flex flex-col gap-3.5">
              <div className="flex flex-col gap-1.5">
                <label className="text-[12px] font-bold text-[var(--ink-2)]">任务名称</label>
                <input
                  type="text"
                  placeholder="例如: 每日代码库健康检查与依赖扫描"
                  value={newTitle}
                  onChange={(e) => setNewTitle(e.target.value)}
                  className="w-full bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-1.5 text-[12.5px] outline-none text-[var(--ink)] focus:border-[var(--celadon)]"
                />
              </div>

              <div className="flex flex-col gap-1.5">
                <label className="text-[12px] font-bold text-[var(--ink-2)]">任务类型</label>
                <select
                  value={newKind}
                  onChange={(e) => setNewKind(e.target.value as RuntimeJobKind)}
                  className="w-full bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-1.5 text-[12.5px] outline-none text-[var(--ink)] cursor-pointer"
                >
                  <option value="generic">通用智能体分析任务 (Generic)</option>
                  <option value="script_planning">脚本剧本编排 (Script Planning)</option>
                  <option value="character_assets">角色资产生成 (Character Assets)</option>
                  <option value="storyboard">分镜生成 (Storyboard)</option>
                  <option value="shot_video">镜头渲染 (Shot Video)</option>
                  <option value="audio_generation">音频与声学合成 (Audio Generation)</option>
                  <option value="episode_assembly">分集装配 (Episode Assembly)</option>
                  <option value="export_jianying">剪映草稿导出 (Export Jianying)</option>
                </select>
              </div>

              <div className="flex flex-col gap-1.5">
                <label className="text-[12px] font-bold text-[var(--ink-2)]">执行步骤 (每行一个步骤)</label>
                <textarea
                  rows={4}
                  value={newSteps}
                  onChange={(e) => setNewSteps(e.target.value)}
                  className="w-full bg-[var(--surface-2)] border border-[var(--line)] rounded-xl p-3 text-[12.5px] outline-none text-[var(--ink)] font-mono focus:border-[var(--celadon)] resize-none"
                />
              </div>
            </div>

            <div className="flex items-center justify-end gap-2 pt-2 border-t border-[var(--line)]">
              <button
                type="button"
                onClick={() => setIsCreating(false)}
                className="px-4 py-1.5 text-[12.5px] font-medium text-[var(--ink-3)] hover:text-[var(--ink)] rounded-xl cursor-pointer"
              >
                取消
              </button>
              <button
                type="button"
                onClick={handleCreateTask}
                disabled={!newTitle.trim()}
                className="px-4 py-1.5 text-[12.5px] font-semibold bg-[var(--celadon)] text-white rounded-xl cursor-pointer disabled:opacity-50 transition-all"
              >
                确认创建
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 日志与事件查看模态弹窗 */}
      {logModalJobId && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-6 bg-black/60 backdrop-blur-xs animate-in fade-in duration-150">
          <div className="w-full max-w-2xl max-h-[85vh] bg-[var(--surface)] border border-[var(--line)] rounded-3xl p-6 shadow-2xl flex flex-col gap-4">
            <div className="flex items-center justify-between">
              <h3 className="text-[15px] font-bold font-mono">任务执行详情与事件日志 (Task Logs)</h3>
              <button
                type="button"
                onClick={() => setLogModalJobId(null)}
                className="p-1 rounded-lg text-[var(--ink-3)] hover:text-[var(--ink)] cursor-pointer"
              >
                ✕
              </button>
            </div>

            <div className="flex-1 min-h-[300px] overflow-y-auto bg-[var(--surface-2)] border border-[var(--line)] rounded-2xl p-4 font-mono text-[11.5px] text-[var(--ink-2)] whitespace-pre-wrap">
              {logLoading ? "加载日志中..." : jobLogContent}
            </div>

            <div className="flex justify-end">
              <button
                type="button"
                onClick={() => setLogModalJobId(null)}
                className="px-4 py-1.5 bg-[var(--surface-3)] text-[12px] font-medium rounded-xl hover:bg-[var(--surface-2)] cursor-pointer"
              >
                关闭
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
