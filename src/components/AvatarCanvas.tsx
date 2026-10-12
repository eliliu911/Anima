import { useEffect, useRef } from "react";
import * as THREE from "three";

/**
 * 电子云 / 关系图谱核心。
 *
 * 目标形态：**离散的粒子云能聚合成一张会发光的关系图谱**（JARVIS 那种感觉）。
 * 为此有三件事必须做对，之前的版本正是这三点没做到：
 *
 * 1. **体积，不是壳**。粒子要填满三维体积、半径按 `cbrt(random)` 分布，
 *    转动时才有前后视差、才像一团云。之前用 `pow(shell, 0.32)` 把几乎所有粒子
 *    推到同一半径，结果是一颗磨砂实心球，完全没有纵深。
 * 2. **要有光**。加法混合 + 双层精灵（紧实亮核 + 大而淡的光晕）。
 *    之前 alpha 上限只有 0.2 且没有辉光，所以又暗又平。
 * 3. **连线要连真实节点**。图谱感来自「节点—边」结构：先撒 N 个节点锚点，
 *    粒子归属到节点，边取节点间 k 近邻；聚合度上升时边才浮现 ——
 *    这才是「像在思考」。之前是 hash 抖动出的锯齿折线，看着像故障噪点。
 *
 * 运动全部在顶点着色器里按「位置 = f(种子, 时间, uniform)」解析计算，
 * 所以 5.8 万粒子每帧的 CPU 开销是零，也不需要 GPGPU 乒乓纹理那套复杂度。
 */

/** 关系图谱的节点数。太多糊成一团，太少撑不起「图谱」感 */
const NODE_COUNT = 96;
/** 粒子总数，每节点约 600 */
const PARTICLE_COUNT = 58_000;
/** 每节点连出的近邻数（去重后实际度数略高） */
const NEIGHBORS_PER_NODE = 2;

export type AvatarState = "idle" | "listening" | "thinking" | "speaking";

export interface AvatarCanvasProps {
  state: AvatarState;
  /** 音量 0..1，驱动脉动 */
  level?: number;
  /** 全局动态速度倍率。1 = 原始速度（很快），0.35 是默认值 */
  speed?: number;
  /**
   * 情感轴对形态的修正。null = 未启用性格系统，按状态机原值走。
   *
   * 刻意做成**在状态目标之上的偏移**而不是替换：状态（待机/聆听/思考/回应）
   * 决定基本形态，情感只是在此之上偏移一点。反过来的话，"心情不好"会让
   * 思考状态看起来和待机一样，用户就分不出它在干什么了。
   */
  emotion?: {
    affection: number;
    mood: number;
    energy: number;
    /** 焦躁强度 0..1，由 mood 与 energy 推出 */
    irritation: number;
  } | null;
  className?: string;
}

/** 每种状态的目标参数。切换由每帧阻尼插值完成，不跳变。 */
const STATE_TARGETS: Record<
  AvatarState,
  { cohesion: number; energy: number; spin: number; cluster: number }
> = {
  // 离散、缓慢呼吸
  idle: { cohesion: 0.18, energy: 0.25, spin: 0.022, cluster: 0.46 },
  // 向内收拢，绷住等着听
  listening: { cohesion: 0.40, energy: 0.55, spin: 0.029, cluster: 0.32 },
  // 完全聚合成图谱，边亮起、能量在边上跑
  thinking: { cohesion: 0.78, energy: 1.0, spin: 0.045, cluster: 0.34 },
  // 保持图谱但更松更亮，跟着声音鼓动
  speaking: { cohesion: 0.60, energy: 0.85, spin: 0.037, cluster: 0.38 },
};

// cluster 是"每颗粒子离它的节点多远"。之前聚合态给到 0.1~0.16，粒子几乎全挤在
// 96 个节点上 —— 加上加法混合的辉光，看起来就是几团发光的球，而不是一张图谱。
// 提到 0.32~0.38 之后节点之间会重叠成连续的云，结构才看得出来。
// cohesion 也不再给到 1.0：完全聚合等于把流场彻底关掉，云会变成标本。
//
// spin 的档位差刻意压到 2 倍以内（idle 0.022 → thinking 0.045）。
//
// 之前是 0.022 / 0.04 / 0.09 / 0.06：一开语音就进 listening，转速立刻接近翻倍，
// 识别和思考时是 idle 的 4 倍。上一轮"降速"只改了下面那个常量整体自转
// （0.055 → 0.018），这四档一个没动 —— 于是按 idle 校准过的观感，
// 一进语音就全废了。活跃度要靠亮度、聚合度和边上跑的能量来表达，
// 不该靠把转速翻几倍。

/** 确定性随机，保证每次刷新形态一致（否则每次热重载都换一张脸） */
function makeRandom(seed: number) {
  let s = seed >>> 0;
  return () => {
    s ^= s << 13;
    s ^= s >>> 17;
    s ^= s << 5;
    s >>>= 0;
    return s / 0xffffffff;
  };
}

/**
 * 两处着色器共用的节点动画。
 *
 * 粒子和连线必须用**完全相同**的函数算节点位置，否则连线会从节点上脱开 ——
 * 动起来时这种错位特别刺眼。
 */
const NODE_MOTION_GLSL = /* glsl */ `
  vec3 animatedNode(vec3 base, float seed, float time, float spin) {
    // **整体同步旋转**，不能每个节点各转各的。
    // 边是按节点基准位置的 k 近邻算出来的；一旦每个节点按自己的角速度乱转，
    // 原本相邻的节点就会跑散，边退化成横穿整个体积的长弦 —— 看起来就是
    // 一团乱麻而不是图谱。这一版就是这么翻车的。
    float a = time * spin;
    float c = cos(a);
    float s = sin(a);
    vec3 p = vec3(base.x * c - base.z * s, base.y, base.x * s + base.z * c);

    // 个体扰动幅度必须很小，只为了让节点"活着"而不破坏邻接关系
    p += vec3(
      sin(time * 0.7 + seed * 11.0),
      sin(time * 0.62 + seed * 7.0),
      cos(time * 0.66 + seed * 13.0)
    ) * 0.028;
    return p;
  }

  // 类 curl 的流场。三角函数堆出来的伪旋量：够便宜，视觉上足够有机。
  //
  // **必须除以 2**：每个分量是两个正弦之和，幅值可达 ±2，而且三轴独立。
  // 不归一化就会把粒子往立方体的八个角上推 —— 整团云的轮廓会被撑成方的，
  // 这正是之前"为什么是正方形"的根因。
  vec3 flow(vec3 p, float t) {
    return vec3(
      sin(p.y * 1.7 + t * 0.55) + cos(p.z * 1.3 - t * 0.41),
      sin(p.z * 1.5 - t * 0.47) + cos(p.x * 1.9 + t * 0.36),
      sin(p.x * 1.4 + t * 0.43) + cos(p.y * 1.6 - t * 0.52)
    ) * 0.5;
  }

  /**
   * 把位置柔和地约束回球内，保证轮廓是圆的。
   *
   * 光归一化 flow 还不够：多层扰动叠加后总有粒子被推到球外，而"哪个方向被推得
   * 更远"是不均匀的，轮廓就会出现棱角。这里超过半径后按渐进方式压回，
   * 不是硬截断（硬截断会在边缘堆出一层壳）。
   */
  vec3 roundOff(vec3 p, float maxR) {
    float r = length(p);
    if (r <= maxR) return p;
    float over = r - maxR;
    // 超出部分按 1/(1+x) 衰减，越远压得越狠，但永远不会完全贴死在球面上
    return p * ((maxR + over / (1.0 + over * 3.0)) / r);
  }
`;

const particleVertex = /* glsl */ `
  attribute vec3 aNodeBase;
  attribute float aNodeSeed;
  attribute vec3 aOffset;
  attribute vec3 aWander;
  attribute float aSeed;
  attribute float aSize;
  attribute float aHue;

  uniform float uTime;
  uniform float uCohesion;
  uniform float uEnergy;
  uniform float uSpin;
  uniform float uCluster;
  uniform float uLevel;
  uniform float uPixelRatio;
  uniform float uSizeScale;
  uniform float uAlphaScale;
  /// 中空程度：把粒子沿半径往壳上搬
  /// 全局动态速度。**所有**由时间驱动的运动都乘它 —— 之前每一项各有自己的
  /// 系数，降速得逐项去改，而我连续四次都改错了项（真正的主体是流场速率，
  /// 不是任何一项旋转）。有了这个总闸，调一个数就能整体快慢。
  uniform float uSpeed;
  uniform float uShell;
  /// 挤压/拉伸，由弹簧速度驱动（正=向内收，负=外放）
  uniform float uSquash;
  /// 重力下坠量，只在外放时非零
  uniform float uSag;

  varying float vHue;
  varying float vAlpha;
  varying float vCore;

  ${NODE_MOTION_GLSL}

  void main() {
    // 所有时间驱动的运动都用 T，不再直接用 uTime
    float T = uTime * uSpeed;

    // 聚合态：团聚在自己的节点周围
    vec3 node = animatedNode(aNodeBase, aNodeSeed, T, uSpin);
    vec3 aggregated = node + aOffset * uCluster;

    // 每颗粒子有自己的时间流速与相位。速度差必须保留 —— 所有粒子共用同一组
    // 三角频率时，整团会像熔岩灯一样"黏糊糊"地整体蠕动。
    //
    // **但整体速率必须降下来：流场是画面里最快的运动。**
    // 按诊断读数算过：旧的 0.55~1.70 让 flow 的相位以 0.3~1.26 rad/s 推进，
    // 也就是 5~20 秒一个周期 —— 比整体自转（349 秒）、节点公转（285 秒）、
    // 差速剪切（56~180 秒）快**一个数量级**，它才是"疯狂旋转"的主体。
    // 而且它是无序翻搅而非刚性旋转，所以看起来"不知道转的是什么"。
    //
    // 前两轮降速我都刻意跳过了这里（注释里写着"流动感靠 flow，不能碰"），
    // 那个判断是错的：要的是"流动"而不是"翻腾"，慢三倍照样有流动感。
    // 系数保持"原始速度"，降速统一交给 uSpeed 总闸。
    // 之前把这里砍到 1/3、剪切也砍到 1/3、再乘 0.35 的总闸 —— 净剩约 1/9 到
    // 1/27，结果是几乎静止：拖滑杆看不出变化，聚合态更是完全不动。
    // 分母不能一层层往下砍，否则旋钮就失去了跨度。
    float rate = 0.55 + fract(aSeed * 7.31) * 1.15;
    float t = T * rate + aSeed * 19.0;

    // 离散态：铺满整个体积，随流场翻搅
    vec3 dispersed = aWander + flow(aWander * 0.9, t) * 0.34;

    vec3 p = mix(dispersed, aggregated, uCohesion);

    // 有机漂移。聚合时压小，否则图谱抖得看不清结构
    // 聚合态的漂移下限也从 0.022 提到 0.06：全压掉之后图谱是一具标本，
    // 留一点让它"呼吸"
    p += flow(p * 0.75, t * 1.35) * mix(0.16, 0.06, uCohesion);

    // 差速自转：越靠外转得越慢，像真实流体的剪切层。
    // 这一条比加大扰动更能制造"在流动"的感觉。
    float shear = (1.0 - smoothstep(0.0, 1.3, length(p))) * 0.55 + 0.25;
    // **这一项是画面里转得最快的运动，也是"疯狂旋转"的真凶。**
    //
    // 靠界面上的诊断读数才定位到：fps 60（只有一个循环）、自转 0.0180、
    // spin 0.0290、uTime 与秒表一致 —— 全部正常。也就是说前面几轮针对
    // rotation.y 和 uTime 的修改都修错了地方，那些值本来就是对的。
    //
    // 旧系数 0.16/0.05 换算下来：待机（聚合 0.18）时 mix≈0.140，乘上 shear
    // 最大 0.8 得 0.112 rad/s —— **56 秒一圈**。而整体自转是 349 秒一圈、
    // 节点公转 217 秒一圈，这一项比它们快 3~5 倍，完全主导了观感。
    // 而且它在**待机时最快**（聚合越低系数越大），这就是"什么都没做反而狂转"。
    //
    // 降到 0.045/0.02：待机约 200 秒一圈，与另外两项同一个量级。
    // 依然只碰转速，不碰 flow 的速率与幅度 —— 流动感靠那两个。
    // 聚合态的下限从 0.02 提到 0.10：0.02 乘上总闸之后等于停住，
    // 而"思考中"恰恰该显得活跃 —— 一动不动看起来像卡死了
    float a = T * shear * mix(0.16, 0.10, uCohesion);
    float ca = cos(a);
    float sa = sin(a);
    p = vec3(p.x * ca - p.z * sa, p.y, p.x * sa + p.z * ca);

    // 声音脉动：整体呼吸 + 每颗相位不同的细颤
    float pulse = uLevel * (0.09 + 0.05 * sin(T * 9.0 + aSeed * 24.0));
    p *= 1.0 + pulse;

    // ── 中空化 ──
    // 体积均匀分布的密度 ∝ r²，所以中心其实最"实"。用小于 1 的指数重映射半径，
    // 就能把内层的粒子搬到壳上：中心变稀、表面变密。
    //
    // 只在非完全聚合时生效：聚合态靠粒子贴在节点上撑起图谱，
    // 那时候把它们推开会让粒子从节点和连线上脱开，图谱就散了。
    float shell = uShell * (1.0 - 0.85 * clamp(uCohesion, 0.0, 1.0));
    float r = length(p);
    if (shell > 0.001 && r > 1e-4) {
      float rn = clamp(r / 1.25, 0.0, 1.0);
      // 每颗粒子的指数不同，否则会挤成一层纸一样的膜而不是有厚度的壳
      float thick = 0.34 + fract(aSeed * 3.77) * 0.26;
      p *= mix(1.0, pow(rn, thick) * 1.25 / r, shell);
    }

    // ── 挤压回弹 ──
    // 弹簧速度驱动的 squash & stretch，横向做反向补偿以近似保持体积 ——
    // 只缩放不补偿的话看起来只是整团在变大变小，没有弹性。
    float sq = clamp(uSquash, -1.0, 1.0);
    p.y *= 1.0 + sq * 0.16;
    p.xz *= 1.0 - sq * 0.08;

    // ── 重力 ──
    // 外放时整体下沉，且越靠下压得越扁，像落在一个看不见的面上。
    // 下坠量来自弹簧速度，所以会跟着回弹一起振 —— 这就是那下 duang。
    p.y -= uSag * 0.14;
    // 注意参数顺序：smoothstep 在 edge0 >= edge1 时结果**未定义**（GLSL 规范），
    // 所以要写成 1.0 - smoothstep(低, 高, y) 而不是 smoothstep(高, 低, y)
    float low = 1.0 - smoothstep(-0.9, 0.35, p.y);
    p.y = mix(p.y, p.y * 0.86, uSag * low);

    // 收口成球。离散态允许更大，聚合态收紧
    p = roundOff(p, mix(1.22, 1.02, uCohesion));

    vec4 mv = modelViewMatrix * vec4(p, 1.0);
    gl_Position = projectionMatrix * mv;

    float tighten = mix(0.85, 1.35, uCohesion);
    gl_PointSize = aSize * uSizeScale * tighten * uPixelRatio / max(1.0, -mv.z);

    // 闪烁幅度随能量升高，让「在想」的时候显得躁动
    float flicker = 1.0 - 0.45 * uEnergy * (0.5 + 0.5 * sin(T * 6.5 + aSeed * 31.0));
    vHue = aHue;
    vCore = uCohesion;
    // aSize 大的那批（约 4%）额外提亮，形成"亮星"，
    // 否则整团亮度均匀会发灰、没有光感
    float star = 1.0 + smoothstep(3.0, 6.0, aSize) * 2.2;
    vAlpha = uAlphaScale * flicker * star * mix(0.55, 1.0, uCohesion);
  }
`;

const particleFragment = /* glsl */ `
  precision highp float;

  varying float vHue;
  varying float vAlpha;
  varying float vCore;
  uniform float uWarmth;
  uniform float uSaturation;
  uniform float uIrritation;

  void main() {
    vec2 uv = gl_PointCoord - 0.5;
    float d = length(uv) * 2.0;
    if (d > 1.0) discard;

    // 指数衰减比 smoothstep 更像真实光点：中心极亮，边缘拖出柔和尾巴
    float glow = exp(-d * 2.6);
    float core = exp(-d * 9.0);

    // ── 关系与情绪的两条感知通道 ──
    //
    // **色相表达好感度、饱和度表达心情。** 两条独立的感知通道对应两条独立
    // 的轴 —— 用一个通道表达两件事会歧义（"发暗"到底是不熟还是心情差？）。
    //
    // 色带走 冷蓝 → 青 → 暖金：冷色在人眼里天然有距离感，暖色有亲近感。
    // 不走"蓝→粉"那种大跨度：粒子云要和界面的玻璃件同色系，
    // 跨到粉紫会和整个界面脱节。
    vec3 distant = vec3(0.32, 0.55, 1.00);  // 初识：偏冷蓝
    vec3 mid     = vec3(0.25, 0.92, 1.00);  // 相识/熟悉：青（原本的基色）
    vec3 warm    = vec3(1.00, 0.82, 0.48);  // 亲近：暖金

    // 两段插值而不是一次三色混合：一次混合会让中段被两端拉灰
    vec3 base = uWarmth < 0.5
      ? mix(distant, mid, uWarmth * 2.0)
      : mix(mid, warm, (uWarmth - 0.5) * 2.0);

    // aHue 保留原来的"少数粒子偏金"的点缀，让整团不是单色
    vec3 tint = mix(base, warm, vHue * 0.55);

    // 心情低落 → 褪色发灰。用亮度加权的灰而不是简单平均，
    // 否则青色褪成的灰会比金色褪成的灰暗一大截
    float grey = dot(tint, vec3(0.299, 0.587, 0.114));
    tint = mix(vec3(grey), tint, uSaturation);
    // 聚合时偏白热
    vec3 color = mix(tint, vec3(1.0), core * 0.7 * vCore);

    // ── 生气 → 发红 ──
    //
    // **必须加在白热之后。** 之前混进 tint 里，而紧接着的那行会把每个粒子的
    // 中心推向纯白 —— 加法混合下 5.8 万个亮核累积起来，红色被白光完全冲掉，
    // 实测怒气 0.54 时球体依然是青的。
    //
    // 怒气**压过**色温而不是与它混合：生气时"我们关系怎样"不是此刻最该传达的
    // 信息，混合会得到一团脏橙色，两个意思都没表达清楚。
    //
    // 同时压掉一部分白热：白核是"平静时的高能"，生气时该是红而不是白亮
    float anger = clamp(uIrritation, 0.0, 1.0);
    vec3 angry = vec3(1.00, 0.22, 0.26);
    color = mix(color, angry, anger * 0.92);
    // 生气时整体略微压暗，红色才"沉"得下来 —— 高亮的红看起来像警报灯
    color *= 1.0 - anger * 0.18;

    gl_FragColor = vec4(color, (glow * 0.5 + core) * vAlpha);
  }
`;

const edgeVertex = /* glsl */ `
  attribute vec3 aNodeBase;
  attribute float aNodeSeed;
  attribute float aEdgeSeed;
  attribute float aT;
  attribute float aLength;

  uniform float uTime;
  uniform float uCohesion;
  uniform float uSpin;
  uniform float uSpeed;
  uniform float uSquash;
  uniform float uSag;

  varying float vT;
  varying float vEdgeSeed;
  varying float vFade;

  ${NODE_MOTION_GLSL}

  void main() {
    vec3 node = animatedNode(aNodeBase, aNodeSeed, uTime * uSpeed, uSpin);

    // 离散时把端点也往外推，边跟着散开并淡出，
    // 而不是原地消失 —— 原地消失很「数字化」，不自然
    vec3 outward = normalize(node + vec3(0.0001)) * 1.5;
    vec3 p = mix(outward, node, uCohesion);

    // 与粒子层用同一组形变：只让粒子弹、图谱不动的话两层会明显错开
    float sq = clamp(uSquash, -1.0, 1.0);
    p.y *= 1.0 + sq * 0.16;
    p.xz *= 1.0 - sq * 0.08;
    p.y -= uSag * 0.14;

    gl_Position = projectionMatrix * modelViewMatrix * vec4(p, 1.0);

    vT = aT;
    vEdgeSeed = aEdgeSeed;
    // 长边压暗：全部同亮会糊成一张网，压暗才有层次
    // 只让短边明显：长边几乎不可见，图谱才有层次而不是一张渔网
    vFade = 1.0 - smoothstep(0.28, 0.62, aLength);
  }
`;

const edgeFragment = /* glsl */ `
  precision highp float;

  uniform float uTime;
  uniform float uCohesion;
  uniform float uEnergy;

  varying float vT;
  varying float vEdgeSeed;
  varying float vFade;

  void main() {
    // 沿边跑的能量脉冲 —— 「在思考」的核心观感就来自这里
    float head = fract(uTime * (0.35 + fract(vEdgeSeed) * 0.5) + vEdgeSeed);
    float d = abs(vT - head);
    d = min(d, 1.0 - d); // 环形距离，脉冲跑到末端能从头接上
    float pulse = exp(-d * 16.0) * uEnergy;

    // 每条边有自己的呼吸节奏，避免整张网同步闪烁（那样很假）
    float breathe = 0.35 + 0.35 * sin(uTime * 1.6 + vEdgeSeed * 21.0);

    // 硬截断 + 平方：聚合度低于 0.3 时边**完全**不可见。
    // 只靠平方衰减不够 —— 离散态下所有端点都被推到同一半径上，
    // 投影后大量长线重叠，加法混合把 0.006 的透明度也累积成可见的笼子。
    float presence = smoothstep(0.3, 1.0, uCohesion);
    presence *= presence;

    vec3 base = vec3(0.38, 0.88, 1.0);
    vec3 hot = vec3(0.90, 1.0, 1.0);
    vec3 color = mix(base, hot, pulse);

    gl_FragColor = vec4(color, presence * vFade * (breathe * 0.55 + pulse * 1.6));
  }
`;

function build(): { particles: THREE.BufferGeometry; edges: THREE.BufferGeometry } {
  const rand = makeRandom(0x5eed1234);

  // 节点锚点：填充球体内部（cbrt 保证体积均匀，否则全挤在表面）
  const nodes: THREE.Vector3[] = [];
  for (let i = 0; i < NODE_COUNT; i++) {
    const dir = new THREE.Vector3(rand() * 2 - 1, rand() * 2 - 1, rand() * 2 - 1);
    if (dir.lengthSq() < 1e-6) dir.set(0, 1, 0);
    dir.normalize();
    // 0.25 起步：中心留空，更像「云」而不是实心团
    nodes.push(dir.multiplyScalar(0.25 + Math.cbrt(rand()) * 0.72));
  }
  const nodeSeeds = Array.from({ length: NODE_COUNT }, () => rand());

  const nodeBase = new Float32Array(PARTICLE_COUNT * 3);
  const offset = new Float32Array(PARTICLE_COUNT * 3);
  const wander = new Float32Array(PARTICLE_COUNT * 3);
  const seedAttr = new Float32Array(PARTICLE_COUNT);
  const nodeSeedAttr = new Float32Array(PARTICLE_COUNT);
  const sizeAttr = new Float32Array(PARTICLE_COUNT);
  const hueAttr = new Float32Array(PARTICLE_COUNT);
  // position 用不到（位置全在着色器算），但 three 需要它确定顶点数
  const position = new Float32Array(PARTICLE_COUNT * 3);

  for (let i = 0; i < PARTICLE_COUNT; i++) {
    const n = i % NODE_COUNT;
    const node = nodes[n];
    nodeBase[i * 3] = node.x;
    nodeBase[i * 3 + 1] = node.y;
    nodeBase[i * 3 + 2] = node.z;
    nodeSeedAttr[i] = nodeSeeds[n];

    const od = new THREE.Vector3(rand() * 2 - 1, rand() * 2 - 1, rand() * 2 - 1);
    if (od.lengthSq() < 1e-6) od.set(1, 0, 0);
    od.normalize().multiplyScalar(Math.cbrt(rand()));
    offset[i * 3] = od.x;
    offset[i * 3 + 1] = od.y;
    offset[i * 3 + 2] = od.z;

    const wd = new THREE.Vector3(rand() * 2 - 1, rand() * 2 - 1, rand() * 2 - 1);
    if (wd.lengthSq() < 1e-6) wd.set(0, 0, 1);
    // 最外圈控制在 1.15 以内。之前到 1.7，超出视锥被画布裁掉，
    // 离散态就变成铺满矩形的背景噪点，而不是一团有柔和边缘的云
    wd.normalize().multiplyScalar(0.42 + Math.cbrt(rand()) * 0.73);
    wander[i * 3] = wd.x;
    wander[i * 3 + 1] = wd.y;
    wander[i * 3 + 2] = wd.z;

    seedAttr[i] = rand();
    // 少数粒子明显更大，形成「亮星」，避免颗粒感过于均匀
    sizeAttr[i] = rand() < 0.04 ? 3.6 + rand() * 2.6 : 0.9 + rand() * 1.5;
    // 约 12% 暖金点缀
    hueAttr[i] = rand() < 0.12 ? 0.6 + rand() * 0.4 : rand() * 0.12;
  }

  const particles = new THREE.BufferGeometry();
  particles.setAttribute("position", new THREE.BufferAttribute(position, 3));
  particles.setAttribute("aNodeBase", new THREE.BufferAttribute(nodeBase, 3));
  particles.setAttribute("aOffset", new THREE.BufferAttribute(offset, 3));
  particles.setAttribute("aWander", new THREE.BufferAttribute(wander, 3));
  particles.setAttribute("aSeed", new THREE.BufferAttribute(seedAttr, 1));
  particles.setAttribute("aNodeSeed", new THREE.BufferAttribute(nodeSeedAttr, 1));
  particles.setAttribute("aSize", new THREE.BufferAttribute(sizeAttr, 1));
  particles.setAttribute("aHue", new THREE.BufferAttribute(hueAttr, 1));
  // 位置在着色器里算，包围球算不出来；不手动给会被视锥剔除掉
  particles.boundingSphere = new THREE.Sphere(new THREE.Vector3(), 4);

  // 边：节点间 k 近邻，去重
  const seen = new Set<string>();
  const edgeList: Array<[number, number]> = [];
  for (let i = 0; i < NODE_COUNT; i++) {
    const near = nodes
      .map((n, j) => ({ j, d: n.distanceTo(nodes[i]) }))
      .filter((x) => x.j !== i)
      .sort((a, b) => a.d - b.d);

    for (let k = 0; k < NEIGHBORS_PER_NODE && k < near.length; k++) {
      const j = near[k].j;
      const key = i < j ? `${i}-${j}` : `${j}-${i}`;
      if (seen.has(key)) continue;
      seen.add(key);
      edgeList.push([i, j]);
    }
  }

  const n2 = edgeList.length * 2;
  const ePos = new Float32Array(n2 * 3);
  const eNodeBase = new Float32Array(n2 * 3);
  const eNodeSeed = new Float32Array(n2);
  const eEdgeSeed = new Float32Array(n2);
  const eT = new Float32Array(n2);
  const eLen = new Float32Array(n2);

  edgeList.forEach(([a, b], e) => {
    const seed = rand();
    const len = nodes[a].distanceTo(nodes[b]);
    for (let v = 0; v < 2; v++) {
      const idx = e * 2 + v;
      const node = v === 0 ? nodes[a] : nodes[b];
      eNodeBase[idx * 3] = node.x;
      eNodeBase[idx * 3 + 1] = node.y;
      eNodeBase[idx * 3 + 2] = node.z;
      eNodeSeed[idx] = nodeSeeds[v === 0 ? a : b];
      eEdgeSeed[idx] = seed;
      eT[idx] = v;
      eLen[idx] = len;
    }
  });

  const edges = new THREE.BufferGeometry();
  edges.setAttribute("position", new THREE.BufferAttribute(ePos, 3));
  edges.setAttribute("aNodeBase", new THREE.BufferAttribute(eNodeBase, 3));
  edges.setAttribute("aNodeSeed", new THREE.BufferAttribute(eNodeSeed, 1));
  edges.setAttribute("aEdgeSeed", new THREE.BufferAttribute(eEdgeSeed, 1));
  edges.setAttribute("aT", new THREE.BufferAttribute(eT, 1));
  edges.setAttribute("aLength", new THREE.BufferAttribute(eLen, 1));
  edges.boundingSphere = new THREE.Sphere(new THREE.Vector3(), 4);

  return { particles, edges };
}

export function AvatarCanvas({
  state,
  level = 0,
  speed = 0.35,
  emotion = null,
  className,
}: AvatarCanvasProps) {
  const mountRef = useRef<HTMLDivElement | null>(null);
  // 用 ref 传状态，避免状态一变就重建整个 WebGL 场景
  const stateRef = useRef<AvatarState>(state);
  const levelRef = useRef(level);
  stateRef.current = state;
  levelRef.current = level;
  const speedRef = useRef(speed);
  speedRef.current = speed;
  const emotionRef = useRef(emotion);
  emotionRef.current = emotion;

  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) return;

    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 40);
    camera.position.z = 5.0;

    const renderer = new THREE.WebGLRenderer({
      alpha: true,
      antialias: false, // 点精灵用不上 MSAA，省下来给分辨率
      preserveDrawingBuffer: true,
      powerPreference: "high-performance",
    });
    renderer.setClearColor(0x000000, 0);
    const pixelRatio = Math.min(window.devicePixelRatio, 2);
    renderer.setPixelRatio(pixelRatio);
    mount.appendChild(renderer.domElement);

    const initial = STATE_TARGETS[stateRef.current];
    const uniforms = {
      uTime: { value: 0 },
      uCohesion: { value: initial.cohesion },
      uEnergy: { value: initial.energy },
      uSpin: { value: initial.spin },
      uCluster: { value: initial.cluster },
      uLevel: { value: 0 },
      uPixelRatio: { value: pixelRatio },
      // 这两个值极度敏感：加法混合下 5.8 万颗粒子叠加，
      // 单颗尺寸/亮度稍大就整体烧成一张白饼（实测 26/1.0 就是这个结果）。
      // 目标是单颗约 2-3 像素、峰值贡献只有百分之几。
      // 舞台从 440 放大到 528px 之后单位面积的粒子更密，加法混合下中心直接
      // 烧成纯白 —— 所有结构都看不见了。尺寸和亮度都要跟着往下调。
      uSizeScale: { value: 4.2 },
      uAlphaScale: { value: 0.30 },
      // 形变相关。三层（粒子/光晕/连线）共享**同一个对象引用**，
      // 所以只需要在主循环里更新一次
      // 出厂 0.35：按诊断算过，1.0 时流场是 5~20 秒一个周期，
      // 比其它运动快一个数量级，观感就是"要起飞"
      uSpeed: { value: 0.35 },
      // 好感度 → 色温（0 冷蓝 / 0.5 青 / 1 暖金），心情 → 饱和度。
      // 起始值对应"关系中性、心情平稳"
      uWarmth: { value: 0.5 },
      uSaturation: { value: 1.0 },
      uIrritation: { value: 0.0 },
      uShell: { value: 0 },
      uSquash: { value: 0 },
      uSag: { value: 0 },
    };
    // 光晕层共享大部分 uniform 对象引用，只有尺寸/透明度是自己的
    const haloUniforms = {
      ...uniforms,
      // 光晕：比亮核大一个量级、淡两个量级，负责把光"洇"开
      uSizeScale: { value: 20.0 },
      uAlphaScale: { value: 0.032 },
    };

    const { particles, edges } = build();

    const coreMaterial = new THREE.ShaderMaterial({
      uniforms,
      vertexShader: particleVertex,
      fragmentShader: particleFragment,
      transparent: true,
      depthWrite: false,
      depthTest: false,
      blending: THREE.AdditiveBlending,
    });
    const corePoints = new THREE.Points(particles, coreMaterial);

    // 光晕层：同一份几何画得更大更淡。加法混合下它负责「发光」，
    // 比后处理 bloom 便宜得多，也不会破坏画布透明度
    const haloMaterial = new THREE.ShaderMaterial({
      uniforms: haloUniforms,
      vertexShader: particleVertex,
      fragmentShader: particleFragment,
      transparent: true,
      depthWrite: false,
      depthTest: false,
      blending: THREE.AdditiveBlending,
    });
    const haloPoints = new THREE.Points(particles, haloMaterial);

    const edgeMaterial = new THREE.ShaderMaterial({
      uniforms,
      vertexShader: edgeVertex,
      fragmentShader: edgeFragment,
      transparent: true,
      depthWrite: false,
      depthTest: false,
      blending: THREE.AdditiveBlending,
    });
    const edgeLines = new THREE.LineSegments(edges, edgeMaterial);

    // 先画光晕再画亮核，亮核才压在最上面
    scene.add(haloPoints, edgeLines, corePoints);

    const resize = () => {
      const { clientWidth: w, clientHeight: h } = mount;
      if (w === 0 || h === 0) return;
      renderer.setSize(w, h, false);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
    };
    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(mount);

    let raf = 0;
    const clock = new THREE.Clock();
    // 使用逻辑累加时间，避免系统息屏/屏保恢复后 performance.now() 600s 跳变导致 GLSL 周期突变
    let accumulatedTime = 0;
    let smoothLevel = 0;

    // 聚合度的弹簧状态。ω=√K≈7 rad/s（周期约 0.9 秒），ζ=C/(2ω)≈0.3
    const SPRING_K = 49;
    const SPRING_C = 4.2;
    let coh = initial.cohesion;
    let cohVel = 0;

    // 监听页面/息屏唤醒事件：休眠恢复时重置时钟与弹簧动量，防止速度爆炸
    const handleVisibility = () => {
      if (document.hidden) {
        clock.stop();
      } else {
        clock.start();
        cohVel = 0; // 重置弹簧速度，防止积累冲量
      }
    };
    document.addEventListener("visibilitychange", handleVisibility);

    const tick = () => {
      // 限制 dt 最高 0.033s (约 30fps 单帧时间)，防止弹簧 Euler 积分 (K=49) 在 dt=0.1s 时数值发散爆表
      const deltaRaw = clock.getDelta();
      const dt = Math.min(deltaRaw, 0.033);

      // 用实际渲染的 dt 累加时间，息屏/屏保期间完全挂起，恢复后无相位突变
      accumulatedTime += dt;
      uniforms.uTime.value = accumulatedTime;

      const target = STATE_TARGETS[stateRef.current];

      // 阻尼趋近而不是直接赋值：状态切换要看得见「聚合/散开」的过程，
      // 这个过程本身就是最重要的观感
      const approach = (u: { value: number }, to: number, rate: number) => {
        u.value += (to - u.value) * Math.min(1, dt * rate);
      };
      // 聚合度改用**欠阻尼弹簧**而不是指数趋近。
      //
      // 指数趋近只会越来越慢地贴上去，永远不会过冲，所以怎么调都是"平滑到位"；
      // 而"duang"的本质就是过冲后回弹。ζ≈0.3 时过冲约四成，肉眼刚好看得出弹性
      // 又不至于晃得难受。
      //
      // 刻意允许 uCohesion 略微越界（mix 会外插），这样粒子会冲过节点再收回来 ——
      // 夹死在 [0,1] 的话弹性会被吃掉一半。
      cohVel += (target.cohesion - coh) * SPRING_K * dt;
      cohVel -= cohVel * SPRING_C * dt;
      coh = Math.max(-0.12, Math.min(1.12, coh + cohVel * dt));
      uniforms.uCohesion.value = coh;

      // 中空：收缩和外放的**过程中**才掏空，静止下来恢复实心
      const shellTarget = Math.min(1, Math.abs(cohVel) * 0.42);
      uniforms.uShell.value +=
        (shellTarget - uniforms.uShell.value) * Math.min(1, dt * 6);
      // 挤压跟着弹簧速度振荡，所以会一起 duang
      uniforms.uSquash.value = Math.max(-0.7, Math.min(0.7, cohVel * 0.14));
      // 重力只在外放（速度为负）时出现
      uniforms.uSag.value = Math.min(1, Math.max(0, -cohVel) * 0.22);

      approach(uniforms.uEnergy, target.energy, 2.5);
      approach(uniforms.uSpin, target.spin, 1.2);
      approach(uniforms.uCluster, target.cluster, 1.6);

      // ── 情感轴的偏移 ──
      //
      // **必须放在所有插值之后。** 之前写在 approach 之前，
      // approach 会立刻把值拉回状态目标 —— 偏移每帧都被抵消掉，
      // 表现是"情感轴变了但画面没反应"。
      //
      // 幅度刻意压得小（±0.12 量级）：状态机决定"它在干什么"，情感只调整
      // "它现在什么样"。偏移给大了两者会互相淹没。
      const emo = emotionRef.current;
      if (emo) {
        // 好感度 → 聚合度：熟悉的人靠得更近
        const affBias = ((emo.affection - 50) / 50) * 0.10;
        // 心情 → 亮度与聚合：低落时散开变暗
        const moodBias = (emo.mood / 100) * 0.12;
        // 活跃度 → 能量（亮度/闪烁）与流动速率
        const energyBias = ((emo.energy - 50) / 50) * 0.18;

        uniforms.uCohesion.value = Math.max(
          0,
          Math.min(1.12, uniforms.uCohesion.value + affBias + moodBias * 0.5),
        );
        uniforms.uEnergy.value = Math.max(
          0.1,
          Math.min(1.2, uniforms.uEnergy.value + moodBias + energyBias * 0.5),
        );

        // ── 颜色 ──
        //
        // 色温直接映射好感度全程（0~100 → 0~1）：这是最该被看见的一条轴，
        // 压缩幅度会让"养了两周"和"刚认识"看起来一样。
        //
        // 饱和度只在**心情为负**时才降：正面情绪已经由色温和亮度表达了，
        // 再叠一层过饱和只会让画面发假。下限 0.35 —— 全灰会让人以为渲染坏了。
        const warmthTarget = Math.max(0, Math.min(1, emo.affection / 100));
        const satTarget =
          emo.mood >= 0 ? 1.0 : 1.0 - Math.min(0.65, (-emo.mood / 100) * 0.65);

        // 慢速趋近而不是直接赋值：颜色突变比位置突变更刺眼，
        // 而且轴值只在轮次边界更新，直接赋值会看到跳变
        approach(uniforms.uWarmth, warmthTarget, 0.35);
        approach(uniforms.uSaturation, satTarget, 0.6);
        // 怒气上得快、退得慢：情绪的真实节奏就是这样，而且退得快会让
        // "刚才它生气了"这件事来不及被看见
        const irr = Math.max(0, Math.min(1, emo.irritation));
        approach(
          uniforms.uIrritation,
          irr,
          irr > uniforms.uIrritation.value ? 1.6 : 0.25,
        );
      }


      // 音量上冲快、回落慢，像真实电平表
      const raw = Math.min(1, Math.max(0, levelRef.current));
      smoothLevel +=
        (raw - smoothLevel) * Math.min(1, dt * (raw > smoothLevel ? 14 : 4));
      uniforms.uLevel.value = smoothLevel;
      uniforms.uSpeed.value = speedRef.current;

      // 光晕层的这几个 uniform 是独立对象，要手动同步
      haloUniforms.uTime.value = uniforms.uTime.value;
      haloUniforms.uCohesion.value = uniforms.uCohesion.value;
      haloUniforms.uEnergy.value = uniforms.uEnergy.value;
      haloUniforms.uSpin.value = uniforms.uSpin.value;
      haloUniforms.uCluster.value = uniforms.uCluster.value;
      haloUniforms.uLevel.value = uniforms.uLevel.value;
      haloUniforms.uShell.value = uniforms.uShell.value;
      haloUniforms.uSquash.value = uniforms.uSquash.value;
      haloUniforms.uSag.value = uniforms.uSag.value;

      // 极缓慢整体自转。由累加时间直接算出角度
      corePoints.rotation.y = accumulatedTime * 0.018;
      haloPoints.rotation.y = corePoints.rotation.y;
      edgeLines.rotation.y = corePoints.rotation.y;

      // 渲染诊断出口。当初为定位"疯转"加的，现在保留成常驻能力 ——
      // fps 掉下来、uTime 增速异常都能一眼看出来，而这类问题
      // 光看画面是分辨不出的（我在它上面错过四次）。
      //
      // 由设置里的开关控制显示，默认关：数值行压在标题下面很干扰阅读。
      const w = window as unknown as Record<string, unknown>;
      const d = (w.__animaDiag ?? {}) as Record<string, number>;
      d.frames = (d.frames ?? 0) + 1;
      d.uTime = uniforms.uTime.value;
      d.rotY = corePoints.rotation.y;
      d.spin = uniforms.uSpin.value;
      d.cohesion = uniforms.uCohesion.value;
      d.dt = dt;
      d.canvases = document.querySelectorAll(".stage canvas").length;
      w.__animaDiag = d;

      // 把数值直接写进界面。**不能靠开发者工具读** ——
      // 预览面板隐藏时浏览器会挂起 requestAnimationFrame，
      // 从外部完全观测不到动画速率（这就是前面几次测不到的原因）。
      const box = document.getElementById("anima-diag");
      if (box) {
        const now = performance.now();
        const prev = (d.probeAt ?? 0) as number;
        if (now - prev > 500) {
          const secs = (now - prev) / 1000;
          const fps = prev ? (d.frames - (d.probeFrames ?? 0)) / secs : 0;
          const rot = prev ? (d.rotY - (d.probeRot ?? 0)) / secs : 0;
          box.textContent =
            `fps ${fps.toFixed(0)} | 自转 ${rot.toFixed(4)} rad/s | ` +
            `spin ${uniforms.uSpin.value.toFixed(4)} | 速度 ${uniforms.uSpeed.value.toFixed(2)} | ` +
            // 色温/饱和/怒气也放进来：颜色不对时能立刻分清是"值没传到"
            // 还是"传到了但映射不对"—— 这两种的修法完全不同
            `暖 ${uniforms.uWarmth.value.toFixed(2)} 饱 ${uniforms.uSaturation.value.toFixed(2)} 怒 ${uniforms.uIrritation.value.toFixed(2)} | ` +
            `聚合 ${uniforms.uCohesion.value.toFixed(2)} | ` +
            `uTime ${uniforms.uTime.value.toFixed(1)}`;
          d.probeAt = now;
          d.probeFrames = d.frames;
          d.probeRot = d.rotY;
        }
      }

      renderer.render(scene, camera);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(raf);
      document.removeEventListener("visibilitychange", handleVisibility);
      observer.disconnect();
      // Three 的资源必须手动释放，否则热重载/切形态会持续泄漏显存
      // （ARCHITECTURE.md 坑点 5）
      particles.dispose();
      edges.dispose();
      coreMaterial.dispose();
      haloMaterial.dispose();
      edgeMaterial.dispose();
      renderer.dispose();
      if (renderer.domElement.parentNode === mount) {
        mount.removeChild(renderer.domElement);
      }
    };
    // 只挂载一次：状态通过 ref 传入
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return <div ref={mountRef} className={className} />;
}
