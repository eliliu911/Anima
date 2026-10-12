import { useChat } from "./chat";
import { useVoice } from "./voice";

/**
 * 粒子云 / 3D 面容的形态，从语音与生成状态推出来。
 *
 * **抽在这里而不是留在 App.tsx**：角色层是独立窗口，它不挂 App，
 * 但需要同一份推导 —— 两边各写一份的话，某天改了一处就会出现
 * 「主窗口显示在听、桌面上那个还在待机」。
 */
export function useAvatarState() {
  const activeStreamId = useChat((s) => s.activeStreamId);
  const running = useVoice((s) => s.running);
  const voiceState = useVoice((s) => s.state);

  if (running) {
    switch (voiceState) {
      case "listening":
        return "listening" as const;
      case "transcribing":
      case "thinking":
        return "thinking" as const;
      case "speaking":
        return "speaking" as const;
      default:
        return "idle" as const;
    }
  }
  return activeStreamId ? ("thinking" as const) : ("idle" as const);
}
