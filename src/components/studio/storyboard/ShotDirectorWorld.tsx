/**
 * 工作区（分镜页）里打开导演世界 —— 和画布里是同一个编辑器，不用跑画布。
 *
 * 用户 2026-09-27：「工作区没有导演世界的入口吗？还得跑一趟画布，再点击项目，再创建节点，再进入导演世界？」
 *
 * 画布节点和这里的区别只在「存哪」：
 * - 3D 片场、眼高 → 场景资产（一个地点一份，第 1~3 场的拆机间十六镜共用）；
 * - 人偶、相机、运镜路径、最近一次输出 → 这一镜的 `director_scene`（后端只由 `cinemaSetDirectorScene` 写）；
 * - 站位写进分镜、机位绑定 —— 编辑器自己按分镜上下文写，和画布一样。
 *
 * 运镜输出这里不跑深度视频：深度只给 LTX 控制用，而分镜出片走的是 H3。
 *
 * 一镜还没摆过时，打开就沿用同一个场景里最近那一镜的站位（`carry.ts`）—— 人摆一次、换机位拍好几镜。
 */
import { useEffect, useRef, useState } from "react";
import { saveChatImageAttachment } from "../../../api";
import { assetsUpdate, type AssetView } from "../../../api/assets";
import { cinemaSetDirectorScene, type DraftActor, type StoryboardShot } from "../../../api/cinema";
import { canvasSaveVideo } from "../../../api/canvasDepth";
import { DirectorWorldEditor, type DirectorOutput, type DirectorScene, type DirectorTrajectoryOutput } from "../../../canvas/director/DirectorWorldEditor";
import { DEFAULT_EYE_HEIGHT, readActors, readCamera, readNamedCameras } from "../../../canvas/director/scene";
import { inheritFrom } from "../../../canvas/director/carry";
import { createDebouncedSave, createSerialWriter } from "./saveQueue";

/** 这一镜存下的场面 → 编辑器初始值。存的是编辑器给的原样，读的时候按画布节点同一套规则校验 */
export function initialDirectorScene(shot: StoryboardShot, projectId: string, episodeId: string, aspect: string, eyeHeight: number): DirectorScene {
  const saved = (shot.director_scene?.scene ?? null) as Partial<DirectorScene> | null;
  return {
    actors: readActors(saved?.actors ?? []),
    camera: readCamera(saved?.camera, eyeHeight),
    aspect: typeof saved?.aspect === "string" && saved.aspect ? saved.aspect : aspect || "16:9",
    // **上下文永远是这一镜**：存下来的那份可能是从别处拷来的
    context: { projectId, episodeId, shotId: shot.id },
    namedCameras: readNamedCameras(saved?.namedCameras, eyeHeight),
    cameraPaths: Array.isArray(saved?.cameraPaths) ? saved.cameraPaths : [],
    activeCameraId: typeof saved?.activeCameraId === "string" ? saved.activeCameraId : null,
  };
}

/** 初始场面 + 这一镜还没摆过时沿用的那一镜（沿用的只有站位，机位不带） */
export function openingDirectorScene(
  shot: StoryboardShot,
  shots: StoryboardShot[],
  projectId: string,
  episodeId: string,
  aspect: string,
  eyeHeight: number,
): { scene: DirectorScene; carriedFrom: number | null } {
  const scene = initialDirectorScene(shot, projectId, episodeId, aspect, eyeHeight);
  const from = inheritFrom(shots, shot);
  return from ? { scene: { ...scene, actors: from.actors }, carriedFrom: from.shotNum } : { scene, carriedFrom: null };
}

export function ShotDirectorWorld({
  projectId,
  episodeId,
  aspect,
  shot,
  shots,
  sceneAsset,
  onClose,
  onError,
}: {
  projectId: string;
  episodeId: string;
  /** 项目画幅 */
  aspect: string;
  shot: StoryboardShot;
  /** 这一集的全部镜头：沿用站位要找同一个场景里摆过的镜头 */
  shots: StoryboardShot[];
  /** 这一镜的场景资产。没有就是一个空世界（只能摆人、定机位） */
  sceneAsset: AssetView | null;
  onClose: () => void;
  onError: (msg: string) => void;
}) {
  const world = sceneAsset?.scene?.world ?? null;
  const [eyeHeight, setEyeHeight] = useState(sceneAsset?.scene?.pano_calibration?.eye_height ?? DEFAULT_EYE_HEIGHT);
  const [groundHeight, setGroundHeight] = useState(sceneAsset?.scene?.pano_calibration?.ground_height ?? 0);
  const [splatScale, setSplatScale] = useState(world?.scale ?? 1);
  const [correction, setCorrection] = useState({ roll: 0, pitch: 0, yaw: 0 });
  const [frontYawDeg, setFrontYawDeg] = useState(0);
  const [opening] = useState(() => openingDirectorScene(shot, shots, projectId, episodeId, aspect, eyeHeight));

  // 眼高、片场缩放存回场景资产（一个地点共用），停手 800ms 再存。两个值一起写 —— 各写各的会拿旧值互相覆盖。
  // 关窗时 flush：以前卸载只清定时器，调完滑块马上关，最后一次就丢了（审查 2026-09-27）
  const calib = useRef({ eyeHeight, groundHeight, splatScale });
  const [calibSaver] = useState(() =>
    createDebouncedSave(
      async () => {
        if (!sceneAsset) return;
        const { eyeHeight: h, groundHeight: gh, splatScale: sc } = calib.current;
        const base = sceneAsset.scene ?? {
          shape: "", zones: [], entrances: [], anchors: [], relations: [],
          materials: [], views: [], inferred: [], no_extra: [], empty_stage: "required" as const,
        };
        await assetsUpdate({
          scope: sceneAsset.scope,
          id: sceneAsset.id,
          scene: {
            ...base,
            pano_calibration: { eye_height: h, ground_height: gh, scale: sceneAsset.scene?.pano_calibration?.scale ?? 1 },
            ...(base.world ? { world: { ...base.world, scale: sc } } : {}),
          },
        });
      },
      800,
      (e) => onError(`校准没存上：${String(e)}`),
    ),
  );
  // 不是从关闭按钮卸载的（极少）：没存的也发出去，不丢
  useEffect(() => () => void calibSaver.flush().catch(() => undefined), [calibSaver]);
  const saveCalibration = (patch: Partial<{ eyeHeight: number; groundHeight: number; splatScale: number }>) => {
    calib.current = { ...calib.current, ...patch };
    if (sceneAsset) calibSaver.schedule();
  };

  // 场面一个接一个写：旧的不会晚于新的落盘；关窗等队列里全部写完 ——
  // 外面一关就重读分镜，下一镜沿用站位读的就是它
  const [writer] = useState(createSerialWriter);
  const sceneFailed = useRef(false);
  const patch = (p: Parameters<typeof cinemaSetDirectorScene>[3]) =>
    writer.run(() => cinemaSetDirectorScene(projectId, episodeId, shot.id, p)).then(
      (v) => {
        sceneFailed.current = false;
        return v;
      },
      (e) => {
        sceneFailed.current = true;
        onError(`导演世界没存上：${String(e)}`);
      },
    );

  // 存盘没落地先不关，说清楚；再点一次关闭就不等了（不能把人困在窗口里）
  const closing = useRef(false);
  const warned = useRef(false);
  const close = async () => {
    if (closing.current) return;
    closing.current = true;
    let problem: string | null = null;
    await writer.idle();
    if (sceneFailed.current) problem = "这一镜的站位 / 机位";
    try {
      await calibSaver.flush();
    } catch {
      problem = problem ? `${problem}和校准` : "眼高 / 片场缩放校准";
    }
    closing.current = false;
    if (problem && !warned.current) {
      warned.current = true;
      onError(`${problem}没存上，窗口先不关 —— 再点一次关闭就不存直接关`);
      return;
    }
    onClose();
  };

  const onOutput = async ({ combined, envOnly, meta }: DirectorOutput) => {
    const stamp = Date.now();
    const [c, e] = await Promise.all([
      saveChatImageAttachment({ dataUrl: combined, filename: `director_${shot.id}_combined_${stamp}.png` }),
      saveChatImageAttachment({ dataUrl: envOnly, filename: `director_${shot.id}_env-only_${stamp}.png` }),
    ]);
    if (!c?.path || !e?.path) throw new Error("截图没保存下来");
    await patch({ outputs: { combined: c.path, env_only: e.path, actors: meta.actors as unknown as DraftActor[], at: stamp } });
  };

  const onTrajectoryOutput = async (output: DirectorTrajectoryOutput) => {
    const stamp = Date.now();
    const [video, first, last] = await Promise.all([
      canvasSaveVideo(output.videoDataUrl, `director_${shot.id}_trajectory_${stamp}.webm`),
      saveChatImageAttachment({ dataUrl: output.firstFrame, filename: `director_${shot.id}_trajectory_${stamp}_first.png` }),
      saveChatImageAttachment({ dataUrl: output.lastFrame, filename: `director_${shot.id}_trajectory_${stamp}_last.png` }),
    ]);
    await patch({ trajectory: { rgb_video: video.path, first: first.path, last: last.path, at: stamp } });
  };

  return (
    <DirectorWorldEditor
      open
      onClose={() => void close()}
      panoSource={sceneAsset?.cover_url ?? null}
      splatSource={world?.path ? { path: world.path, format: (world.format as "ply" | "ksplat" | "spz") || "ply", convention: world.convention === "opencv_camera" ? "opencv_camera" : undefined, scale: splatScale } : null}
      correction={correction}
      frontYawDeg={frontYawDeg}
      eyeHeight={eyeHeight}
      groundHeight={groundHeight}
      initial={opening.scene}
      carriedFrom={opening.carriedFrom}
      onSave={(scene) => void patch({ scene })}
      onCalibrationChange={(p) => {
        if (p.correction) setCorrection(p.correction);
        if (typeof p.frontYawDeg === "number") setFrontYawDeg(p.frontYawDeg);
        if (typeof p.eyeHeight === "number") {
          setEyeHeight(p.eyeHeight);
          saveCalibration({ eyeHeight: p.eyeHeight });
        }
        if (typeof p.groundHeight === "number") {
          setGroundHeight(p.groundHeight);
          saveCalibration({ groundHeight: p.groundHeight });
        }
        if (typeof p.splatScale === "number") {
          setSplatScale(p.splatScale);
          saveCalibration({ splatScale: p.splatScale });
        }
      }}
      onOutput={onOutput}
      onTrajectoryOutput={onTrajectoryOutput}
    />
  );
}
