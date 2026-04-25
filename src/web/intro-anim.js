import * as THREE from 'three';
import { S } from './state.js';

// Requested sequence timing (ms):
// 1) Flat gray plane initially (no height)
// 2) Heightmap sync wave at 250% + grayscale heightmap look
// 3) Wait 3s
// 4) Texture diagonal reveal
// 4) Water rise to ocean level
// 5) Reveal battle zone + objects
// 6) UI popup handled by viewer-app on terrain-intro-complete event (2s CSS)
const PHASE_WAIT_MS = 0;
const PHASE_HEIGHT_MS = 5000;
const PHASE_PRE_TEX_DELAY_MS = 2000;
const PHASE_TEX_MS = 3000;
const PHASE_WATER_MS = 1000;
const PHASE_OBJECTS_MS = 1200;
const WAVE_END = 2.20;

const TILE_SPACING_M = 100.0;

const ease = (x) => {
  const c = Math.max(0, Math.min(1, x));
  return c * c * (3 - 2 * c);
};

function materialList(root) {
  const list = [];
  root.traverse((child) => {
    if (!child.material) return;
    if (Array.isArray(child.material)) {
      for (const m of child.material) if (m) list.push(m);
    } else {
      list.push(child.material);
    }
  });
  return list;
}

function captureObjectState(obj) {
  const mats = materialList(obj).map((mat) => ({
    mat,
    opacity: mat.opacity,
    transparent: mat.transparent,
  }));
  return { obj, mats };
}

function setObjectAlpha(state, alpha) {
  const a = Math.max(0, Math.min(1, alpha));
  state.obj.visible = a > 0.005;
  for (const m of state.mats) {
    m.mat.transparent = true;
    m.mat.opacity = m.opacity * a;
    m.mat.needsUpdate = true;
  }
}

function restoreObjectState(state) {
  state.obj.visible = true;
  for (const m of state.mats) {
    m.mat.opacity = m.opacity;
    m.mat.transparent = m.transparent;
    m.mat.needsUpdate = true;
  }
}

function injectIntroShader(material, opts = {}) {
  if (!material || material.userData.introUniforms) return null;

  const invDiag = opts.invDiag || 1;
  const tileStep = Math.max(0.0001, TILE_SPACING_M * invDiag * 2.0);

  const uniforms = {
    uHeightWave: { value: -0.08 },
    uHeightHueMix: { value: 0.0 },
    uTexWave: { value: -0.08 },
    uHeightMul: { value: 2.0 },
    uInvDiag: { value: invDiag },
    uTileStep: { value: tileStep },
    uOriginXZ: { value: opts.originXZ ?? new THREE.Vector2(0, 0) },
  };

  material.userData.introUniforms = uniforms;
  material.transparent = true;

  const prev = material.onBeforeCompile;
  material.onBeforeCompile = (shader) => {
    if (typeof prev === 'function') prev(shader);

    Object.assign(shader.uniforms, {
      uHeightWave: uniforms.uHeightWave,
      uHeightHueMix: uniforms.uHeightHueMix,
      uTexWave: uniforms.uTexWave,
      uHeightMul: uniforms.uHeightMul,
      uInvDiag: uniforms.uInvDiag,
      uTileStep: uniforms.uTileStep,
      uOriginXZ: uniforms.uOriginXZ,
    });

    shader.vertexShader = shader.vertexShader.replace(
      'void main() {',
      `uniform float uHeightWave;
    uniform float uTexWave;
uniform float uHeightMul;
uniform float uInvDiag;
uniform float uTileStep;
uniform vec2 uOriginXZ;
varying vec2 vIntroXZ;
varying float vIntroR;
varying float vIntroH;
void main() {`
    );

    if (shader.vertexShader.includes('#include <displacementmap_vertex>')) {
      shader.vertexShader = shader.vertexShader.replace(
        '#include <displacementmap_vertex>',
        `
// Corner-diagonal metric from UV top-left -> bottom-right.
vIntroXZ = position.xz - uOriginXZ;
float _d = (uv.x + (1.0 - uv.y));
float _rTile = floor(_d / uTileStep) * uTileStep;
vIntroR = _rTile;
vIntroH = 0.5;

#ifdef USE_DISPLACEMENTMAP
  float _hm = texture2D(displacementMap, vDisplacementMapUv).x;
  vIntroH = _hm;
  float _hPassed = 1.0 - smoothstep(uHeightWave - 0.025, uHeightWave + 0.020, _rTile);
  transformed += normalize(objectNormal) * (
    (_hm * displacementScale * uHeightMul + displacementBias) * _hPassed
  );
#endif
`
      );
    } else {
      shader.vertexShader = shader.vertexShader.replace(
        '#include <begin_vertex>',
        `#include <begin_vertex>
vIntroXZ = position.xz - uOriginXZ;
float _d = (uv.x + (1.0 - uv.y));
float _rTile = floor(_d / uTileStep) * uTileStep;
vIntroR = _rTile;
vIntroH = 0.5;`
      );
    }

    shader.fragmentShader = shader.fragmentShader.replace(
      'void main() {',
      `uniform float uHeightWave;
    uniform float uHeightHueMix;
    uniform float uTexWave;
varying vec2 vIntroXZ;
varying float vIntroR;
varying float vIntroH;
void main() {`
    );

    shader.fragmentShader = shader.fragmentShader.replace(
      /\}\s*$/,
      `
  // Flat gray intro material with diagonal texture replacement.
  {
    // Texture takes over from corner (diagonal metric).
    float _texPassed = 1.0 - smoothstep(uTexWave - 0.020, uTexWave + 0.020, vIntroR);
    float _heightPassed = 1.0 - smoothstep(uHeightWave - 0.020, uHeightWave + 0.020, vIntroR);

    float _h = clamp(vIntroH, 0.0, 1.0);
    vec3 _flatGray = vec3(0.46, 0.46, 0.46);
    // Match panel "Heightmap" look: grayscale from height value.
    float _hmGray = clamp(_h, 0.0, 1.0);
    vec3 _heightMapGray = vec3(_hmGray);
    vec3 _heightColor = mix(_flatGray, _heightMapGray, clamp(uHeightHueMix * _heightPassed, 0.0, 1.0));

    vec3 _texColor = gl_FragColor.rgb;
    gl_FragColor.rgb = mix(_heightColor, _texColor, _texPassed);
    gl_FragColor.a = gl_FragColor.a;
  }
}`
    );
  };

  material.needsUpdate = true;
  return uniforms;
}

function clearIntroShader(material) {
  if (!material || !material.userData.introUniforms) return;
  delete material.userData.introUniforms;
  material.onBeforeCompile = () => {};
  material.needsUpdate = true;
}

function buildHandles(meshes, opts) {
  const handles = [];
  for (const mesh of meshes) {
    if (!mesh || !mesh.material || !mesh.geometry) continue;

    mesh.geometry.computeBoundingBox();
    const bb = mesh.geometry.boundingBox;
    const originXZ = new THREE.Vector2(bb.min.x, bb.min.z);
    const dx = bb.max.x - bb.min.x;
    const dz = bb.max.z - bb.min.z;
    const maxDiag = Math.max(1, dx + dz);

    const uniforms = injectIntroShader(mesh.material, {
      ...opts,
      invDiag: 1 / maxDiag,
      originXZ,
    });
    if (!uniforms) continue;

    handles.push({
      mesh,
      uniforms,
      finalOpacity: mesh.material.opacity,
    });

    // Prevent one-frame base-texture flash before shader-driven intro kicks in.
    mesh.material.transparent = true;
    mesh.material.opacity = 0;
    mesh.material.needsUpdate = true;
  }
  return handles;
}

function runIntroTimeline({ handles, waterMesh, waterFinalOpacity, objectStates }) {
  if (handles.length === 0) return;

  const tWaitEnd = PHASE_WAIT_MS;
  const tHeightEnd = tWaitEnd + PHASE_HEIGHT_MS;
  const tPreTexEnd = tHeightEnd + PHASE_PRE_TEX_DELAY_MS;
  const tTexEnd = tPreTexEnd + PHASE_TEX_MS;
  const tWaterEnd = tTexEnd + PHASE_WATER_MS;
  const tObjectsEnd = tWaterEnd + PHASE_OBJECTS_MS;

  const start = performance.now();

  function frame(now) {
    const t = now - start;
    const pHeight = ease((t - tWaitEnd) / PHASE_HEIGHT_MS);
    const pTex = ease((t - tPreTexEnd) / PHASE_TEX_MS);
    const pWater = ease((t - tTexEnd) / PHASE_WATER_MS);
    const pObjects = ease((t - tWaterEnd) / PHASE_OBJECTS_MS);

    const heightWave = -0.08 + Math.max(0, Math.min(1, pHeight)) * WAVE_END;
    const texWave = -0.08 + Math.max(0, Math.min(1, pTex)) * WAVE_END;
    const heightMul = 2.5;
    const heightHueMix = Math.max(0, Math.min(1, pHeight));

    for (const h of handles) {
      h.uniforms.uHeightWave.value = heightWave;
      h.uniforms.uHeightHueMix.value = heightHueMix;
      h.uniforms.uTexWave.value = texWave;
      h.uniforms.uHeightMul.value = heightMul;
      h.mesh.material.opacity = 1;
    }

    if (waterMesh) {
      waterMesh.material.opacity = Math.max(0, Math.min(1, pWater)) * waterFinalOpacity;
    }

    for (const state of objectStates) {
      setObjectAlpha(state, Math.max(0, Math.min(1, pObjects)));
    }

    S.needsRender = true;

    if (t < tObjectsEnd) {
      requestAnimationFrame(frame);
      return;
    }

    for (const h of handles) {
      clearIntroShader(h.mesh.material);
      if (h.finalOpacity != null) h.mesh.material.opacity = h.finalOpacity;
    }

    if (waterMesh) {
      waterMesh.material.opacity = waterFinalOpacity;
      waterMesh.material.needsUpdate = true;
    }

    for (const state of objectStates) restoreObjectState(state);

    S.terrainIntroDone = true;
    // Tell UI layer it can do the 2s slide/fade in now.
    window.dispatchEvent(new CustomEvent('terrain-intro-complete'));

    S.needsRender = true;
  }

  requestAnimationFrame(frame);
}

export function playTerrainIntro(meshes) {
  S.terrainIntroDone = false;
  const handles = buildHandles(meshes, {});
  if (handles.length === 0) return;

  const objectStates = [];
  for (const key of ['tankZoneMesh', 'rendinstGroup']) {
    const obj = S[key];
    if (!obj) continue;
    const st = captureObjectState(obj);
    objectStates.push(st);
    setObjectAlpha(st, 0);
  }

  let waterMesh = null;
  let waterFinalOpacity = 0;
  if (S.waterMesh && S.waterMesh.visible && S.waterMesh.material) {
    waterMesh = S.waterMesh;
    waterFinalOpacity = waterMesh.material.opacity ?? 0.92;
    waterMesh.material.transparent = true;
    waterMesh.material.opacity = 0;
    waterMesh.material.needsUpdate = true;
  }

  runIntroTimeline({ handles, waterMesh, waterFinalOpacity, objectStates });
}

export function playMeshIntro(mesh, opts = {}) {
  if (!mesh || !mesh.material || !mesh.geometry) return;

  const handles = buildHandles([mesh], {});
  if (handles.length === 0) return;

  const totalMs = Math.max(2400, Math.round(opts.durationMs ?? 13000));
  const localWaitMs = 0;
  const localHeightMs = Math.max(1200, Math.round(totalMs * 0.55));
  const localPreTexDelayMs = Math.max(800, Math.round(totalMs * 0.23));
  const localTexMs = Math.max(800, totalMs - localHeightMs - localPreTexDelayMs);

  const start = performance.now();
  const tWaitEnd = localWaitMs;
  const tHeightEnd = localWaitMs + localHeightMs;
  const tPreTexEnd = tHeightEnd + localPreTexDelayMs;
  const total = localWaitMs + localHeightMs + localPreTexDelayMs + localTexMs;

  function frame(now) {
    const t = now - start;
    const pHeight = ease((t - tWaitEnd) / localHeightMs);
    const pTex = ease((t - tPreTexEnd) / localTexMs);
    const heightWave = -0.08 + Math.max(0, Math.min(1, pHeight)) * WAVE_END;
    const texWave = -0.08 + Math.max(0, Math.min(1, pTex)) * WAVE_END;
    const heightMul = 2.5;
    const heightHueMix = Math.max(0, Math.min(1, pHeight));

    for (const h of handles) {
      h.uniforms.uHeightWave.value = heightWave;
      h.uniforms.uHeightHueMix.value = heightHueMix;
      h.uniforms.uTexWave.value = texWave;
      h.uniforms.uHeightMul.value = heightMul;
      h.mesh.material.opacity = 1;
    }
    S.needsRender = true;

    if (t < total) {
      requestAnimationFrame(frame);
      return;
    }

    for (const h of handles) {
      clearIntroShader(h.mesh.material);
      if (h.finalOpacity != null) h.mesh.material.opacity = h.finalOpacity;
    }
    S.needsRender = true;
  }

  requestAnimationFrame(frame);
}
