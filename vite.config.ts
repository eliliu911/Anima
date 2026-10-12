import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri 会注入这些环境变量，用于区分目标平台
const host = process.env.TAURI_DEV_HOST;

/**
 * 应用**运行时**会往项目目录里写的地方 —— 必须排除在 vite 的 watch 之外。
 *
 * Rust 侧的 `find_project_characters` / `find_project_skills` /
 * `find_project_dir` / `find_project_library` 都优先选项目内的目录，
 * 所以开发期在设置面板里改一次角色绑定，就是往 `characters/<id>/` 里
 * 写 voice.json + avatar.json + skills.json 三个文件。
 *
 * 这些文件不在模块图里，vite 对付不了，于是退回**整页 reload**，
 * 而且是广播给每一个连着的窗口。表现出来就是：改个音色，
 * 所有面板一起白屏，3D 模型从头重新加载，几秒后才回来 ——
 * 看着像渲染层崩了，其实是热更新在尽职工作。
 *
 * 新增「运行时可写的项目目录」时，这个列表要跟着加，
 * 否则那个目录一被写就把整个界面掀一遍。src/viteWatch.test.ts 会盯着这件事。
 */
export const RUNTIME_DIRS = [
  "characters",
  "skills",
  "avatars",
  "voice-library",
  "animations",
  // 2026-09-18 加：`cinema_projects_dir` 改走共用的候选列表之后，仓库里的
  // `projects/` 也会被认。现在仓库里没有这个目录，但**代码能往那儿写就得忽略**
  // —— 否则谁哪天建了它，应用每写一次任务记录 vite 就整页 reload
  "projects",
  // 内置资源：风格缩略图 / 模板示例图。`render-style-thumbs` 会往里写
  "resources",
];

/**
 * 不在模块图里、但会被编辑的目录。
 *
 * `tools/` 下是 Python sidecar 的源码。vite 处理不了 .py，于是退回**整页
 * reload** —— 改一行 sidecar 代码，桌面上所有窗口一起白屏、3D 模型重载。
 * 和 `characters/` 那次是同一个坑，只是这次的触发者是我自己在改代码。
 */
const NON_MODULE_DIRS = ["tools"];

/**
 * **真正被 vite 忽略的那一份**。
 *
 * 分成两个数组是为了说清楚两类目录为什么在这儿（一类应用会写、一类 vite
 * 处理不了），但**不变量是「被忽略」**，所以测试对着这份并集断言 ——
 * 对着其中一个数组断言的话，把目录从 A 挪到 B 就会误报。
 */
export const IGNORED_DIRS = [...RUNTIME_DIRS, ...NON_MODULE_DIRS];

export default defineConfig({
  plugins: [react(), tailwindcss()],

  // Tauri 期望一个固定端口，端口被占用时直接失败而不是静默换端口
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    watch: {
      // src-tauri 由 cargo 自己 watch，vite 不用管
      ignored: ["**/src-tauri/**", ...IGNORED_DIRS.map((d) => `**/${d}/**`)],
    },
  },

  // Tauri 使用 Safari (macOS) / Edge WebView2 (Windows)，可以放心用较新的 target
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari15",
    minify: process.env.TAURI_ENV_DEBUG ? false : "esbuild",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },

  clearScreen: false,
});
