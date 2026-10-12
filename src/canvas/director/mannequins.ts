/**
 * 导演世界的人偶：加载、克隆、上色、摆姿势。
 *
 * 改写自 DramaClaw（ClaymoreLab, Elastic License 2.0）
 * frontend/src/features/viewer-kit/three-d/engine/viewerApp.ts 的
 * loadActorProxyModel / instantiateActorProxyModel / sampleActorAnimationPose。
 * DC 用 PlayCanvas，这里换成 three。出处规则见 DECISIONS.md 2026-09-16。
 *
 * # 三件不做就出错、而且不报错的事
 *
 * 1. **克隆必须用 `SkeletonUtils.clone`**。`Object3D.clone()` 会让所有副本共用
 *    同一副骨骼，摆一个人的姿势全场人一起动。
 * 2. **材质要换成无光照的纯色**。模型自带的是 PBR 材质，场景里没灯就是一团黑；
 *    而且下游靠颜色认人，受光照影响的颜色认不准。
 * 3. **包围盒要按蒙皮后的顶点算**（`setFromObject(obj, true)`）。不加 `true`
 *    算的是 T-pose 的盒子，躺倒的人偶框出来是竖着的。
 */
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { clone as cloneSkinned } from "three/examples/jsm/utils/SkeletonUtils.js";
import { resolvePoseClip, type PoseName } from "./poses";

/** 人偶模型取 UAL2，动作片段两个库合在一起（同一套 65 根骨骼） */
const MODEL_URL = "/director/quaternius/ual2/UAL2_Standard.glb";
const EXTRA_CLIP_URLS = ["/director/quaternius/ual1/UAL1_Standard.glb"];

interface Library {
  template: THREE.Object3D;
  clips: Map<string, THREE.AnimationClip>;
}

let libraryPromise: Promise<Library> | null = null;

/** 只加载一次。两个 GLB 各 8 MB，打开一次导演世界不该下两遍 */
export function loadMannequinLibrary(): Promise<Library> {
  if (!libraryPromise) {
    const loader = new GLTFLoader();
    libraryPromise = Promise.all([MODEL_URL, ...EXTRA_CLIP_URLS].map((u) => loader.loadAsync(u)))
      .then(([model, ...extra]) => {
        const clips = new Map<string, THREE.AnimationClip>();
        for (const g of [model, ...extra]) for (const c of g.animations) clips.set(c.name, c);
        return { template: model.scene, clips };
      })
      .catch((e) => {
        // 失败了下次还能重试，不要把一个失败的 promise 永久缓存住
        libraryPromise = null;
        throw new Error(`人偶模型加载失败：${e instanceof Error ? e.message : String(e)}`);
      });
  }
  return libraryPromise;
}

export interface Mannequin {
  root: THREE.Group;
  setColor(hex: string): void;
  setPose(pose: PoseName): void;
  /** 截遮罩时临时换成纯白 */
  setMaskMaterial(on: boolean): void;
  /** 蒙皮后的世界包围盒 */
  bounds(): THREE.Box3;
  dispose(): void;
}

export function createMannequin(lib: Library, color: string, pose: PoseName): Mannequin {
  const root = new THREE.Group();
  const model = cloneSkinned(lib.template);
  const material = new THREE.MeshBasicMaterial({ color: new THREE.Color(color) });
  const mask = new THREE.MeshBasicMaterial({ color: 0xffffff });
  const meshes: THREE.Mesh[] = [];
  model.traverse((o) => {
    const m = o as THREE.Mesh;
    if (m.isMesh) {
      m.material = material;
      // 蒙皮网格的包围球按 T-pose 算，摆了姿势之后会被错误地裁掉
      m.frustumCulled = false;
      meshes.push(m);
    }
  });
  root.add(model);

  const mixer = new THREE.AnimationMixer(model);
  let action: THREE.AnimationAction | null = null;

  const api: Mannequin = {
    root,
    setColor(hex) {
      material.color.set(hex);
    },
    setPose(p) {
      const hit = resolvePoseClip(p, new Set(lib.clips.keys()));
      if (!hit) throw new Error(`姿势「${p}」在人偶动作库里找不到对应片段`);
      action?.stop();
      action = mixer.clipAction(lib.clips.get(hit.name)!);
      action.reset().play();
      // 定格在那一刻：推进到采样时间后不再更新 mixer
      mixer.setTime(hit.time);
      model.updateMatrixWorld(true);
    },
    setMaskMaterial(on) {
      for (const m of meshes) m.material = on ? mask : material;
    },
    bounds() {
      root.updateMatrixWorld(true);
      return new THREE.Box3().setFromObject(root, true);
    },
    dispose() {
      mixer.stopAllAction();
      material.dispose();
      mask.dispose();
    },
  };
  api.setPose(pose);
  return api;
}
