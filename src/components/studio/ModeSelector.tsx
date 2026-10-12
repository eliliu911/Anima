import type { CapabilityCatalog } from "../../api/studio";

/**
 * 模式选择器 —— 分「图像」「视频」两组，照搬 LumenX 的分段控件。
 *
 * **不可用的模式不隐藏，是禁用 + 说明为什么。** 隐藏会让人以为功能不存在；
 * 禁用并写清「只有 SDXL 支持」才是有信息量的。
 */
/** 按钮上放不下的完整说明。 */
const MODE_HINT: Record<string, string> = {
  t2i: "从文字生成图片",
  i2i: "以一张图为基础重画（修图 / 换风格）",
  sheet: "角色设定卡：一张图里出多个视角，给跨镜头一致性用",
  pose: "用一张姿势参考图锁住动作（ControlNet OpenPose）",
  t2v: "从文字生成视频，带音轨",
  i2v: "用一张图当首帧生成视频",
  fl2v: "给首帧和末帧，生成中间的过程",
  r2v: "参考图生视频：用参考图锁住人物身份，首帧不是参考图本身",
};

export function ModeSelector({
  catalog,
  value,
  onChange,
}: {
  catalog: CapabilityCatalog;
  value: string;
  onChange: (mode: string) => void;
}) {
  const groups = [
    { kind: "image", label: "图像" },
    { kind: "video", label: "视频" },
  ];
  return (
    <div className="flex flex-col gap-3">
      {groups.map((g) => (
        <div key={g.kind}>
          <div
            className="mb-1.5 font-mono text-[0.5625rem] uppercase tracking-[0.14em]"
            style={{ color: "var(--studio-text-muted)" }}
          >
            {g.label}
          </div>
          <div className="studio-seg">
            {catalog.modes
              .filter((m) => m.kind === g.kind)
              .map((m) => {
                const only =
                  m.engines.length === 1
                    ? catalog.engines.find((e) => e.id === m.engines[0])?.label
                    : null;
                return (
                  <button
                    key={m.id}
                    type="button"
                    data-active={m.id === value}
                    // **完整说明放悬浮，不放按钮上** —— 按钮是等宽分段，
                    // 长名字会换行把整排撑高
                    title={[m.label, MODE_HINT[m.id], only ? `只有 ${only} 支持` : ""]
                      .filter(Boolean)
                      .join("\n")}
                    onClick={() => onChange(m.id)}
                    aria-label={m.label}
                  >
                    {m.short}
                  </button>
                );
              })}
          </div>
        </div>
      ))}
    </div>
  );
}
