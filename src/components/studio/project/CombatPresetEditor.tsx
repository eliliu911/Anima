import { useEffect, useState } from "react";
import { listLoras } from "../../../api";
import { COMBAT_STYLES, type CombatPreset } from "../../../api/cinema";

/**
 * **战斗 LoRA 预设**（项目出片设置 `render.combat`）：打斗镜按它的动作类型挂这一组 LoRA + 触发词，
 * 不是打斗镜（`combat_style` 为空）一律不挂。见 Rust `combat::extras_for`。
 *
 * 默认全空 —— 哪类配哪几个、强度多少，等打斗 A/B 定（docs/plans/2026-10-12-H3打斗编排与战斗LoRA测试方案.md）。
 * 强度填 0 等于这一条不挂（零对照用）。
 */
export function CombatPresetEditor({
  value,
  onChange,
}: {
  value: Record<string, CombatPreset> | undefined;
  onChange: (next: Record<string, CombatPreset>) => void;
}) {
  const [names, setNames] = useState<string[]>([]);
  useEffect(() => {
    listLoras().then(setNames).catch(() => setNames([]));
  }, []);
  const presets = value ?? {};

  const put = (style: string, p: CombatPreset) => {
    const next = { ...presets };
    if (!p.loras.length && !p.trigger?.trim()) delete next[style];
    else next[style] = p;
    onChange(next);
  };

  return (
    <div className="flex flex-col gap-2">
      <datalist id="combat-lora-names">
        {names.map((n) => (
          <option key={n} value={n} />
        ))}
      </datalist>
      {COMBAT_STYLES.map(({ id, label }) => {
        const p = presets[id] ?? { loras: [] };
        return (
          <div
            key={id}
            className="rounded-xl px-3 py-2.5"
            style={{ background: "var(--studio-bg-inset)", boxShadow: "0 0 0 1px var(--studio-border-subtle)" }}
          >
            <div className="mb-1.5 flex items-center justify-between">
              <span className="text-[12.5px] font-medium">{label}</span>
              <button
                type="button"
                className="text-[11px]"
                style={{ color: "var(--studio-text-2)" }}
                onClick={() => put(id, { ...p, loras: [...p.loras, { name: "", strength: 0.7 }] })}
              >
                + LoRA
              </button>
            </div>
            {p.loras.map((l, i) => (
              <div key={i} className="mb-1.5 flex items-center gap-1.5">
                <input
                  list="combat-lora-names"
                  value={l.name}
                  placeholder="h3-combat/xxx.safetensors"
                  onChange={(e) => put(id, { ...p, loras: p.loras.map((x, j) => (j === i ? { ...x, name: e.target.value } : x)) })}
                  className="min-w-0 flex-1 rounded-md px-2 py-1 text-[11.5px]"
                  style={{ background: "var(--studio-bg-input)", boxShadow: "0 0 0 1px var(--studio-border-subtle)" }}
                />
                <input
                  type="number"
                  step={0.05}
                  min={0}
                  max={2}
                  value={l.strength}
                  onChange={(e) => put(id, { ...p, loras: p.loras.map((x, j) => (j === i ? { ...x, strength: Number(e.target.value) } : x)) })}
                  className="w-16 rounded-md px-2 py-1 text-[11.5px]"
                  style={{ background: "var(--studio-bg-input)", boxShadow: "0 0 0 1px var(--studio-border-subtle)" }}
                />
                <button
                  type="button"
                  aria-label="移除"
                  className="px-1 text-[12px]"
                  style={{ color: "var(--studio-text-muted)" }}
                  onClick={() => put(id, { ...p, loras: p.loras.filter((_, j) => j !== i) })}
                >
                  ×
                </button>
              </div>
            ))}
            <input
              value={p.trigger ?? ""}
              placeholder="触发词（可空），加在提示词最前面，如 Melee weapon interaction."
              onChange={(e) => put(id, { ...p, trigger: e.target.value })}
              className="w-full rounded-md px-2 py-1 text-[11.5px]"
              style={{ background: "var(--studio-bg-input)", boxShadow: "0 0 0 1px var(--studio-border-subtle)" }}
            />
          </div>
        );
      })}
    </div>
  );
}
