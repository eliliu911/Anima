"""
Wav2Lip 网络结构（256×256 变体）。

## 为什么这个文件必须存在

`.pth` 里**只有参数，没有结构** —— PyTorch 的 checkpoint 是一张
「名字 → 张量」的表。没有与之逐层对应的 nn.Module，权重就是一堆数字。

## 结构是反推出来的，不是凭记忆写的

每一层的形状都从 state_dict 里量出来。关键的一处：**ConvTranspose2d 的
权重是 `(in, out, kH, kW)`，而 Conv2d 是 `(out, in, kH, kW)`** ——
按 Conv2d 去读解码器会得到一条根本对不上的通道链，很容易误判成
「权重坏了」。按转置卷积读，整条链才闭合：

    audio 512@1  →cat face 512→ 1024 →T4→ 512@4  →cat 512→ 1024 →T3→ 512@8
    →cat 512→ 1024 →T3→ 512@16 →cat 256→ 768 →T3→ 384@32 →cat 128→ 512
    →T3→ 256@64 →cat 64→ 320 →T3→ 128@128 →cat 32→ 160 →T3→ 64@256
    →cat 16→ 80 → output 3@256

验证方式是二元的：`load_state_dict(strict=True)` 不报缺键/多键就说明
每一层的名字和形状都对上了。差一个通道都过不去。
"""

import torch
from torch import nn
from torch.nn import functional as F


class Conv2d(nn.Module):
    def __init__(self, cin, cout, kernel_size, stride, padding, residual=False):
        super().__init__()
        self.conv_block = nn.Sequential(
            nn.Conv2d(cin, cout, kernel_size, stride, padding),
            nn.BatchNorm2d(cout),
        )
        self.act = nn.ReLU()
        self.residual = residual

    def forward(self, x):
        out = self.conv_block(x)
        if self.residual:
            out = out + x
        return self.act(out)


class Conv2dTranspose(nn.Module):
    def __init__(self, cin, cout, kernel_size, stride, padding, output_padding=0):
        super().__init__()
        self.conv_block = nn.Sequential(
            nn.ConvTranspose2d(cin, cout, kernel_size, stride, padding, output_padding),
            nn.BatchNorm2d(cout),
        )
        self.act = nn.ReLU()

    def forward(self, x):
        return self.act(self.conv_block(x))


class Wav2Lip(nn.Module):
    """输入 6 通道人脸（3 遮罩 + 3 参考）+ 1 通道梅尔，输出重绘的 3 通道人脸。"""

    def __init__(self):
        super().__init__()

        self.face_encoder_blocks = nn.ModuleList([
            nn.Sequential(Conv2d(6, 16, 7, 1, 3)),                                    # 256
            nn.Sequential(Conv2d(16, 32, 3, 2, 1),                                    # 128
                          Conv2d(32, 32, 3, 1, 1, residual=True),
                          Conv2d(32, 32, 3, 1, 1, residual=True)),
            nn.Sequential(Conv2d(32, 64, 3, 2, 1),                                    # 64
                          Conv2d(64, 64, 3, 1, 1, residual=True),
                          Conv2d(64, 64, 3, 1, 1, residual=True),
                          Conv2d(64, 64, 3, 1, 1, residual=True)),
            nn.Sequential(Conv2d(64, 128, 3, 2, 1),                                   # 32
                          Conv2d(128, 128, 3, 1, 1, residual=True),
                          Conv2d(128, 128, 3, 1, 1, residual=True)),
            nn.Sequential(Conv2d(128, 256, 3, 2, 1),                                  # 16
                          Conv2d(256, 256, 3, 1, 1, residual=True),
                          Conv2d(256, 256, 3, 1, 1, residual=True)),
            nn.Sequential(Conv2d(256, 512, 3, 2, 1),                                  # 8
                          Conv2d(512, 512, 3, 1, 1, residual=True)),
            nn.Sequential(Conv2d(512, 512, 3, 2, 1),                                  # 4
                          Conv2d(512, 512, 3, 1, 1, residual=True)),
            nn.Sequential(Conv2d(512, 512, 4, 1, 0),                                  # 1
                          Conv2d(512, 512, 1, 1, 0)),
        ])

        self.audio_encoder = nn.Sequential(
            Conv2d(1, 32, 3, 1, 1),
            Conv2d(32, 32, 3, 1, 1, residual=True),
            Conv2d(32, 32, 3, 1, 1, residual=True),
            Conv2d(32, 64, 3, (3, 1), 1),
            Conv2d(64, 64, 3, 1, 1, residual=True),
            Conv2d(64, 64, 3, 1, 1, residual=True),
            Conv2d(64, 128, 3, 3, 1),
            Conv2d(128, 128, 3, 1, 1, residual=True),
            Conv2d(128, 128, 3, 1, 1, residual=True),
            Conv2d(128, 256, 3, (3, 2), 1),
            Conv2d(256, 256, 3, 1, 1, residual=True),
            Conv2d(256, 512, 3, 1, 0),
            Conv2d(512, 512, 1, 1, 0),
        )

        # 每块的 in 通道 = 上一块输出 + 跳连过来的编码器特征
        self.face_decoder_blocks = nn.ModuleList([
            nn.Sequential(Conv2d(512, 512, 1, 1, 0)),
            nn.Sequential(Conv2dTranspose(1024, 512, 4, 1, 0),                        # 4
                          Conv2d(512, 512, 3, 1, 1, residual=True)),
            nn.Sequential(Conv2dTranspose(1024, 512, 3, 2, 1, output_padding=1),      # 8
                          Conv2d(512, 512, 3, 1, 1, residual=True)),
            nn.Sequential(Conv2dTranspose(1024, 512, 3, 2, 1, output_padding=1),      # 16
                          Conv2d(512, 512, 3, 1, 1, residual=True),
                          Conv2d(512, 512, 3, 1, 1, residual=True)),
            nn.Sequential(Conv2dTranspose(768, 384, 3, 2, 1, output_padding=1),       # 32
                          Conv2d(384, 384, 3, 1, 1, residual=True),
                          Conv2d(384, 384, 3, 1, 1, residual=True)),
            nn.Sequential(Conv2dTranspose(512, 256, 3, 2, 1, output_padding=1),       # 64
                          Conv2d(256, 256, 3, 1, 1, residual=True),
                          Conv2d(256, 256, 3, 1, 1, residual=True)),
            nn.Sequential(Conv2dTranspose(320, 128, 3, 2, 1, output_padding=1),       # 128
                          Conv2d(128, 128, 3, 1, 1, residual=True),
                          Conv2d(128, 128, 3, 1, 1, residual=True)),
            nn.Sequential(Conv2dTranspose(160, 64, 3, 2, 1, output_padding=1),        # 256
                          Conv2d(64, 64, 3, 1, 1, residual=True),
                          Conv2d(64, 64, 3, 1, 1, residual=True)),
        ])

        self.output_block = nn.Sequential(
            Conv2d(80, 32, 3, 1, 1),
            nn.Conv2d(32, 3, 1, 1, 0),
            nn.Sigmoid(),
        )

    def forward(self, audio_sequences, face_sequences):
        audio_embedding = self.audio_encoder(audio_sequences)  # B, 512, 1, 1

        feats = []
        x = face_sequences
        for f in self.face_encoder_blocks:
            x = f(x)
            feats.append(x)

        x = audio_embedding
        for f in self.face_decoder_blocks:
            x = f(x)
            # **跳连在每块之后拼**，所以下一块的 in 通道是「本块输出 + 跳连」
            try:
                x = torch.cat((x, feats[-1]), dim=1)
            except RuntimeError as err:
                raise RuntimeError(f"跳连拼接失败 x={x.shape} skip={feats[-1].shape}") from err
            feats.pop()

        return self.output_block(x)


def load(checkpoint_path: str, device: torch.device) -> Wav2Lip:
    """加载权重。**用 weights_only=True** —— 这些 .pth 多半是从别处下载的，
    pickle 会执行任意代码。"""
    ckpt = torch.load(checkpoint_path, map_location="cpu", weights_only=True)
    state = ckpt.get("state_dict", ckpt)
    state = {k[len("module."):] if k.startswith("module.") else k: v for k, v in state.items()}

    model = Wav2Lip()
    # strict=True 是这份结构唯一的验证手段：差一个通道就过不去
    model.load_state_dict(state, strict=True)
    return model.to(device).eval()
