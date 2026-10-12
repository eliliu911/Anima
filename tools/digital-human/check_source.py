#!/usr/bin/env python
"""
底模素材体检。**找素材之前先跑这个,别找回来一个个试。**

口型这条链路对素材有几条硬要求,不满足时的表现各不相同、而且都不报错：

- 人脸太小  → 嘴被缩到 256 再放回去,糊成一片
- 检测不到  → 整段直接退回形变引擎,「完全没有口型,只有下巴在位移」
- 嘴被遮挡  → 模型没见过那种构图,会画出伪影（工具测不出来,要自己看）
- 画面不动  → 技术上没问题,但和一张静图差别不大

用法：
    .venv/bin/python check_source.py <文件或目录>...
"""

from __future__ import annotations

import math
import sys
from pathlib import Path

import cv2
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from wav2lip.avatar import IMG_SIZE, WORK_HEIGHT, _limit_height, _to_training_box  # noqa: E402

# 逐帧全跑太慢，而且这些量都不需要每帧
SAMPLE_STEP = 5
VIDEO_EXT = {".mp4", ".mov", ".webm", ".avi", ".mkv"}
IMAGE_EXT = {".jpg", ".jpeg", ".png", ".webp", ".bmp"}


def sample_frames(path: Path) -> tuple[list[np.ndarray], float, int]:
    if path.suffix.lower() in IMAGE_EXT:
        img = cv2.imread(str(path))
        if img is None:
            raise ValueError("读不出这张图")
        return [_limit_height(img)], 0.0, 1

    cap = cv2.VideoCapture(str(path))
    if not cap.isOpened():
        raise ValueError("打不开这个视频")
    fps = cap.get(cv2.CAP_PROP_FPS) or 25.0
    total = int(cap.get(cv2.CAP_PROP_FRAME_COUNT)) or 0
    frames, i = [], 0
    while True:
        ok, f = cap.read()
        if not ok:
            break
        if i % SAMPLE_STEP == 0:
            frames.append(_limit_height(f))
        i += 1
    cap.release()
    if not frames:
        raise ValueError("没解出任何帧")
    return frames, fps, total or i


def measure(path: Path) -> dict:
    frames, fps, total = sample_frames(path)
    face_cascade = cv2.CascadeClassifier(cv2.data.haarcascades + "haarcascade_frontalface_default.xml")
    eye_cascade = cv2.CascadeClassifier(cv2.data.haarcascades + "haarcascade_eye.xml")

    boxes, angles = [], []
    for f in frames:
        gray = cv2.cvtColor(f, cv2.COLOR_BGR2GRAY)
        found = face_cascade.detectMultiScale(gray, 1.1, 5)
        if len(found) == 0:
            boxes.append(None)
            continue
        x, y, w, h = max(found, key=lambda a: a[2] * a[3])
        boxes.append(_to_training_box(x, y, w, h, f.shape[1], f.shape[0]))

        crop = cv2.resize(f[y : y + h, x : x + w], (IMG_SIZE, IMG_SIZE))
        eyes = [
            e
            for e in eye_cascade.detectMultiScale(cv2.cvtColor(crop, cv2.COLOR_BGR2GRAY), 1.05, 6, minSize=(24, 24))
            if e[1] + e[3] / 2 < IMG_SIZE * 0.63
        ]
        if len(eyes) >= 2:
            l, r = sorted(eyes, key=lambda e: e[0])[:2]
            angles.append(
                math.degrees(math.atan2((r[1] + r[3] / 2) - (l[1] + l[3] / 2), (r[0] + r[2] / 2) - (l[0] + l[2] / 2)))
            )

    good = [b for b in boxes if b is not None]
    hit = len(good) / len(boxes)
    sides = np.array([b[2] - b[0] for b in good]) if good else np.array([0])
    cx = np.array([(b[0] + b[2]) / 2 for b in good]) if good else np.array([0])
    cy = np.array([(b[1] + b[3]) / 2 for b in good]) if good else np.array([0])

    # 画面动态度：相邻采样帧的像素差。**这就是「基本上不太动」的那个量** ——
    # 动态壁纸类素材通常只有背景在飘，人几乎是静止的
    motion = 0.0
    if len(frames) > 1:
        diffs = [
            np.abs(frames[i].astype(np.int16) - frames[i - 1].astype(np.int16)).mean()
            for i in range(1, min(len(frames), 60))
        ]
        motion = float(np.median(diffs))

    # 人脸区域的动态度单独算 —— 背景在飘不算「人在动」
    face_motion = 0.0
    if len(frames) > 1 and good:
        x1, y1, x2, y2 = good[0]
        crops = [f[y1:y2, x1:x2] for f in frames[: min(len(frames), 60)]]
        face_motion = float(
            np.median([np.abs(crops[i].astype(np.int16) - crops[i - 1].astype(np.int16)).mean() for i in range(1, len(crops))])
        )

    return {
        "分辨率": f"{frames[0].shape[1]}x{frames[0].shape[0]}",
        "时长": (total / fps) if fps else 0.0,
        "帧数": total,
        "命中率": hit,
        "人脸边长": float(np.median(sides)),
        "位移": float(max(cx.max() - cx.min(), cy.max() - cy.min())) if good else 0.0,
        "倾角": float(np.median(angles)) if angles else 0.0,
        "倾角四分位距": float(np.percentile(angles, 75) - np.percentile(angles, 25)) if len(angles) > 3 else 0.0,
        "整体动态": motion,
        "人脸动态": face_motion,
    }


def verdict(m: dict) -> list[str]:
    """每条都对应一种具体的失败表现，不是泛泛的好坏。"""
    out = []
    if m["命中率"] < 0.9:
        out.append(f"✗ 人脸只在 {m['命中率']*100:.0f}% 的帧里检测到 —— 漏检的帧没有口型，嘴会一顿一顿")
    if m["人脸边长"] < 180:
        out.append(f"✗ 人脸只有 {m['人脸边长']:.0f}px（模型吃 256）—— 嘴会糊。要么原图更大，要么构图更近")
    elif m["人脸边长"] < 240:
        out.append(f"△ 人脸 {m['人脸边长']:.0f}px，偏小。能用，但细节有限")
    if abs(m["倾角"]) > 20:
        out.append(f"△ 头倾斜 {m['倾角']:+.0f}° —— 已能自动摆正，但角度越大重绘越吃力")
    if m["倾角四分位距"] > 6:
        out.append(f"△ 倾角在整段里变化 {m['倾角四分位距']:.0f}° —— 目前整段用一个角度，转头幅度大时会对不准")
    if m["帧数"] > 1:
        if m["人脸动态"] < 1.5:
            out.append(f"✗ 人脸几乎不动（帧间差 {m['人脸动态']:.1f} 灰阶）—— 和一张静图差别不大")
        elif m["人脸动态"] < 3.0:
            out.append(f"△ 人脸动得很少（帧间差 {m['人脸动态']:.1f}）—— 有呼吸感但谈不上生动")
        if m["时长"] < 8:
            out.append(f"△ 只有 {m['时长']:.0f} 秒，循环会看得出来。10 秒以上明显更自然")
        if m["位移"] < m["人脸边长"] * 0.05:
            out.append("△ 头基本没有位移 —— 视频里「她在动」的感觉主要来自这个")
    if not out:
        out.append("✓ 各项都过关")
    return out


def main(paths: list[str]) -> None:
    targets: list[Path] = []
    for p in paths:
        path = Path(p)
        if path.is_dir():
            targets += sorted(f for f in path.iterdir() if f.suffix.lower() in VIDEO_EXT | IMAGE_EXT)
        else:
            targets.append(path)

    for path in targets:
        print(f"\n\033[1m{path.name}\033[0m")
        try:
            m = measure(path)
        except Exception as err:
            print(f"  ✗ {err}")
            continue
        kind = "静图" if m["帧数"] <= 1 else f"{m['时长']:.0f}s / {m['帧数']} 帧"
        print(f"  {m['分辨率']}  {kind}   人脸 {m['人脸边长']:.0f}px  命中 {m['命中率']*100:.0f}%  倾角 {m['倾角']:+.1f}°")
        if m["帧数"] > 1:
            print(f"  整体动态 {m['整体动态']:.1f}   人脸动态 {m['人脸动态']:.1f}   头部位移 {m['位移']:.0f}px")
        for line in verdict(m):
            print(f"  {line}")


if __name__ == "__main__":
    main(sys.argv[1:] or ["../../avatars/neural"])
