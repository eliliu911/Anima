import { describe, expect, it } from "vitest";
import { encodeMarkdownLocalImageUrls } from "./MarkdownRenderer";

describe("encodeMarkdownLocalImageUrls", () => {
  it("percent-encodes spaces in local file image URLs", () => {
    const markdown =
      "![Logo](file:///Users/you/Library/Application Support/com.anima.desktop/anima/generated/logo.png)";

    expect(encodeMarkdownLocalImageUrls(markdown)).toBe(
      "![Logo](file:///Users/you/Library/Application%20Support/com.anima.desktop/anima/generated/logo.png)"
    );
  });

  it("keeps already encoded local file image URLs stable", () => {
    const markdown =
      "![Logo](file:///Users/you/Library/Application%20Support/com.anima.desktop/anima/generated/logo.png)";

    expect(encodeMarkdownLocalImageUrls(markdown)).toBe(markdown);
  });
});
