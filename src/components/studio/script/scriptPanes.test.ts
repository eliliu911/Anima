import { describe, it, expect } from "vitest";
import { activeGroup } from "./scriptPanes";
import { groupDocuments } from "./ScriptTree";
import type { ScriptDocument, ScriptProject } from "../../../api/scriptLibrary";

const doc = (
  id: string,
  title: string,
  kind: "source" | "screenplay",
  projectId: string | null,
  extra: Partial<ScriptDocument> = {},
): ScriptDocument =>
  ({
    id,
    title,
    text: "正文".repeat(10),
    kind,
    project_id: projectId,
    updated_at: 0,
    ...extra,
  }) as ScriptDocument;

const proj = (id: string, title: string): ScriptProject =>
  ({ id, title, note: "", created_at: 0 }) as ScriptProject;

const 余波 = proj("p1", "余波");
const 米小团 = proj("p2", "米小团");

const docs = [
  doc("s1", "原著第一章", "source", "p1"),
  doc("e1", "第 1 集", "screenplay", "p1", { derived_from: { source_doc_id: "s1" } as never }),
  doc("e3", "第 3 集", "screenplay", "p1"),
  doc("x", "别的故事", "screenplay", "p2"),
];
const groups = groupDocuments([余波, 米小团], docs, "");

describe("剧本编辑器的两栏", () => {
  describe("右栏显示哪个故事", () => {
    it("跟着左栏的选择", () => {
      expect(activeGroup(groups, "p1")?.title).toBe("余波");
      expect(activeGroup(groups, "p2")?.title).toBe("米小团");
      expect(activeGroup(groups, null)?.title).toBe("未归档");
    });

    /**
     * **选中的项目没了要回落，不能显示空白。**
     *
     * 项目可以被别处删掉（或者换台机器读旧配置）。空白会让人以为稿子丢了 ——
     * 而这个项目里「用户的稿子从界面上消失」已经被当成不可接受的失败样子
     * 处理过一次（见 `groupDocuments` 里那段注释）。
     */
    it("选中的项目没了就回落到第一个", () => {
      expect(activeGroup(groups, "已经删掉的")?.title).toBe("余波");
    });

    it("一个故事都没有时给 null，调用方自己兜", () => {
      expect(activeGroup([], "p1")).toBeNull();
    });

    /**
     * **`undefined`（还没选过）和 `null`（明确选了未归档）不是一回事。**
     *
     * 一开始我把这两个合成了一个 `null`，结果刚进页面默认停在「未归档」上 ——
     * 而未归档往往是最空的那一组，用户第一眼看到的是「我的稿子呢」。
     * 一个值两个意思，实跑第一眼就撞出来了。
     *
     * **注意这条测不到真正的修复点。** 修好它的是组件里
     * `useState<…>(undefined)` 那个初值，而 React state 的初值没有 jsdom
     * 测不了（这个项目没装）。那一处是**看图确认**的。
     * 这条只钉住 `activeGroup` 这一侧的契约。
     */
    it("还没选过时落到第一个故事，不是未归档", () => {
      expect(activeGroup(groups, undefined)?.title).toBe("余波");
      expect(activeGroup(groups, null)?.title).toBe("未归档");
    });
  });
});
