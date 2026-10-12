"""
Anima 神经数字人音频处理器 (Audio Feature & Viseme Processor)
============================================================
负责接收 16kHz 16-bit 单声道 PCM 音频数据流，计算实时能量、频谱包络及唇形开合特征。
"""

import numpy as np
from typing import Dict, Tuple

class AudioProcessor:
    def __init__(self, sample_rate: int = 16000):
        self.sample_rate = sample_rate
        self.energy_history = []
        self.smoothed_energy = 0.0
        self.smoothed_mouth_open = 0.0
        self.smoothed_mouth_form = 0.0

    def process_pcm_chunk(self, pcm_bytes: bytes) -> Dict[str, float]:
        """
        处理单段 PCM 字节块（例如 20ms~40ms 的音频数据）
        返回归一化的能量 (0.0~1.0) 和基础口型特征
        """
        if not pcm_bytes or len(pcm_bytes) < 2:
            self.smoothed_energy *= 0.85
            self.smoothed_mouth_open *= 0.82
            self.smoothed_mouth_form *= 0.85
            return {
                "energy": max(0.0, float(self.smoothed_energy)),
                "mouth_open": max(0.0, float(self.smoothed_mouth_open)),
                "mouth_form": float(self.smoothed_mouth_form),
                "is_speaking": self.smoothed_energy > 0.02,
            }

        # 转为 float32 [-1.0, 1.0]
        samples = np.frombuffer(pcm_bytes, dtype=np.int16).astype(np.float32) / 32768.0

        # 计算短时能量 (RMS)
        rms = float(np.sqrt(np.mean(samples ** 2))) if len(samples) > 0 else 0.0
        normalized_energy = min(1.0, rms * 4.5)

        # 简单频域分析 (FFT) 估计元音分布 (低频 aa/O vs 高频 I/E)
        if len(samples) >= 64:
            fft_mag = np.abs(np.fft.rfft(samples * np.hanning(len(samples))))
            freqs = np.fft.rfftfreq(len(samples), 1.0 / self.sample_rate)
            
            # 低频区 (200Hz - 900Hz, 对应大开口 aa/O/U)
            low_mask = (freqs >= 200) & (freqs <= 900)
            low_energy = float(np.sum(fft_mag[low_mask])) if np.any(low_mask) else 0.0

            # 中高频区 (1200Hz - 3500Hz, 对应扁唇与齿音 I/E/S)
            high_mask = (freqs >= 1200) & (freqs <= 3500)
            high_energy = float(np.sum(fft_mag[high_mask])) if np.any(high_mask) else 0.0

            total_mag = low_energy + high_energy + 1e-6
            form_bias = (high_energy - low_energy) / total_mag  # 正值偏向扁唇 (I/E), 负值偏向圆唇 (O/U)
        else:
            form_bias = 0.0

        # 平滑过渡滤波器 (EMA)
        alpha = 0.45
        self.smoothed_energy = (1 - alpha) * self.smoothed_energy + alpha * normalized_energy
        self.smoothed_mouth_open = (1 - alpha) * self.smoothed_mouth_open + alpha * min(1.0, normalized_energy * 1.35)
        self.smoothed_mouth_form = (1 - alpha) * self.smoothed_mouth_form + alpha * np.clip(form_bias * 1.5, -1.0, 1.0)

        return {
            "energy": float(self.smoothed_energy),
            "mouth_open": float(self.smoothed_mouth_open),
            "mouth_form": float(self.smoothed_mouth_form),
            "is_speaking": self.smoothed_energy > 0.03,
        }
