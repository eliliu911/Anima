/**
 * 前端测试配置。
 *
 * `include` 必须锁死在 src/ 下。默认的匹配范围会一路扫进
 * `tools/gpt-sovits/upstream/.venv/` —— 那里面装着 gradio，而 gradio 自带
 * 一整套前端测试。跑起来是十几个和本项目无关的失败，把真正的失败淹掉。
 */
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
    // tools/ 下是几个外部服务的完整源码树，不属于本项目的前端
    exclude: ["node_modules/**", "tools/**", "src-tauri/**"],
  },
});
