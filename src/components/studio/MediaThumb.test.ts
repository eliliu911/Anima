import { describe, expect, it } from "vitest";
import { mediaSrc, pathFromMediaUrl } from "./MediaThumb";

describe("媒体 URL", () => {
  it("带空格的真实路径来回一趟能还原", () => {
    // 真实路径长这样：`~/Library/Application Support/…`
    const p = "/Users/x/Library/Application Support/com.anima.desktop/a b.png";
    const url = mediaSrc(p);
    expect(url).not.toContain(" ");
    expect(pathFromMediaUrl(url)).toBe(p);
  });

  it("中文路径也能还原", () => {
    const p = "/tmp/资产/老茶师 01.png";
    expect(pathFromMediaUrl(mediaSrc(p))).toBe(p);
  });

  it("已经是绝对路径的原样返回", () => {
    // 调用方两种输入都会给（后端下发 URL，本地选文件给路径），
    // 不能只认一种 —— 分情况处理迟早漏一处。
    expect(pathFromMediaUrl("/tmp/a.png")).toBe("/tmp/a.png");
  });

  it("URL 里没有 p 参数时原样返回，不抛", () => {
    expect(pathFromMediaUrl("anima-media://localhost/")).toBe("anima-media://localhost/");
  });

  it("前端拼的 URL 必须和 Rust 侧 media_url 同构", () => {
    // Rust: format!("anima-media://localhost/?p={}", urlencoding::encode(path))
    expect(mediaSrc("/a/b.png")).toBe("anima-media://localhost/?p=%2Fa%2Fb.png");
  });
});
