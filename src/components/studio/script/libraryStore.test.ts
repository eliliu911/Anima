import { beforeEach, expect, it, vi } from "vitest";
import { listScriptDocuments, saveScriptDocument, type ScriptDocument } from "../../../api/scriptLibrary";
vi.mock("../../../api/scriptLibrary", () => ({ listScriptDocuments: vi.fn(), saveScriptDocument: vi.fn(), listScriptProjects: vi.fn(async () => []) }));
beforeEach(() => { vi.resetModules(); vi.resetAllMocks(); vi.mocked(listScriptDocuments).mockResolvedValue([]); vi.mocked(saveScriptDocument).mockImplementation(async (d) => ({ ...d, revision: d.revision + 1 })); });
it("creates and saves independent documents without a project", async () => {
  const { useScriptLibrary: store } = await import("./libraryStore");
  await store.getState().load();
  const id = store.getState().add("小说", "第一章");
  await store.getState().flush(id);
  expect(store.getState().documents[0]).toMatchObject({ id, title: "小说", text: "第一章" });
  expect(store.getState().status[id]).toBe("已保存");
});
it("retains edits when the backend rejects a save and can retry", async () => {
  const { useScriptLibrary: store } = await import("./libraryStore");
  await store.getState().load(); const id = store.getState().add(); await store.getState().flush(id);
  store.getState().edit(id, { text: "unsaved" });
  vi.mocked(saveScriptDocument).mockRejectedValueOnce(new Error("disk full"));
  expect(await store.getState().flush(id)).toBe(false);
  expect(store.getState().documents[0].text).toBe("unsaved");
  expect(store.getState().status[id]).toContain("disk full");
  expect(await store.getState().flush(id)).toBe(true);
});
it("late saves do not replace newer draft text", async () => {
  const { useScriptLibrary: store } = await import("./libraryStore");
  await store.getState().load(); const id = store.getState().add(); await store.getState().flush(id);
  let resolve!: (d: ScriptDocument) => void; let submitted!: ScriptDocument;
  vi.mocked(saveScriptDocument).mockImplementationOnce((d) => { submitted = d; return new Promise((r) => { resolve = r; }); });
  store.getState().edit(id, { text: "old" }); const first = store.getState().flush(id); await Promise.resolve();
  store.getState().edit(id, { text: "new" }); resolve({ ...submitted, revision: submitted.revision + 1 }); await first;
  expect(store.getState().documents[0].text).toBe("new");
  expect(store.getState().status[id]).toBe("未保存");
  await store.getState().flush(id);
  expect(store.getState().status[id]).toBe("已保存");
});
it("deduplicates loading so a late list cannot erase a newly created draft", async () => {
  const { useScriptLibrary: store } = await import("./libraryStore");
  const one = store.getState().load(); const two = store.getState().load(); await one; await two;
  expect(listScriptDocuments).toHaveBeenCalledTimes(1);
});

it("refresh picks up documents written outside the app but keeps unsaved local edits", async () => {
  const { useScriptLibrary: store } = await import("./libraryStore");
  const doc = (id: string, text: string, revision = 1): ScriptDocument => ({ id, title: id, text, doc: null, revision, project_id: null, kind: "source", derived_from: null });
  vi.mocked(listScriptDocuments).mockResolvedValue([doc("a", "disk a")]);
  await store.getState().load();
  store.getState().edit("a", { text: "my unsaved edit" });
  // 新建稿还在存（还没落盘）
  vi.mocked(saveScriptDocument).mockImplementationOnce(() => new Promise(() => {}));
  const draft = store.getState().add("新稿");
  await Promise.resolve();
  vi.mocked(listScriptDocuments).mockResolvedValue([doc("a", "disk a v2", 2), doc("b", "imported outside")]);
  await store.getState().refresh();
  const docs = store.getState().documents;
  expect(docs.find((d) => d.id === "b")?.text).toBe("imported outside");
  expect(docs.find((d) => d.id === "a")?.text).toBe("my unsaved edit");
  expect(docs.some((d) => d.id === draft)).toBe(true);
});
