"""
底模预处理与缓存。

## 为什么要预处理

Wav2Lip 能实时，靠的不是模型快，而是**每帧的人脸框预先算好**。
运行时只做「取裁剪 → 推理 → 贴回」，不跑检测。原版部署包里那个
353MB 的 `wav2lip256_avatar1.tar.gz` 就是这个东西：
`full_imgs/` + `face_imgs/` + `coords.pkl`。

我们的片段每段只有 81 帧，比它小 7 倍，预处理是秒级的。

## 人脸框要平滑

逐帧独立检测出来的框会抖 —— 哪怕人没动，框也会一两像素地跳。
贴回去之后表现为**嘴部区域整体在颤**，比不做口型还难看。
所以检测完沿时间轴做一次滑动平均。
"""

from __future__ import annotations

import json
import math
from pathlib import Path

import cv2
import numpy as np

IMG_SIZE = 256
SMOOTH_WINDOW = 5

# Haar 框 → 训练时那种框的换算系数。
#
# **这个数是量出来的，不是调出来的。** 模型是在 S3FD 的框上训练的，
# 换个检测器框得不一样，输入分布就偏了 —— 表现是「嘴在动但看着怪」，
# 而且没有任何报错可循。
#
# 原部署包里的 `wav2lip256_avatar1` 正好是一份 S3FD 框法的真实样本：
# 把它的 `face_imgs/00000000.png` 用模板匹配塞回 `full_imgs/00000000.png`，
# 量到框是 132×132 @ (205,166)；同一张图上 Haar 给的是 143×143 @ (205,162)。
# 中心相差不到 5px，边长比 132/143 = 0.923。
FACE_BOX_SCALE = 0.92


def _detect_boxes(frames: list[np.ndarray]) -> list[tuple[int, int, int, int]]:
    cascade = cv2.CascadeClassifier(cv2.data.haarcascades + "haarcascade_frontalface_default.xml")

    boxes: list[tuple[int, int, int, int] | None] = []
    for f in frames:
        found = cascade.detectMultiScale(cv2.cvtColor(f, cv2.COLOR_BGR2GRAY), 1.1, 5)
        if len(found) == 0:
            boxes.append(None)
            continue
        x, y, w, h = max(found, key=lambda a: a[2] * a[3])
        boxes.append(_to_training_box(x, y, w, h, f.shape[1], f.shape[0]))

    # 漏检的帧用前面最近的有效框补上。**不能跳过** ——
    # 缺一帧就等于那一帧不做口型，看起来是嘴突然定住
    valid = [b for b in boxes if b is not None]
    if not valid:
        raise ValueError("整段视频一帧人脸都没检测到")
    for i, b in enumerate(boxes):
        if b is None:
            boxes[i] = next((boxes[j] for j in range(i, -1, -1) if boxes[j]), valid[0])

    return _smooth(boxes)  # type: ignore[arg-type]


def _eye_angle(frames: list[np.ndarray], boxes: list[tuple[int, int, int, int]]) -> float:
    """整段素材的人脸倾角（度）。

    **wav2lip 是在大致正立的人脸上训练的。** 头歪着时它画出来的嘴是正的，
    而贴回去的脸是斜的 —— 两者对不上，嘴角会拖出一道смear。
    实测：雏田那张倚在手上的图倾了 +12.2°，小雅和 SeeU 都在 ±0.5° 内，
    而只有雏田出现了那道伪影。

    取整段的中位数而不是逐帧：一段素材里头的倾斜基本不变，
    逐帧测反而会因为个别帧误检而抖，抖起来比不对齐更难看。
    """
    cascade = cv2.CascadeClassifier(cv2.data.haarcascades + "haarcascade_eye.xml")
    angles = []
    step = max(1, len(frames) // 12)
    for f, (x1, y1, x2, y2) in list(zip(frames, boxes))[::step]:
        crop = cv2.resize(f[y1:y2, x1:x2], (IMG_SIZE, IMG_SIZE))
        eyes = cascade.detectMultiScale(cv2.cvtColor(crop, cv2.COLOR_BGR2GRAY), 1.05, 6, minSize=(24, 24))
        # 只认上半张脸里的 —— 鼻孔和嘴角常被误判成眼睛
        eyes = [e for e in eyes if e[1] + e[3] / 2 < IMG_SIZE * 0.63]
        if len(eyes) < 2:
            continue
        left, right = sorted(eyes, key=lambda e: e[0])[:2]
        dy = (right[1] + right[3] / 2) - (left[1] + left[3] / 2)
        dx = (right[0] + right[2] / 2) - (left[0] + left[2] / 2)
        angles.append(math.degrees(math.atan2(dy, dx)))

    if not angles:
        return 0.0
    angle = float(np.median(angles))
    # 大角度多半是误检（人不会歪 40°坐着说话），宁可不转
    return angle if abs(angle) <= 25.0 else 0.0


def _to_training_box(x: int, y: int, w: int, h: int, iw: int, ih: int) -> tuple[int, int, int, int]:
    """Haar 框按测得的比例缩到训练时那种框，中心不动。"""
    cx, cy = x + w / 2, y + h / 2
    half = max(w, h) * FACE_BOX_SCALE / 2
    return (
        max(0, int(cx - half)),
        max(0, int(cy - half)),
        min(iw, int(cx + half)),
        min(ih, int(cy + half)),
    )


def _smooth(boxes: list[tuple[int, int, int, int]]) -> list[tuple[int, int, int, int]]:
    """平滑框：**边长锁死，只让中心动。**

    模型的输出恒为 256×256，贴回时按框的边长缩放。所以边长每帧浮动一点，
    下半脸就跟着缩放一点 —— 实测同一段里边长在 303~314 之间晃（±1.8%），
    表现为脸有种说不出的「呼吸感」，看着像形变，其实是缩放在抖。

    边长取整段的中位数（不是均值 —— 个别帧漏检补进来的框会把均值带偏），
    中心仍然沿时间轴滑动平均，这样头部真的移动时还跟得住。
    """
    arr = np.array(boxes, dtype=np.float32)
    cx = (arr[:, 0] + arr[:, 2]) / 2
    cy = (arr[:, 1] + arr[:, 3]) / 2
    side = float(np.median(arr[:, 2] - arr[:, 0]))

    half = SMOOTH_WINDOW // 2
    out = []
    for i in range(len(arr)):
        lo, hi = max(0, i - half), min(len(arr), i + half + 1)
        mx, my = cx[lo:hi].mean(), cy[lo:hi].mean()
        out.append((
            int(mx - side / 2), int(my - side / 2),
            int(mx + side / 2), int(my + side / 2),
        ))
    return out


# 静图当一帧的视频。wav2lip 本来就是逐帧工作的，底模是不是在动对它没区别 ——
# 动的是嘴。推流帧率随便定，因为梅尔的取列只跟经过的时间有关，fps 会约掉
STILL_FPS = 25.0


# 工作分辨率。**不是画质设置，是让人脸落在模型的量程里。**
#
# 模型永远看 256×256 的裁剪。原图 4752px 高时人脸框有 2012px，缩到 256 再放回
# 2012px，等于把嘴那一块降到十二分之一再拉回来 —— 糊得没法看。
# 而且那么大的帧编成 JPEG 推流也不现实。
# 缩到 720 高之后人脸大约 300px，和 256 是同一量级。
WORK_HEIGHT = 720
# 静图取景成竖构图，和桌宠窗口一致
STILL_ASPECT = 3 / 4


def _read_source(path: str) -> tuple[list[np.ndarray], float, list | None]:
    """视频解成帧序列；静图取景后解成只有一帧的序列。

    第三个返回值是**已知的人脸框**（静图取景时顺手算出来的）。给出来就不用
    再检测一次 —— 而且更可靠：取景之后脸占了大半个画面、还常有手挡着或侧倾，
    Haar 在那种构图上反而认不出来（实测这张图就是），表现为「换了这个形态
    就完全没有口型」。
    """
    if Path(path).suffix.lower() in {".jpg", ".jpeg", ".png", ".webp", ".bmp"}:
        img = cv2.imread(path)
        if img is None:
            raise FileNotFoundError(f"读不出这张图：{path}")
        framed, box = _frame_still(img)
        return [framed], STILL_FPS, ([box] if box else None)

    cap = cv2.VideoCapture(path)
    if not cap.isOpened():
        raise FileNotFoundError(path)
    fps = cap.get(cv2.CAP_PROP_FPS) or 25.0
    frames = []
    while True:
        ok, frame = cap.read()
        if not ok:
            break
        frames.append(_limit_height(frame))
    cap.release()
    if not frames:
        raise ValueError(f"没有解出任何帧：{path}")
    return frames, fps, None


def _limit_height(img: np.ndarray) -> np.ndarray:
    h = img.shape[0]
    if h <= WORK_HEIGHT:
        return img
    scale = WORK_HEIGHT / h
    return cv2.resize(img, (int(img.shape[1] * scale), WORK_HEIGHT), interpolation=cv2.INTER_AREA)


def _frame_still(img: np.ndarray) -> tuple[np.ndarray, tuple[int, int, int, int] | None]:
    """静图按人脸取景成竖构图。

    **不能居中裁。** 写真的构图千奇百怪，人脸常常不在正中 ——
    实测那张 8448×4752 的图里脸在右侧 62%~86% 处，居中裁会把脸整个切掉，
    然后表现为「换了这个形态就完全没有口型」（检测不到脸）。
    """
    probe = _limit_height(img)
    scale = img.shape[0] / probe.shape[0]
    cascade = cv2.CascadeClassifier(cv2.data.haarcascades + "haarcascade_frontalface_default.xml")
    found = cascade.detectMultiScale(cv2.cvtColor(probe, cv2.COLOR_BGR2GRAY), 1.1, 5)

    h, w = img.shape[:2]
    face = None
    if len(found) == 0:
        # 找不到脸就居中裁，至少还有画面。口型那一步会再报一次
        cx, cy = w / 2, h / 2
        crop_h = h
    else:
        fx, fy, fw, fh = max(found, key=lambda a: a[2] * a[3])
        cx = (fx + fw / 2) * scale
        # 脸放在竖构图的上三分之一处，下面留出身体
        crop_h = min(h, fh * scale * 4.5)
        cy = (fy + fh / 2) * scale + crop_h * 0.15
        face = (fx * scale, fy * scale, fw * scale, fh * scale)

    crop_w = crop_h * STILL_ASPECT
    if crop_w > w:
        crop_w = w
        crop_h = crop_w / STILL_ASPECT

    x1 = int(max(0, min(w - crop_w, cx - crop_w / 2)))
    y1 = int(max(0, min(h - crop_h, cy - crop_h / 2)))
    cropped = img[y1 : y1 + int(crop_h), x1 : x1 + int(crop_w)]
    out_w, out_h = int(WORK_HEIGHT * STILL_ASPECT), WORK_HEIGHT
    framed = cv2.resize(cropped, (out_w, out_h), interpolation=cv2.INTER_AREA)

    if face is None:
        return framed, None
    # 把原图坐标系里的脸换算到取景后的坐标系
    k = out_h / crop_h
    fx, fy, fw, fh = face
    return framed, _to_training_box(
        int((fx - x1) * k), int((fy - y1) * k), int(fw * k), int(fh * k), out_w, out_h
    )


class Avatar:
    """一段底模视频的预处理产物：全帧 + 人脸框 + 256×256 裁剪。"""

    def __init__(self, frames: np.ndarray, faces: np.ndarray, coords: list, fps: float, angle: float = 0.0):
        self.frames = frames
        self.faces = faces
        self.coords = coords
        self.fps = fps
        self.angle = angle
        self.count = len(frames)

    @staticmethod
    def build(video_path: str, cache_dir: Path) -> "Avatar":
        cache_dir.mkdir(parents=True, exist_ok=True)
        meta_path = cache_dir / "meta.json"
        if meta_path.exists():
            meta = json.loads(meta_path.read_text())
            return Avatar(
                np.load(cache_dir / "frames.npy", mmap_mode="r"),
                np.load(cache_dir / "faces.npy", mmap_mode="r"),
                meta["coords"],
                meta["fps"],
                meta.get("angle", 0.0),
            )

        frames, fps, known = _read_source(video_path)

        coords = known if known else _detect_boxes(frames)
        angle = _eye_angle(frames, coords)
        faces = np.stack([
            cv2.warpAffine(f, _affine(box, angle), (IMG_SIZE, IMG_SIZE), flags=cv2.INTER_AREA)
            for f, box in zip(frames, coords)
        ])

        frames_arr = np.stack(frames)
        np.save(cache_dir / "frames.npy", frames_arr)
        np.save(cache_dir / "faces.npy", faces)
        meta_path.write_text(
            json.dumps({"coords": [list(c) for c in coords], "fps": fps, "angle": angle})
        )
        return Avatar(frames_arr, faces, [list(c) for c in coords], fps, angle)

    def batch_input(self, indices: list[int]) -> np.ndarray:
        """按 Wav2Lip 的约定拼 6 通道：[下半脸置零, 同帧原图]。

        参考通道给的是**同一帧的完整脸** —— 模型靠它拿身份和光照，
        靠置零那一半决定重画什么。两者顺序反了模型不会报错，
        出来的是一张糊脸。
        """
        faces = self.faces[indices].astype(np.float32)
        masked = faces.copy()
        masked[:, IMG_SIZE // 2 :] = 0
        stacked = np.concatenate((masked, faces), axis=3) / 255.0
        return stacked.transpose(0, 3, 1, 2)

    def paste(self, index: int, face: np.ndarray) -> np.ndarray:
        """把重绘好的脸转回原来的角度、贴回全帧，**只贴嘴那一块椭圆**。

        ## 为什么不是整框、也不是下半张

        模型即使拿到**纯静音**也会把脸重画一遍，和原图差 2.4~3.0 灰阶
        （下半脸 3.6~4.5）—— 那是它的固有重建误差，与说话无关。
        而真正的口型信号在下半脸也只有 4.7~6.0 灰阶。

        **噪声和信号一样大。** 贴回的区域越大，就有越多好像素被换成
        重建过的像素，表现就是「嘴是会动了，但脸糊了、还在不正常形变」。

        椭圆覆盖 84% 的口型信号，只占 29% 的面积；剩下那 71% 贴过去基本
        只有噪声，所以不贴。代价是下颌大幅运动会被裁在椭圆外 ——
        真人说话时下颌位移很小，这个取舍划算，但**换成夸张动画角色要重新量**。
        """
        out = np.array(self.frames[index])
        h, w = out.shape[:2]
        m = _affine(self.coords[index], self.angle)

        # 逆变换把 256 的输出摊回原图坐标系（含转回原来的倾角）
        redrawn = cv2.warpAffine(face, m, (w, h), flags=cv2.WARP_INVERSE_MAP | cv2.INTER_LINEAR)
        alpha = cv2.warpAffine(
            _mouth_mask(IMG_SIZE, IMG_SIZE), m, (w, h),
            flags=cv2.WARP_INVERSE_MAP | cv2.INTER_LINEAR,
        )[..., np.newaxis]

        blended = redrawn.astype(np.float32) * alpha + out.astype(np.float32) * (1 - alpha)
        return blended.astype(np.uint8)


def _affine(box, angle: float) -> np.ndarray:
    """人脸框 → 256×256 正立裁剪的仿射矩阵。

    裁剪、旋转、缩放一次算完。**旋转是为了把脸摆正** ——
    模型是在大致正立的人脸上训练的，头歪着时它画出来的嘴和实际唇线对不上。
    """
    x1, y1, x2, y2 = box
    cx, cy = (x1 + x2) / 2, (y1 + y2) / 2
    m = cv2.getRotationMatrix2D((cx, cy), angle, IMG_SIZE / max(1, x2 - x1))
    m[0, 2] += IMG_SIZE / 2 - cx
    m[1, 2] += IMG_SIZE / 2 - cy
    return m


# 椭圆的位置与大小。
#
# **中心不能用「信号重心」算。** 第一版就是那么定的，结果被伪影本身带偏了：
# 脸上有手挡着时，模型在手那边画出的垃圾也算进重心，把中心往手的方向拉
# （实测 0.50 → 0.56），于是蒙版正好把伪影框了进来，越量越错。
#
# 改用人脸裁剪的几何位置 —— Wav2Lip 那种框法里嘴大致就在这儿 ——
# 再按几张真实素材逐个看过定的半径。横向收窄是为了不把嘴角外的
# 手、头发扫进来，那些地方模型画得最不可靠。
MOUTH_CENTER = (0.74, 0.50)   # (行, 列) 相对裁剪
MOUTH_RADIUS = (0.28, 0.28)   # (行, 列) 半径
# 边缘softness：0=硬边（会看见椭圆轮廓）1=整块都在过渡
MOUTH_FEATHER = 0.45

_mask_cache: dict[tuple[int, int], np.ndarray] = {}


def _mouth_mask(h: int, w: int) -> np.ndarray:
    """椭圆羽化蒙版。按尺寸缓存 —— 每帧重算一次是纯浪费。"""
    key = (h, w)
    cached = _mask_cache.get(key)
    if cached is not None:
        return cached

    ys, xs = np.mgrid[0:h, 0:w]
    cy, cx = MOUTH_CENTER[0] * h, MOUTH_CENTER[1] * w
    ry, rx = MOUTH_RADIUS[0] * h, MOUTH_RADIUS[1] * w
    r = np.sqrt(((ys - cy) / ry) ** 2 + ((xs - cx) / rx) ** 2)

    # r<=1-feather 全用重绘，r>=1 全用原图，中间平滑过渡。
    # **必须平滑** —— 硬边会在脸上留下一圈看得见的椭圆
    inner = 1.0 - MOUTH_FEATHER
    alpha = np.clip((1.0 - r) / max(1e-6, 1.0 - inner), 0.0, 1.0)
    alpha = alpha * alpha * (3 - 2 * alpha)  # smoothstep，两端导数为 0
    mask = alpha[..., np.newaxis].astype(np.float32)
    _mask_cache[key] = mask
    return mask
