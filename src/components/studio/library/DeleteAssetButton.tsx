/**
 * 删资产。**资产页卡片的「⋯」菜单（`AssetCardMenu`）和出图工作台用的是这一个**，规矩和资产库页详情面板里那个一样：
 *
 * 1. 点一下只是「准备删」，按钮变成「再点一次删除」—— 删了连图片文件一起没，不能撤
 * 2. 还有镜头在用它，后端会拦下、把「哪一集哪一镜」写在报错里；看过之后才给「强制删除」
 *
 * 用户 2026-09-30：「你得给我角色删除按钮，不然我没地方删」。资产页点卡片直接开工作台，
 * 删除原来在另一个详情面板里，而那个面板从 5cbc149 起就不出现了 —— 这一页一直没有地方删。
 */
import { useState, type MouseEvent } from "react";
import { assetsDelete, type AssetView } from "../../../api/assets";

export type DeleteStep = "none" | "arm" | "force";

/**
 * 按钮上写什么。抽出来是为了测得到 —— 三步写错一步就是「点了直接删」或者「怎么点都删不掉」。
 * `short`：卡片「⋯」菜单里那一行，地方窄（引用的详情在下面那段报错里）
 */
export function deleteLabel(step: DeleteStep, short = false): string {
  if (step === "arm") return "再点一次删除";
  if (step === "force") return short ? "强制删除" : "强制删除（镜头里留下悬空引用）";
  return short ? "删除资产" : "删除这个资产";
}

/**
 * 删除的那几步：准备删 → 再点一次删 → 被引用拦下之后强制删。卡片的「⋯」菜单和工作台的按钮共用这一份，
 * 两处各写各的话，改了一处的步骤另一处不动
 */
export function useAssetDelete(asset: AssetView, onDeleted: () => void) {
  const [step, setStep] = useState<DeleteStep>("none");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const advance = async () => {
    if (step === "none") {
      setStep("arm");
      return;
    }
    setBusy(true);
    try {
      await assetsDelete(asset.scope, asset.id, step === "force");
      onDeleted();
    } catch (err) {
      // 后端把「哪一集哪一镜在用」写在报错里了 —— 原样显示，看过之后下一次点就是强制删
      setMsg(String(err).replace(/^Error:\s*/, ""));
      setStep("force");
    } finally {
      setBusy(false);
    }
  };
  /** 准备删之后移开就算了；已经看过引用（force）的保留，免得再拦一次 */
  const cancelArm = () => step === "arm" && setStep("none");
  return { step, busy, msg, advance, cancelArm };
}

export function DeleteAssetButton({ asset, onDeleted }: { asset: AssetView; onDeleted: () => void }) {
  const { step, busy, msg, advance, cancelArm } = useAssetDelete(asset, onDeleted);

  const click = async (e: MouseEvent) => {
    // 卡片本身点了是打开工作台 —— 删除这一下不能冒上去
    e.stopPropagation();
    await advance();
  };

  const button = (
    <button
      type="button"
      disabled={busy}
      onClick={(e) => void click(e)}
      onBlur={cancelArm}
      title={`删除「${asset.name}」—— 连图片文件一起删，不能撤`}
      className="rounded-md px-2.5 py-1 text-[11.5px]"
      style={{
        background: step === "none" ? "var(--studio-bg-elevated)" : "var(--studio-failed-fg)",
        color: step === "none" ? "var(--studio-failed-fg)" : "var(--studio-on-primary)",
        boxShadow: "0 0 0 1px var(--studio-border)",
        opacity: busy ? 0.6 : 1,
      }}
    >
      {busy ? "删除中…" : deleteLabel(step)}
    </button>
  );

  return (
    <div className="flex flex-col items-start gap-1">
      {button}
      {msg && (
        <p className="text-[11px] leading-snug" style={{ color: "var(--studio-failed-fg)" }}>
          {msg}
        </p>
      )}
    </div>
  );
}
