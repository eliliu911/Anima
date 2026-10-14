import { useState } from "react";
import type { Exchange, Performance, ShotSegment } from "../../../api/cinema";

/**
 * 时间段卡片里的两个折叠区：**表演细节**（表演合同）和**攻防交换**（2026-10-12）。
 *
 * 镜头表、细化时间线会自动填初稿；用户只在重要镜头上看一眼、改选或清空。
 * 交互是「短语标签 + 少量自由文本」，不给一张 JSON 表单（Codex 方案「二、在现有分镜 UI 中如何呈现」）。
 * 改了合同，提示词指纹跟着变，这一镜标「要重新合成」（Rust `prompt_input::fields_json` 里 segments 整个进指纹）。
 */

const INPUT = "w-full min-w-0 rounded-md px-2 py-1 text-[12px] outline-none";
const INPUT_STYLE = { background: "var(--studio-bg-input)", color: "var(--studio-text)" } as const;

/** 预设标签：label + 什么时候能用（悬停看） */
export const FACE_TAGS: { label: string; when: string }[] = [
  { label: "慢眨", when: "近景 / 特写；思考、克制、疲惫" },
  { label: "短眨", when: "近景；被打断、意外" },
  { label: "下眼睑收紧", when: "近景；警惕、怀疑、压着情绪" },
  { label: "眉心短收", when: "近景；不解、不满" },
  { label: "嘴角停住", when: "近景；话到嘴边收住" },
  { label: "短促失笑", when: "近景 / 中景；荒唐、无奈" },
  { label: "瞳孔高光", when: "只在近景 / 特写，且有可见光源" },
];
export const BODY_TAGS: { label: string; when: string }[] = [
  { label: "吸气", when: "说话前、受惊、要发力" },
  { label: "吐气", when: "放松、认命、说完之后" },
  { label: "吞咽", when: "紧张、心虚、忍住" },
  { label: "肩线起伏", when: "喘、哭、压着情绪" },
  { label: "重心后移", when: "退让、防备、受力" },
  { label: "手指收紧后放开", when: "下决心、放弃" },
  { label: "衣料拉扯", when: "有动作或被拽" },
  { label: "受力回弹", when: "格挡、撞击之后" },
];
export const GAZE_TAGS = ["避开", "追随", "短暂确认", "重新锁定"];
export const MATERIAL_SOURCES = ["雨水", "汗", "泪", "蒸汽", "灰尘", "血", "油污"];

/** 合同里看得见的信号数（和 Rust `combat::signal_count` 同一口径） */
export function signalCount(p: Performance | undefined): number {
  if (!p) return 0;
  const n = (xs?: string[]) => (xs ?? []).filter((x) => x.trim()).length;
  return n(p.face) + n(p.body) + (p.hand?.trim() ? 1 : 0) + (p.material && (p.material.source?.trim() || p.material.motion?.trim()) ? 1 : 0);
}

export function isEmptyPerformance(p: Performance | undefined): boolean {
  if (!p) return true;
  return ![p.performer, p.trigger, p.gaze, p.hand, p.aftermath, p.camera_proof, p.material?.source, p.material?.motion, p.material?.light].some((x) => x?.trim())
    && signalCount(p) === 0;
}

export function isEmptyExchange(e: Exchange | undefined): boolean {
  if (!e) return true;
  return ![e.attacker, e.opening, e.intent, e.attack, e.defense, e.result, e.new_state, e.camera_sound].some((x) => x?.trim()) && e.contact_at == null;
}

/** 点一下加、再点一下去掉 */
export function toggleTag(list: string[] | undefined, tag: string): string[] {
  const xs = list ?? [];
  return xs.includes(tag) ? xs.filter((x) => x !== tag) : [...xs, tag];
}

function Row({ label, value, onChange, onBlur, placeholder }: { label: string; value?: string; onChange: (v: string) => void; onBlur: () => void; placeholder?: string }) {
  return (
    <label className="flex items-start gap-1 text-[12px]">
      <span className="w-14 shrink-0 pt-1" style={{ color: "var(--studio-text-muted)" }}>{label}</span>
      <input className={INPUT} style={INPUT_STYLE} value={value ?? ""} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} onBlur={onBlur} />
    </label>
  );
}

function Tags({ label, tags, picked, onToggle, max }: { label: string; tags: { label: string; when: string }[]; picked?: string[]; onToggle: (t: string) => void; max: number }) {
  const n = (picked ?? []).length;
  return (
    <div className="flex items-start gap-1 text-[12px]">
      <span className="w-14 shrink-0 pt-0.5" style={{ color: "var(--studio-text-muted)" }}>{label}</span>
      <div className="flex flex-wrap gap-1">
        {tags.map((t) => {
          const on = (picked ?? []).includes(t.label);
          return (
            <button key={t.label} type="button" title={`适用：${t.when}`} onClick={() => onToggle(t.label)}
              className="rounded px-1.5 py-0.5 text-[11px]"
              style={{ background: on ? "var(--studio-running-bg)" : "var(--studio-bg-input)", color: on ? "var(--studio-text)" : "var(--studio-text-muted)" }}>
              {t.label}
            </button>
          );
        })}
        {(picked ?? []).filter((p) => !tags.some((t) => t.label === p)).map((p) => (
          <button key={p} type="button" onClick={() => onToggle(p)} className="rounded px-1.5 py-0.5 text-[11px]" style={{ background: "var(--studio-bg-input)", color: "var(--studio-text)" }} title="点一下去掉">{p}</button>
        ))}
        {n > max && <span className="text-[11px]" style={{ color: "var(--studio-pending-fg)" }}>最多 {max} 个</span>}
      </div>
    </div>
  );
}

export function SegmentContracts({ seg, wide, onChange, onCommit, onBlur }: {
  seg: ShotSegment;
  /** 这一镜是远景 / 全景：面部标签提示不要用 */
  wide: boolean;
  /** 打字：只改草稿，失焦时 `onBlur` 保存 */
  onChange: (p: Partial<ShotSegment>) => void;
  /** 点标签、清空：改完**立刻保存**（调用方用改后的那份去存 —— 不能先改再调 save()，那时读到的还是旧草稿） */
  onCommit: (p: Partial<ShotSegment>) => void;
  onBlur: () => void;
}) {
  const [openPerf, setOpenPerf] = useState(false);
  const [openEx, setOpenEx] = useState(false);
  const p = seg.performance ?? {};
  const e = seg.exchange ?? {};
  const setP = (q: Partial<Performance>) => onChange({ performance: { ...p, ...q } });
  const setE = (q: Partial<Exchange>) => onChange({ exchange: { ...e, ...q } });
  const commitP = (q: Partial<Performance>) => onCommit({ performance: { ...p, ...q } });
  const perfFilled = !isEmptyPerformance(seg.performance);
  const exFilled = !isEmptyExchange(seg.exchange);
  const signals = signalCount(seg.performance);

  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-2 text-[12px]">
        <button type="button" onClick={() => setOpenPerf((v) => !v)} style={{ color: "var(--studio-text-muted)" }}>
          {openPerf ? "▾" : "▸"} 表演细节{perfFilled ? ` · ${signals} 个信号` : " · 没写"}
        </button>
        {perfFilled && signals > 4 && <span className="text-[11px]" style={{ color: "var(--studio-pending-fg)" }}>超过 4 个，过度表演</span>}
        <button type="button" onClick={() => setOpenEx((v) => !v)} style={{ color: "var(--studio-text-muted)" }}>
          {openEx ? "▾" : "▸"} 攻防交换{exFilled ? " · 已写" : " · 没写"}
        </button>
      </div>

      {openPerf && (
        <div className="flex flex-col gap-1 rounded-md px-2 py-1.5" style={{ boxShadow: "0 0 0 1px var(--studio-border)" }}>
          <Row label="谁" value={p.performer} onChange={(v) => setP({ performer: v })} onBlur={onBlur} placeholder="人物名，和资产一致" />
          <Row label="触发" value={p.trigger} onChange={(v) => setP({ trigger: v })} onBlur={onBlur} placeholder="听见 / 看见 / 受了什么力" />
          <div className="flex flex-col gap-0.5">
            <Row label="视线" value={p.gaze} onChange={(v) => setP({ gaze: v })} onBlur={onBlur} placeholder="从哪到哪" />
            <div className="ml-14 flex flex-wrap gap-1">
              {GAZE_TAGS.map((t) => (
                <button key={t} type="button" onClick={() => commitP({ gaze: p.gaze?.trim() ? `${p.gaze}，${t}` : t })} className="rounded px-1.5 py-0.5 text-[11px]" style={{ background: "var(--studio-bg-input)", color: "var(--studio-text-muted)" }}>{t}</button>
              ))}
            </div>
          </div>
          {wide && <p className="ml-14 text-[11px]" style={{ color: "var(--studio-pending-fg)" }}>这一镜是远景 / 全景：五官看不清，不要写面部，写姿态、重心、轮廓</p>}
          <Tags label="面部" tags={FACE_TAGS} picked={p.face} onToggle={(t) => commitP({ face: toggleTag(p.face, t) })} max={2} />
          <Tags label="呼吸身体" tags={BODY_TAGS} picked={p.body} onToggle={(t) => commitP({ body: toggleTag(p.body, t) })} max={2} />
          <Row label="手" value={p.hand} onChange={(v) => setP({ hand: v })} onBlur={onBlur} placeholder="手实际在做什么（不是「紧张」）" />
          <div className="flex items-start gap-1 text-[12px]">
            <span className="w-14 shrink-0 pt-1" style={{ color: "var(--studio-text-muted)" }}>物理</span>
            <div className="grid flex-1 grid-cols-3 gap-1">
              <input className={INPUT} style={INPUT_STYLE} list="anima-material-sources" value={p.material?.source ?? ""} placeholder="来源：雨 / 汗 / 蒸汽"
                onChange={(ev) => setP({ material: { ...p.material, source: ev.target.value } })} onBlur={onBlur} />
              <input className={INPUT} style={INPUT_STYLE} value={p.material?.motion ?? ""} placeholder="运动：从发梢甩出两滴"
                onChange={(ev) => setP({ material: { ...p.material, motion: ev.target.value } })} onBlur={onBlur} />
              <input className={INPUT} style={INPUT_STYLE} value={p.material?.light ?? ""} placeholder="光：青色侧逆光可见"
                onChange={(ev) => setP({ material: { ...p.material, light: ev.target.value } })} onBlur={onBlur} />
            </div>
            <datalist id="anima-material-sources">{MATERIAL_SOURCES.map((m) => <option key={m} value={m} />)}</datalist>
          </div>
          <Row label="之后" value={p.aftermath} onChange={(v) => setP({ aftermath: v })} onBlur={onBlur} placeholder="说完 / 动完之后留下的状态（进下一段）" />
          <Row label="怎么拍" value={p.camera_proof} onChange={(v) => setP({ camera_proof: v })} onBlur={onBlur} placeholder="什么景别、焦点、停留能拍到" />
          {perfFilled && (
            <button type="button" className="self-end text-[11px]" style={{ color: "var(--studio-text-muted)" }}
              onClick={() => onCommit({ performance: undefined })}>清空表演细节</button>
          )}
        </div>
      )}

      {openEx && (
        <div className="flex flex-col gap-1 rounded-md px-2 py-1.5" style={{ boxShadow: "0 0 0 1px var(--studio-border)" }}>
          <Row label="发起方" value={e.attacker} onChange={(v) => setE({ attacker: v })} onBlur={onBlur} placeholder="人物名" />
          <Row label="开场" value={e.opening} onChange={(v) => setE({ opening: v })} onBlur={onBlur} placeholder="谁在画左 / 画右、距离、高度、架势、武器状态" />
          <Row label="意图" value={e.intent} onChange={(v) => setE({ intent: v })} onBlur={onBlur} placeholder="读到了什么、想抢什么" />
          <Row label="攻击" value={e.attack} onChange={(v) => setE({ attack: v })} onBlur={onBlur} placeholder="哪只手 / 什么武器、从哪起、什么轨迹、打向哪里" />
          <Row label="防守" value={e.defense} onChange={(v) => setE({ defense: v })} onBlur={onBlur} placeholder="格 / 挡 / 闪 / 卸 / 盾" />
          <Row label="结果" value={e.result} onChange={(v) => setE({ result: v })} onBlur={onBlur} placeholder="命中 / 擦过 / 落空 / 对撞，力和环境的反应" />
          <Row label="新站位" value={e.new_state} onChange={(v) => setE({ new_state: v })} onBlur={onBlur} placeholder="脚下、距离、朝向、武器状态、破绽（下一招从这里开始）" />
          <Row label="镜头声音" value={e.camera_sound} onChange={(v) => setE({ camera_sound: v })} onBlur={onBlur} placeholder="证明这一下的镜头 + 同步音效" />
          <label className="flex items-center gap-1 text-[12px]">
            <span className="w-14 shrink-0" style={{ color: "var(--studio-text-muted)" }}>接触时刻</span>
            <input type="number" step={0.1} min={seg.start} max={seg.end} className="w-24 shrink-0 rounded-md px-2 py-1 text-[12px] outline-none" style={INPUT_STYLE} value={e.contact_at ?? ""}
              placeholder={`${seg.start}~${seg.end}`}
              onChange={(ev) => setE({ contact_at: ev.target.value === "" ? undefined : Number(ev.target.value) })} onBlur={onBlur} />
            <span className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>秒（本镜内）；导出 FCPX 时在这里打冲击帧标记</span>
          </label>
          {exFilled && (
            <button type="button" className="self-end text-[11px]" style={{ color: "var(--studio-text-muted)" }}
              onClick={() => onCommit({ exchange: undefined })}>清空攻防交换</button>
          )}
        </div>
      )}
    </div>
  );
}
