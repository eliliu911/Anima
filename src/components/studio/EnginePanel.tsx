import { useTranslation } from "react-i18next";
import type { EngineSpec, ModeSpec } from "../../api/studio";
import { LoraPanel } from "./LoraPanel";
import { ParamsPanel } from "./ParamsPanel";

/**
 * 引擎与参数。
 *
 * **只渲染这个引擎真正支持的控件。** 步数为 `null` 就不显示 —— 视频那两条
 * 由预设内部决定，暴露出来只会被人按 SDXL 的习惯改坏（Z-Image 传 20 步慢
 * 2.5 倍还不更好）。每个引擎的实测提示直接显示出来，不藏进文档。
 */
/** 分节线。LumenX 用它把「模型」和「参数」隔开，这里沿用同一个节奏。 */
function Divider() {
  return <div className="h-px" style={{ background: "var(--studio-border-subtle)" }} />;
}

export function EnginePanel({
  engines,
  engine,
  onChange,
  mode,
}: {
  engines: EngineSpec[];
  engine: EngineSpec | null;
  onChange: (id: string) => void;
  mode: ModeSpec | null;
}) {
  const { t } = useTranslation(["studio", "common"]);

  return (
    <div className="flex flex-col gap-3">
      {/* 引擎多，排两行：一行挤六个时每个按钮被压成三四行竖排（2026-09-15 用户截图） */}
      <div
        className={engines.length > 4 ? "studio-seg studio-seg--grid" : "studio-seg"}
        style={engines.length > 4 ? ({ "--seg-cols": Math.ceil(engines.length / 2) } as React.CSSProperties) : undefined}
      >
        {engines.map((e) => (
          <button
            key={e.id}
            type="button"
            data-active={e.id === engine?.id}
            onClick={() => onChange(e.id)}
          >
            {e.label}
          </button>
        ))}
      </div>

      {engine && (
        <>
          <Divider />
          <ParamsPanel engine={engine} />

          {/* **风格化必须排在引擎之后** —— 它依赖引擎：
              哪些 LoRA 能用是按架构过滤的，架构不匹配挂上去
              是 0 adapters 且不报错。放在引擎之前会让人先选了
              LoRA 再换引擎，然后发现选的全没了。 */}
          {engine.loras && (
            <>
              <Divider />
              <div className="studio-eyebrow">
                {t("studio:loraLabel", { defaultValue: "风格化 LORA" })}
              </div>
              <LoraPanel engine={engine} />
            </>
          )}

          {/* 实测提示：为什么某个框被禁用了、这个引擎有什么脾气 */}
          {engine.notes.length > 0 && (
            <ul className="flex flex-col gap-1">
              {engine.notes.map((n, i) => (
                <li
                  key={i}
                  className="text-[11px] leading-relaxed"
                  style={{ color: "var(--studio-text-muted)" }}
                >
                  · {n}
                </li>
              ))}
            </ul>
          )}
        </>
      )}

      {engines.length === 0 && mode && (
        <p className="text-[12px]" style={{ color: "var(--studio-failed-fg)" }}>
          {t("studio:noEngine", {
            mode: mode.label,
            defaultValue: "「{{mode}}」没有可用引擎",
          })}
        </p>
      )}
    </div>
  );
}
