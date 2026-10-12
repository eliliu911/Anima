"""
Wav2Lip 的梅尔频谱。

## 为什么一个字都不能改

模型是在这一套具体参数上训练的。参数对不上时它**不会报错**，
照样输出一张脸、嘴也照样在动 —— 只是动的和说的对不上。
而「嘴在动但对不上」正是最难往回推的那种症状：看上去像模型不行，
其实是喂进去的特征不是它认识的那一种。

所以这里直接用 librosa（Wav2Lip 训练时用的就是它），不自己复刻滤波器组。
自己写 mel 很容易在 Slaney/HTK 归一化、居中补零这些地方悄悄差一点。

参数取自 Wav2Lip 的 `hparams.py`，逐条对齐。
"""

import librosa
import numpy as np

SAMPLE_RATE = 16000
N_FFT = 800
HOP_SIZE = 200
WIN_SIZE = 800
NUM_MELS = 80
FMIN = 55
FMAX = 7600
MIN_LEVEL_DB = -100
REF_LEVEL_DB = 20
PREEMPHASIS = 0.97
MAX_ABS_VALUE = 4.0

# 模型一次吃 16 帧梅尔（约 200ms 的上下文），这是它的感受野
MEL_STEP_SIZE = 16

_mel_basis = None


def _basis():
    global _mel_basis
    if _mel_basis is None:
        _mel_basis = librosa.filters.mel(
            sr=SAMPLE_RATE, n_fft=N_FFT, n_mels=NUM_MELS, fmin=FMIN, fmax=FMAX
        )
    return _mel_basis


def melspectrogram(wav: np.ndarray) -> np.ndarray:
    """16kHz 单声道 float32 → (80, T) 的归一化梅尔，取值落在 [-4, 4]。"""
    # 预加重
    wav = np.append(wav[0], wav[1:] - PREEMPHASIS * wav[:-1])

    spec = librosa.stft(y=wav, n_fft=N_FFT, hop_length=HOP_SIZE, win_length=WIN_SIZE)
    mel = np.dot(_basis(), np.abs(spec))

    # 转 dB。20*log10(max(1e-5, x))，再减去参考电平
    mel = 20.0 * np.log10(np.maximum(1e-5, mel)) - REF_LEVEL_DB

    # 对称归一化到 [-max_abs, max_abs]
    mel = np.clip(
        (2 * MAX_ABS_VALUE) * ((mel - MIN_LEVEL_DB) / -MIN_LEVEL_DB) - MAX_ABS_VALUE,
        -MAX_ABS_VALUE,
        MAX_ABS_VALUE,
    )
    return mel.astype(np.float32)


def chunk_for_frame(mel: np.ndarray, frame_idx: int, fps: float) -> np.ndarray | None:
    """取第 `frame_idx` 帧画面对应的那 16 列梅尔。

    每秒 80 列梅尔（16000 / 200），所以第 i 帧的起点是 80 * i / fps。
    末尾不足 16 列就返回 None —— **不要补零**，补出来的静音会让嘴闭上，
    表现为每句话结尾嘴突然合死。
    """
    start = int(80.0 * (frame_idx / float(fps)))
    end = start + MEL_STEP_SIZE
    if end > mel.shape[1]:
        return None
    return mel[:, start:end]


def load_wav(path: str) -> np.ndarray:
    wav, _ = librosa.load(path, sr=SAMPLE_RATE)
    return wav
