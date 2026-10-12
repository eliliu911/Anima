import * as THREE from "three";

import type { HologramStyle } from "../store/face";

/**
 * 将整个 3D 模型转换为全息粒子网格风格
 * @param root 模型的根节点
 * @param style 全息风格
 * @param hexColor 颜色的 HEX 字符串，例如 "#1a9fff"
 */
export function applyHologramEffect(root: THREE.Object3D, style: HologramStyle = "dot", hexColor: string = "#1a9fff") {
  const baseColor = new THREE.Color(hexColor);
  root.traverse((child) => {
    const mesh = child as THREE.Mesh;
    if (mesh.isMesh) {
      // 必须用带有法线和完整顶点属性的材质，为了确保 Shader chunk 齐全，
      // 用 MeshStandardMaterial 做基底
      const holoMat = new THREE.MeshStandardMaterial({
        color: 0x000000,
        emissive: 0x0055ff,
        emissiveIntensity: 1.0,
        transparent: true,
        blending: THREE.AdditiveBlending,
        depthWrite: false,
        side: THREE.DoubleSide,
        wireframe: false,
      });

      holoMat.onBeforeCompile = (shader) => {
        shader.uniforms.uTime = { value: 0 };
        shader.uniforms.uTalkIntensity = { value: 0 };
        shader.uniforms.uBaseColor = { value: baseColor };

        mesh.userData.shader = shader; // 把 shader 引用存下来，方便在 render loop 里更新

        // 顶点着色器：加入微弱噪波流动
        shader.vertexShader = `
          uniform float uTime;
          uniform float uTalkIntensity;
          varying vec3 vViewPositionHolo;
          varying vec3 vNormalHolo;
          varying vec2 vUvHolo;
        ` + shader.vertexShader;

        let vertexMotion = `
          // 基础微弱呼吸
          float noise = sin(position.y * 30.0 + uTime * 5.0) * cos(position.x * 20.0 + uTime * 3.0);
          transformed += normal * noise * 0.003 * uTalkIntensity;
        `;
        
        if (style === "glitch") {
          vertexMotion = `
            // 剧烈横向撕裂故障
            float glitchLine = step(0.9, fract(sin(position.y * 100.0 + uTime * 20.0) * 43758.5453));
            transformed.x += glitchLine * 0.05 * uTalkIntensity;
            transformed.z += glitchLine * 0.02 * uTalkIntensity;
          `;
        } else if (style === "ripple") {
          vertexMotion = `
            // 嘴部为中心的声波涟漪膨胀
            float distToMouth = length(position.y + 1.4); // 粗略假设嘴巴在 Y=-1.4
            float ripple = sin(distToMouth * 30.0 - uTime * 15.0) * exp(-distToMouth * 5.0);
            transformed += normal * ripple * 0.015 * uTalkIntensity;
          `;
        }

        shader.vertexShader = shader.vertexShader.replace(
          '#include <begin_vertex>',
          `
          #include <begin_vertex>
          vUvHolo = uv;

          ${vertexMotion}
          `
        );

        shader.vertexShader = shader.vertexShader.replace(
          '#include <project_vertex>',
          `
          #include <project_vertex>
          // 保存用来做边缘发光的法线和视角方向。这必须在蒙皮后获取，否则会导致 "T-pose shader abnormalities"
          vNormalHolo = normalize(normalMatrix * objectNormal);
          vViewPositionHolo = -mvPosition.xyz;
          `
        );

        // 片段着色器：点阵遮罩 + 菲涅尔轮廓光 + 扫描线
        shader.fragmentShader = `
          uniform float uTime;
          uniform float uTalkIntensity;
          uniform vec3 uBaseColor;
          varying vec3 vViewPositionHolo;
          varying vec3 vNormalHolo;
          varying vec2 vUvHolo;
        ` + shader.fragmentShader;

        let fragmentEffect = "";
        
        if (style === "dot") {
          fragmentEffect = `
            vec2 grid = fract(vUvHolo * 150.0);
            float dist = length(grid - vec2(0.5));
            float dotAlpha = smoothstep(0.4, 0.2, dist);
            float scanLine = 0.5 + 0.5 * sin(uTime * 3.0 - gl_FragCoord.y * 0.02);
            // 只有点才有透明度，去掉底层的表面发光填充
            float finalAlpha = dotAlpha * 0.8 * (0.6 + 0.4 * scanLine);
            // 边缘高光用来增强点的亮度，而不是填充表面
            holoColor += uBaseColor * fresnel; 
            holoColor += uBaseColor * uTalkIntensity * 0.8;
            gl_FragColor = vec4(holoColor, finalAlpha);
          `;
        } else if (style === "wireframe") {
          fragmentEffect = `
            // 等高线
            float lines = fract(vViewPositionHolo.y * 60.0 + uTime * 2.0);
            float lineAlpha = step(0.85, lines) * 0.8;
            float finalAlpha = max(fresnel * 0.8, lineAlpha) * 0.8;
            holoColor += uBaseColor * uTalkIntensity * 0.8;
            gl_FragColor = vec4(holoColor, finalAlpha);
          `;
        } else if (style === "glitch") {
          fragmentEffect = `
            float finalAlpha = fresnel * 0.8;
            // RGB 分离
            vec3 glitchColor = holoColor;
            float rShift = uTalkIntensity * 0.5 * step(0.9, fract(uTime * 15.0));
            float bShift = uTalkIntensity * -0.5 * step(0.95, fract(uTime * 12.0));
            glitchColor.r += rShift;
            glitchColor.b += bShift;
            
            // 扫描横纹干扰
            float scanInterference = step(0.95, fract(gl_FragCoord.y * 0.05 + uTime * 10.0));
            finalAlpha += scanInterference * 0.5 * uTalkIntensity;
            
            gl_FragColor = vec4(glitchColor, finalAlpha);
          `;
        } else if (style === "ripple") {
          fragmentEffect = `
            // 从中心向外扩散的波纹蒙版
            float distFromCenter = length(vViewPositionHolo.xy);
            float waveAlpha = sin(distFromCenter * 20.0 - uTime * 10.0) * 0.5 + 0.5;
            waveAlpha *= exp(-distFromCenter * 0.2); // 边缘淡出
            float finalAlpha = max(fresnel * 0.8, waveAlpha * uTalkIntensity * 0.8);
            holoColor += uBaseColor * uTalkIntensity;
            gl_FragColor = vec4(holoColor, finalAlpha);
          `;
        }

        shader.fragmentShader = shader.fragmentShader.replace(
          '#include <dithering_fragment>',
          `
          #include <dithering_fragment>
          
          // Fresnel (边缘发光)
          vec3 viewDir = normalize(vViewPositionHolo);
          float fresnel = dot(viewDir, normalize(vNormalHolo));
          fresnel = clamp(1.0 - fresnel, 0.0, 1.0);
          fresnel = pow(fresnel, 2.0);
          
          vec3 holoColor = uBaseColor; // 基础色
          
          ${fragmentEffect}
          `
        );
      };

      mesh.material = holoMat;
    }
  });
}

/**
 * 在 render loop 里调用，更新所有全息材质的 uniform
 */
export function updateHologram(root: THREE.Object3D, time: number, talkIntensity: number) {
  root.traverse((child) => {
    const mesh = child as THREE.Mesh;
    if (mesh.isMesh && mesh.userData.shader) {
      mesh.userData.shader.uniforms.uTime.value = time;
      mesh.userData.shader.uniforms.uTalkIntensity.value = talkIntensity;
    }
  });
}
