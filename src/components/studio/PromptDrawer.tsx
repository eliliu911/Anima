import { useEffect, useMemo, useState } from "react";
import { ComposingInput } from "../ComposingInput";
import { useTranslation } from "react-i18next";
import {
  studioDeleteTemplate,
  studioHistory,
  studioSaveTemplate,
  studioTemplates,
  type PromptHistoryItem,
  type PromptTemplate,
} from "../../api/studio";
import { SidePanel, useSidePanelClose } from "./SidePanel";

export type DrawerKind = "history" | "templates" | null;

/**
 * 右侧滑出抽屉：Prompt 历史 / 模板。
 *
 * **壳走共用的 [`SidePanel`]。** 这里原来自己写了一份 —— 宽度 380 vs 340、
 * 底色用 `--studio-bg-surface` 而不是 `-elevated`、没有滑入滑出、
 * 没有 Esc、关闭按钮是个文本 ✕ 而不是图标。用户一眼就看出来
 * 「这个和 QUEUE 不是一个东西」。
 *
 * 更要命的是它**没有 portal**：`position: fixed` 的参照物会被祖先链上任何
 * 带 transform / filter / backdrop-filter 的元素接管，而 Studio 里这三样
 * 到处都是。表现就是面板不贴窗口顶边、高度也不满 —— CSS 没错，参照物错了，
 * 而且不报任何错。（它那个 `className="anima-studio"` 是当时为了让
 * `--studio-*` 变量能解析而打的补丁，说明作者撞上过症状但没找到根因。）
 *
 * **点一条就填进输入框并关闭** —— 这两个面板的唯一用途是「拿回一段文字」，
 * 多一步确认只是碍事。关闭要走 [`useSidePanelClose`]，不能直接调 `onClose`：
 * 后者是硬卸载，没有出场动画，同一个面板会出现「有时候会滑、有时候不会」。
 */
export function PromptDrawer({
  kind,
  onClose,
  onPick,
  currentPrompt,
  currentKind,
}: {
  kind: DrawerKind;
  onClose: () => void;
  onPick: (text: string) => void;
  currentPrompt: string;
  /** 当前是图像还是视频，用来给新模板预设分类 */
  currentKind: string;
}) {
  const { t } = useTranslation(["studio", "common"]);
  // **hook 全在提前返回之前** —— 见 src/hooksOrder.test.ts。
  if (!kind) return null;
  return (
    <SidePanel
      title={
        kind === "history"
          ? t("studio:drawerHistory", { defaultValue: "Prompt 历史" })
          : t("studio:drawerTemplates", { defaultValue: "Prompt 模板" })
      }
      onClose={onClose}
    >
      <DrawerBody
        kind={kind}
        onPick={onPick}
        currentPrompt={currentPrompt}
        currentKind={currentKind}
      />
    </SidePanel>
  );
}

/**
 * 抽屉内容。**单独拎出来是因为 [`useSidePanelClose`] 必须在 `SidePanel`
 * 内部才拿得到** —— 而「选中一条就关掉」一定要走它，直接调父组件的
 * `onClose` 是硬卸载、没有出场动画，同一个面板会出现
 * 「有时候会滑、有时候不会」。
 */
function DrawerBody({
  kind,
  onPick,
  currentPrompt,
  currentKind,
}: {
  kind: Exclude<DrawerKind, null>;
  onPick: (text: string) => void;
  currentPrompt: string;
  currentKind: string;
}) {
  const { t } = useTranslation(["studio", "common"]);
  const close = useSidePanelClose();
  const [q, setQ] = useState("");
  const [tab, setTab] = useState("all");
  const [history, setHistory] = useState<PromptHistoryItem[]>([]);
  const [templates, setTemplates] = useState<PromptTemplate[]>([]);
  /** 正在填的新模板。null = 没在新建。 */
  const [draft, setDraft] = useState<{ title: string; body: string } | null>(null);

  useEffect(() => {
    if (kind === "history")
      studioHistory()
        .then(setHistory)
        .catch(() => {});
    if (kind === "templates")
      studioTemplates()
        .then(setTemplates)
        .catch(() => {});
    setQ("");
  }, [kind]);

  const shownHistory = useMemo(
    () =>
      history.filter((h) =>
        q.trim()
          ? h.prompt.toLowerCase().includes(q.trim().toLowerCase())
          : true,
      ),
    [history, q],
  );
  const shownTemplates = useMemo(
    () => templates.filter((x) => tab === "all" || x.kind === tab),
    [templates, tab],
  );

  /**
   * 存成模板。
   *
   * # 以前这里是两个 `window.prompt`
   *
   * Tauri 的 WKWebView **没有实现 `window.prompt`** —— 不弹框、不报错，
   * 直接返回 null。所以这个「新建模板」按钮点下去**什么都不发生**，
   * 而且一句提示也没有。同一个坑在剧本库的「新建项目」上先撞过一次。
   *
   * 就地填还顺手解决了原来那个设计问题：第二个框里要重打一遍模板内容，
   * 而人想存的几乎总是**输入框里现成的那一段**。现在它是预填的。
   */
  async function saveTemplate() {
    if (!draft || !draft.title.trim()) return;
    setTemplates(
      await studioSaveTemplate({
        id: `${Date.now()}`,
        title: draft.title.trim(),
        body: draft.body,
        kind: currentKind,
      }),
    );
    setDraft(null);
  }

  const tabs = [
    { id: "all", label: t("studio:tabAll", { defaultValue: "全部" }) },
    { id: "image", label: t("studio:tabImage", { defaultValue: "图像" }) },
    { id: "video", label: t("studio:tabVideo", { defaultValue: "视频" }) },
    { id: "general", label: t("studio:tabGeneral", { defaultValue: "通用" }) },
  ];

  return (
    <>
      {kind === "history" ? (
        <input
          className="studio-input mx-4 mt-3 px-3 py-2 text-[13px]"
          placeholder={t("studio:searchHistory", {
            defaultValue: "搜索历史 prompt…",
          })}
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
      ) : (
        <div className="studio-seg mx-4 mt-3">
          {tabs.map((x) => (
            <button
              key={x.id}
              type="button"
              data-active={tab === x.id}
              onClick={() => setTab(x.id)}
            >
              {x.label}
              {x.id !== "all" && (
                <span style={{ opacity: 0.5 }}>
                  {" "}
                  {templates.filter((v) => v.kind === x.id).length}
                </span>
              )}
            </button>
          ))}
        </div>
      )}

      <div className="studio-scroll flex-1 overflow-y-auto px-4 py-3">
        {kind === "history" &&
          (shownHistory.length === 0 ? (
            <Empty
              text={t("studio:noHistory", { defaultValue: "暂无生成历史" })}
            />
          ) : (
            shownHistory.map((h) => (
              <button
                key={`${h.at}`}
                type="button"
                onClick={() => {
                  onPick(h.prompt);
                  close();
                }}
                className="mb-2 w-full rounded-lg px-3 py-2 text-left"
                style={{ background: "var(--studio-bg-inset)" }}
              >
                <p className="line-clamp-3 text-[12px] leading-relaxed">
                  {h.prompt}
                </p>
                <span
                  className="font-mono text-[0.5625rem]"
                  style={{ color: "var(--studio-text-muted)" }}
                >
                  {h.mode} · {h.engine} · {new Date(h.at).toLocaleString()}
                </span>
              </button>
            ))
          ))}

        {kind === "templates" &&
          (shownTemplates.length === 0 ? (
            <Empty
              text={t("studio:noTemplate", { defaultValue: "暂无模板" })}
              hint={t("studio:noTemplateHint", {
                defaultValue: "点下方「新建」把当前 prompt 存成模板",
              })}
            />
          ) : (
            shownTemplates.map((x) => (
              <div
                key={x.id}
                className="mb-2 rounded-lg px-3 py-2"
                style={{ background: "var(--studio-bg-inset)" }}
              >
                <div className="flex items-center justify-between">
                  <button
                    type="button"
                    className="flex-1 text-left text-[13px] font-medium"
                    onClick={() => {
                      onPick(x.body);
                      close();
                    }}
                  >
                    {x.title}
                  </button>
                  <button
                    type="button"
                    className="font-mono text-[0.5625rem]"
                    style={{ color: "var(--studio-text-muted)" }}
                    onClick={async () =>
                      setTemplates(await studioDeleteTemplate(x.id))
                    }
                  >
                    删除
                  </button>
                </div>
                <p
                  className="line-clamp-2 text-[11px] leading-relaxed"
                  style={{ color: "var(--studio-text-muted)" }}
                >
                  {x.body}
                </p>
              </div>
            ))
          ))}
      </div>

      {kind === "templates" &&
        (draft ? (
          <div
            className="flex shrink-0 flex-col gap-2 px-4 py-3"
            style={{ borderTop: "1px solid var(--studio-border-subtle)" }}
          >
            {/* 模板名几乎必然是中文 —— 普通受控 input 会把拼音串原样打进去，
                见 `ComposingInput`。 */}
            <ComposingInput
              autoFocus
              value={draft.title}
              onCommit={(v) => setDraft((d) => (d ? { ...d, title: v } : d))}
              onKeyDown={(e) => {
                if (e.key === "Escape") setDraft(null);
              }}
              placeholder={t("studio:tplTitle", { defaultValue: "模板名称" })}
              className="rounded bg-black/15 px-2 py-1.5 text-[12.5px]"
            />
            <textarea
              value={draft.body}
              onChange={(e) => setDraft((d) => (d ? { ...d, body: e.target.value } : d))}
              rows={4}
              placeholder={t("studio:tplBody", { defaultValue: "模板内容" })}
              className="resize-none rounded bg-black/15 px-2 py-1.5 text-[12px] leading-relaxed"
            />
            <div className="flex justify-end gap-2 text-[12px]">
              <button type="button" onClick={() => setDraft(null)}
                style={{ color: "var(--studio-text-muted)" }}>
                {t("common:cancel", { defaultValue: "取消" })}
              </button>
              {/* 没名字存不了 —— 但**不要禁用得一声不吭**，占位符已经说了要填什么 */}
              <button type="button" disabled={!draft.title.trim()}
                onClick={() => void saveTemplate()}
                style={{ color: draft.title.trim() ? "var(--studio-primary)" : "var(--studio-text-muted)" }}>
                {t("common:save", { defaultValue: "保存" })}
              </button>
            </div>
          </div>
        ) : (
          <button
            type="button"
            // 内容**预填当前输入框那一段** —— 想存模板的人手里就是它。
            onClick={() => setDraft({ title: currentPrompt.slice(0, 20), body: currentPrompt })}
            className="shrink-0 px-4 py-3 text-left text-[13px]"
            style={{ borderTop: "1px solid var(--studio-border-subtle)" }}
          >
            + {t("studio:newTemplate", { defaultValue: "新建模板" })}
          </button>
        ))}
    </>
  );
}

function Empty({ text, hint }: { text: string; hint?: string }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-1.5">
      <p className="text-[13px]" style={{ color: "var(--studio-text-2)" }}>
        {text}
      </p>
      {hint && (
        <p
          className="text-[11px]"
          style={{ color: "var(--studio-text-muted)" }}
        >
          {hint}
        </p>
      )}
    </div>
  );
}
