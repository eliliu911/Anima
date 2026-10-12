declare module "@mkkellogg/gaussian-splats-3d" {
  export const SceneFormat: {
    Splat: number;
    KSplat: number;
    Ply: number;
    Spz: number;
  };
  export class Viewer {
    constructor(options?: Record<string, unknown>);
    addSplatScene(path: string, options?: Record<string, unknown>): Promise<unknown>;
    removeSplatScenes(indexes: number[], showLoadingUI?: boolean): Promise<unknown>;
    getSceneCount(): number;
    update(): void;
    render(): void;
    dispose(): Promise<void>;
  }
}
