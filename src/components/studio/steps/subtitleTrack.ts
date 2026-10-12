/**
 * SRT → WebVTT。**播放器只认 VTT**，而我们出的是 SRT（剪映、达芬奇、播放器通吃的那个）。
 *
 * 两处差别，少一处字幕就整个不显示、而且不报错：
 * - 头一行必须是 `WEBVTT`
 * - 时间戳的毫秒用点，不是逗号
 */
export function srtToVtt(srt: string): string {
  const body = srt
    .replace(/\r\n/g, "\n")
    .replace(/(\d{2}:\d{2}:\d{2}),(\d{3})/g, "$1.$2")
    .trim();
  return `WEBVTT\n\n${body}\n`;
}

/** 给 `<track src>` 用的 blob URL。用完要 `URL.revokeObjectURL` */
export function vttUrl(srt: string): string {
  return URL.createObjectURL(new Blob([srtToVtt(srt)], { type: "text/vtt" }));
}
