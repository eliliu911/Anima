/**
 * 导演世界里的道具（导演台界面升级 U3.5，用户 2026-09-27：「道具也可以增加进来了」）。
 *
 * 画成一块**立牌**：竖着的一张道具资产图，底边贴地、两面都看得见，外面一圈描边。
 * 没有图就是一个方块。人偶稿里它就以这张图出现，换成角色那一步把图里的道具画成真的。
 *
 * 实现和人偶同一个接口（`Mannequin`），所以选中、拖动、撤销、存盘、输出都直接复用。
 */
import * as THREE from "three";
import type { Mannequin } from "./mannequins";
import type { PoseName } from "./poses";

/** 立牌的默认高度（米）。缩放滑块在这个基础上放大缩小 */
const STANDEE_H = 1.0;
const BOX = 0.6;

export function createPropObject(color: string, imageUrl: Promise<string> | null): Mannequin {
  const root = new THREE.Group();
  const frameMat = new THREE.LineBasicMaterial({ color: new THREE.Color(color) });
  const mask = new THREE.MeshBasicMaterial({ color: 0xffffff, side: THREE.DoubleSide });
  const meshes: THREE.Mesh[] = [];
  const own = new Map<THREE.Mesh, THREE.Material>();
  let frame: THREE.LineSegments | null = null;
  let disposed = false;

  const setFrame = (w: number, h: number) => {
    if (frame) {
      root.remove(frame);
      frame.geometry.dispose();
    }
    const pts = [[-w / 2, 0], [w / 2, 0], [w / 2, 0], [w / 2, h], [w / 2, h], [-w / 2, h], [-w / 2, h], [-w / 2, 0]].map(
      ([x, y]) => new THREE.Vector3(x, y, 0.002),
    );
    frame = new THREE.LineSegments(new THREE.BufferGeometry().setFromPoints(pts), frameMat);
    root.add(frame);
  };

  if (imageUrl) {
    const mat = new THREE.MeshBasicMaterial({ color: 0xffffff, side: THREE.DoubleSide, transparent: true });
    const plane = new THREE.Mesh(new THREE.PlaneGeometry(STANDEE_H, STANDEE_H), mat);
    plane.position.y = STANDEE_H / 2;
    root.add(plane);
    meshes.push(plane);
    own.set(plane, mat);
    setFrame(STANDEE_H, STANDEE_H);
    void imageUrl
      .then((url) => new THREE.TextureLoader().loadAsync(url))
      .then((tex) => {
        if (disposed) {
          tex.dispose();
          return;
        }
        tex.colorSpace = THREE.SRGBColorSpace;
        mat.map = tex;
        mat.needsUpdate = true;
        // 按图的长宽比改立牌宽度，高度不变
        const img = tex.image as { width?: number; height?: number } | undefined;
        const ar = img?.width && img?.height ? img.width / img.height : 1;
        const w = STANDEE_H * ar;
        plane.geometry.dispose();
        plane.geometry = new THREE.PlaneGeometry(w, STANDEE_H);
        setFrame(w, STANDEE_H);
      })
      .catch(() => {
        // 图读不出来就留白牌 + 描边，不影响摆放
      });
  } else {
    const mat = new THREE.MeshBasicMaterial({ color: new THREE.Color(color) });
    const box = new THREE.Mesh(new THREE.BoxGeometry(BOX, BOX, BOX), mat);
    box.position.y = BOX / 2;
    root.add(box);
    meshes.push(box);
    own.set(box, mat);
  }

  return {
    root,
    setColor(hex) {
      frameMat.color.set(hex);
      for (const [m, mat] of own) if (!m.geometry.type.startsWith("Plane")) (mat as THREE.MeshBasicMaterial).color.set(hex);
    },
    setPose(_pose: PoseName) {
      // 道具没有姿势
    },
    setMaskMaterial(on) {
      for (const m of meshes) m.material = on ? mask : own.get(m)!;
      if (frame) frame.visible = !on;
    },
    bounds() {
      return new THREE.Box3().setFromObject(root);
    },
    dispose() {
      disposed = true;
      for (const m of meshes) {
        m.geometry.dispose();
        const mat = own.get(m) as THREE.MeshBasicMaterial;
        mat.map?.dispose();
        mat.dispose();
      }
      frame?.geometry.dispose();
      frameMat.dispose();
      mask.dispose();
    },
  };
}
