/**
 * Anima Studio 的外壳。
 *
 * # 两级导航
 *
 * - **没进项目时**：全局侧栏（工作区 / 资产库 / 创作台）
 * - **进了项目**：换成六步导航栏（剧本 → 风格 → 资产 → 分镜 → 动效 → 组装）
 *
 * 这是照 LumenX 的结构来的。关键在六步是**路标不是路障** ——
 * 见 `project/StepRail.tsx` 的文件头。
 */
import { onStudioNavigate, takePendingStudioLink, type StudioLink } from "../../api/studioNav";
import type { StepId } from "./project/StepRail";
import { useEffect, useState } from "react";
import { AnimaStudio } from "./AnimaStudio";
import { Toasts } from "./Toasts";
import { useJobToasts } from "./jobs/useJobToasts";
import { StudioSidebar } from "./StudioSidebar";
import { AssetLibraryPage } from "./library/AssetLibraryPage";
import { WorkspacePage } from "./workspace/WorkspacePage";
import { ProjectPage } from "./project/ProjectPage";
import { ScriptsPage } from "./script/ScriptsPage";
import { CanvasView } from "../../canvas/CanvasView";

export type StudioPage = "workspace" | "library" | "scripts" | "canvas" | "workbench";

const STEP_IDS = ["script", "style", "assets", "storyboard", "assembly"] as const;

export function StudioShell() {
  const [page, setPage] = useState<StudioPage>("workspace");
  /** 非 null = 进到某个项目里，此时左侧换成六步导航 */
  const [projectId, setProjectId] = useState<string | null>(null);
  /**
   * 画布当前绑定的项目 ID。**`undefined` = 没指定，画布用它自己记住的那个。**
   * 以前初值是 `null`，画布一打开就当成「切到自由创作」—— 把恢复好的工程换掉，连记住的「上次工程」一起删了
   * （用户 2026-10-13：「没有记录当前选择的工程…每次关闭打开都要重新设置一遍」）
   */
  const [canvasProjectId, setCanvasProjectId] = useState<string | null | undefined>(undefined);
  /** 从剧本一览跳进来时，直接落在那一集的第 1 步 */
  const [entry, setEntry] = useState<{ episodeId?: string; step: StepId } | null>(null);
  /** 每次从 Chat 跳过来都换一个，让项目页按新的分集 / 步骤重新挂（同一个项目里也生效） */
  const [navKey, setNavKey] = useState(0);
  /** 从系列的「故事原文」跳过来：剧本编辑器落到同名的项目 */
  const [scriptsStory, setScriptsStory] = useState<{ cinemaProjectId: string; title: string; scriptProjectId?: string | null; key: number } | null>(null);
  // 后台任务（出图 / 出片 / 生成镜头）的通知条。在两个分支之前挂，切进切出项目都不退订。
  useJobToasts();

  // 从 Anima Chat 跳过来（「在 Studio 打开」）：窗口刚打开时取备份，开着时听事件
  useEffect(() => {
    const go = (l: StudioLink) => {
      setEntry({ episodeId: l.episode_id, step: (STEP_IDS as readonly string[]).includes(l.page ?? "") ? (l.page as StepId) : "script" });
      setProjectId(l.project_id);
      setNavKey((n) => n + 1);
    };
    const pending = takePendingStudioLink();
    if (pending) go(pending);
    let off: (() => void) | undefined;
    let alive = true;
    void onStudioNavigate(go).then((f) => {
      if (alive) off = f;
      else f();
    });
    return () => {
      alive = false;
      off?.();
    };
  }, []);

  if (projectId) {
    return (
      <div className="anima-studio flex h-full overflow-hidden">
        <Toasts />
        <ProjectPage
          key={`${projectId}:${navKey}`}
          projectId={projectId}
          initialEpisodeId={entry?.episodeId}
          initialStep={entry?.step}
          onBack={() => {
            setProjectId(null);
            setEntry(null);
          }}
          onOpenScripts={(story) => {
            setProjectId(null);
            setEntry(null);
            setScriptsStory({ ...story, key: Date.now() });
            setPage("scripts");
          }}
        />
      </div>
    );
  }

  return (
    <div className="anima-studio flex h-full overflow-hidden">
      <Toasts />
      <StudioSidebar
        active={page}
        onSelect={(id) => {
          if (
            id === "workspace" ||
            id === "library" ||
            id === "scripts" ||
            id === "canvas" ||
            id === "workbench"
          ) {
            setPage(id);
          }
        }}
      />
      {page === "workspace" && (
        <WorkspacePage
          onOpen={setProjectId}
          onOpenCanvas={(pid) => {
            setCanvasProjectId(pid);
            setPage("canvas");
          }}
        />
      )}
      {page === "scripts" && (
        <ScriptsPage key={scriptsStory?.key} initialStory={scriptsStory ?? undefined} />
      )}
      {page === "canvas" && (
        <CanvasView
          projectId={canvasProjectId}
          onProjectChange={setCanvasProjectId}
        />
      )}
      {page === "library" && <AssetLibraryPage />}
      {page === "workbench" && <AnimaStudio />}
    </div>
  );
}

export default StudioShell;
