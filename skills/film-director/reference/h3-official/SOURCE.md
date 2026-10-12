# MiniMax H3 官方提示词规范（原文，未改动）

| 文件 | 来源 | sha256 |
|---|---|---|
| `VIDEO_PROMPT_WRITING_GUIDE_base_en.md` | https://huggingface.co/MiniMaxAI/MiniMax-H3/raw/42ed227ee7df40d41602854ae760620d6eb651fe/docs/VIDEO_PROMPT_WRITING_GUIDE_base_en.md | 2cfebc096a6e08370f288d468d90b60f7f9bcb938f94bf090816e910e48e75fc |
| `VIDEO_PROMPT_WRITING_GUIDE_ref_en.md` | https://huggingface.co/MiniMaxAI/MiniMax-H3/raw/42ed227ee7df40d41602854ae760620d6eb651fe/docs/VIDEO_PROMPT_WRITING_GUIDE_ref_en.md | 1e574f356716ad55612247ffb7bbccbcdb484ad96599d63c7dca1af186b1fab7 |
| `h3-prompt-writing-SKILL.md` | https://github.com/MiniMax-AI/MiniMax-H3/blob/d21241f0a4b3acbb34c97dae47fa417b7065e438/skills/h3-prompt-writing/SKILL.md（引用文件 base-en.txt / ref-en.txt 与上面两份逐字相同） | a7000443588ca3f145e3b3fd8900f14e0325dc460bd811268fac89a9dc8e56d0 |

取于 2026-10-06。HF 版本 `42ed227ee7df40d41602854ae760620d6eb651fe`，GitHub 版本 `d21241f0a4b3acbb34c97dae47fa417b7065e438`。

- 代码里编译时嵌入的就是这两份（`src-tauri/src/h3_spec.rs`），**更新时整份替换、改 `SOURCE_REVISION`**，不要在原文里改字。
- 官方没公开 Context-IR 的系统提示词；HF 讨论区 #28 / #47 的「系统提示词」是社区写的，不算官方（核查见 `docs/reports/2026-10-06-H3提示词规范核查.md`）。
- 中文摘要在上一级的 `h3-prompt-format.md` / `h3-reference-mode.md`，**以这里的原文为准**。
