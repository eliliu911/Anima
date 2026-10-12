#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Anima 4D 动态动作视频生成引擎 (LivePortrait Multi-Clip Batch Generator)
批量从单张肖像图为角色生成待机 (Idle)、说话 (Speaking)、倾听 (Listening) 三段真实神经面部与动态循环视频。
"""

import os
import sys
import argparse
import subprocess
from pathlib import Path

# Add official LivePortrait to sys.path
LIVEPORTRAIT_ROOT = Path("/Users/you/AI/ComfyUI/custom_nodes/official_liveportrait")
if str(LIVEPORTRAIT_ROOT) not in sys.path:
    sys.path.insert(0, str(LIVEPORTRAIT_ROOT))

from src.config.argument_config import ArgumentConfig
from src.config.inference_config import InferenceConfig
from src.config.crop_config import CropConfig
from src.live_portrait_pipeline import LivePortraitPipeline


def partial_fields(target_class, kwargs):
    return target_class(**{k: v for k, v in kwargs.items() if hasattr(target_class, k)})


def run_batch(source_img: str, char_id: str, output_dir: str, intensity: str = "balanced", duration: float = 5.0):
    source_path = Path(source_img).resolve()
    out_path = Path(output_dir).resolve()
    out_path.mkdir(parents=True, exist_ok=True)

    if not source_path.exists():
        raise FileNotFoundError(f"肖像图未找到: {source_path}")

    # Set up ffmpeg path
    ffmpeg_candidates = ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg", "ffmpeg"]
    for c in ffmpeg_candidates:
        if Path(c).exists():
            os.environ["PATH"] = str(Path(c).parent) + os.pathsep + os.environ.get("PATH", "")
            break

    # Initialize LivePortrait Pipeline once
    inf_cfg = InferenceConfig(
        flag_use_half_precision=False, # Stable on Mac MPS/CPU
        flag_stitching=True,
        flag_relative_motion=True,
        flag_pasteback=True,
        flag_do_crop=True,
    )
    crop_cfg = CropConfig()
    pipeline = LivePortraitPipeline(inference_cfg=inf_cfg, crop_cfg=crop_cfg)

    # 3 Motion Driving Templates
    driving_map = {
        "idle": LIVEPORTRAIT_ROOT / "assets/examples/driving/d0.mp4",
        "speaking": LIVEPORTRAIT_ROOT / "assets/examples/driving/d6.mp4",
        "listening": LIVEPORTRAIT_ROOT / "assets/examples/driving/d11.mp4",
    }

    results = {}

    for mode, driving_video in driving_map.items():
        print(f"\n[Anima 4D] 🎬 正在为「{char_id}」生成 {mode} 动作视频 ({duration}s)...")
        if not driving_video.exists():
            print(f"警告: 驱动视频不存在: {driving_video}")
            continue

        temp_dir = out_path / f"temp_{mode}"
        temp_dir.mkdir(parents=True, exist_ok=True)

        # 智能裁剪驱动模版至指定时长，避免长视频产生上千帧计算开销
        trimmed_driving = temp_dir / "trimmed_driving.mp4"
        subprocess.run([
            "ffmpeg", "-y", "-i", str(driving_video),
            "-t", str(duration),
            "-c", "copy",
            str(trimmed_driving)
        ], check=False, capture_output=True)
        actual_driving = trimmed_driving if trimmed_driving.exists() and trimmed_driving.stat().st_size > 0 else driving_video

        args = ArgumentConfig(
            source=str(source_path),
            driving=str(actual_driving),
            output_dir=str(temp_dir),
            flag_use_half_precision=False,
            flag_pasteback=True,
            flag_do_crop=True,
            driving_option="expression-friendly",
            driving_multiplier=1.2 if intensity == "expressive" else (0.85 if intensity == "gentle" else 1.0),
        )

        try:
            pipeline.execute(args)

            # Find generated output video (ends with .mp4 and not _concat.mp4)
            found_vids = [
                f for f in temp_dir.glob("*.mp4")
                if not f.name.endswith("_concat.mp4") and not f.name.startswith("trimmed_")
            ]
            if found_vids:
                src_vid = found_vids[0]
                target_vid = out_path / f"{char_id}_{mode}.mp4"
                
                # Convert/Re-encode to standard web-compatible H.264 MP4 with faststart
                subprocess.run([
                    "ffmpeg", "-y", "-i", str(src_vid),
                    "-c:v", "libx264", "-pix_fmt", "yuv420p",
                    "-movflags", "+faststart",
                    str(target_vid)
                ], check=True, capture_output=True)
                
                results[mode] = str(target_vid)
                print(f"[Anima 4D] ✅ {mode} 视频已生成: {target_vid}")
        except Exception as e:
            print(f"[Anima 4D] ❌ 生成 {mode} 失败: {e}", file=sys.stderr)
        finally:
            import shutil
            shutil.rmtree(temp_dir, ignore_errors=True)

    print(f"\n🎉 [Anima 4D] 所有 3 段动作视频批处理完成！共生成 {len(results)}/3 段视频。")
    return results


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="LivePortrait 批量生成 3 段动作视频")
    parser.add_argument("--source", "-s", required=True, help="源肖像图路径")
    parser.add_argument("--char_id", "-c", required=True, help="角色英文 ID")
    parser.add_argument("--output_dir", "-o", required=True, help="输出目录")
    parser.add_argument("--intensity", "-i", default="balanced", choices=["gentle", "balanced", "expressive"], help="动作幅度")
    parser.add_argument("--duration", "-d", type=float, default=5.0, help="视频时长(秒)")

    args = parser.parse_args()
    run_batch(args.source, args.char_id, args.output_dir, args.intensity, args.duration)
