import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import LanguageDetector from "i18next-browser-languagedetector";

import enCommon from "./en-US/common.json";
import enChat from "./en-US/chat.json";
import enSettings from "./en-US/settings.json";
import enTrajectory from "./en-US/trajectory.json";
import enStudio from "./en-US/studio.json";

import zhCommon from "./zh-CN/common.json";
import zhChat from "./zh-CN/chat.json";
import zhSettings from "./zh-CN/settings.json";
import zhTrajectory from "./zh-CN/trajectory.json";
import zhStudio from "./zh-CN/studio.json";

export const resources = {
  "en-US": {
    common: enCommon,
    chat: enChat,
    settings: enSettings,
    trajectory: enTrajectory,
    studio: enStudio,
  },
  "zh-CN": {
    common: zhCommon,
    chat: zhChat,
    settings: zhSettings,
    trajectory: zhTrajectory,
    studio: zhStudio,
  },
} as const;

void i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources,
    fallbackLng: "en-US",
    defaultNS: "common",
    ns: ["common", "chat", "settings", "trajectory", "studio"],
    interpolation: {
      escapeValue: false,
    },
    detection: {
      order: ["localStorage", "navigator"],
      caches: ["localStorage"],
      lookupLocalStorage: "anima_i18n_lang",
    },
  });

import { invoke } from "@tauri-apps/api/core";

// 跨窗口语言同步 (Tauri 独立多 Webview 架构)：
// 1. 监听 window 'storage' 事件（其他 Webview 写入 localStorage 时触发）
if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key === "anima_i18n_lang" && e.newValue) {
      if (i18n.language !== e.newValue) {
        void i18n.changeLanguage(e.newValue);
      }
    }
  });

  // 2. 监听 BroadcastChannel（现代浏览器跨标签页/跨 Webview 标准信道）
  try {
    const channel = new BroadcastChannel("anima_i18n_channel");
    channel.onmessage = (event) => {
      if (event.data?.type === "CHANGE_LANGUAGE" && event.data?.lang) {
        if (i18n.language !== event.data.lang) {
          void i18n.changeLanguage(event.data.lang);
        }
      }
    };
  } catch (err) {
    console.warn("[i18n] BroadcastChannel not supported:", err);
  }
}


/**
 * 启动时 config.json 里的语言要不要补写。
 *
 * **修的是这个 bug：设置里显示简体中文，重启后托盘菜单还是英文。**
 *
 * 托盘是 Rust 在启动时建的，只读得到 `config.json`；而界面语言一直只存在
 * 前端的 localStorage 里。`changeAppLanguage` 会把两边都写，但它**只在用户
 * 主动切换时触发** —— 语言从旧版本沿用下来、一次都没重新选过的话，
 * `config.language` 永远是 `null`，托盘每次启动都退回英文。
 *
 * 实测本机 `config.json` 就是 `"language": null`，而界面显示简体中文。
 *
 * 所以不能等切换事件，启动就得对一次账。
 */
export function languageNeedsSync(
  configLang: string | null | undefined,
  uiLang: string,
): boolean {
  if (!uiLang) return false;
  return (configLang ?? "") !== uiLang;
}

/**
 * 把当前界面语言补写进 config.json（只在和里面存的不一致时写）。
 *
 * 每个 Webview 起来都会跑一次，但值相同时不写，所以不会有多余的落盘。
 */
export async function syncLanguageToConfig(
  readConfigLang: () => Promise<string | null | undefined>,
  writeLang: (lang: string) => Promise<unknown>,
  uiLang: string = i18n.language,
): Promise<boolean> {
  try {
    const configLang = await readConfigLang();
    if (!languageNeedsSync(configLang, uiLang)) return false;
    await writeLang(uiLang);
    return true;
  } catch (err) {
    // 补写失败不能影响界面 —— 顶多托盘这次还是旧语言
    console.warn("[i18n] Failed to sync language to config:", err);
    return false;
  }
}

/** 切换并全局同步语言到所有窗口与原生系统托盘 */
export function changeAppLanguage(lang: string) {
  void i18n.changeLanguage(lang);
  try {
    localStorage.setItem("anima_i18n_lang", lang);
    // **同时落到 config.json。** localStorage 只有前端读得到，而托盘菜单是
    // Rust 在启动时建的 —— 不存这一份的话，每次启动托盘都会回到英文。
    void invoke("set_language", { lang }).catch((err) => {
      console.warn("[i18n] Failed to persist language to config:", err);
    });
    const channel = new BroadcastChannel("anima_i18n_channel");
    channel.postMessage({ type: "CHANGE_LANGUAGE", lang });
  } catch (err) {
    console.warn("[i18n] Failed to broadcast language change:", err);
  }

  // 动态更新 macOS 系统托盘菜单文字
  void invoke("update_tray_menu", { lang }).catch((err) => {
    console.warn("[i18n] Failed to update native tray menu:", err);
  });
}

// 启动即对账。放在模块尾部，等 i18n 的语言探测跑完。
if (typeof window !== "undefined") {
  void syncLanguageToConfig(
    async () => {
      const cfg = (await invoke("get_config")) as { language?: string | null };
      return cfg?.language;
    },
    async (lang) => {
      await invoke("set_language", { lang });
      // 托盘菜单是启动时建好的，补写完要让它立刻跟上，否则要等下次重启
      await invoke("update_tray_menu", { lang });
    },
  );
}

export default i18n;
