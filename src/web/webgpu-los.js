import * as THREE from 'three';
import { S } from './state.js';
import { sceneToWorld } from './hover.js';
import { buildLosMesh } from './tools/los.js';
import { worldToSceneXZ } from './coords.js';

// LoS constants (GPU path only)
const _LOS_RANGE = 1500;
const _LOS_STEP_M = 2.0;
// Eye height above the ground at the LoS origin (metres). Matches the
// "extra 5 m above ground" assumption used elsewhere in the viewer so a
// shooter standing on the highlighted terrain pixel is treated as a
// 5 m-tall observer rather than ground-level.
const _LOS_EYE_HEIGHT_M = 5.0;
const _GPU_LOS_RAYS = 720;
const _GPU_LOS_STEPS = Math.max(1, Math.floor(_LOS_RANGE / _LOS_STEP_M));

const _LOS_SHADER_WGSL = `
struct Params {
  originX: f32, originZ: f32, originH: f32, range_m: f32,
  rayCount: u32, stepsPerRay: u32, hmW: u32, hmH: u32,
  weX0: f32, weZ0: f32, weX1: f32, weZ1: f32,
  hMin: f32, hMax: f32,
  hm2X0: f32, hm2Z0: f32, hm2X1: f32, hm2Z1: f32,
  hm2Min: f32, hm2Max: f32,
  hm2W: u32, hm2H: u32, hasHm2: u32, hasOcc: u32,
  occX0: f32, occZ0: f32, occCell: f32, _pad1: f32,
  occW: u32, occH: u32, _pad2: u32, _pad3: u32,
};

@group(0) @binding(0) var<storage, read> hm: array<f32>;
@group(0) @binding(1) var<storage, read> hm2: array<f32>;
@group(0) @binding(2) var<uniform> p: Params;
@group(0) @binding(3) var<storage, read_write> result: array<f32>;
@group(0) @binding(4) var<storage, read> occ: array<f32>;

fn sampleH(wx: f32, wz: f32) -> f32 {
  if (p.hasHm2 != 0u) {
    let x0 = min(p.hm2X0, p.hm2X1);
    let x1 = max(p.hm2X0, p.hm2X1);
    let z0 = min(p.hm2Z0, p.hm2Z1);
    let z1 = max(p.hm2Z0, p.hm2Z1);
    if (wx >= x0 && wx <= x1 && wz >= z0 && wz <= z1) {
      let u2 = (wx - x0) / max(x1 - x0, 0.001);
      let v2 = (wz - z0) / max(z1 - z0, 0.001);
      let px = clamp(u32(round(u2 * f32(p.hm2W - 1u))), 0u, p.hm2W - 1u);
      let py = clamp(u32(round(v2 * f32(p.hm2H - 1u))), 0u, p.hm2H - 1u);
      return p.hm2Min + hm2[py * p.hm2W + px] * (p.hm2Max - p.hm2Min);
    }
  }
  let weW = max(p.weX1 - p.weX0, 0.001);
  let weH = max(p.weZ1 - p.weZ0, 0.001);
  let u = (wx - p.weX0) / weW;
  let v = (wz - p.weZ0) / weH;
  let px = clamp(u32(round(u * f32(p.hmW - 1u))), 0u, p.hmW - 1u);
  let py = clamp(u32(round(v * f32(p.hmH - 1u))), 0u, p.hmH - 1u);
  return p.hMin + hm[py * p.hmW + px] * (p.hMax - p.hMin);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let ray = gid.x;
  if (ray >= p.rayCount) { return; }
  let angle = f32(ray) / f32(p.rayCount) * 6.2831853;
  let dx = cos(angle);
  let dz = sin(angle);
  let stepM: f32 = 2.0;
  var maxSlope: f32 = -1e10;
  var objBlocked: bool = false;
  for (var s: u32 = 0u; s < p.stepsPerRay; s = s + 1u) {
    let dist = min(p.range_m, stepM * f32(s + 1u));
    let wx = p.originX + dx * dist;
    let wz = p.originZ + dz * dist;
    let th = sampleH(wx, wz);
    let slope = (th - p.originH) / dist;
    let terrainMaxSlopeBefore = maxSlope;
    var vis: f32 = 0.0;
    if (slope >= terrainMaxSlopeBefore) { vis = 1.0; }
    if (objBlocked) { vis = 0.0; }
    if (p.hasOcc != 0u && vis > 0.5) {
      let prevDist = select(0.0, min(p.range_m, stepM * f32(s)), s > 0u);
      let gap = max(0.001, dist - prevDist);
      let nSamples = min(8u, max(1u, u32(ceil(gap / max(0.5, p.occCell)))));
      for (var c: u32 = 0u; c < nSamples; c = c + 1u) {
        let t = prevDist + gap * (f32(c) + 0.5) / f32(nSamples);
        let owx = p.originX + dx * t;
        let owz = p.originZ + dz * t;
        let gx = i32(floor((owx - p.occX0) / p.occCell));
        let gz = i32(floor((owz - p.occZ0) / p.occCell));
        if (gx >= 0 && u32(gx) < p.occW && gz >= 0 && u32(gz) < p.occH) {
          let objTop = occ[u32(gz) * p.occW + u32(gx)];
          if (objTop > 0.0) {
            let eyeHAtT = p.originH + terrainMaxSlopeBefore * t;
            if (objTop > eyeHAtT) {
              vis = 0.0;
              objBlocked = true;
              break;
            }
          }
        }
      }
    }
    if (slope > maxSlope) { maxSlope = slope; }
    let idx = (ray * p.stepsPerRay + s) * 4u;
    result[idx] = wx;
    result[idx + 1u] = wz;
    result[idx + 2u] = th;
    result[idx + 3u] = vis;
  }
}
`;

export async function initWebGPU() {
  if (typeof navigator === 'undefined' || !navigator.gpu) { console.info('WebGPU: not supported'); return false; }
  try {
    const adapter = await navigator.gpu.requestAdapter({ powerPreference: 'high-performance' });
    if (!adapter) { console.info('WebGPU: no adapter'); return false; }
    const lim = adapter.limits;
    S.gpuDevice = await adapter.requestDevice({
      requiredLimits: {
        maxStorageBufferBindingSize: Math.min(lim.maxStorageBufferBindingSize, 256 * 1024 * 1024),
        maxBufferSize: Math.min(lim.maxBufferSize, 256 * 1024 * 1024),
      }
    });
    S.gpuDevice.lost.then(info => {
      console.warn('WebGPU device lost:', info.message);
      S.gpuAvailable = false; S.gpuDevice = null;
    });
    console.info('WebGPU: device ready');
    return true;
  } catch (e) { console.warn('WebGPU init failed:', e); return false; }
}

export function gpuUploadHeightmaps() {
  if (!S.gpuDevice || !S.hmPixelData) return;
  const maxBuf = S.gpuDevice.limits.maxStorageBufferBindingSize;
  const hmSize = S.hmPixelW * S.hmPixelH;
  if (hmSize * 4 > maxBuf) { console.warn('Heightmap too large for GPU buffer'); return; }
  const hmF = new Float32Array(hmSize);
  for (let i = 0; i < hmSize; i++) hmF[i] = S.hmPixelData[i * 4] / 255;
  S.gpuHmBuf = S.gpuDevice.createBuffer({ size: hmF.byteLength, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST });
  S.gpuDevice.queue.writeBuffer(S.gpuHmBuf, 0, hmF);
  if (S.hm2PixelData && S.hm2PixelW > 0) {
    const h2Size = S.hm2PixelW * S.hm2PixelH;
    const h2F = new Float32Array(h2Size);
    for (let i = 0; i < h2Size; i++) h2F[i] = S.hm2PixelData[i * 4] / 255;
    S.gpuHm2Buf = S.gpuDevice.createBuffer({ size: h2F.byteLength, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST });
    S.gpuDevice.queue.writeBuffer(S.gpuHm2Buf, 0, h2F);
  } else {
    S.gpuHm2Buf = S.gpuDevice.createBuffer({ size: 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST });
    S.gpuDevice.queue.writeBuffer(S.gpuHm2Buf, 0, new Float32Array([0]));
  }
  console.info(`GPU heightmaps uploaded: LR ${S.hmPixelW}\u00d7${S.hmPixelH}` +
    (S.hm2PixelData ? `, HM2 ${S.hm2PixelW}\u00d7${S.hm2PixelH}` : ''));
}

function _gpuEnsureLosPipeline() {
  if (S.gpuLosPipeline) return;
  const mod = S.gpuDevice.createShaderModule({ code: _LOS_SHADER_WGSL });
  S.gpuLosBGL = S.gpuDevice.createBindGroupLayout({ entries: [
    { binding: 0, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'read-only-storage' } },
    { binding: 1, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'read-only-storage' } },
    { binding: 2, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'uniform' } },
    { binding: 3, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'storage' } },
    { binding: 4, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'read-only-storage' } },
  ]});
  S.gpuLosPipeline = S.gpuDevice.createComputePipeline({
    layout: S.gpuDevice.createPipelineLayout({ bindGroupLayouts: [S.gpuLosBGL] }),
    compute: { module: mod, entryPoint: 'main' },
  });
}

function _sampleHeightAtWorld(wx, wz) {
  const M = S.M;
  if (!S.hmPixelData || !M || !M.heightmap) return 0;
  const we = M.heightmap.world_extent || [M.mapCoord0[0], M.mapCoord0[1], M.mapCoord1[0], M.mapCoord1[1]];
  const weW = we[2] - we[0], weH = we[3] - we[1];
  const hMin = M.heightmap.height_min_m || 0, hMax = M.heightmap.height_max_m || 1, hRange = (hMax - hMin) || 1;
  if (S.hm2PixelData && M.heightmapDetail) {
    const D = M.heightmapDetail;
    const x0 = Math.min(D.world_x0, D.world_x1), x1 = Math.max(D.world_x0, D.world_x1);
    const z0 = Math.min(D.world_z0, D.world_z1), z1 = Math.max(D.world_z0, D.world_z1);
    if (wx >= x0 && wx <= x1 && wz >= z0 && wz <= z1) {
      const u2 = (wx - x0) / ((x1 - x0) || 1), v2 = (wz - z0) / ((z1 - z0) || 1);
      const px2 = Math.min(S.hm2PixelW - 1, Math.max(0, Math.round(u2 * (S.hm2PixelW - 1))));
      const py2 = Math.min(S.hm2PixelH - 1, Math.max(0, Math.round(v2 * (S.hm2PixelH - 1))));
      return (D.height_min_m || 0) + (S.hm2PixelData[(py2 * S.hm2PixelW + px2) * 4] / 255) * ((D.height_max_m - D.height_min_m) || 1);
    }
  }
  const u = (wx - we[0]) / weW, v = (wz - we[1]) / weH;
  const px = Math.round(u * (S.hmPixelW - 1)), py = Math.round(v * (S.hmPixelH - 1));
  if (px < 0 || px >= S.hmPixelW || py < 0 || py >= S.hmPixelH) return hMin;
  return hMin + (S.hmPixelData[(py * S.hmPixelW + px) * 4] / 255) * hRange;
}

async function _gpuDoLoS(originScene) {
  const M = S.M;
  if (!S.gpuDevice || !S.gpuHmBuf || !M || !M.heightmap) return false;
  const t0 = performance.now();
  _gpuEnsureLosPipeline();

  const planeSize = Math.max(M.mapSize[0], M.mapSize[1]);
  const we = M.heightmap.world_extent || [M.mapCoord0[0], M.mapCoord0[1], M.mapCoord1[0], M.mapCoord1[1]];
  const weW = we[2] - we[0], weH = we[3] - we[1];
  const hMin = M.heightmap.height_min_m, hMax = M.heightmap.height_max_m;
  const hRange = (hMax - hMin) || 1;
  const origW = sceneToWorld(originScene);
  const originH = _sampleHeightAtWorld(origW.x, origW.z) + _LOS_EYE_HEIGHT_M;

  const D = M.heightmapDetail;
  const hasHm2 = (S.gpuHm2Buf && D && S.hm2PixelW > 0) ? 1 : 0;

  const hasOcc = S.gpuOccBuf && S.occGrid ? 1 : 0;
  const ab = new ArrayBuffer(128);
  const fv = new Float32Array(ab), uv = new Uint32Array(ab);
  fv[0] = origW.x; fv[1] = origW.z; fv[2] = originH; fv[3] = _LOS_RANGE;
  uv[4] = _GPU_LOS_RAYS; uv[5] = _GPU_LOS_STEPS; uv[6] = S.hmPixelW; uv[7] = S.hmPixelH;
  fv[8] = we[0]; fv[9] = we[1]; fv[10] = we[2]; fv[11] = we[3];
  fv[12] = hMin; fv[13] = hMax;
  fv[14] = hasHm2 ? D.world_x0 : 0; fv[15] = hasHm2 ? D.world_z0 : 0;
  fv[16] = hasHm2 ? D.world_x1 : 0; fv[17] = hasHm2 ? D.world_z1 : 0;
  fv[18] = hasHm2 ? (D.height_min_m || 0) : 0; fv[19] = hasHm2 ? (D.height_max_m || 0) : 0;
  uv[20] = hasHm2 ? S.hm2PixelW : 0; uv[21] = hasHm2 ? S.hm2PixelH : 0;
  uv[22] = hasHm2; uv[23] = hasOcc;
  fv[24] = S.occGridX0; fv[25] = S.occGridZ0; fv[26] = S.occGridCellSize; fv[27] = 0;
  uv[28] = S.occGridW; uv[29] = S.occGridH; uv[30] = 0; uv[31] = 0;

  const paramBuf = S.gpuDevice.createBuffer({ size: 128, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
  S.gpuDevice.queue.writeBuffer(paramBuf, 0, ab);

  const total = _GPU_LOS_RAYS * _GPU_LOS_STEPS;
  const outBytes = total * 16;
  const outBuf = S.gpuDevice.createBuffer({ size: outBytes, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC });
  const stageBuf = S.gpuDevice.createBuffer({ size: outBytes, usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST });

  let occBufForBind = S.gpuOccBuf;
  if (!occBufForBind) {
    occBufForBind = S.gpuDevice.createBuffer({ size: 4, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST });
    S.gpuDevice.queue.writeBuffer(occBufForBind, 0, new Float32Array([0]));
  }
  const bg = S.gpuDevice.createBindGroup({ layout: S.gpuLosBGL, entries: [
    { binding: 0, resource: { buffer: S.gpuHmBuf } },
    { binding: 1, resource: { buffer: S.gpuHm2Buf } },
    { binding: 2, resource: { buffer: paramBuf } },
    { binding: 3, resource: { buffer: outBuf } },
    { binding: 4, resource: { buffer: occBufForBind } },
  ]});

  const enc = S.gpuDevice.createCommandEncoder();
  const pass = enc.beginComputePass();
  pass.setPipeline(S.gpuLosPipeline);
  pass.setBindGroup(0, bg);
  pass.dispatchWorkgroups(Math.ceil(_GPU_LOS_RAYS / 64));
  pass.end();
  enc.copyBufferToBuffer(outBuf, 0, stageBuf, 0, outBytes);
  S.gpuDevice.queue.submit([enc.finish()]);

  await stageBuf.mapAsync(GPUMapMode.READ);
  const data = new Float32Array(stageBuf.getMappedRange().slice(0));
  stageBuf.unmap();
  paramBuf.destroy(); outBuf.destroy(); stageBuf.destroy();
  if (occBufForBind !== S.gpuOccBuf) occBufForBind.destroy();

  function w2s(wx, wz, hM) {
    const sp = worldToSceneXZ(M, wx, wz, planeSize);
    let sy = 0;
    if (S.displacedMeshes.length > 0) {
      const cs = S.displacedMeshes[0].material.displacementScale || 1;
      sy = ((hM - hMin) / hRange) * cs;
    }
    return new THREE.Vector3(sp.x, sy, sp.z);
  }

  const NRAYS = _GPU_LOS_RAYS, NSTEPS = _GPU_LOS_STEPS;
  const gridPos = new Float32Array(NRAYS * NSTEPS * 3);
  const gridVis = new Uint8Array(NRAYS * NSTEPS);
  const eyePtW = w2s(origW.x, origW.z, originH);
  for (let r = 0; r < NRAYS; r++) {
    for (let s = 0; s < NSTEPS; s++) {
      const i = r * NSTEPS + s;
      const b = i * 4;
      const pt = w2s(data[b], data[b + 1], data[b + 2]);
      pt.y += 0.5;
      gridPos[i * 3] = pt.x; gridPos[i * 3 + 1] = pt.y; gridPos[i * 3 + 2] = pt.z;
      gridVis[i] = data[b + 3] > 0.5 ? 1 : 0;
    }
  }

  buildLosMesh({
    THREE,
    group: S.losGroup,
    gridPos,
    gridVis,
    nRays: NRAYS,
    nSteps: NSTEPS,
    eyePt: eyePtW,
    planeSize,
    waterY: S.waterMesh ? S.waterMesh.position.y : null,
  });

  // Range circle
  const cPts = [];
  for (let i = 0; i <= 64; i++) {
    const a = (i / 64) * Math.PI * 2;
    const cwx = origW.x + Math.cos(a) * _LOS_RANGE;
    const cwz = origW.z + Math.sin(a) * _LOS_RANGE;
    const cp = w2s(cwx, cwz, _sampleHeightAtWorld(cwx, cwz));
    cp.y += 1; cPts.push(cp);
  }
  const cLine = new THREE.Line(
    new THREE.BufferGeometry().setFromPoints(cPts),
    new THREE.LineBasicMaterial({ color: 0xffff88, depthTest: false, transparent: true, opacity: 0.5 })
  );
  cLine.renderOrder = 5000;
  S.losGroup.add(cLine);

  S.scene.add(S.losGroup);
  if (typeof S.needsRender !== 'undefined') S.needsRender = true;
  return true;
}

export function clearLoS() {
  if (S.losGroup) {
    S.scene.remove(S.losGroup);
    S.losGroup.traverse(c => { if (c.geometry) c.geometry.dispose(); if (c.material) c.material.dispose(); });
    S.losGroup = null;
  }
  S.losOrigin = null;
}

// CPU fallback LoS: 360 rays × 375 steps at 4 m/step (1500 m range)
function _cpuDoLoS(originScene) {
  const M = S.M;
  const t0 = performance.now();
  const planeSize = Math.max(M.mapSize[0], M.mapSize[1]);
  const hMin = M.heightmap.height_min_m, hMax = M.heightmap.height_max_m;
  const hRange = (hMax - hMin) || 1;
  const origW = sceneToWorld(originScene);
  const originH = _sampleHeightAtWorld(origW.x, origW.z) + _LOS_EYE_HEIGHT_M;

  const CPU_RAYS = 360;
  const CPU_STEP_M = 4.0;
  const CPU_STEPS = Math.max(1, Math.floor(_LOS_RANGE / CPU_STEP_M));
  const hasOcc = !!(S.occGrid && S.occGridW > 0 && S.occGridH > 0 && S.occGridCellSize > 0);

  function w2s(wx, wz, hM) {
    const sp = worldToSceneXZ(M, wx, wz, planeSize);
    let sy = 0;
    if (S.displacedMeshes.length > 0) {
      const cs = S.displacedMeshes[0].material.displacementScale || 1;
      sy = ((hM - hMin) / hRange) * cs;
    }
    return new THREE.Vector3(sp.x, sy, sp.z);
  }

  const NRAYS = CPU_RAYS, NSTEPS = CPU_STEPS;
  const gridPos = new Float32Array(NRAYS * NSTEPS * 3);
  const gridVis = new Uint8Array(NRAYS * NSTEPS);
  const eyePtW = w2s(origW.x, origW.z, originH);

  for (let r = 0; r < NRAYS; r++) {
    const angle = (r / NRAYS) * Math.PI * 2;
    const dx = Math.cos(angle), dz = Math.sin(angle);
    let maxSlope = -1e10;
    let objBlocked = false;
    for (let s = 0; s < NSTEPS; s++) {
      const dist = CPU_STEP_M * (s + 1);
      const wx = origW.x + dx * dist;
      const wz = origW.z + dz * dist;
      const terrH = _sampleHeightAtWorld(wx, wz);
      const slope = (terrH - originH) / dist;
      const terrainMaxSlopeBefore = maxSlope;
      let vis = slope >= terrainMaxSlopeBefore ? 1 : 0;
      if (objBlocked) vis = 0;

      if (hasOcc && vis > 0) {
        const prevDist = s > 0 ? (CPU_STEP_M * s) : 0;
        const gap = Math.max(0.001, dist - prevDist);
        const nSamples = Math.min(8, Math.max(1, Math.ceil(gap / Math.max(0.5, S.occGridCellSize))));
        for (let c = 0; c < nSamples; c++) {
          const t = prevDist + gap * ((c + 0.5) / nSamples);
          const owx = origW.x + dx * t;
          const owz = origW.z + dz * t;
          const gx = Math.floor((owx - S.occGridX0) / S.occGridCellSize);
          const gz = Math.floor((owz - S.occGridZ0) / S.occGridCellSize);
          if (gx < 0 || gx >= S.occGridW || gz < 0 || gz >= S.occGridH) continue;
          const objTop = S.occGrid[gz * S.occGridW + gx];
          if (objTop <= 0) continue;
          const eyeHAtT = originH + terrainMaxSlopeBefore * t;
          if (objTop > eyeHAtT) {
            vis = 0;
            objBlocked = true;
            break;
          }
        }
      }

      if (slope > maxSlope) maxSlope = slope;
      const i = r * NSTEPS + s;
      const pt = w2s(wx, wz, terrH + 0.5);
      gridPos[i * 3] = pt.x; gridPos[i * 3 + 1] = pt.y; gridPos[i * 3 + 2] = pt.z;
      gridVis[i] = vis;
    }
  }

  buildLosMesh({ THREE, group: S.losGroup, gridPos, gridVis, nRays: NRAYS, nSteps: NSTEPS, eyePt: eyePtW, planeSize, waterY: S.waterMesh ? S.waterMesh.position.y : null });

  // Range circle
  const cPts = [];
  for (let i = 0; i <= 64; i++) {
    const a = (i / 64) * Math.PI * 2;
    const cwx = origW.x + Math.cos(a) * _LOS_RANGE;
    const cwz = origW.z + Math.sin(a) * _LOS_RANGE;
    const cp = w2s(cwx, cwz, _sampleHeightAtWorld(cwx, cwz));
    cp.y += 1; cPts.push(cp);
  }
  const cLine = new THREE.Line(
    new THREE.BufferGeometry().setFromPoints(cPts),
    new THREE.LineBasicMaterial({ color: 0xffff88, depthTest: false, transparent: true, opacity: 0.5 })
  );
  cLine.renderOrder = 5000;
  S.losGroup.add(cLine);

  S.scene.add(S.losGroup);
  if (typeof S.needsRender !== 'undefined') S.needsRender = true;
}

export async function computeLoS(originScene) {
  if (S.losComputing) return;
  S.losComputing = true;
  try {
    clearLoS();
    S.losOrigin = originScene.clone();
    const M = S.M;
    if (!M || !S.hmPixelData || !M.heightmap) return;

    S.losGroup = new THREE.Group();
    S.losGroup.renderOrder = 999;

    if (S.gpuAvailable && S.gpuHmBuf) {
      try {
        if (await _gpuDoLoS(originScene)) return;
      } catch (e) {
        console.error('GPU LoS failed, falling back to CPU:', e);
      }
      // GPU path failed — clean up and fall through to CPU
      if (S.losGroup) {
        S.scene.remove(S.losGroup);
        S.losGroup.traverse(c => { if (c.geometry) c.geometry.dispose(); if (c.material) c.material.dispose(); });
        S.losGroup = new THREE.Group();
        S.losGroup.renderOrder = 999;
      }
    }

    // CPU fallback
    const statusEl = document.getElementById('status');
    if (statusEl) statusEl.textContent = 'Computing LoS (CPU)...';
    try {
      _cpuDoLoS(originScene);
    } catch (e) {
      console.error('CPU LoS failed:', e);
      if (statusEl) statusEl.textContent = 'LoS compute failed: ' + (e && e.message || e);
    } finally {
      if (statusEl) statusEl.textContent = '';
    }
  } finally {
    S.losComputing = false;
  }
}

export function updateLoSVisuals() {
  if (!S.losOrigin || !S.losGroup) return;
  const curScale = S.displacedMeshes.length > 0 ? (S.displacedMeshes[0].material.displacementScale || 1) : 1;
  if (S.losLastScale !== null && Math.abs(curScale - S.losLastScale) > 0.001) {
    computeLoS(S.losOrigin);
    if (typeof S.needsRender !== 'undefined') S.needsRender = true;
  }
  S.losLastScale = curScale;
}
