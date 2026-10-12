// 应用图标：把设计稿铺成 Tauri 要的全套尺寸。
//
// **设计稿是 `docs/design/icon-mine/icon-source-clean.png`**（1024×1024 RGBA，圆角外透明）。
// 这个脚本只做两件事：把它复制成 `src-tauri/icons/app-icon.png`，再调 `tauri icon` 派生
// 32/64/128/128@2x、icns、ico、Windows Square*、iOS、Android 那一整套。
//
// # 为什么不再是程序化生成
//
// 2026-09-15 之前这里画的是一张占位图（电子云粒子），因为那时还没有设计稿。
// 现在有了真图，占位生成器**必须删掉**：留着的话，谁重跑一次 `npm run gen:icon`
// 就把真图全套覆盖成占位图，而且不报错 —— 只是某天发现图标变回去了。
//
// 换图标：替换那张设计稿（或传路径进来），再跑 `npm run gen:icon`。
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";

const DESIGN = resolve("docs/design/icon-mine/icon-source-clean.png");
const APP_ICON = resolve("src-tauri/icons/app-icon.png");

const source = resolve(process.argv[2] ?? DESIGN);
if (!existsSync(source)) {
  console.error(`找不到设计稿：${source}`);
  console.error("用法：node scripts/gen-icon.mjs [1024×1024 的 PNG 路径]");
  process.exit(1);
}

mkdirSync(dirname(APP_ICON), { recursive: true });
if (source !== APP_ICON) copyFileSync(source, APP_ICON);
console.log(`源图：${source}`);

// tauri icon 自己会算所有尺寸，别在这儿手写缩放 —— icns / ico 的容器格式不是缩放能凑出来的
execFileSync("npx", ["tauri", "icon", APP_ICON], { stdio: "inherit" });
console.log("图标已铺开：src-tauri/icons/");
