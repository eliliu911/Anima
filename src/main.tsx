import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { AvatarWindow } from "./AvatarWindow";
import { PanelWindow } from "./PanelWindow";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { useVoice } from "./store/voice";
import { useChat } from "./store/chat";
import { getConfig } from "./api";
import type { ThemeMode } from "./api/types";
import { applyThemeMode } from "./components/studio/useThemeMode";
import { installContextMenuGuard } from "./utils/nativeContextMenu";
import "./locales";
import "./theme.css";
import "./studio-theme.css";

// 系统默认的右键菜单（图片上的「Open Image in New Window」那些）在 Tauri 里全都无效 ——
// 每个窗口都走这个入口，所以装在这里（见 utils/nativeContextMenu.ts）
installContextMenuGuard();

const root = document.getElementById("root");
if (!root) throw new Error("找不到 #root 挂载点");

/**
 * 按窗口分流。
 *
 * 角色层（`?window=avatar`）是**独立窗口**，里面只有她 —— 没有侧栏、
 * 没有设置、没有对话。主窗口关掉她也还在桌面上，那正是「桌宠」和
 * 「一个可以缩小的应用」最根本的区别。
 */
const which = new URLSearchParams(location.search).get("window");

/**
 * **每个窗口都要自己订阅语音事件流，而且必须在入口做。**
 *
 * `useVoice().init()` 才是订阅 `anima://voice` 的地方。少了它，麦克风电平、
 * 语音状态、转写结果全都收不到 —— 而且**不报任何错**：麦克风确实开着、
 * Rust 侧确实在听，只是前端一个字都收不到，看起来就是"她听不见我说话"。
 *
 * 这个 bug 犯过两次：第一次是重做外壳时删掉了唯一调用它的组件，第二次是
 * 拆窗口时删掉了 App.tsx。两次都是"删了个组件，顺手把这行也带走了"。
 * 所以现在放在入口文件里 —— 这里不会因为换外壳而消失。
 *
 * 窗口之间不共享 JS 上下文，所以是每个窗口各订阅一份，不是一份广播给所有窗口。
 */
void useVoice.getState().init();
void useVoice.getState().primeEmotion();
void useChat.getState().init();

/**
 * **主题必须在入口应用，每个窗口各来一次。**
 *
 * 放在某个组件里的话，那个组件没挂载的窗口就不跟着翻 ——
 * 比如开着对话窗、Studio 没开，切了主题对话窗纹丝不动。
 * 窗口之间不共享 JS 上下文，所以是各读各的配置，不是广播。
 *
 * 读不到配置就跟随系统 —— 一个偏好读不到，不该把界面弄坏。
 */
void getConfig()
  .then((c) => applyThemeMode((c.theme_mode ?? "system") as ThemeMode))
  .catch(() => applyThemeMode("system"));

/**
 * 没带参数时开对话窗。
 *
 * **原来那个 1100×760 的主窗口已经取消了** —— 它是"一个可以缩小的应用"，
 * 和桌宠形态是矛盾的。现在启动只有角色层，面板全从托盘开。
 */
createRoot(root).render(
  <StrictMode>
    <ErrorBoundary fallbackTitle="应用核心窗口加载异常">
      {which === "avatar" ? <AvatarWindow /> : <PanelWindow label={which ?? "chat"} />}
    </ErrorBoundary>
  </StrictMode>,
);
