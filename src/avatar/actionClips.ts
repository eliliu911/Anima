import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";

import { avatarAnimationBytes, avatarAnimations, logUi } from "../api/voice";
import { generateAnimation } from "./animations";
import { retargetVrma } from "./vrma";
import type { Bones, IdleState } from "./pose";

/**
 * 一个动作要播什么：优先用 `animations/` 里的 `.vrma`，没有就用代码生成的手势。
 *
 * 代码生成的那套是**兜底，不是目标**。手写的欧拉角只能做到"方向大致对"，
 * 而真人动作的自然感来自动捕数据里那些说不清的细节。所以只要用户往
 * `animations/` 里放了对应的文件，就用它。
 *
 * ## 文件名就是动作名
 *
 * `animations/wave.vrma` 对应 `<action:wave>`。不做映射表 ——
 * 多一层配置就多一处"配错了但看不出来"的地方，而文件名本身已经够表达了。
 * 大小写不敏感，另外认几个中文别名（见 ALIASES）。
 */

/** 动作名的别名。模型有时会直接写中文 */
const ALIASES: Record<string, string> = {
  挥手: "wave",
  打招呼: "wave",
  伸懒腰: "stretch",
  捂嘴: "cover_mouth",
  捂嘴笑: "cover_mouth",
  惊讶: "cover_mouth",
  比心: "heart",
  爱心: "heart",
};

/**
 * 动作名归一化。文件名和模型写的标签要用**同一套**规则，否则
 * `Peace sign.vrma` 永远匹配不上 `<action:peace_sign>`。
 */
function normalize(action: string): string {
  const trimmed = action.trim();
  return (ALIASES[trimmed] ?? trimmed).toLowerCase().replace(/[ -]+/g, "_");
}

/** 文件名（去扩展名，小写）→ vrma 的 id。加载一次就缓存 */
let index: Map<string, string> | null = null;
/** 已经解析并重定向过的片段。按 `动作名|模型骨骼` 缓存 */
const clipCache = new Map<string, THREE.AnimationClip>();

async function ensureIndex(): Promise<Map<string, string>> {
  if (index) return index;
  const next = new Map<string, string>();
  try {
    for (const a of await avatarAnimations()) {
      next.set(normalize(a.name), a.id);
    }
  } catch (err) {
    // 目录不存在、命令不可用都不是错误 —— 退回生成的手势
    logUi("avatar", `列动作库失败（退回内置手势）：${err}`);
  }
  index = next;
  return next;
}

/**
 * 拿一个动作的片段。
 *
 * @param cacheKey 用来区分不同模型的骨骼 —— 同一个 vrma 在不同模型上要
 *                 重定向到不同的骨骼对象，缓存不能串
 */
export async function clipFor(
  action: string,
  bones: Bones,
  restPose: IdleState,
  cacheKey: string,
  /** 模型的全量标准骨骼表。缺了它肩膀/手指/腿都不会动，见 humanoid.ts */
  humanoidAll?: Map<string, THREE.Object3D>,
): Promise<THREE.AnimationClip | null> {
  const name = normalize(action);
  const memo = `${name}|${cacheKey}`;
  const cached = clipCache.get(memo);
  if (cached) return cached;

  const id = (await ensureIndex()).get(name);
  if (id) {
    try {
      const bytes = await avatarAnimationBytes(id);
      const gltf = await new GLTFLoader().parseAsync(bytes, "");
      const { clip, bound, skipped } = retargetVrma(gltf, bones, humanoidAll);
      if (clip) {
        logUi(
          "avatar",
          `动作 ${name} 用 ${id}：接上 ${bound} 根骨骼` +
            (skipped.length ? `，模型缺 ${skipped.join("/")}` : ""),
        );
        clipCache.set(memo, clip);
        return clip;
      }
      // 解析成功但一根骨头都接不上：**必须说出来**。
      // 静默退回的话，用户会以为自己下载的动作文件在生效
      logUi("avatar", `动作 ${name} 的 ${id} 一根骨骼都没接上，退回内置手势`);
    } catch (err) {
      logUi("avatar", `读 ${id} 失败（退回内置手势）：${err}`);
    }
  }

  const generated = generateAnimation(name, bones, restPose);
  if (generated) clipCache.set(memo, generated);
  return generated;
}
