"""局部重画脸换表情的实验脚本（2026-09-15，思路来自 ComfyUI_VNCCS 的 Emotion Studio）。

用法：python3 scripts/experiments/face_expression_test.py <工作目录>
工作目录里放 base.png（带大脸特写的设定图）；BOX 是脸部裁剪框、遮罩椭圆是眉毛到下巴，都按那张图手写的。
需要本机 ComfyUI（127.0.0.1:8188）和 Z-Image 权重。结论见 docs/STAGE_LOG.md「局部重画脸换表情」。
"""
import json, time, uuid, urllib.request, io, sys
from PIL import Image, ImageDraw, ImageFilter
S = sys.argv[1]; API = "http://127.0.0.1:8188"
base = Image.open(f"{S}/base.png").convert("RGB")
BOX = (40, 110, 490, 560); SIZE = 1024
crop = base.crop(BOX).resize((SIZE, SIZE), Image.LANCZOS)
buf = io.BytesIO(); crop.save(buf, "PNG")
def post(path, data, ctype):
    req = urllib.request.Request(API + path, data=data, headers={"Content-Type": ctype}); return json.load(urllib.request.urlopen(req))
b = uuid.uuid4().hex
body = (f"--{b}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"expr_face_crop.png\"\r\nContent-Type: image/png\r\n\r\n").encode() + buf.getvalue() + f"\r\n--{b}\r\nContent-Disposition: form-data; name=\"overwrite\"\r\n\r\ntrue\r\n--{b}--\r\n".encode()
up = post("/upload/image", body, f"multipart/form-data; boundary={b}")
IDENT = "盛唐女官林婉清的面部特写，28岁女性，瓜子脸，黑瞳，肤色苍白，黑发束于冠内，头戴黑色官帽。"
EMO = {
 "smile": "她在微笑，嘴角上扬，露出一点牙齿，眼睛微微弯起，眼神温和。",
 "angry": "她非常愤怒，眉头紧锁下压，目光凌厉地瞪视，嘴唇紧抿，面带怒容。",
 "surprised": "她非常惊讶，双眼睁大，眉毛高高扬起，嘴巴微微张开。",
 "sad": "她很难过，眉头内侧上扬，眼眶泛红含着泪光，嘴角向下。",
}
TAIL = "写实电影摄影，影棚柔光，灰色背景，面部清晰锐利。"
def graph(prompt, denoise, seed=424242):
    return {
     "1": {"class_type": "UNETLoader", "inputs": {"unet_name": "z_image_turbo_bf16.safetensors", "weight_dtype": "default"}},
     "2": {"class_type": "CLIPLoader", "inputs": {"clip_name": "qwen_3_4b.safetensors", "type": "lumina2", "device": "default"}},
     "3": {"class_type": "VAELoader", "inputs": {"vae_name": "z_image_ae.safetensors"}},
     "4": {"class_type": "ModelSamplingAuraFlow", "inputs": {"model": ["1", 0], "shift": 3.0}},
     "5": {"class_type": "CLIPTextEncode", "inputs": {"text": prompt, "clip": ["2", 0]}},
     "6": {"class_type": "ConditioningZeroOut", "inputs": {"conditioning": ["5", 0]}},
     "11": {"class_type": "LoadImage", "inputs": {"image": up["name"]}},
     "12": {"class_type": "VAEEncode", "inputs": {"pixels": ["11", 0], "vae": ["3", 0]}},
     "8": {"class_type": "KSampler", "inputs": {"seed": seed, "steps": 8, "cfg": 1.0, "sampler_name": "res_multistep", "scheduler": "simple", "denoise": denoise,
            "model": ["4", 0], "positive": ["5", 0], "negative": ["6", 0], "latent_image": ["12", 0]}},
     "9": {"class_type": "VAEDecode", "inputs": {"samples": ["8", 0], "vae": ["3", 0]}},
     "10": {"class_type": "SaveImage", "inputs": {"images": ["9", 0], "filename_prefix": "anima_expr_test"}},
    }
# 脸部遮罩（原图坐标）：眉毛到下巴的椭圆，羽化，头发帽子衣服不动
mask = Image.new("L", base.size, 0)
ImageDraw.Draw(mask).ellipse((135, 210, 435, 540), fill=255)
mask = mask.filter(ImageFilter.GaussianBlur(22))
results = {}
for d in (0.45, 0.6):
    for k, e in EMO.items():
        t0 = time.time()
        pid = post("/prompt", json.dumps({"prompt": graph(IDENT + e + TAIL, d), "client_id": "anima-expr-test"}).encode(), "application/json")["prompt_id"]
        while True:
            h = json.load(urllib.request.urlopen(f"{API}/history/{pid}"))
            if pid in h and h[pid].get("outputs"): break
            if pid in h and h[pid].get("status", {}).get("status_str") == "error": raise SystemExit(json.dumps(h[pid]["status"])[:800])
            time.sleep(2)
        img = h[pid]["outputs"]["10"]["images"][0]
        raw = urllib.request.urlopen(f"{API}/view?filename={img['filename']}&subfolder={img['subfolder']}&type=output").read()
        face = Image.open(io.BytesIO(raw)).convert("RGB").resize((BOX[2]-BOX[0], BOX[3]-BOX[1]), Image.LANCZOS)
        layer = base.copy(); layer.paste(face, BOX[:2])
        out = Image.composite(layer, base, mask)
        out.save(f"{S}/{k}_{d}.png"); results[(k, d)] = out
        print(k, d, f"{time.time()-t0:.0f}s", flush=True)
# 对比图：每行一个强度，第一列原图；只截左侧特写那一栏
cols = ["base"] + list(EMO)
W, H = 495, 600
sheet = Image.new("RGB", (W * len(cols), H * 2), "white")
for r, d in enumerate((0.45, 0.6)):
    for c, k in enumerate(cols):
        im = base if k == "base" else results[(k, d)]
        sheet.paste(im.crop((0, 60, W, 60 + H)), (c * W, r * H))
sheet.resize((sheet.width // 2, sheet.height // 2), Image.LANCZOS).save(f"{S}/compare.jpg", quality=88)
print("done")
