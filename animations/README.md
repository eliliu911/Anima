# 动作库（.vrma）

把 **VRM Animation**（`.vrma`）文件丢进这个目录，文件名就是动作名。
**只认 `.vrma`** —— Mixamo 下下来的 `.fbx` 这条管线吃不了（见文末）。

动作名 = 文件名去扩展名、转小写、空格和连字符换下划线：

    Peace sign.vrma  →  <action:peace_sign>
    Show full body.vrma  →  <action:show_full_body>

子目录也扫（一层），所以按来源分文件夹是可以的。

```
animations/wave.vrma          → <action:wave>
animations/stretch.vrma       → <action:stretch>
animations/cover_mouth.vrma   → <action:cover_mouth>
animations/heart.vrma         → <action:heart>
```

内置的四个手势（wave / stretch / cover_mouth / heart）**没有文件也能用** ——
它们是代码生成的（`src/avatar/animations.ts`）。放一个同名 `.vrma` 就会替换掉它。

**动作清单是扫这个目录生成的**，会写进给模型的 system prompt。
所以放进来之后**重启语音会话**（不是热重载）才会生效 —— 模型得先知道它存在。

## 为什么是 VRMA

VRMA 绑的是 VRM 规范里的**标准骨骼名**（`leftLowerArm` 这样的），
不是某个模型的节点名 —— 所以**一份动作所有角色通用**，换模型不用重做。

我们只取其中的**旋转**轨道，位移（hips 的 translation）会丢掉：
不同模型身高比例不同，照搬别人的位移会让人飘起来或者陷进地里。

## 哪里找

- VRoid 官方放出过一组免费示例动作（打招呼、比心、鞠躬之类）
- Booth / VRoid Hub 上有创作者发布的 VRMA

下载回来直接改名成上面的动作名即可。想加新动作的话，
`src-tauri/src/voice/mod.rs` 的 `CAPABILITY_PROMPT` 里也要告诉模型有这个动作，
否则它不知道可以用。

## 验证有没有生效

```bash
grep -a "动作 " ~/Library/Logs/Anima/anima.log | tail
```

- `动作 wave 用 wave.vrma：接上 9 根骨骼` → 用上了
- `一根骨骼都没接上，退回内置手势` → 文件能读但重定向失败（多半不是 VRMA）
- 没有任何一行 → 没找到文件，用的是内置手势


## Mixamo 的 .fbx 怎么办

这条管线只吃 `.vrma`。Mixamo 的 FBX 用的是它自己的骨骼命名，
要重定向到 VRM humanoid 才能用 —— 那是另一件事（当初评估时的"方案 B"）。

放在这个目录里的 `.fbx` 会被忽略，不会报错、也不会出现在动作清单里。
想用的话有两条路：

1. 找工具把 FBX 转成 VRMA（社区有转换器）；
2. 引 `@pixiv/three-vrm`，用它的 humanoid 抽象做运行时重定向 ——
   代价是它会接管整个模型加载，现有的 humanoid/vertexColors/framing
   那套都要重新对。
