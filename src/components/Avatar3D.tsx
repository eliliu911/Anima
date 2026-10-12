/**
 * 3D 面容。加载一个带 morph target 的 glTF 模型，用嘴型时间线驱动它说话。
 *
 * ## 和粒子云的关系
 *
 * 两者是**互斥的两种面容形态**，不是叠加。粒子云是抽象态，3D 是具象态，
 * 在设置里切。粒子云仍然是默认 —— 它不需要任何素材就能跑。
 *
 * ## 模型从哪来
 *
 * 前端不持有文件权限，所以模型字节由 Rust 的 `avatar_model_bytes` 读出来，
 * 这里用 `GLTFLoader.parse()` 解析内存里的 ArrayBuffer。
 *
 * ## 嘴不动的时候先看这里
 *
 * 模型能加载、能显示、就是嘴不动，而且不报任何错 —— 这是这一段最典型的失败样子。
 * 原因只有两种：模型压根没带 morph target，或者带了但命名不认识。
 *
 * 所以这里把两件事都报出去：认出来的通道数，以及模型里**实际有哪些**
 * morph target 名。后者是命名不认识时唯一能查的线索。匹配逻辑在
 * `avatar/morphMap.ts`，那里认三套命名并且有测试。
 *
 * 手上没有模型也能看：内置头（`avatar/defaultHead.ts`）是代码生成的，
 * 走的是完全相同的匹配路径。
 */
import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { avatarModelBytes, logUi } from "../api/voice";
import { mouth } from "../avatar/mouth";
import { buildDefaultHead, DEFAULT_HEAD_ID } from "../avatar/defaultHead";
import { mapMorphTargets, type MorphScheme, type Target } from "../avatar/morphMap";
import { captureRest, findBones, idle, relaxArms, type Bones, type IdleState } from "../avatar/pose";
import { readAllHumanoidNodes, readHumanoidBones } from "../avatar/humanoid";
import { portraitFraming } from "../avatar/framing";
import { faceCamera, restoreMorphNames } from "../avatar/gltfNames";
import { disableDegenerateVertexColors } from "../avatar/vertexColors";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { applyHologramEffect, updateHologram } from "../avatar/hologramMaterial";
import { clipFor } from "../avatar/actionClips";
import { Gaze, targetFromPointer } from "../avatar/gaze";
import { desktopCursor, hasDesktopCursor } from "../avatar/desktopCursor";

/**
 * 一个动作最多占用画面多久。
 *
 * 下载来的动捕片段动辄 8~20 秒（`Happy Synthesiser` 是 19.9 秒），
 * 播完之前人物完全被动画接管 —— 你说话它还在那儿跳。
 * 超时就淡出交还给 idle，片段本身不动（下次换个短的用就行）。
 */
const MAX_ACTION_SECONDS = 6;
/** 动作淡入时长。太短就是硬切，太长动作起手会拖 */
const ACTION_FADE = 0.18;
/**
 * 动作**淡出**时长。比淡入长一截是有意的：
 * 起手可以干脆，收手拖沓才像人 —— 收得太快就是"啪一下弹回去"
 */
const ACTION_FADE_OUT = 0.45;
import type { HologramStyle } from "../store/face";
import type { Viseme } from "../api/voice";
import type { EmotionState } from "../store/voice";

export type { AvatarReport };

interface AvatarReport {
  /** 识别到的口型通道数。0 = 没认出任何口型 */
  channels: number;
  scheme: MorphScheme;
  /** 模型里实际有哪些 morph target。认不出来时这是唯一能查的线索 */
  found: string[];
  error?: string;
}

interface Props {
  modelId: string;
  hologramMode?: boolean;
  hologramStyle?: HologramStyle;
  hologramColor?: string;
  /** 实时的情绪状态。用来调表情底色 —— 比如心情差的时候不该一直笑着，唤醒度高时呼吸变快 */
  emotion?: EmotionState;
  /** 触发的动作指令（如 stretch, cover_mouth 等） */
  action?: string;
  /** 动作序号。同一个动作重复触发时靠它区分「第几次」 */
  actionSeq?: number;
  /**
   * 往下取几个头高。默认是半身像。
   *
   * **桌宠窗口要传大得多的值** —— 那个窗口又窄又高，半身像会让她缩在
   * 中间一小块、上下大片留白，看起来"人太小"
   */
  bodyHeads?: number;
  className?: string;
  onReport?: (r: AvatarReport) => void;
}

export function Avatar3D({ modelId, hologramMode = false, hologramStyle = "dot", hologramColor = "#1a9fff", emotion, action, actionSeq = 0, bodyHeads, className, onReport }: Props) {
  const mountRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  // 心情每轮才变，但渲染循环每帧要读 —— 用 ref 而不是让 effect 重建场景
  const emotionRef = useRef(emotion);
  emotionRef.current = emotion;
  // 动作也是每帧读取并响应
  const actionRef = useRef(action);
  actionRef.current = action;
  const actionSeqRef = useRef(actionSeq);
  actionSeqRef.current = actionSeq;

  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) return;

    let disposed = false;
    let raf = 0;
    const renderer = new THREE.WebGLRenderer({
      antialias: true,
      alpha: true,
      // **宠物模式下不加这条会有一圈白边。**
      //
      // three 默认输出预乘 alpha 的颜色。页面背景不透明时看不出来，
      // 但窗口本身透明（宠物模式）时，头发/衣服边缘那些半透明像素
      // 会以预乘后的亮色直接叠在桌面上 —— 表现为整个人被描了一圈白边，
      // 像贴纸。关掉预乘，边缘才会正确地和桌面混合。
      premultipliedAlpha: false,
      preserveDrawingBuffer: true,
      powerPreference: "high-performance",
    });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    mount.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    // 扩展 Far 面到 2000，防止尺度为厘米 (cm) 或毫米 (mm) 的模型被相机远裁剪面剔除
    const camera = new THREE.PerspectiveCamera(24, 1, 0.05, 2000);

    // 三点光。RPM 的皮肤材质在单光源下会显得很平
    scene.add(new THREE.AmbientLight(0xffffff, 0.55));
    const key = new THREE.DirectionalLight(0xffffff, 1.5);
    key.position.set(0.6, 1.2, 1.4);
    scene.add(key);
    const rim = new THREE.DirectionalLight(0x88bbff, 0.7);
    rim.position.set(-1.2, 0.6, -0.8);
    scene.add(rim);

    // 滚轮缩放 + 拖动转视角。
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enablePan = false;
    controls.enableDamping = true;
    // 滚轮只缩放，不要按住拖就跑掉视角 —— 这是个半身像，不是模型查看器。
    // 但转一点角度是有用的（看侧面的动作），所以旋转保留、幅度由极角限制
    controls.zoomSpeed = 0.8;
    controls.rotateSpeed = 0.5;
    controls.dampingFactor = 0.08;
    controls.minPolarAngle = Math.PI * 0.15;
    controls.maxPolarAngle = Math.PI * 0.85;

    const resize = () => {
      const { clientWidth: w, clientHeight: h } = mount;
      if (w === 0 || h === 0) return;
      renderer.setSize(w, h, false);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
    };
    const observer = new ResizeObserver(resize);
    observer.observe(mount);

    /** 口型 → 落点。匹配逻辑在 avatar/morphMap.ts —— 那里有测试 */
    let targets = new Map<Viseme, Target[]>();
    let blinkTargets: Target[] = [];
    let smileTargets: Target[] = [];
    let joyTargets: Target[] = [];
    let angryTargets: Target[] = [];
    let sorrowTargets: Target[] = [];
    let funTargets: Target[] = [];
    let surprisedTargets: Target[] = [];
    let root: THREE.Object3D | null = null;
    /** 释放为外部资源创建的 blob URL。不释放就是内存泄漏 */
    let release: () => void = () => {};
    let bones: Bones | null = null;
    let restPose: IdleState | null = null;
    let mixer: THREE.AnimationMixer | null = null;
    let cleanupReframe: (() => void) | null = null;
    let playedActionSeq = -1;
    let fadeOutAt = 0;
    let playing: THREE.AnimationAction | null = null;
    // 动作重定向用的全量骨骼表。**这一张表决定肩膀/手指/腿动不动**
    let humanoidAll: Map<string, THREE.Object3D> | undefined;
    // 视线跟随。眼睛的"鲜活"收益远高于身体动作 —— 见 avatar/gaze.ts
    const gaze = new Gaze();
    const eyeRestMap = new Map<THREE.Object3D, THREE.Quaternion>();
    const _gazeEuler = new THREE.Euler();
    const _gazeTmp = new THREE.Quaternion();
    let eyeL: THREE.Object3D | null = null;
    let eyeR: THREE.Object3D | null = null;
    let gazeTarget: { x: number; y: number } | null = null;
    const onPointerMove = (e: PointerEvent) => {
      // 角色层在轮询系统光标时以那一份为准 —— 见下面 readGaze 的说明
      if (hasDesktopCursor()) return;
      gazeTarget = targetFromPointer(e.clientX, e.clientY, mount.getBoundingClientRect());
    };
    // 光标离开窗口就没有目标了，视线会自己回正（而不是僵在最后那个方向）
    const onPointerLeave = () => {
      if (hasDesktopCursor()) return;
      gazeTarget = null;
    };
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerleave", onPointerLeave);

    /**
     * 这一帧该看哪儿。
     *
     * **角色层收不到 pointermove** —— 那个窗口是鼠标穿透的，浏览器
     * 拿不到任何指针事件。所以它自己轮询系统光标并喂进
     * `desktopCursor`，这里优先读那一份。别的窗口没人喂，
     * 就退回普通的 pointermove。
     */
    const readGaze = () => {
      if (!hasDesktopCursor()) return gazeTarget;
      const c = desktopCursor();
      if (!c) return null;
      const r = mount.getBoundingClientRect();
      return targetFromPointer(c.x + r.left, c.y + r.top, r);
    };

    (async () => {
      try {
        let humanoidHint: Partial<Bones> | undefined;
        if (modelId === DEFAULT_HEAD_ID) {
          root = buildDefaultHead();
        } else {
          const loaded = await loadModel(modelId);
          if (disposed) {
            loaded.release();
            return;
          }
          root = loaded.scene;
          humanoidHint = loaded.humanoid;
          humanoidAll = loaded.humanoidAll;
          // VRM 的 humanoid 里就有眼骨。没有的话只转头 —— 仍然比不动强
          eyeL = loaded.humanoidAll.get("leftEye") ?? null;
          eyeR = loaded.humanoidAll.get("rightEye") ?? null;
          if (eyeL) eyeRestMap.set(eyeL, eyeL.quaternion.clone());
          if (eyeR) eyeRestMap.set(eyeR, eyeR.quaternion.clone());
          release = loaded.release;
        }
        scene.add(root);

        // 自动缩放矫正：某些 MMD / FBX / VRM 模型导出单位为 cm (厘米) 或 mm (毫米)，
        // 导致模型高度为 170m 或 0.017m，超出相机视锥或极其微小。
        // 这里将模型自动归一化到标准人体高度 (约 1.65 米)
        const initialBox = new THREE.Box3().setFromObject(root);
        const initialSize = initialBox.getSize(new THREE.Vector3());
        if (initialSize.y > 0.001) {
          if (initialSize.y > 10 || initialSize.y < 0.3) {
            const scale = 1.65 / initialSize.y;
            root.scale.setScalar(scale);
            root.updateMatrixWorld(true);
          }
        }

        // 蒙皮网格与材质初始化：在对框相机前先禁用视锥剔除并修正材质
        root.traverse((o) => {
          const m = o as THREE.Mesh;
          if (!m.isMesh) return;
          if ((m as THREE.SkinnedMesh).isSkinnedMesh) m.frustumCulled = false;
          if (m.material) {
            const mats = Array.isArray(m.material) ? m.material : [m.material];
            mats.forEach((mat) => {
              mat.side = THREE.DoubleSide;
            });
          }
        });

        // 拦截全息粒子渲染模式
        if (hologramMode) {
          applyHologramEffect(root, hologramStyle, hologramColor);
        }

        const map = mapMorphTargets(root);
        targets = map.targets;
        blinkTargets = map.blink;
        smileTargets = map.smile;
        joyTargets = map.joy;
        angryTargets = map.angry;
        sorrowTargets = map.sorrow;
        funTargets = map.fun;
        surprisedTargets = map.surprised;
        const { scheme, found } = map;

        // 摆姿势必须在框相机之前：T-pose 的双臂展开会把包围盒撑得很宽，
        // 而相机要按摆好之后的样子去对
        bones = findBones(root, humanoidHint);
        relaxArms(bones);
        root.updateMatrixWorld(true);
        restPose = captureRest(bones);
        
        // 挂载动作播放器
        mixer = new THREE.AnimationMixer(root);

        // 双击复位取景。**拉远之后必须有办法回来** ——
        // 没有的话用户滚出去就只能重启应用，那等于这个缩放不敢用
        const reframe = () => {
          if (!root) return;
          frameFace(root, map.faceMeshes, bones?.head ?? null, camera, controls, bodyHeads);
        };
        renderer.domElement.addEventListener("dblclick", reframe);
        cleanupReframe = () => renderer.domElement.removeEventListener("dblclick", reframe);

        // 按**脸**定位相机，不按整体包围盒。理由见 MorphMap.faceMeshes
        frameFace(root, map.faceMeshes, bones.head, camera, controls, bodyHeads);

        // 把这次加载的实情写进日志。"模型不显示"在画面上没有任何线索，
        // 而这几个数字（网格数、包围盒、相机位置）一眼就能定位是
        // 没加载、还是加载了但相机没对上、还是几何是空的
        const box = new THREE.Box3().setFromObject(root);
        let meshes = 0;
        let skinned = 0;
        let tris = 0;
        root.traverse((o) => {
          const m = o as THREE.Mesh;
          if (!m.isMesh) return;
          meshes++;
          if ((m as THREE.SkinnedMesh).isSkinnedMesh) skinned++;
          const pos = m.geometry?.getAttribute("position");
          if (pos) tris += pos.count / 3;
          // **蒙皮网格关掉视锥剔除。** three 用绑定姿势的几何算包围球，
          // 骨骼把顶点带到别处之后那个球就不准了 —— 结果是模型整个被剔掉、
          // 画面上什么都没有，而且不报任何错
          if ((m as THREE.SkinnedMesh).isSkinnedMesh) m.frustumCulled = false;
        });
        logUi(
          "avatar",
          `${modelId} 网格${meshes}(蒙皮${skinned}) 面${Math.round(tris)} ` +
            `包围盒 y[${box.min.y.toFixed(2)},${box.max.y.toFixed(2)}] ` +
            `x[${box.min.x.toFixed(2)},${box.max.x.toFixed(2)}] ` +
            `通道${targets.size}/${scheme} 骨骼头=${bones.head?.name ?? "无"} ` +
            `相机 y=${camera.position.y.toFixed(2)} z=${camera.position.z.toFixed(2)}`,
        );

        onReport?.({ channels: targets.size, scheme, found });
        if (targets.size === 0) {
          setError(
            found.length === 0
              ? "这个模型完全没有 morph target —— 导出时没勾口型/表情，见 avatars/README.md"
              : `模型有 ${found.length} 个 morph target，但没有一个是认得的口型命名。` +
                `头几个是：${found.slice(0, 6).join("、")}`,
          );
        }
      } catch (err) {
        if (disposed) return;
        const message = `${err}`;
        setError(message);
        logUi("avatar", `${modelId} 加载失败：${message}`);
        onReport?.({ channels: 0, scheme: "none", found: [], error: message });
      }
    })();

    // 眨眼：随机间隔。固定节奏比不眨还假
    let nextBlink = performance.now() + 2000;
    let blinkPhase = -1;
    
    // 情绪表情状态
    let smoothSmile = 0;
    let smoothJoy = 0;
    let smoothAngry = 0;
    let smoothSorrow = 0;
    let smoothFun = 0;
    let smoothSurprised = 0;

    /**
     * 每帧要先清零的落点。
     *
     * 下面用 `Math.max` 往通道里写（同一个 ARKit 通道会被多个口型写到），
     * 而 max 只会往上走 —— **不先清零的话嘴张开一次就再也回不去了**。
     * 这一步和取 max 是一对，少一半就坏。
     */
    const touched: Target[] = [];
    const remember = (t: Target) => {
      if (!touched.includes(t)) touched.push(t);
    };

    const clock = new THREE.Clock();

    const tick = () => {
      const delta = clock.getDelta();
      const now = performance.now();
      const w = mouth.sample();

      for (const t of touched) t.influences[t.index] = 0;

      let talkIntensitySum = 0;
      for (const [v, bucket] of targets) {
        const value = w[v] ?? 0;
        talkIntensitySum += value;
        for (const t of bucket) {
          remember(t);
          // 同一通道可能被多个口型写到（ARKit 降级时 jawOpen 到处都用），
          // 所以取最大而不是覆盖 —— 覆盖会让后算的那个把前面的抹掉
          t.influences[t.index] = Math.max(t.influences[t.index], value * t.gain);
        }
      }

      // 如果启用了全息模式，更新 Shader 的 Uniform
      if (hologramMode && root) {
        updateHologram(root, now / 1000, Math.min(1, talkIntensitySum * 0.8));
      }

      // 眨眼
      if (blinkPhase < 0 && now > nextBlink) {
        blinkPhase = 0;
        nextBlink = now + 2200 + Math.random() * 3600;
      }
      if (blinkPhase >= 0) {
        // **要用真实帧间隔，不能写死 1/60。** rAF 在 ProMotion / 120Hz 屏上
        // 一秒回调 120 次，写死 60 的话眨眼会快一倍（60ms 而不是 120ms）——
        // 看起来是抽搐，不是眨眼。而这个 bug 只在高刷屏上出现
        blinkPhase += delta;
        // 120ms 闭合再张开
        const k = Math.sin(Math.min(1, blinkPhase / 0.12) * Math.PI);
        for (const t of blinkTargets) {
          remember(t);
          t.influences[t.index] = k;
        }
        if (blinkPhase > 0.12) blinkPhase = -1;
      }

      // 情绪 → 表情插值。慢速趋近，情绪突变比嘴型突变更刺眼
      const currentEmo = emotionRef.current;
      const mood = currentEmo?.mood ?? 0;
      const energy = currentEmo?.energy ?? 0;
      const irritation = currentEmo?.irritation ?? 0;

      const targetSmile = Math.max(0, mood / 100) * 0.45;
      const targetJoy = mood > 10 && energy > 10 ? Math.min(1, (mood + energy) / 150) : 0;
      const targetSorrow = mood < -10 ? Math.min(1, -mood / 100) : 0;
      const targetAngry = irritation > 10 ? Math.min(1, irritation / 100) : 0;
      const targetFun = mood > 30 && energy > 50 ? Math.min(1, (mood + energy) / 200) : 0;
      const targetSurprised = energy > 50 && mood > 10 ? Math.min(1, (energy - 50) / 100) : 0;
      
      smoothSmile += (targetSmile - smoothSmile) * 0.02;
      smoothJoy += (targetJoy - smoothJoy) * 0.05;
      smoothSorrow += (targetSorrow - smoothSorrow) * 0.03;
      smoothAngry += (targetAngry - smoothAngry) * 0.06;
      smoothFun += (targetFun - smoothFun) * 0.04;
      smoothSurprised += (targetSurprised - smoothSurprised) * 0.08;

      for (const t of smileTargets) {
        remember(t);
        t.influences[t.index] = Math.max(t.influences[t.index], smoothSmile);
      }
      for (const t of joyTargets) {
        remember(t);
        t.influences[t.index] = Math.max(t.influences[t.index], smoothJoy);
      }
      for (const t of sorrowTargets) {
        remember(t);
        t.influences[t.index] = Math.max(t.influences[t.index], smoothSorrow);
      }
      for (const t of angryTargets) {
        remember(t);
        t.influences[t.index] = Math.max(t.influences[t.index], smoothAngry);
      }
      for (const t of funTargets) {
        remember(t);
        t.influences[t.index] = Math.max(t.influences[t.index], smoothFun);
      }
      for (const t of surprisedTargets) {
        remember(t);
        t.influences[t.index] = Math.max(t.influences[t.index], smoothSurprised);
      }

      // 动作系统
      // **按序号触发，不是按名字。** 按名字的话「再挥一次手」永远不会播 ——
      // 名字没变，条件就不成立。这正是"我让它挥手，它没反应"的直接原因
      const currentAction = actionRef.current;
      if (currentAction && actionSeqRef.current !== playedActionSeq && bones && mixer && restPose) {
        playedActionSeq = actionSeqRef.current;
        // 取片段是异步的（可能要去读 .vrma），拿到再播。
        // 期间 idle 照常跑，所以延迟表现为"晚半拍"而不是卡住
        const theBones = bones;
        const theRest = restPose;
        const theMixer = mixer;
        const theAll = humanoidAll;
        void clipFor(currentAction, theBones, theRest, modelId, theAll).then((clip) => {
          if (disposed || !clip || clip.tracks.length === 0) {
            if (!clip || clip.tracks.length === 0) {
              // 片段是空的说明这个模型没有对应的骨骼 —— 画面上什么都不会发生
              // 而且不报错，必须说出来
              logUi("avatar", `动作播不了（没有可驱动的骨骼）：${currentAction}`);
            }
            return;
          }
          theMixer.stopAllAction();
          const actionObj = theMixer.clipAction(clip);
          actionObj.setLoop(THREE.LoopOnce, 1);
          // **不能停在最后一帧。** clampWhenFinished 会让 mixer 一直施加片段的
          // 收尾姿态，而 VRMA 的收尾往往是个摆好的 pose（手举着）——
          // 人物就再也放不下手了。播完让 idle 接管，手臂由 idle 每帧还原
          actionObj.clampWhenFinished = false;
          // 淡入而不是硬切：直接 play 会从当前的呼吸姿态瞬间跳到片段第一帧
          actionObj.reset().fadeIn(ACTION_FADE).play();
          // **每个片段都要安排淡出，不只是超长的那些。**
          //
          // 原先只有 duration > 6s 才淡出，普通长度的片段是这样收尾的：
          // LoopOnce 播完 → three 立刻停掉这个 action → 骨骼瞬间弹回
          // "动作开始那一帧捕获的姿态" → 下一帧 idle 接管。中间**一帧过渡都没有**。
          // 这就是"动作很突然"。
          //
          // 现在按 min(片段长度, 上限) 提前 ACTION_FADE_OUT 开始淡出，
          // 和 idle 有一段重叠 —— 手是"放下来"的，不是"啪一下回去"
          const runFor = Math.min(clip.duration, MAX_ACTION_SECONDS);
          fadeOutAt =
            performance.now() / 1000 + Math.max(runFor - ACTION_FADE_OUT, ACTION_FADE);
          playing = actionObj;
        });
      }

      if (fadeOutAt && performance.now() / 1000 >= fadeOutAt) {
        playing?.fadeOut(ACTION_FADE_OUT);
        fadeOutAt = 0;
      }

      let isPlayingAction = false;
      if (mixer) {
        mixer.update(delta);
        for (let i = 0; i < (mixer as any)._actions.length; i++) {
          const actionObj = (mixer as any)._actions[i];
          if (actionObj.isRunning()) {
            isPlayingAction = true;
            break;
          }
        }
      }

      const t = now / 1000;

      if (bones && restPose && !isPlayingAction) {
        idle(bones, restPose, t, currentEmo);
        // **必须叠在 idle 之后**：idle 每帧重写 head 的四元数，
        // 先写视线的话会被它抹掉 —— 症状是"眼睛动、头完全不动"
        const g = gaze.update(readGaze(), delta);
        if (bones.head) {
          bones.head.rotateY(g.headYaw);
          bones.head.rotateX(g.headPitch);
        }
        // 眼骨有就转眼球。没有的话上面那段只转头，仍然比完全不动强
        for (const eye of [eyeL, eyeR]) {
          if (!eye) continue;
          const rest = eyeRestMap.get(eye);
          if (rest) {
            _gazeEuler.set(g.eyePitch, g.eyeYaw, 0);
            _gazeTmp.setFromEuler(_gazeEuler);
            eye.quaternion.copy(rest).multiply(_gazeTmp);
          } else {
            eye.rotation.set(g.eyePitch, g.eyeYaw, 0);
          }
        }
      } else if (root && !isPlayingAction) {
        root.rotation.y = Math.sin(t * 0.31) * 0.045;
        root.rotation.x = Math.sin(t * 0.23 + 1.1) * 0.02;
      }

      controls.update();

      renderer.render(scene, camera);
      raf = requestAnimationFrame(tick);
    };
    resize();
    raf = requestAnimationFrame(tick);

    return () => {
      disposed = true;
      try {
        window.removeEventListener("pointermove", onPointerMove);
        window.removeEventListener("pointerleave", onPointerLeave);
      } catch {}
      try {
        cancelAnimationFrame(raf);
      } catch {}
      try {
        observer.disconnect();
      } catch {}
      // 必须显式释放：换模型时不释放会把显存一路吃光
      try {
        scene.traverse((obj) => {
          const mesh = obj as THREE.Mesh;
          mesh.geometry?.dispose();
          const mat = mesh.material;
          if (Array.isArray(mat)) mat.forEach((m) => m.dispose());
          else mat?.dispose();
        });
      } catch {}
      try {
        cleanupReframe?.();
      } catch {}
      try {
        controls.dispose();
      } catch {}
      try {
        release();
      } catch {}
      try {
        renderer.dispose();
      } catch {}
      try {
        renderer.domElement.remove();
      } catch {}
    };
    // modelId、全息参数或表现模式变了要整个重建
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [modelId, hologramMode, hologramStyle, hologramColor, bodyHeads]);

  return (
    <div className={className} ref={mountRef}>
      {error && <div className="avatar3d__error">{error}</div>}
    </div>
  );
}

/**
 * 读出模型并解析。
 *
 * `.glb` / `.vrm` 是自包含的，一次读完就行。**`.gltf` 不是** —— 它是 JSON，
 * 几何数据在同目录的 `.bin` 里、贴图在 `textures/` 里，全都用相对路径引用。
 *
 * 而前端没有文件系统权限，GLTFLoader 自己去 fetch 那些相对路径只会全部失败 ——
 * 失败的结果是**加载出一个没有贴图、甚至没有几何体的模型，而且不报错**。
 * 所以这里先把 JSON 里引用到的文件全部通过 command 读回来、做成 blob URL，
 * 再用 `setURLModifier` 把加载器的请求重定向过去。
 */
async function loadModel(
  modelId: string,
): Promise<{
  scene: THREE.Object3D;
  release: () => void;
  humanoid: Partial<Bones>;
  /** 全量标准骨骼 —— 动作重定向要用，见 humanoid.ts */
  humanoidAll: Map<string, THREE.Object3D>;
}> {
  const bytes = await avatarModelBytes(modelId);

  // .glb / .vrm：二进制，自包含
  if (!modelId.toLowerCase().endsWith(".gltf")) {
    const gltf = await new GLTFLoader().parseAsync(bytes, "");
    // three 只读 mesh 层的 targetNames，很多导出器写在 primitive 层 ——
    // 不补的话通道会全叫 "0"、"1"、"2"，按名字的匹配全废
    restoreMorphNames(gltf);
    // 退化的顶点色会让模型完全不显示（alpha 测试丢掉每一个片元）
    // 或者整个染成一片黄绿，两种都不报错。见 vertexColors.ts
    disableDegenerateVertexColors(gltf.scene);
    // VRM 0.x 朝向 -Z，不转就是背对着你
    faceCamera(gltf);
    // VRM 自带的骨骼映射 —— 比按名字猜可靠，见 humanoid.ts
    return {
      scene: gltf.scene,
      release: () => {},
      humanoid: readHumanoidBones(gltf),
      humanoidAll: readAllHumanoidNodes(gltf),
    };
  }

  const text = new TextDecoder().decode(bytes);
  const json = JSON.parse(text) as {
    buffers?: Array<{ uri?: string }>;
    images?: Array<{ uri?: string }>;
  };
  // 模型所在目录 —— 相对路径要从这里拼
  const dir = modelId.includes("/")
    ? modelId.slice(0, modelId.lastIndexOf("/") + 1)
    : "";

  const uris = new Set<string>();
  for (const b of json.buffers ?? []) {
    // data: 开头的是内联的，不用去读文件
    if (b.uri && !b.uri.startsWith("data:")) uris.add(b.uri);
  }
  for (const image of json.images ?? []) {
    if (image.uri && !image.uri.startsWith("data:")) uris.add(image.uri);
  }

  const blobs = new Map<string, string>();
  for (const uri of uris) {
    try {
      // URI 是百分号编码的（含空格或中文的文件名很常见），要解码再拼路径
      const raw = await avatarModelBytes(dir + decodeURIComponent(uri));
      blobs.set(uri, URL.createObjectURL(new Blob([raw])));
    } catch {
      // 单个贴图缺失不该让整个模型加载失败 —— 缺贴图的模型仍然能对口型。
      // 这里静默跳过，GLTFLoader 会用默认材质
    }
  }

  const manager = new THREE.LoadingManager();
  manager.setURLModifier((url) => blobs.get(url) ?? url);
  const gltf = await new GLTFLoader(manager).parseAsync(text, "");
  restoreMorphNames(gltf);
  disableDegenerateVertexColors(gltf.scene);
  faceCamera(gltf);
  return {
    scene: gltf.scene,
    humanoid: readHumanoidBones(gltf),
    humanoidAll: readAllHumanoidNodes(gltf),
    release: () => {
      for (const url of blobs.values()) URL.revokeObjectURL(url);
    },
  };
}

/**
 * 把相机摆到半身像的位置。
 *
 * 基准是**带口型通道的那些 mesh**（那就是脸），不是整个模型的包围盒 ——
 * 理由见 `MorphMap.faceMeshes`。取景的几何在 `avatar/framing.ts`，那里有测试。
 */
function frameFace(
  root: THREE.Object3D,
  faceMeshes: THREE.Object3D[],
  headBone: THREE.Object3D | null,
  camera: THREE.PerspectiveCamera,
  controls: OrbitControls,
  bodyHeads?: number,
) {
  const head = new THREE.Box3();
  for (const mesh of faceMeshes) head.expandByObject(mesh);
  const model = new THREE.Box3().setFromObject(root);
  // 一个口型通道都没认出来时退回整体 —— 至少让人看得见模型，
  // 而"为什么嘴不动"由设置面板的通道数去解释
  if (head.isEmpty()) head.copy(model);

  // 头骨骼的世界坐标优先 —— 蒙皮网格的包围盒**位置**不可靠（几何体在绑定
  // 空间里），实测会让相机瞄到空处、表现成"模型不显示"。尺寸仍用包围盒
  let headPos: THREE.Vector3 | undefined;
  if (headBone) {
    headBone.updateWorldMatrix(true, false);
    headPos = new THREE.Vector3().setFromMatrixPosition(headBone.matrixWorld);
  }

  const { center, distance } = portraitFraming(
    head,
    model,
    camera.fov,
    camera.aspect,
    headPos,
    bodyHeads,
  );

  camera.position.set(center.x, center.y, center.z + distance);
  controls.target.copy(center);
  // 缩放范围按取景距离定，不写死 —— 模型的尺度可能差好几个数量级
  controls.minDistance = distance * 0.2;
  // 上限放宽到 12 倍：半身构图下看不全 squat / spin 这类全身动作，
  // 用户得能滚轮拉到看得见脚
  controls.maxDistance = distance * 12;
  controls.update();
}
