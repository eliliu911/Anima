/**
 * 神经数字人窗口里**到底谁在画面上**。
 *
 * ## 为什么值得单独一个函数
 *
 * 这条规则原先藏在一个 className 三元里（`hasWsFrames || !isVideo`），
 * 而它错了：只要 sidecar 在推帧，画布就盖住下面三个 `<video>`。
 * 偏偏 sidecar 一次只持有一个 VideoEngine —— 也就是只有待机那一段。
 *
 * 结果是 **speaking / listening 两段一次都不会出现**，用户看到的是
 * 「永远在播待机」，控制台一行错误都没有，素材、状态机、WebSocket
 * 每一处单独看都是好的。
 *
 * ## 现在的规则
 *
 * 两条路能做的事是互斥的：
 *
 * - 神经流重绘嘴部 —— 实测张到最大和闭嘴在嘴部框内只差 3.3 个灰阶
 *   （底模自身运动是 1.75），肉眼分不出；
 * - 状态机换体态 —— 整段动作都不一样，一眼能看出来。
 *
 * 所以有三段素材时让状态机赢。等 sidecar 能按状态切片段、
 * 或者真换上 wav2lip 之后，这条规则要重写。
 */

export type NeuralLayer = "canvas" | "video";

export interface LayerInput {
  /** 有 idle / speaking / listening 三段动作素材 */
  isMultiClip: boolean;
  /** 素材是视频（否则是静图写真） */
  isVideo: boolean;
  /** sidecar 正在推帧 */
  hasWsFrames: boolean;
  /**
   * sidecar 报告它能做**真口型**（wav2lip 重绘嘴部）。
   *
   * 它在 WebSocket 连上时自报（`{"type":"hello","lipsync":true}`）。
   * 这一位是整条规则的分水岭：能重绘嘴，画布就值得盖上去；
   * 只能做形变，就该让位给三段体态。
   */
  hasLipsync: boolean;
}

export function activeLayer({ isMultiClip, isVideo, hasWsFrames, hasLipsync }: LayerInput): NeuralLayer {
  // 真口型：画布赢。这时 sidecar 同时管口型**和**体态切换 ——
  // 三段素材是它的底模，前端那三个 <video> 反而是多余的
  if (hasLipsync) return hasWsFrames ? "canvas" : "video";
  // 没有真口型、却有三段素材：状态机赢。
  // 让画布盖上去只会让 speaking / listening 永远不出现（那是它原先的 bug）
  if (isMultiClip) return "video";
  // 静图没有可播的视频，只能靠画布 —— 哪怕 sidecar 还没推帧
  if (!isVideo) return "canvas";
  // 单段视频：sidecar 在推就用它的，否则播原片
  return hasWsFrames ? "canvas" : "video";
}

/**
 * 神经流那条路要不要起。
 *
 * **必须先连上才知道有没有口型** —— 能力是 sidecar 在 hello 里报的。
 * 所以这里按「假设它有口型」去问：连上之后如果发现没有、而本地又有三段素材，
 * 显示规则会让画布让位，那时再断开。
 *
 * 断开是有意义的，不只是省资源：连着的话每秒 30 帧 JPEG 要解成 ImageBitmap
 * 再画进一个 `opacity-0` 的画布，实测白烧 1.1MB/s 带宽和一条 GPU 解码路径。
 */
export function neuralStreamWanted(input: LayerInput): boolean {
  if (input.hasLipsync) return true;
  // 还没连上（hasLipsync 未知）时，先连一次去问 —— 但只在
  // 「就算没有口型也用得上画布」的情况下保持连接
  return activeLayer({ ...input, hasWsFrames: true }) === "canvas";
}
