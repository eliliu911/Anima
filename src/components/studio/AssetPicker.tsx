import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { studioAssets, studioDeleteAsset, type Asset } from "../../api/studio";
import { assetsList, KIND_LABEL, type AssetView } from "../../api/assets";
import { MediaThumb, pathFromMediaUrl } from "./MediaThumb";
import { SidePanel, useSidePanelClose } from "./SidePanel";

/**
 * 给媒体输入槽位选一张图。
 *
 * # 两个来源，不是一个
 *
 * - **资产库**：有身份的角色/场景/道具，取它当前选中的那张图。
 *   「用老茶师这张脸」应该在这里选，而不是去一堆文件名里认。
 * - **最近产物**：`.anima/generated/` 下刚生成的散图，还没有身份。
 *
 * 只列后者的话，用户就得靠 `img_391322652.png` 这种文件名认图 ——
 * 那正是「资产」和「文件」的区别所在。
 *
 * 浏览资产库本身走的是侧边栏的资产库页，不是这个面板。
 *
 * # 壳走共用的 SidePanel
 *
 * 这里原来自己写了一份，和 `PromptDrawer` 一模一样的漂法：宽度 460、
 * 底色 `--studio-bg-surface`、文本 ✕、没有滑动和 Esc，而且**没有 portal**
 * —— `position: fixed` 的参照物被祖先接管，面板贴不到窗口边。
 * （那个 `className="anima-studio"` 是当时为了让 `--studio-*` 能解析
 * 打的补丁，说明作者撞上过症状但没找到根因。）
 */
export function AssetPicker({
  open,
  onClose,
  onPick,
  onPickAsset,
  /** 只列这一类。不给就都列 */
  only,
}: {
  open: boolean;
  onClose: () => void;
  onPick?: (path: string) => void;
  onPickAsset?: (asset: AssetView) => void;
  only?: "image" | "video";
}) {
  if (!open) return null;
  return (
    <SidePanel title="选一张图" onClose={onClose}>
      <PickerBody onPick={onPick} onPickAsset={onPickAsset} only={only} />
    </SidePanel>
  );
}

/**
 * 面板内容。**单独拎出来是因为 [`useSidePanelClose`] 必须在 `SidePanel`
 * 内部才拿得到** —— 「选中一张就关掉」要走它，直接调 `onClose`
 * 是硬卸载、没有出场动画。
 */
function PickerBody({
  onPick,
  onPickAsset,
  only,
}: {
  onPick?: (path: string) => void;
  onPickAsset?: (asset: AssetView) => void;
  only?: "image" | "video";
}) {
  const { t } = useTranslation(["studio", "common"]);
  const close = useSidePanelClose();
  const [tab, setTab] = useState<"assets" | "recent">("assets");
  const [items, setItems] = useState<Asset[] | null>(null);
  const [assets, setAssets] = useState<AssetView[] | null>(null);
  const [loadError, setLoadError] = useState("");

  // 面板每次打开都是新挂载的（关掉就卸载），所以拉一次就够。
  useEffect(() => {
    studioAssets()
      .then(setItems)
      .catch((e) => { setItems([]); setLoadError(`读取最近产物失败：${e}`); });
    assetsList()
      .then(setAssets)
      .catch((e) => { setAssets([]); setLoadError(`读取资产库失败：${e}`); });
  }, []);

  const shown = (items ?? []).filter((a) => !only || a.kind === only);
  // 只有带图的资产能被选走 —— 列一个空封面的卡片点下去什么都不会发生。
  const pickableAssets = (assets ?? []).filter((a) => !!a.cover_url);

  return (
    <>
      {loadError && <p role="alert" className="px-4 pt-3 text-xs" style={{ color: "var(--studio-failed-fg)" }}>{loadError}</p>}
      {/* 计数挂在标签行右边 —— 共用壳的标题区只放标题，不放动态数字 */}
      <div className="flex shrink-0 items-center gap-1 px-4 pt-3">
        {(["assets", "recent"] as const).map((k) => (
          <button
            key={k}
            type="button"
            onClick={() => setTab(k)}
            className="rounded-full px-3 py-1.5 text-[12px]"
            style={{
              background:
                tab === k ? "var(--studio-bg-elevated)" : "transparent",
              color:
                tab === k ? "var(--studio-text)" : "var(--studio-text-muted)",
            }}
          >
            {k === "assets" ? "资产库" : "最近产物"}
          </button>
        ))}
        {/* 有多少个能选，得看得见 —— 否则空列表和还没读完长得一样 */}
        <span
          className="ml-auto font-mono text-[0.5625rem]"
          style={{ color: "var(--studio-text-muted)" }}
        >
          {tab === "assets" ? pickableAssets.length : shown.length}{" "}
          {t("studio:items", { defaultValue: "项" })}
          {only ? ` · ${only}` : ""}
        </span>
      </div>

      {tab === "assets" && (
        <div className="studio-scroll flex-1 overflow-y-auto p-4">
          {assets === null ? (
            <p
              className="text-[12px]"
              style={{ color: "var(--studio-text-muted)" }}
            >
              {t("studio:loading", { defaultValue: "读取中…" })}
            </p>
          ) : pickableAssets.length === 0 ? (
            <div className="flex h-full flex-col items-center justify-center gap-1.5">
              <p
                className="text-[13px]"
                style={{ color: "var(--studio-text-2)" }}
              >
                资产库里还没有带图的资产
              </p>
              <p
                className="text-[11px]"
                style={{ color: "var(--studio-text-muted)" }}
              >
                在「最近产物」里挑一张，点卡片上的「录入」给它一个身份
              </p>
            </div>
          ) : (
            <div className="grid grid-cols-2 gap-3">
              {pickableAssets.map((a) => (
                <button
                  key={`${a.scope_label}/${a.id}`}
                  type="button"
                  className="studio-card overflow-hidden text-left"
                  onClick={() => {
                    // 取当前选中变体的**本机路径** —— 引擎要的是路径，不是显示 URL。
                    if (onPickAsset) onPickAsset(a);
                    else onPick?.(pathFromMediaUrl(a.cover_url!));
                    close();
                  }}
                >
                  <div
                    className="flex aspect-square w-full items-center justify-center"
                    style={{ background: "var(--studio-bg-inset)" }}
                  >
                    <MediaThumb
                      path={a.cover_url}
                      alt={a.name}
                      className="h-full w-full object-contain"
                    />
                  </div>
                  <div className="px-2.5 py-2">
                    <p className="truncate text-[11px]">{a.name}</p>
                    <p
                      className="truncate font-mono text-[0.5rem]"
                      style={{ color: "var(--studio-text-muted)" }}
                    >
                      {KIND_LABEL[a.kind]} · {a.scope_label}
                    </p>
                  </div>
                </button>
              ))}
            </div>
          )}
        </div>
      )}

      <div
        className="studio-scroll flex-1 overflow-y-auto p-4"
        style={{ display: tab === "recent" ? undefined : "none" }}
      >
        {items === null ? (
          <p
            className="text-[12px]"
            style={{ color: "var(--studio-text-muted)" }}
          >
            {t("studio:loading", { defaultValue: "读取中…" })}
          </p>
        ) : shown.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-1.5">
            <p
              className="text-[13px]"
              style={{ color: "var(--studio-text-2)" }}
            >
              {t("studio:noAsset", { defaultValue: "还没有生成过东西" })}
            </p>
            <p
              className="text-[11px]"
              style={{ color: "var(--studio-text-muted)" }}
            >
              {t("studio:noAssetHint", {
                defaultValue: "生成的图和视频会自动出现在这里",
              })}
            </p>
          </div>
        ) : (
          <div className="grid grid-cols-2 gap-3">
            {shown.map((a) => (
              <div key={a.path} className="studio-card overflow-hidden">
                <button
                  type="button"
                  className="flex aspect-square w-full items-center justify-center"
                  style={{ background: "var(--studio-bg-inset)" }}
                  onClick={() => {
                    if (onPick) {
                      onPick(a.path);
                      close();
                    }
                  }}
                >
                  <MediaThumb
                    path={a.path}
                    alt={a.name}
                    kind={a.kind === "video" ? "video" : "image"}
                    className="h-full w-full object-contain"
                  />
                </button>
                <div className="flex items-center justify-between px-2.5 py-2">
                  <div className="min-w-0">
                    <p className="truncate text-[11px]">{a.name}</p>
                    <p
                      className="font-mono text-[0.5rem]"
                      style={{ color: "var(--studio-text-muted)" }}
                    >
                      {(a.bytes / 1e6).toFixed(2)} MB ·{" "}
                      {new Date(a.at).toLocaleDateString()}
                    </p>
                  </div>
                  {/* 删除是不可逆的，所以要确认一次 */}
                  <button
                    type="button"
                    className="shrink-0 font-mono text-[0.5rem]"
                    style={{ color: "var(--studio-text-muted)" }}
                    onClick={async () => {
                      if (!window.confirm(`删除 ${a.name}？不可恢复`)) return;
                      await studioDeleteAsset(a.path);
                      setItems(await studioAssets());
                    }}
                  >
                    {t("studio:delete", { defaultValue: "删除" })}
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </>
  );
}
