/**
 * 取景的测试。
 *
 * 上一个取景 bug（T-pose 的双臂把包围盒撑宽，"是不是全身"的判断失效，
 * 整个人被框进画面、脸只剩几十个像素）是埋在组件里没法测才留到用户那边的。
 * 取景全是几何，本来就该单独测。
 */
import { describe, expect, it } from "vitest";
import { Box3, Vector3 } from "three";
import { portraitFraming } from "./framing";

const FOV = 24;

/** 一个 1.6 米高的全身模型，头在最上面 0.22 米 */
function fullBody() {
  const model = new Box3(new Vector3(-0.3, 0, -0.2), new Vector3(0.3, 1.6, 0.2));
  const head = new Box3(
    new Vector3(-0.09, 1.38, -0.09),
    new Vector3(0.09, 1.6, 0.09),
  );
  return { model, head };
}

/** 画面上下沿在世界空间里的位置 */
function frameSpan(fovDeg: number, distance: number) {
  return 2 * distance * Math.tan((fovDeg * Math.PI) / 180 / 2);
}

describe("半身取景", () => {
  it("画面里装得下头和上半身", () => {
    // 用户的原话是"往后站一站，至少看到上半身"
    const { model, head } = fullBody();
    const f = portraitFraming(head, model, FOV);
    const span = frameSpan(FOV, f.distance);
    const headHeight = head.max.y - head.min.y;

    // 实际调出来大约 2.35 个头高（头 + 肩，胸带一点点）。
    // 上下界留得比这宽 —— 这条守的是"别退成全身、也别糊到脸上"，
    // 具体松紧是看着画面调的，不该被测试钉死
    expect(span / headHeight, "太紧了，看不到肩膀").toBeGreaterThan(1.9);
    expect(span / headHeight, "退太远脸就小了").toBeLessThan(3.0);
  });

  it("头在上三分之一，不居中", () => {
    // 头顶正好贴中线会显得头重脚轻
    const { model, head } = fullBody();
    const f = portraitFraming(head, model, FOV);
    const span = frameSpan(FOV, f.distance);
    const top = f.center.y + span / 2;
    const headroom = (top - head.max.y) / span;

    expect(headroom, `头顶留白 ${(headroom * 100).toFixed(0)}%`).toBeGreaterThan(0.02);
    expect(headroom, "顶部空太多，人会显得往下沉").toBeLessThan(0.2);
  });

  it("头发和头饰不能被切掉", () => {
    // **实测每个模型都被切了头顶。** 原因是"脸盒"只含带口型通道的网格，
    // 而头发是单独的网格、没有口型通道 —— 脸盒的顶端只到发际线附近
    const model = new Box3(new Vector3(-0.3, 0, -0.2), new Vector3(0.3, 1.75, 0.2));
    // 脸盒顶端 1.60，而头发/头饰一直到 1.75
    const face = new Box3(new Vector3(-0.09, 1.38, -0.09), new Vector3(0.09, 1.6, 0.09));
    const f = portraitFraming(face, model, FOV, 1, new Vector3(0, 1.45, 0));
    const top = f.center.y + frameSpan(FOV, f.distance) / 2;

    expect(top, `画面上沿 ${top.toFixed(3)}，头顶在 ${model.max.y}`).toBeGreaterThan(
      model.max.y,
    );
  });

  it("举过头顶的手不该把镜头带飞", () => {
    // 视觉最高点通常是头，但不总是。高出脸盒太多就不采信
    const model = new Box3(new Vector3(-0.8, 0, -0.2), new Vector3(0.8, 2.6, 0.2));
    const face = new Box3(new Vector3(-0.09, 1.38, -0.09), new Vector3(0.09, 1.6, 0.09));
    const f = portraitFraming(face, model, FOV, 1, new Vector3(0, 1.45, 0));
    expect(f.center.y).toBeLessThan(2.0);
  });

  it("头饰很高时把画框撑大，而不是把身体挤出去", () => {
    // **这一条是第四次取景 bug 的回归。** 当时只把上沿抬到模型顶端、
    // 画框高度不变，结果整个框上移，胸口被切掉，画面里只剩一个头。
    const face = new Box3(new Vector3(-0.09, 1.38, -0.09), new Vector3(0.09, 1.6, 0.09));
    const bone = new Vector3(0, 1.45, 0);
    const plain = new Box3(new Vector3(-0.3, 0, -0.2), new Vector3(0.3, 1.75, 0.2));
    // 同一个人，头上多了 0.2 米高的尖角头饰
    const spiky = new Box3(new Vector3(-0.3, 0, -0.2), new Vector3(0.3, 1.95, 0.2));

    const a = portraitFraming(face, plain, FOV, 1, bone);
    const b = portraitFraming(face, spiky, FOV, 1, bone);

    // 画框要变大（人变小），不是原样上移
    expect(b.distance).toBeGreaterThan(a.distance);
    // 而且两种情况下，下沿都要落在同一个高度附近（胸口），不能被顶上去
    const bottomOf = (f: typeof a) => f.center.y - frameSpan(FOV, f.distance) / 2;
    expect(Math.abs(bottomOf(b) - bottomOf(a))).toBeLessThan(0.05);
    // 头饰也要完整在画面里
    const topOf = (f: typeof a) => f.center.y + frameSpan(FOV, f.distance) / 2;
    expect(topOf(b)).toBeGreaterThan(spiky.max.y);
  });

  it("画面底沿落在身体上，不是悬空", () => {
    const { model, head } = fullBody();
    const f = portraitFraming(head, model, FOV);
    const bottom = f.center.y - frameSpan(FOV, f.distance) / 2;
    expect(bottom).toBeGreaterThan(model.min.y);
    expect(bottom).toBeLessThan(head.min.y);
  });

  it("只有头的模型不会硬退成半身，否则画面一大半是空的", () => {
    // 内置头、或者只导出头部的模型
    const head = new Box3(new Vector3(-0.1, 0, -0.1), new Vector3(0.1, 0.24, 0.1));
    const f = portraitFraming(head, head.clone(), FOV);
    const span = frameSpan(FOV, f.distance);
    expect(span / 0.24, "只有头时不该硬退成半身").toBeLessThan(1.6);
  });

  it("模型尺度变一百倍，取景比例不变", () => {
    // 不同工具导出的单位不一样（米 / 厘米 / 英寸），写死距离只能对一种
    const small = fullBody();
    const a = portraitFraming(small.head, small.model, FOV);

    const scale = 100;
    const model2 = new Box3(
      small.model.min.clone().multiplyScalar(scale),
      small.model.max.clone().multiplyScalar(scale),
    );
    const head2 = new Box3(
      small.head.min.clone().multiplyScalar(scale),
      small.head.max.clone().multiplyScalar(scale),
    );
    const b = portraitFraming(head2, model2, FOV);

    expect(b.distance / a.distance).toBeCloseTo(scale, 1);
  });

  it("很窄的画面要退得更远，否则肩膀会被切掉", () => {
    // 常规宽高比下竖直方向才是约束（半身像本来就是竖着的），
    // 所以横向那条只在画面明显偏窄时才起作用 —— 用 0.3 才测得到它
    const { model, head } = fullBody();
    const wide = portraitFraming(head, model, FOV, 1.6);
    const narrow = portraitFraming(head, model, FOV, 0.3);
    expect(narrow.distance).toBeGreaterThan(wide.distance);
    // 常规比例下横向不该反过来影响取景
    expect(portraitFraming(head, model, FOV, 1.0).distance).toBeCloseTo(
      wide.distance,
      6,
    );
  });

  it("脸盒等于全身时不能把全身高当头高", () => {
    // 有的模型整个身体是一个网格、口型也挂在上面，于是"带口型的网格"的
    // 包围盒等于全身盒。拿它当头高会乘出天文数字 —— 实测把相机推到 5.79 米，
    // 模型缩成画面里的一小点，看起来就是"不显示"
    const { model } = fullBody();
    const headBone = new Vector3(0, 1.45, 0);
    const sane = portraitFraming(
      new Box3(new Vector3(-0.09, 1.38, -0.09), new Vector3(0.09, 1.6, 0.09)),
      model,
      FOV,
      1,
      headBone,
    );
    const degenerate = portraitFraming(model.clone(), model, FOV, 1, headBone);

    expect(
      degenerate.distance,
      `脸盒退化时相机被推到 ${degenerate.distance.toFixed(2)}m，正常应当在 ${sane.distance.toFixed(2)}m 附近`,
    ).toBeLessThan(sane.distance * 2.5);
  });

  it("退化的输入不产生 NaN 或负距离", () => {
    const zero = new Box3(new Vector3(), new Vector3());
    const f = portraitFraming(zero, zero, FOV);
    expect(Number.isFinite(f.distance)).toBe(true);
    expect(f.distance).toBeGreaterThan(0);
  });
});
