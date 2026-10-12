import { describe, expect, it } from "vitest";
import { srtToVtt } from "./subtitleTrack";

describe("SRT → VTT", () => {
  const srt = "1\n00:00:00,000 --> 00:00:06,000\n大逆不道！\n\n2\n00:00:10,000 --> 00:00:18,000\n这是……唐朝？\n";

  it("加 WEBVTT 头，毫秒的逗号换成点 —— 少一样播放器就一条都不显示", () => {
    const vtt = srtToVtt(srt);
    expect(vtt.startsWith("WEBVTT\n\n")).toBe(true);
    expect(vtt).toContain("00:00:00.000 --> 00:00:06.000");
    expect(vtt).toContain("00:00:10.000 --> 00:00:18.000");
    expect(vtt).not.toContain(",000");
    expect(vtt).toContain("这是……唐朝？");
  });

  it("Windows 换行和首尾空行都不影响", () => {
    expect(srtToVtt("\r\n\r\n" + srt.replace(/\n/g, "\r\n"))).toBe(srtToVtt(srt));
  });
});
