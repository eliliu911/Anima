import type { CinemaProject, EpisodeData } from "../../../api/cinema";

/** 每一步都拿到同一份项目/分集数据 —— 各自去拉的话会看到不同版本。 */
export interface StepProps {
  project: CinemaProject | null;
  episode: EpisodeData | null;
  /** 重新拉项目详情。生成完东西之后调它 */
  reload: () => Promise<void>;
  onError: (msg: string | null) => void;
  /**
   * 打开「生成设置」。
   *
   * **模型是项目级的，所以选择入口只有一个** —— 页头那个。
   * 步骤页里需要的不是再放一份选择器（放了就有两份真相，而且
   * 在镜头卡上改一下会静默改掉整个项目），而是一条能点过去的路。
   */
  onOpenSettings: () => void;
}
