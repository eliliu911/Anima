import { useRef, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { convertFileSrc } from "@tauri-apps/api/core";
import { TaskLedgerCard } from "./TaskLedgerCard";
import { stripActionTags } from "../avatar/actionText";
import { useChat } from "../store/chat";
import { visibleTurns } from "../store/sessionTurns";
import { useProject } from "../store/project";
import { useCodeViewer } from "../store/codeViewer";
import { AgentActivityTrajectory } from "./AgentActivityTrajectory";
import { MarkdownRenderer, ImageArtifactCard, VideoArtifactCard } from "./MarkdownRenderer";
import { StreamStatusBar } from "./StreamStatusBar";
import { openInStudio, studioLinkFromToolResult } from "../api/studioNav";
import { createReplaceDiff, parseRawDiff, type DiffData } from "./DiffViewer";
import { CopyIcon, CheckIcon, TrashIcon, UndoIcon } from "./Icons";

export function getTurnImageCalls(toolCalls: any[], toolResults?: any[]) {
  if (!toolCalls || toolCalls.length === 0) return [];
  const images: Array<{
    prompt: string;
    path?: string;
    width?: number;
    height?: number;
    /** 工具本身报了失败 —— 卡片必须显示失败，不能继续转骨架屏 */
    failed?: boolean;
  }> = [];

  for (const tc of toolCalls) {
    const name = (tc.name || "").toLowerCase();
    // 严禁将 animate_image 误判为生图工具 —— 它是图生视频，必须由 getTurnVideoCalls 处理
    if (name.includes("animate_image") || name.includes("video")) continue;
    if (name.includes("generate_image") || name === "image") {
      let args: any = tc.arguments || tc.args || {};
      if (typeof args === "string") {
        try {
          args = JSON.parse(args);
        } catch {
          args = {};
        }
      }

      let path: string | undefined = undefined;

      // 1. 从 toolResults 中寻找对应 call_id 或结果
      if (toolResults && toolResults.length > 0) {
        // **只按 call id 匹配。**
        //
        // 这里原来还有一条兜底：`|| r.name.includes("generate_image")` ——
        // 意思是"call id 对不上就随便找一条生成图的结果"。后果是
        // **每张卡都拿到第一张图的路径**：用户生成两个不同的 logo，
        // 界面上并排显示两张一模一样的图，而磁盘上那两个文件其实完全不同
        // （821816 vs 737810 字节，SHA 也不一样）。
        //
        // 对不上就该没有路径 —— 进度事件里的 `output_path` 会接手（见
        // `srcFromProgress`），实在没有就显示骨架屏。宁可少显示，
        // 不能张冠李戴。
        const tr = toolResults.find((r) => r.callId === tc.id || r.call_id === tc.id);
        if (tr?.content) {
          const resStr = String(tr.content);
          const match = resStr.match(/!\[.*?\]\((file:\/\/[^\)]+)\)/i) || resStr.match(/(file:\/\/[^\s\)\n]+)/i);
          if (match) {
            path = match[1];
          }
        }
      }

      // 2. 兼容直接挂在 tc 上的 result
      if (!path && tc.result) {
        const resStr = typeof tc.result === "string" ? tc.result : JSON.stringify(tc.result);
        const match = resStr.match(/!\[.*?\]\((file:\/\/[^\)]+)\)/i) || resStr.match(/(file:\/\/[^\s\)\n]+)/i);
        if (match) {
          path = match[1];
        }
      }

      // **工具失败要带出来。**
      //
      // 卡片的失败态原来只认 ComfyUI 的进度事件，可有些失败发生在**提交
      // ComfyUI 之前**（比如 LoRA 名字对不上），那时一个进度事件都不会发 ——
      // 骨架屏就永远转下去。用户看到的是「第二次生成一直卡在生成阶段」。
      const failed =
        toolResults?.find((r) => r.callId === tc.id || r.call_id === tc.id)?.ok === false;

      images.push({
        prompt: args.prompt || "AI 生成图像",
        path,
        width: args.width || 1024,
        height: args.height || 1024,
        failed,
      });
    }
  }
  // 按 prompt 去重，**留下已经有 path 的那一条**。
  //
  // 卡片原来是「一次工具调用一张」。第二张超时后模型会**重试** —— 那是一次
  // 新的工具调用，于是又多一张卡：2 张变 3 张，再重试就是 4 张，只增不减。
  // 用户的原话是「岂不是越加越多？为什么不在第二个骨架屏上更新？」
  //
  // 同一个 prompt 就是同一张预期中的图，重试应当**替换**而不是追加。
  const bySlot = new Map<string, (typeof images)[number]>();
  for (const img of images) {
    const slot = `${img.prompt}\u0001${img.width}x${img.height}`;
    const prev = bySlot.get(slot);
    // 已经出图的优先：重试成功后不该再被后来那次失败的调用盖回骨架屏
    if (!prev || (!prev.path && img.path)) {
      bySlot.set(slot, img);
    }
  }
  return [...bySlot.values()];
}

/**
 * 正文里是否已经把视频**渲染**出来了 —— 而不只是"提到了路径"。
 *
 * 这两件事必须分开。判据原来是 `content.includes(".mp4")`，而模型经常用
 * 大白话写「视频已保存到 /Users/…/a.mp4」—— 裸路径，没有 markdown 语法。
 * 于是流式一结束条件就成立，卡片被卸载，而 MarkdownRenderer 那边又没有
 * 图片语法可渲染，**视频直接消失**。用户看到的是「播放一会儿就自动关闭」。
 *
 * 图片那边一开始就判的是语法（`includes("![")`），视频这里当初写成了判后缀。
 *
 * 传 `path` 时还要求那条 markdown 指向的就是这一段视频 —— 同一轮生成两段时
 * 不能因为另一段被渲染了就把这一段的卡片撤掉。
 */
export function contentRendersVideo(content: string, path?: string): boolean {
  if (content.includes("<video")) return true;
  const embeds = content.match(/!\[[^\]]*\]\([^)]+\)/g) ?? [];
  if (embeds.length === 0) return false;
  if (!path) return embeds.some((e) => /\.(mp4|webm|mov)\b/i.test(e));
  return embeds.some((e) => e.includes(path));
}

export function contentRendersImage(content: string, path?: string): boolean {
  if (content.includes("<img")) return true;
  const embeds = content.match(/!\[[^\]]*\]\([^)]+\)/g) ?? [];
  if (embeds.length === 0) return false;
  if (!path) return embeds.some((e) => /\.(png|jpg|jpeg|webp|gif|svg)\b/i.test(e));

  const encodedPath = encodeURI(path);
  return embeds.some((e) => e.includes(path) || e.includes(encodedPath));
}

/**
 * 从一轮的工具调用里取出视频生成任务，供 `VideoArtifactCard` 渲染。
 *
 * 和 `getTurnImageCalls` 一个模子 —— 分开写是因为参数名不同
 * （`generate_video` 用 `prompt`，`animate_image` 用 `motion_prompt`），
 * 而且视频要把 `duration_secs` 和 call id 一起带出去：进度事件是按这两者
 * 建索引的，少传一个就查不到（见 `VideoArtifactCard` 的注释）。
 *
 * 匹配工具返回**只按 call id**。图片那边曾经有条"对不上就随便找一条"的
 * 兜底，后果是两张卡显示同一张图；视频这里从一开始就不留那个口子。
 */
export function getTurnVideoCalls(toolCalls: any[], toolResults?: any[]) {
  if (!toolCalls || toolCalls.length === 0) return [];
  const videos: Array<{
    callId: string;
    prompt: string;
    path?: string;
    durationSecs: number;
    /** 工具本身报了失败 —— 见 getTurnImageCalls 里同名字段的注释 */
    failed?: boolean;
  }> = [];

  for (const tc of toolCalls) {
    const name = (tc.name || "").toLowerCase();
    if (!name.includes("generate_video") && !name.includes("animate_image")) continue;

    let args: any = tc.arguments || tc.args || {};
    if (typeof args === "string") {
      try {
        args = JSON.parse(args);
      } catch {
        args = {};
      }
    }

    let path: string | undefined;
    const tr = toolResults?.find((r) => r.callId === tc.id || r.call_id === tc.id);
    if (tr?.content) {
      const res = String(tr.content);
      const m =
        res.match(/!\[.*?\]\((file:\/\/[^)]+\.mp4)\)/i) ||
        res.match(/(file:\/\/[^\s)\n]+\.mp4)/i);
      if (m) path = m[1];
    }

    videos.push({
      callId: tc.id,
      prompt: args.prompt || args.motion_prompt || "AI 生成视频",
      path,
      durationSecs: args.duration_secs ?? 2,
      failed: tr?.ok === false,
    });
  }

  // 同一个 prompt 的重试应当**替换**而不是追加 —— 图片那边踩过：
  // 超时重试会让卡片只增不减（"岂不是越加越多"）。视频耗时更长、
  // 重试更容易发生，这里从一开始就去重。
  const bySlot = new Map<string, (typeof videos)[number]>();
  for (const v of videos) {
    const slot = `${v.prompt}\u0001${v.durationSecs}`;
    const prev = bySlot.get(slot);
    if (!prev || (!prev.path && v.path)) bySlot.set(slot, v);
  }
  return [...bySlot.values()];
}

function getTurnEditsSummary(toolCalls: any[]) {
  if (!toolCalls || toolCalls.length === 0) return null;
  let totalAdditions = 0;
  let totalDeletions = 0;
  const files = new Set<string>();
  let firstDiff: DiffData | null = null;

  for (const tc of toolCalls) {
    if (
      tc.name === "replace_file_content" ||
      tc.name === "edit_file" ||
      tc.name === "write_file" ||
      tc.name === "write_text_file" ||
      tc.name === "apply_patch"
    ) {
      let diff: DiffData | null = null;
      if (tc.name === "replace_file_content" || (tc.args?.search && tc.args?.replace)) {
        const file = tc.args?.TargetFile || tc.args?.target_file || tc.args?.path || "file";
        files.add(file);
        diff = createReplaceDiff(
          file,
          tc.args?.StartLine || tc.args?.start_line || 1,
          tc.args?.TargetContent || tc.args?.target_content || tc.args?.search || "",
          tc.args?.ReplacementContent || tc.args?.replacement_content || tc.args?.replace || ""
        );
      } else if (tc.args?.patch) {
        diff = parseRawDiff(String(tc.args.patch), tc.args?.path || "modified_file");
        if (diff.fileName) files.add(diff.fileName);
      } else if (tc.result) {
        diff = parseRawDiff(String(tc.result), tc.args?.path || "modified_file");
        if (diff.fileName) files.add(diff.fileName);
      }

      if (diff) {
        totalAdditions += diff.additions;
        totalDeletions += diff.deletions;
        if (!firstDiff) firstDiff = diff;
      }
    }
  }

  if (files.size === 0 && totalAdditions === 0 && totalDeletions === 0) return null;
  return {
    fileCount: files.size || 1,
    totalAdditions,
    totalDeletions,
    firstDiff,
  };
}

export function ChatHistory() {
  const { t } = useTranslation(["chat", "common"]);
  const { activeSessionId } = useProject();
  const allTurns = useChat((s) => s.turns);
  // 只显示当前会话的轮次。
  //
  // `turns` 里可能躺着**别的会话**里还在流的轮次 —— 它们必须留在 store 里，
  // 否则增量就写进了空气（切走再切回来内容全没，见 `loadSession` 的注释）；
  // 但它们不该显示在这儿。没有 `sessionId` 的是语音轮次之类，一律显示。
  const turns = visibleTurns(allTurns, activeSessionId);
  const deleteTurn = useChat((s) => s.deleteTurn);
  const rollbackTurn = useChat((s) => s.rollbackTurn);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const bottomRef = useRef<HTMLDivElement>(null);

  // Scroll to bottom
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ block: "end" });
  }, [turns.length, turns[turns.length - 1]?.content]);

  const handleCopy = (id: string, text: string) => {
    void navigator.clipboard.writeText(stripActionTags(text));
    setCopiedId(id);
    setTimeout(() => setCopiedId(null), 1500);
  };

  const handleRollback = async (turnId: string) => {
    const text = await rollbackTurn(turnId);
    if (text) {
      // 触发自定义事件将撤销的文本填回输入框
      window.dispatchEvent(new CustomEvent("anima:set-draft", { detail: text }));
    }
  };

  return (
    <div className="flex flex-col flex-1 min-h-0 w-full bg-[var(--surface)] min-w-0 overflow-hidden">
      {/* 任务台账。**常驻在滚动区之外** —— 它是宏观里程碑，不该跟着消息
          列表滚走。和 AgentActivityTrajectory（微观物理操作）明确分层。 */}
      <TaskLedgerCard />

      <div className="flex flex-col gap-[11px] overflow-y-auto overflow-x-hidden flex-1 min-h-0 px-4 pt-[18px] pb-4 pr-3.5 [scrollbar-width:thin] [scrollbar-color:var(--ink-3)_transparent]">
        {turns.length === 0 && (
          <div className="text-[13px] text-[var(--ink-2)] mt-4">
            {t("emptyPrompt", { defaultValue: "还没有对话。在下方输入，或点麦克风开始语音。" })}
          </div>
        )}

        {turns.map((turn) => (
          // data-turn-id 是侧栏 Artifacts 点击时滚过来用的锚点。
          // 生成的图片/视频**已经在对话里渲染好了**，点侧栏应该滚到它，
          // 而不是塞进代码查看器 —— 那里只能显示代码和 diff，PNG 会变成乱码。
          <div
            key={turn.id}
            data-turn-id={turn.id}
            className="msg flex flex-col gap-[3px] group relative"
          >
            {/* 标签与操作按钮行 */}
            <div
              className={`msg__who text-[10.5px] tracking-[.08em] uppercase text-[var(--ink-3)] font-semibold flex items-center gap-2 ${
                turn.role === "user" ? "self-end flex-row-reverse" : "self-start"
              }`}
            >
              <div className="flex items-center">
                <span>{turn.role === "user" ? "你" : "Anima"}</span>
                {turn.model && <span className="ml-1.5 opacity-60 text-[9px]">{turn.model}</span>}
                {turn.durationSeconds && !turn.streaming && (
                  <span className="ml-1.5 opacity-50 text-[9px] font-mono">· {turn.durationSeconds}s</span>
                )}
                {turn.streaming && <span className="ml-1.5 text-[var(--celadon)]">生成中</span>}
                {turn.cancelled && <span className="ml-1.5 text-[var(--amber)]">已打断</span>}
                {/* 撞轮次上限和正常结束在视觉上必须能区分。
                    之前只写日志，用户看到的是一段半截的话就停了 ——
                    分不清是模型不行还是我们截断了它。 */}
                {turn.roundsExhausted && (
                  <span
                    className="ml-1.5 text-[var(--amber)] cursor-help"
                    title={
                      "工具往返次数用尽（本地 12 轮 / 云端 25 轮），Anima 收掉工具让它收场，" +
                      "所以回答可能是半截的。\n\n可以直接说「继续」让它接着做，" +
                      "或者把任务拆小一点。切到后台链路能有 25 轮。"
                    }
                  >
                    工具轮次已用尽 ⓘ
                  </span>
                )}
              </div>

              {/* 悬停操作工具栏 (复制、撤销/回滚至此、物理删除) */}
              {!turn.streaming && (
                <div className="opacity-0 group-hover:opacity-100 transition-opacity flex items-center gap-1 bg-[var(--surface-2)]/90 backdrop-blur-xs border border-[var(--line)] rounded-md px-1 py-0.5 shadow-2xs">
                  {turn.content && (
                    <button
                      type="button"
                      onClick={() => handleCopy(turn.id, turn.displayText ?? turn.content)}
                      className="p-1 hover:text-[var(--ink)] hover:bg-[var(--surface-3)] rounded text-[var(--ink-3)] transition-colors cursor-pointer"
                      title={copiedId === turn.id ? "已复制" : "复制正文"}
                    >
                      {copiedId === turn.id ? <CheckIcon className="w-3.5 h-3.5" /> : <CopyIcon className="w-3.5 h-3.5" />}
                    </button>
                  )}

                  {turn.role === "user" && (
                    <button
                      type="button"
                      onClick={() => void handleRollback(turn.id)}
                      className="p-1 hover:text-[var(--celadon)] hover:bg-[var(--surface-3)] rounded text-[var(--ink-3)] transition-colors cursor-pointer"
                      title="撤销并回滚至此 (Undo changes up to this point)"
                    >
                      <UndoIcon className="w-3.5 h-3.5" />
                    </button>
                  )}

                  <button
                    type="button"
                    onClick={() => void deleteTurn(turn.id)}
                    className="p-1 hover:text-red-500 hover:bg-[var(--surface-3)] rounded text-[var(--ink-3)] transition-colors cursor-pointer"
                    title="从数据库物理删除此条记录"
                  >
                    <TrashIcon className="w-3.5 h-3.5" />
                  </button>
                </div>
              )}
            </div>

            {turn.role === "assistant" && (turn.reasoning || turn.toolCalls.length > 0 || turn.streaming) && (
              /* streaming 必须是真实的流状态。曾经写成
                 `turn.streaming && !turn.content` —— 模型一吐出第一个正文字
                 它就变 false，计时器的 effect 直接 early return，秒数冻在
                 那一刻，而这一轮后面还有好几个工具往返在跑。
                 用户看到的就是"时间停了但界面还在转"，很像卡死。 */
              <AgentActivityTrajectory
                reasoning={turn.reasoning}
                toolCalls={turn.toolCalls}
                streaming={turn.streaming}
                startedAt={turn.startedAt}
                durationSeconds={turn.durationSeconds}
                streamedChars={turn.streamedChars}
                usage={turn.usage}
              />
            )}

            {turn.images && turn.images.length > 0 && (
              <div className={`flex gap-2 flex-wrap my-1 ${turn.role === "user" ? "self-end justify-end" : "self-start"}`}>
                {turn.images.map((img, i) => {
                  const resolvedImgSrc = (() => {
                    if (img.startsWith("data:") || img.startsWith("http://") || img.startsWith("https://") || img.startsWith("asset:")) {
                      return img;
                    }
                    const clean = img.replace(/^file:\/\//, "");
                    try {
                      return convertFileSrc(clean);
                    } catch {
                      return img;
                    }
                  })();

                  return (
                    <img
                      key={i}
                      src={resolvedImgSrc}
                      alt="attachment"
                      onClick={() => {
                        void useCodeViewer.getState().openFile(img);
                      }}
                      className="max-w-[240px] max-h-[180px] object-cover rounded-xl border border-[var(--line)] shadow-2xs cursor-pointer hover:opacity-90 transition-opacity"
                    />
                  );
                })}
              </div>
            )}

            {turn.content && (
              <div className={`bubble text-[13px] leading-[1.65] ${
                turn.role === "user"
                  ? "bg-[var(--paper)] border border-[var(--line)] text-[var(--ink)] self-end py-2 px-3.5 rounded-2xl rounded-tr-xs max-w-[85%] shadow-2xs whitespace-pre-wrap break-words"
                  : "text-[var(--ink)] self-start py-1 px-0.5 max-w-[96%] w-full"
              }`}>
                {turn.role === "assistant" ? (
                  <MarkdownRenderer content={stripActionTags(turn.content)} />
                ) : (
                  /* displayText 只影响显示：斜杠命令展开后是几百字的规程，
                     气泡里该显示的是用户敲的那一行 `/review src/lru.ts`。
                     发给模型和算历史预算用的仍然是 content。 */
                  stripActionTags(turn.displayText ?? turn.content)
                )}
              </div>
            )}

            {/* Studio 工具建了 / 改了东西：给一个「在 Studio 打开」。一轮里多个取最后一个 */}
            {turn.role === "assistant" && !turn.streaming && (() => {
              const link = [...(turn.toolResults ?? [])]
                .reverse()
                .filter((r) => r.name?.startsWith("studio_") && r.ok !== false)
                .map((r) => studioLinkFromToolResult(r.content))
                .find(Boolean);
              if (!link) return null;
              return (
                <button
                  type="button"
                  onClick={() => void openInStudio(link)}
                  className="self-start mt-1 rounded-full border border-[var(--line)] bg-[var(--surface-2)] px-3 py-1 text-[12px] text-[var(--ink)] hover:bg-[var(--surface-3)]"
                >
                  在 Studio 打开 →
                </button>
              );
            })()}

            {/* 流式生成期间/未完全写入 Markdown 时的直接图片骨架屏与展示卡片 */}
            {turn.role === "assistant" && (() => {
              const imageCalls = getTurnImageCalls(turn.toolCalls, turn.toolResults);
              if (imageCalls.length === 0) return null;

              // 只在正文里真的已经渲染出这张图时撤卡片。
              // 单纯看到 `![` 不能算数：流式输出里 markdown 语法会先吐半截，
              // 这时卸载稳定卡片会让用户看到图片消失、骨架屏闪回。
              const pendingImages = imageCalls.filter(
                (img) => !img.path || !contentRendersImage(turn.content, img.path),
              );
              if (pendingImages.length === 0) return null;

              return (
                <div className="flex flex-wrap gap-3 my-2 self-start w-full">
                  {pendingImages.map((img) => (
                    <ImageArtifactCard
                      key={`${img.prompt}\u0001${img.width || 1024}x${img.height || 1024}`}
                      src={img.path}
                      alt={img.prompt}
                      width={img.width}
                      height={img.height}
                      failed={img.failed}
                    />
                  ))}
                </div>
              );
            })()}

              {/* 视频生成卡片。
                  `VideoArtifactCard` 早就写好并导出了，但**一个调用方都没有** ——
                  所以生成出来的视频只在模型自己把 markdown 抄进正文时才显示，
                  和图片修好之前是同一个形态（"提示已写入本地文件，但 Chat 里没有"）。 */}
              {turn.role === "assistant" && (() => {
                const videoCalls = getTurnVideoCalls(turn.toolCalls, turn.toolResults);
                if (videoCalls.length === 0) return null;

                // 只有正文**真的渲染出**视频了才撤卡片；仅仅提到路径不算
                if (!turn.streaming && contentRendersVideo(turn.content)) return null;

                const pending = videoCalls.filter(
                  (v) => !v.path || !contentRendersVideo(turn.content, v.path),
                );
                if (pending.length === 0) return null;

                return (
                  <div className="flex flex-wrap gap-3 my-2 self-start w-full">
                    {pending.map((v) => (
                      <VideoArtifactCard
                        key={v.callId}
                        src={v.path}
                        alt={v.prompt}
                        durationSecs={v.durationSecs}
                        callId={v.callId}
                        failed={v.failed}
                      />
                    ))}
                  </div>
                );
              })()}

            {/* Antigravity 风格文件变更统计与 Review 胶囊 */}
            {turn.role === "assistant" && (() => {
              const summary = getTurnEditsSummary(turn.toolCalls);
              if (!summary) return null;

              return (
                <div className="flex items-center gap-2 mt-1.5 self-start">
                  <button
                    type="button"
                    onClick={() => {
                      if (summary.firstDiff) {
                        useCodeViewer.getState().openDiff(summary.firstDiff);
                      }
                    }}
                    className="flex items-center gap-2 px-2.5 py-1 rounded-xl bg-[var(--surface-2)]/90 hover:bg-[var(--surface-3)] border border-[var(--line)] text-[11.5px] font-mono text-[var(--ink)] cursor-pointer transition-colors shadow-2xs group"
                    title={t("reviewChanges", { defaultValue: "在右侧查看代码差异 Review" })}
                  >
                    <span className="font-sans font-medium text-[var(--ink-2)]">
                      {t("filesModified", { count: summary.fileCount, defaultValue: `${summary.fileCount} 个文件已修改` })}
                    </span>
                    <span className="text-emerald-500 font-bold">+{summary.totalAdditions}</span>
                    <span className="text-rose-500 font-bold">-{summary.totalDeletions}</span>
                    <span className="text-[var(--ink-3)] group-hover:text-[var(--ink)]">&gt;</span>
                    <span className="px-2 py-0.5 rounded bg-[var(--surface-3)] text-[10.5px] font-sans font-semibold text-[var(--celadon)] border border-[var(--line)] flex items-center gap-1">
                      <CopyIcon className="w-3 h-3" />
                      <span>{t("common:review", { defaultValue: "Review" })}</span>
                    </span>
                  </button>
                </div>
              );
            })()}

            {turn.error && <div className="text-[11.5px] text-[var(--clay)] bg-[color-mix(in_srgb,var(--clay)_10%,transparent)] border border-[color-mix(in_srgb,var(--clay)_32%,transparent)] rounded-md p-2 mt-1">{turn.error}</div>}
          </div>
        ))}
        <div ref={bottomRef} />
      </div>

      {/* 生成状态钉在这里 —— 滚动区**之外**，所以正文再长也顶不走它。
          它是等待期间唯一能证明"还在推进"的东西，跑掉了就等于没有。 */}
      <StreamStatusBar />
    </div>
  );
}
