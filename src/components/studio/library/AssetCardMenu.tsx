/**
 * 资产卡片右下角的「⋯」菜单。删除在菜单里，不直接摆在卡片上（用户 2026-09-30：「不要把删除按钮直接放在卡片上，
 * 在每个卡片右下角增加三个点的菜单按钮，点击弹窗可删除」）。
 *
 * **用应用里现成的那套菜单**（`studio-menu` + `MenuItem`，资产库详情的「⋯」也是它）—— 第一版自己拼了个带标题和描边按钮的小面板，
 * 宽 220px 比卡片还宽，靠左那张卡的菜单被左边裁掉一半（用户截图）。现在 140px：卡片最窄 148px，靠右对齐不会越出卡片左边。
 *
 * 卡片本身是个 `<button>`（点了开工作台），这个菜单放在它旁边的外层里、绝对定位到右下角，点击不冒泡。
 */
import { useState } from "react";
import type { AssetView } from "../../../api/assets";
import { MenuIcon, MenuItem } from "../Menu";
import { useDismissable } from "../useDismissable";
import { deleteLabel, useAssetDelete } from "./DeleteAssetButton";

export function AssetCardMenu({ asset, onDeleted }: { asset: AssetView; onDeleted: () => void }) {
  const [open, setOpen] = useState(false);
  const del = useAssetDelete(asset, () => {
    setOpen(false);
    onDeleted();
  });
  const ref = useDismissable<HTMLDivElement>(open, () => {
    setOpen(false);
    del.cancelArm();
  });

  return (
    <div ref={ref} className="relative" onClick={(e) => e.stopPropagation()}>
      <button
        type="button"
        aria-label={`「${asset.name}」的更多操作`}
        title="更多操作"
        onClick={() => setOpen((v) => !v)}
        className="grid h-6 w-6 place-items-center rounded-md text-[14px] leading-none"
        style={{
          color: "var(--studio-text-muted)",
          background: open ? "var(--studio-bg-elevated)" : "transparent",
        }}
      >
        ⋯
      </button>
      {open && (
        // 往上弹：卡片在最后一排时往下弹会被滚动区裁掉
        <div role="menu" className="studio-menu absolute bottom-full right-0 z-30 mb-1 w-[140px]">
          <MenuItem
            icon={MenuIcon.trash}
            danger
            label={del.busy ? "删除中…" : deleteLabel(del.step, true)}
            onClick={() => void del.advance()}
          />
          {del.msg && (
            <p className="px-2 pb-1 pt-0.5 text-[10.5px] leading-snug" style={{ color: "var(--studio-danger)" }}>
              {del.msg}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
