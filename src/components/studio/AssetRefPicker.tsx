/**
 * `@` 引用选择器：从资产库挑一个，插一段 `[角色:老茶师]` 进提示词。
 *
 * 插的是**文本标签**，不是某个隐藏的数据结构 —— 这是从 LumenX 抄来的
 * 最聪明的一处设计：提示词本身就是绑定关系。用户能直接看见、直接改、
 * 直接删，不用先在下拉框里选角色再回来写句子。
 */
import { useEffect, useMemo, useState } from "react";
import {
  assetsList,
  refTag,
  type AssetView,
} from "../../api/assets";
import { MediaThumb } from "./MediaThumb";
import { SidePanel, useSidePanelClose } from "./SidePanel";
import {
  groupAssets,
  loadViewMode,
  saveViewMode,
  type AssetViewMode,
} from "./assetGroups";

export function AssetRefPicker({
  projectId,
  episodeId,
  onClose,
  onPick,
}: {
  /**
   * 不传 = 没有项目上下文（通用创作台），列全部作用域。
   * 在分镜里**必须传** —— 不传就会冒出别的片子的角色。
   */
  projectId?: string;
  episodeId?: string;
  onClose: () => void;
  onPick: (tag: string) => void;
}) {
  const [assets, setAssets] = useState<AssetView[] | null>(null);
  const [q, setQ] = useState("");
  const [view, setView] = useState<AssetViewMode>(loadViewMode);
  const close = useSidePanelClose();
  const pick = (a: AssetView) => {
    onPick(refTag(a.kind, a.name));
    // 走面板自己的关闭 —— 父组件直接卸载的话没有出场动画
    close();
  };

  useEffect(() => {
    // **只列这个项目的：本集 + 本项目。**
    //
    // `assetsList()` 不带参数列的是 `all_scopes` —— 全局 + **每一个项目**
    // 的每一集。于是这里会冒出别的片子的角色，插进去还真能解析
    // （`resolve_refs` 也扫所有作用域），出片时挂上一张完全不相干的脸。
    //
    // **全局库也不列。** 它是跨项目共享的池子，混在这一集的角色里
    // 只是噪音（真事：一个别的片子的角色出现在猫的分镜里）。
    // 要用全局资产，先在第 3 步把它收进项目 —— 那一步本来就是干这个的。
    //
    // 这条是**暂缓**不是定论，见 `docs/DECISIONS.md` 的「待决策事项 ·
    // 分镜里要不要够得着全局资产库」。注意限制的只是选择器列什么，
    // 手打 `[角色:苏菲]` 仍然解析得到（`resolve_refs` 扫所有作用域）。
    //
    // 顺序是"由近及远"：同名时近的排前面，和解析的优先级一致。
    //
    // 没有项目上下文时（通用创作台）保持原样列全部 —— 那里本来就是
    // 「随便挑一张参考图」的地方，缩成只剩全局库等于悄悄拿走了东西。
    let alive = true;
    const load = projectId
      ? Promise.all([
          episodeId
            ? assetsList({ kind: "episode", project_id: projectId, episode_id: episodeId })
            : Promise.resolve([] as AssetView[]),
          assetsList({ kind: "project", project_id: projectId }),
        ]).then(([ep, proj]) => [...ep, ...proj])
      : assetsList();
    void load.then((l) => alive && setAssets(l)).catch(() => alive && setAssets([]));
    return () => {
      alive = false;
    };
  }, [projectId, episodeId]);

  const shown = useMemo(() => {
    const k = q.trim().toLowerCase();
    return (assets ?? []).filter((a) => !k || a.name.toLowerCase().includes(k));
  }, [assets, q]);

  return (
    <SidePanel title="资产库" subtitle="点一下插进提示词" onClose={onClose}>
        <input
          autoFocus
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") onClose();
            if (e.key === "Enter" && shown[0]) onPick(refTag(shown[0].kind, shown[0].name));
          }}
          placeholder="搜资产名字…回车选第一个"
          className="w-full shrink-0 px-4 py-3 text-[13px] outline-none"
          style={{
            background: "transparent",
            borderBottom: "1px solid var(--studio-border-subtle)",
            color: "var(--studio-text)",
          }}
        />
        {/* 平铺 / 列表切换。**平铺是默认** —— 资产靠图认，
            一列小方图比一行文字快得多。名字长的时候列表更好读，
            所以两种都留着，并记住上次的选择。 */}
        <div className="flex shrink-0 items-center justify-end gap-1 px-3 pt-2">
          {([
            ["grid", "平铺"],
            ["list", "列表"],
          ] as const).map(([m, label]) => (
            <button
              key={m}
              type="button"
              aria-pressed={view === m}
              onClick={() => {
                setView(m);
                saveViewMode(m);
              }}
              className="rounded-md px-2 py-1 text-[11px]"
              style={{
                background: view === m ? "var(--studio-bg-inset)" : "transparent",
                color: view === m ? "var(--studio-text)" : "var(--studio-text-muted)",
              }}
            >
              {label}
            </button>
          ))}
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3">
          {assets === null && (
            <p className="px-1 py-3 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
              读取中…
            </p>
          )}
          {assets !== null && shown.length === 0 && (
            <p className="px-1 py-3 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
              {q.trim()
                ? `没有叫「${q.trim()}」的资产`
                : "这个项目还没有资产 —— 去第 3 步从剧本里提取"}
            </p>
          )}
          {groupAssets(shown).map((g) => (
            <section key={g.kind} className="mt-3 first:mt-1">
              <div className="mb-1.5 flex items-center gap-1.5 px-1">
                <span className="studio-eyebrow">{g.label}</span>
                <span
                  className="font-mono text-[0.55rem] tabular-nums"
                  style={{ color: "var(--studio-text-muted)" }}
                >
                  {g.items.length}
                </span>
              </div>
              {view === "grid" ? (
                <div className="grid grid-cols-2 gap-1.5">
                  {g.items.map((a) => (
                    <button
                      key={a.id}
                      type="button"
                      onClick={() => pick(a)}
                      title={a.description || a.name}
                      className="flex flex-col gap-1 rounded-lg p-1.5 text-left"
                      style={{ background: "var(--studio-bg-inset)" }}
                    >
                      <span
                        className="flex aspect-[4/3] w-full items-center justify-center overflow-hidden rounded-md"
                        style={{ background: "var(--studio-bg-base)" }}
                      >
                        {a.cover_url ? (
                          <MediaThumb path={a.cover_url} className="h-full w-full object-contain" />
                        ) : (
                          <span className="text-[9px] opacity-40">无图</span>
                        )}
                      </span>
                      <span className="truncate px-0.5 text-[11.5px]">{a.name}</span>
                    </button>
                  ))}
                </div>
              ) : (
                <div className="flex flex-col">
                  {g.items.map((a) => (
                    <button
                      key={a.id}
                      type="button"
                      onClick={() => pick(a)}
                      title={a.description || a.name}
                      className="flex items-center gap-2.5 rounded-lg px-1.5 py-1.5 text-left"
                    >
                      <span
                        className="flex h-[34px] w-[34px] shrink-0 items-center justify-center overflow-hidden rounded-lg"
                        style={{ background: "var(--studio-bg-inset)" }}
                      >
                        {a.cover_url ? (
                          <MediaThumb path={a.cover_url} className="h-full w-full object-contain" />
                        ) : (
                          <span className="text-[9px] opacity-40">无图</span>
                        )}
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[12.5px]">{a.name}</span>
                        {a.description && (
                          <span
                            className="block truncate text-[10.5px]"
                            style={{ color: "var(--studio-text-muted)" }}
                          >
                            {a.description}
                          </span>
                        )}
                      </span>
                    </button>
                  ))}
                </div>
              )}
            </section>
          ))}
        </div>
    </SidePanel>
  );
}
