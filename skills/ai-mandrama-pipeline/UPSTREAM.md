# 上游来源

- 仓库：https://github.com/cyuanxv/ai-mandrama-skills
- 提交：`0142fc16742b38b63ca896fd855fb747f5202155`
- 目录：`skills/ai-mandrama-pipeline`
- 许可：MIT（原文见同目录 `LICENSE`）
- 装进 Anima：2026-09-28

## 相对上游的改动

只改了 `SKILL.md` 的 frontmatter：追加 `name_zh`、`marker_zh`、`description_zh`、`source` 四个字段，让 Anima 的技能列表能显示中文名和来源。正文与其它文件和上游一致。

## 在 Anima 里的作用范围

这是**对话技能**：勾选后正文进对话的 system prompt。**工作区的生成按钮不读它**（按钮走 `src-tauri/src/prompts.rs` 的提示词槽位，见 CLAUDE.md「工作区的按钮不走 SKILL」）。
