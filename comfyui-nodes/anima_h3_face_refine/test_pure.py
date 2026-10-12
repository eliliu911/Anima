"""纯计算部分的测试（不碰 MLX / ComfyUI）。直接跑：`~/AI/ComfyUI/.venv/bin/python comfyui-nodes/anima_h3_face_refine/test_pure.py`
（ComfyUI 的环境里没有 pytest，不为了测试往里装东西；有 pytest 的话 `pytest` 也能收到这些 test_ 函数）"""
import contextlib
import importlib.util
import pathlib
import re

import numpy as np


class pytest:  # noqa: N801 —— 只要 raises 这一个
    @staticmethod
    @contextlib.contextmanager
    def raises(exc, match=""):
        try:
            yield
        except exc as e:
            assert re.search(match, str(e)), f"报错对不上：{e}"
            return
        raise AssertionError(f"没有抛出 {exc.__name__}")

_spec = importlib.util.spec_from_file_location("afr", pathlib.Path(__file__).with_name("__init__.py"))
afr = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(afr)


def test_帧数换时长要换算回同一个帧数():
    for n in (73, 107, 124, 192, 294, 328, 362):
        d = afr.duration_for_frames(n)
        assert 2.5 <= d <= 15.0
        assert afr.align_frames(round(d * 24)) == n, (n, d)
    assert afr.duration_for_frames(362) == 15.0
    with pytest.raises(ValueError, match="17k\\+5"):
        afr.duration_for_frames(100)
    with pytest.raises(ValueError, match="2.5~15"):
        afr.duration_for_frames(56)  # 2.3 秒，太短
    with pytest.raises(ValueError, match="2.5~15"):
        afr.duration_for_frames(379)  # 15.8 秒，太长


def test_编码尺寸放大一倍半正好回到画布():
    assert afr.source_size_for(768, 768) == (512, 512)
    assert afr.hires_target(512, 512) == (768, 768)
    assert afr.source_size_for(960, 576) == (640, 384)
    with pytest.raises(ValueError, match="1.5"):
        afr.source_size_for(800, 800)  # 800/1.5 取整后放不回 800


def test_音频变成32k双声道长度对上帧数():
    mono = np.ones((1, 1, 48000), dtype=np.float32)  # 1 秒 48 kHz 单声道
    out = afr.stereo_32k(mono, 48000, 73)
    assert out.shape == (2, round(73 / 24 * 32000))
    assert np.allclose(out[:, :30000], 1.0) and np.allclose(out[:, -1000:], 0.0), "不够长的补零"
    st = np.zeros((1, 2, 32000 * 20), dtype=np.float32)
    assert afr.stereo_32k(st, 32000, 362).shape == (2, round(362 / 24 * 32000)), "太长的截掉"


def test_画面缩到编码尺寸转成uint8():
    import torch
    img = torch.full((5, 768, 768, 3), 0.5)
    out = afr.resized_uint8(img, 512, 512)
    assert out.shape == (5, 512, 512, 3) and out.dtype == np.uint8 and int(out[0, 0, 0, 0]) == 128


if __name__ == "__main__":
    names = [k for k in list(globals()) if k.startswith("test_")]
    for k in names:
        globals()[k]()
        print("ok", k)
    print(f"{len(names)} passed")
