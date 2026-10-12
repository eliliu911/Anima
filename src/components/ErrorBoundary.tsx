import React, { Component, type ReactNode } from "react";
import i18next from "i18next";

interface Props {
  children: ReactNode;
  fallbackTitle?: string;
  onReset?: () => void;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

/**
 * 通用 React 异常保护边界
 *
 * 防止个别组件或渲染管线异常导致整个窗口白屏
 */
export class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    console.error("[ErrorBoundary] 捕获渲染异常:", error, errorInfo);
  }

  handleReset = () => {
    this.setState({ hasError: false, error: null });
    this.props.onReset?.();
  };

  render() {
    if (this.state.hasError) {
      return (
        <div className="p-4 m-2 bg-[var(--surface-2)] border border-red-500/40 rounded-2xl flex flex-col items-center justify-center text-center gap-3 text-[var(--ink)]">
          <div className="w-10 h-10 rounded-full bg-red-500/20 text-red-400 flex items-center justify-center text-xl font-bold">
            ⚠️
          </div>
          <div className="text-[14px] font-bold">
            {this.props.fallbackTitle || i18next.t("common:errorBoundary.defaultTitle", { defaultValue: "该区域组件渲染出现异常" })}
          </div>
          <div className="text-[11.5px] text-[var(--ink-3)] font-mono max-w-md bg-[var(--surface-3)] p-2 rounded-lg break-all">
            {this.state.error?.message || i18next.t("common:errorBoundary.unknownError", { defaultValue: "未知渲染错误" })}
          </div>
          <button
            type="button"
            className="px-4 py-1.5 bg-[var(--celadon)] text-white text-[12px] font-semibold rounded-xl hover:opacity-90 transition-all cursor-pointer shadow-xs"
            onClick={this.handleReset}
          >
            {i18next.t("common:errorBoundary.recoverButton", { defaultValue: "恢复并重新加载" })}
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
