import { describe, expect, it } from "vitest";
import type { ScriptProject } from "../../../api/scriptLibrary";
import { normalizeTitle, resolveScriptProject } from "./scriptLink";

const sp = (id: string, title: string) => ({ id, title, note: "", created_at: 0, updated_at: 0 }) as ScriptProject;

describe("系列 ↔ 剧本编辑器项目", () => {
  const projects = [sp("a", "回收师"), sp("b", "余波")];

  it("书名号、空格不影响按名字认", () => {
    expect(normalizeTitle("《回收师》 ")).toBe("回收师");
    expect(resolveScriptProject(projects, { title: "《回收师》" })).toEqual({ project: projects[0], via: "name" });
  });

  it("绑定的 id 优先，名字对不上也能找到", () => {
    expect(resolveScriptProject(projects, { title: "改过名的系列", script_project_id: "b" })).toEqual({
      project: projects[1],
      via: "id",
    });
  });

  it("绑定的项目被删了就退回按名字", () => {
    expect(resolveScriptProject(projects, { title: "回收师", script_project_id: "gone" })?.via).toBe("name");
  });

  it("同名的不止一个就不猜", () => {
    expect(resolveScriptProject([...projects, sp("c", "《回收师》")], { title: "回收师" })).toBeNull();
  });

  it("对不上就是 null", () => {
    expect(resolveScriptProject(projects, { title: "每日小离谱" })).toBeNull();
  });
});
