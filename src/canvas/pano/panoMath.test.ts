import { describe, it, expect } from "vitest";
import {
  fovToFocal,
  clampFov,
  normalizeDeg,
  angleBetween,
  cropRectForAspect,
  aspectRatio,
  ringShots,
  viewGrid,
  shotReverseOf,
  applyFrontOffset,
  frontToCurrent,
  displayDeg,
  shotFileName,
  CAPTURE_ASPECTS,
  FOV_MIN,
  FOV_MAX,
} from "./panoMath";

describe("全景取景的数学", () => {
  /** 给导演看的是毫米数，不是角度 —— 分镜表上写的也是焦段。 */
  it("视场角换成等效焦距", () => {
    // 70° ≈ 26mm，那是 DramaClaw 界面上显示的那个数
    expect(fovToFocal(70)).toBe(26);
    // 视角越窄焦距越长
    expect(fovToFocal(20)).toBeGreaterThan(fovToFocal(50));
    expect(fovToFocal(120)).toBeLessThan(fovToFocal(50));
  });

  it("视场角夹在上下限里", () => {
    expect(clampFov(1)).toBe(FOV_MIN);
    expect(clampFov(999)).toBe(FOV_MAX);
    expect(fovToFocal(999)).toBe(fovToFocal(FOV_MAX));
  });

  /** **角度必须归一化**，否则「差 180°」这种判断会漏。 */
  it("角度归一化到 [-180, 180)", () => {
    expect(normalizeDeg(0)).toBe(0);
    // **左闭右开：180 落成 -180。** 同一个方向，但给人看的地方不能用它
    expect(normalizeDeg(180)).toBe(-180);
    expect(normalizeDeg(181)).toBe(-179);
    expect(normalizeDeg(-190)).toBe(170);
    expect(normalizeDeg(720)).toBe(0);
    expect(normalizeDeg(450)).toBe(90);
  });

  /**
   * **给人看的角度是 0~359。**
   *
   * 「正后 = 180°」是所有人都认的写法，「正后 = -180°」不是 ——
   * 而 `normalizeDeg` 恰好会把 180 变成 -180。这两个函数分开正是为了这个。
   */
  it("显示角度是 0 到 359", () => {
    expect(displayDeg(180)).toBe(180);
    expect(displayDeg(-180)).toBe(180);
    expect(displayDeg(-90)).toBe(270);
    expect(displayDeg(450)).toBe(90);
    expect(displayDeg(0)).toBe(0);
  });

  it("两个方位的夹角永远是 0~180", () => {
    expect(angleBetween(0, 180)).toBe(180);
    expect(angleBetween(10, 350)).toBe(20);
    expect(angleBetween(-170, 170)).toBe(20);
  });

  describe("一圈机位", () => {
    /**
     * **这是这条路的全部价值。**
     *
     * 模型转不过去（实测两轮「反打」都还是朝着锚点看），
     * 但相机一定转得过去 —— 四个机位就是差 90°，这是算出来的不是求来的。
     */
    it("均分一圈，相邻正好差 360/n", () => {
      const four = ringShots(4);
      expect(four.map((s) => s.yawDeg)).toEqual([0, 90, -180, -90]);
      // 给人看的时候是 0/90/180/270
      expect(four.map((s) => displayDeg(s.yawDeg))).toEqual([0, 90, 180, 270]);
      const six = ringShots(6);
      for (let i = 1; i < six.length; i++) {
        expect(angleBetween(six[i].yawDeg, six[i - 1].yawDeg)).toBeCloseTo(60, 5);
      }
    });

    it("八向以内用方位名，不是编号", () => {
      // 无语义命名（视角1、视角2）是 drama-skills 明令的反面教材 ——
      // 下游读不出它是哪个方向
      const names = ringShots(4).map((s) => s.name);
      expect(names).not.toContain("视角1");
      for (const n of names) expect(n).toMatch(/[前后左右]/);
    });

    it("超过八向退回角度命名", () => {
      expect(ringShots(12).map((s) => s.name).every((n) => n.endsWith("°"))).toBe(true);
    });
  });

  /** **反打差整 180°，几何保证。** 不可能不转过去。 */
  it("反打就是加 180 度", () => {
    const front = { name: "正前", yawDeg: 30, pitchDeg: 12, fovDeg: 50 };
    const back = shotReverseOf(front);
    expect(angleBetween(back.yawDeg, front.yawDeg)).toBe(180);
    // 俯仰也翻过来：正打俯视对方，反打就是仰视
    expect(back.pitchDeg).toBe(-12);
    expect(back.fovDeg).toBe(50);
    expect(back.name).toContain("反打");
  });

  it("反打的反打绕回原处", () => {
    const s = { name: "正前", yawDeg: 137, pitchDeg: 0, fovDeg: 70 };
    expect(angleBetween(shotReverseOf(shotReverseOf(s)).yawDeg, s.yawDeg)).toBe(0);
  });

  describe("取景框", () => {
    /**
     * 取景框必须是**裁**出来的，不是改渲染尺寸。
     *
     * 改尺寸会改变 FOV 的含义（同样 70° 在 16:9 和 9:16 下框住的东西
     * 完全不同），而导演调的是「这一镜多广」，不该被画幅偷偷改掉。
     */
    it("按画幅居中裁最大的一块", () => {
      const wide = cropRectForAspect({ width: 1600, height: 900 }, "1:1");
      expect(wide).toEqual({ x: 350, y: 0, width: 900, height: 900 });

      const tall = cropRectForAspect({ width: 900, height: 1600 }, "16:9");
      expect(tall.width).toBe(900);
      expect(tall.height).toBe(506);
      expect(tall.x).toBe(0);
    });

    it("同画幅裁出来就是那个比例", () => {
      for (const a of CAPTURE_ASPECTS) {
        const r = cropRectForAspect({ width: 1280, height: 720 }, a.id);
        expect(r.width / r.height).toBeCloseTo(aspectRatio(a.id), 1);
        // 不能裁出画布外
        expect(r.x + r.width).toBeLessThanOrEqual(1280);
        expect(r.y + r.height).toBeLessThanOrEqual(720);
      }
    });
  });

  /** 「正前方」只改读数的原点，**不改图**。 */
  it("正前方偏移只挪原点", () => {
    expect(applyFrontOffset(0, 90)).toBe(90);
    expect(applyFrontOffset(180, 90)).toBe(-90);
    expect(applyFrontOffset(0, 0)).toBe(0);
  });

  /**
   * 「设为当前视角」：**画面不能动，只是读数归零**。
   * 原来的实现一点就把画面甩回全景图正中。
   */
  it("设为正前方时画面朝向不变、读数归零", () => {
    for (const [yaw, front] of [[37, 0], [-120, 45], [170, 30], [0, 90]]) {
      const before = applyFrontOffset(yaw, front);
      const next = frontToCurrent(yaw, front);
      expect(next.yawDeg).toBe(0);
      expect(applyFrontOffset(next.yawDeg, next.frontYawDeg)).toBe(before);
    }
  });

  it("连按两次结果一样", () => {
    const once = frontToCurrent(60, 20);
    const twice = frontToCurrent(once.yawDeg, once.frontYawDeg);
    expect(twice).toEqual(once);
  });

  describe("文件名", () => {
    /** **带上角度** —— 只写「反打」的话事后看不出它是哪个方向。 */
    it("带上方位和角度", () => {
      const n = shotFileName("安和面馆", { name: "正后", yawDeg: 180, pitchDeg: 0, fovDeg: 70 });
      expect(n).toBe("安和面馆_正后_yaw180_fov70.png");
    });

    it("路径分隔符要被清掉", () => {
      const n = shotFileName("a/b\\c:d", { name: "正前", yawDeg: 0, pitchDeg: 0, fovDeg: 70 });
      expect(n).not.toMatch(/[/\\:]/);
    });

    it("俯仰不为 0 时写进文件名，12 视角不会撞名", () => {
      const names = viewGrid("twelve").shots.map((s) => shotFileName("场景", s));
      expect(new Set(names).size).toBe(12);
      expect(shotFileName("场景", { name: "前方·仰视", yawDeg: 0, pitchDeg: 40, fovDeg: 75 })).toContain("_pitch40_");
    });

    it("角度归一化之后才进文件名", () => {
      const n = shotFileName("场景", { name: "正前", yawDeg: 450, pitchDeg: 0, fovDeg: 70 });
      expect(n).toContain("yaw90");
    });
  });

  describe("多视角截图组", () => {
    it("4 视角：前右后左、平视、90°，排两列", () => {
      const g = viewGrid("four");
      expect(g.cols).toBe(2);
      expect(g.shots.map((s) => [s.name, s.yawDeg, s.pitchDeg, s.fovDeg])).toEqual([
        ["前方", 0, 0, 90],
        ["右侧", 90, 0, 90],
        ["后方", 180, 0, 90],
        ["左侧", -90, 0, 90],
      ]);
    });

    /** 排 4 列时**一行就是一个俯仰** —— 顺序错了组里就是乱的。 */
    it("12 视角：每行一个俯仰，行内是前右后左", () => {
      const g = viewGrid("twelve");
      expect(g.cols).toBe(4);
      expect(g.shots).toHaveLength(12);
      for (let row = 0; row < 3; row++) {
        const line = g.shots.slice(row * 4, row * 4 + 4);
        expect(new Set(line.map((s) => s.pitchDeg)).size).toBe(1);
        expect(line.map((s) => s.yawDeg)).toEqual([0, 90, 180, -90]);
      }
      expect(g.shots.map((s) => s.pitchDeg).filter((_, i) => i % 4 === 0)).toEqual([40, 0, -40]);
      expect(g.shots.every((s) => s.fovDeg === 75)).toBe(true);
    });
  });
});
