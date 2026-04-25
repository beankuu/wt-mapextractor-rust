import * as THREE from 'three';
import { S } from './state.js';
import { DATA, loadTexture, progress, buildBoxWireframeVerts } from './helpers.js';
import { applySunSettings } from './scene-setup.js';
import { initWebGPU, gpuUploadHeightmaps } from './webgpu-los.js';
import { getRendInstStyle, isStructureOccluder } from './tools/rendinst-style.js';
import { getMapFrame, sceneToWorldXZ, worldToSceneXZ } from './coords.js';
import { playTerrainIntro, playMeshIntro } from './intro-anim.js';

// Lazy loading cache for TIER 3+ textures
const lazyTextureCache = new Map();
let detailLoadingPromise = null;

let _lastClampTs = 0;

function sampleSceneFloorY(sx, sz) {
  const M = S.M;
  if (!M || !M.heightmap || !S.hmPixelData || S.hmPixelW <= 0 || S.hmPixelH <= 0) return null;

  const world = sceneToWorldXZ(M, sx, sz);
  const hMin = M.heightmap.height_min_m || 0;
  const hMax = M.heightmap.height_max_m || 1;
  const hRange = (hMax - hMin) || 1;

  let heightM = hMin;
  const detail = M.heightmapDetail;
  if (detail && S.hm2PixelData && S.hm2PixelW > 0 && S.hm2PixelH > 0) {
    const x0 = Math.min(detail.world_x0, detail.world_x1);
    const x1 = Math.max(detail.world_x0, detail.world_x1);
    const z0 = Math.min(detail.world_z0, detail.world_z1);
    const z1 = Math.max(detail.world_z0, detail.world_z1);
    if (world.x >= x0 && world.x <= x1 && world.z >= z0 && world.z <= z1) {
      const u2 = (world.x - x0) / Math.max(x1 - x0, 0.001);
      const v2 = (world.z - z0) / Math.max(z1 - z0, 0.001);
      const px2 = Math.min(S.hm2PixelW - 1, Math.max(0, Math.round(u2 * (S.hm2PixelW - 1))));
      const py2 = Math.min(S.hm2PixelH - 1, Math.max(0, Math.round(v2 * (S.hm2PixelH - 1))));
      const hm2Norm = S.hm2PixelData[(py2 * S.hm2PixelW + px2) * 4] / 255;
      heightM = (detail.height_min_m || 0) + hm2Norm * (((detail.height_max_m || 0) - (detail.height_min_m || 0)) || 1);
    } else {
      const we = Array.isArray(M.heightmap.world_extent) && M.heightmap.world_extent.length === 4
        ? M.heightmap.world_extent
        : null;
      const wx0 = we ? Math.min(we[0], we[2]) : M.mapCoord0[0];
      const wx1 = we ? Math.max(we[0], we[2]) : M.mapCoord1[0];
      const wz0 = we ? Math.min(we[1], we[3]) : M.mapCoord0[1];
      const wz1 = we ? Math.max(we[1], we[3]) : M.mapCoord1[1];
      const u = (world.x - wx0) / Math.max(wx1 - wx0, 0.001);
      const v = (world.z - wz0) / Math.max(wz1 - wz0, 0.001);
      const px = Math.min(S.hmPixelW - 1, Math.max(0, Math.round(u * (S.hmPixelW - 1))));
      const py = Math.min(S.hmPixelH - 1, Math.max(0, Math.round(v * (S.hmPixelH - 1))));
      const hmNorm = S.hmPixelData[(py * S.hmPixelW + px) * 4] / 255;
      heightM = hMin + hmNorm * hRange;
    }
  } else {
    const we = Array.isArray(M.heightmap.world_extent) && M.heightmap.world_extent.length === 4
      ? M.heightmap.world_extent
      : null;
    const wx0 = we ? Math.min(we[0], we[2]) : M.mapCoord0[0];
    const wx1 = we ? Math.max(we[0], we[2]) : M.mapCoord1[0];
    const wz0 = we ? Math.min(we[1], we[3]) : M.mapCoord0[1];
    const wz1 = we ? Math.max(we[1], we[3]) : M.mapCoord1[1];
    const u = (world.x - wx0) / Math.max(wx1 - wx0, 0.001);
    const v = (world.z - wz0) / Math.max(wz1 - wz0, 0.001);
    const px = Math.min(S.hmPixelW - 1, Math.max(0, Math.round(u * (S.hmPixelW - 1))));
    const py = Math.min(S.hmPixelH - 1, Math.max(0, Math.round(v * (S.hmPixelH - 1))));
    const hmNorm = S.hmPixelData[(py * S.hmPixelW + px) * 4] / 255;
    heightM = hMin + hmNorm * hRange;
  }

  const curScale = S.displacedMeshes.length > 0
    ? (S.displacedMeshes[0].material.displacementScale || 1)
    : 1;
  return ((heightM - hMin) / hRange) * curScale;
}

/**
 * Clamp the camera above the terrain surface after any controls event.
 * Directly sets camera.position.y and shifts controls.target.y by the same
 * delta so the orbit geometry is preserved. Also resets minDistance when
 * above the floor to allow free zooming.
 */
export function clampCameraAboveTerrain() {
  const now = performance.now();
  if (now - _lastClampTs < 80) return;
  _lastClampTs = now;

  const floorY = sampleSceneFloorY(S.camera.position.x, S.camera.position.z);
  if (floorY == null) {
    S.controls.minDistance = 5;
    return;
  }
  const margin = 20; // keep at least 20 m above terrain
  const clearance = S.camera.position.y - floorY;
  if (clearance < margin) {
    const delta = margin - clearance;
    S.camera.position.y += delta;
    S.controls.target.y += delta;
    S.controls.update();
  } else {
    // Comfortably above terrain — allow free zooming
    S.controls.minDistance = 5;
  }
}

/**
 * Apply UV repeat/offset so a texture covering `worldExtent` is positioned
 * correctly on a terrain plane that spans the full mapCoord range.
 *
 * worldExtent: [wx0, wz0, wx1, wz1]  (the texture's actual coverage)
 * mc0, mc1:   mapCoord0, mapCoord1   (the plane's full coordinate range)
 *
 * Three.js samples: texcoord = uv * repeat + offset
 * We solve so texcoord=0 at uv=(we0-mc0)/mcW and texcoord=1 at uv=(we1-mc0)/mcW.
 */
function applyWorldExtentUV(tex, worldExtent, mc0, mc1) {
  if (!worldExtent) return;
  const x0 = Math.min(worldExtent[0], worldExtent[2]);
  const z0 = Math.min(worldExtent[1], worldExtent[3]);
  const x1 = Math.max(worldExtent[0], worldExtent[2]);
  const z1 = Math.max(worldExtent[1], worldExtent[3]);
  const mcW = mc1[0] - mc0[0];
  const mcH = mc1[1] - mc0[1];
  const weW = x1 - x0;
  const weH = z1 - z0;
  if (weW <= 0 || weH <= 0 || mcW <= 0 || mcH <= 0) return;
  // If world extent matches mapCoord, no correction needed
  if (Math.abs(weW - mcW) < 1 && Math.abs(weH - mcH) < 1) return;
  const repX = mcW / weW;
  const repY = mcH / weH;
  const offX = -(x0 - mc0[0]) / weW;
  const offY = -(z0 - mc0[1]) / weH;
  tex.repeat.set(repX, repY);
  tex.offset.set(offX, offY);
  tex.wrapS = tex.wrapT = THREE.ClampToEdgeWrapping;
}

// Load texture with caching for lazy loading
function getCachedTexturePromise(key, urlPath) {
  if (lazyTextureCache.has(key)) {
    const cached = lazyTextureCache.get(key);
    return cached.promise || Promise.resolve(cached.texture);
  }
  const promise = loadTexture(urlPath)
    .then(tex => {
      lazyTextureCache.set(key, { texture: tex, loaded: true });
      return tex;
    })
    .catch(err => {
      console.warn(`Failed to load ${key}:`, err);
      return null;
    });
  lazyTextureCache.set(key, { promise });
  return promise;
}

/**
 * Lazy load detail heightmap in background (TIER 3)
 * This loads after initial terrain is visible
 */
async function lazyLoadDetailHeightmap(params) {
  try {
    const { manifest: M, planeSize, defaultHeight, detailGeo, detailScale, detailBias, 
             detPosX, detPosZ, normalMapTex, baseHeightmapTex } = params;
    const D = M.heightmapDetail;
    
    // Load detail textures (may take 1-5 seconds for large textures)
    const detailPaint = M.terrainPaint && M.terrainPaint.detail;
    // Prefer HM2 patch color first; it usually matches authored HM2 region
    // better than a world-wide remapped terrain_paint.
    const detailColorSrc = (detailPaint && detailPaint.file)
      ? detailPaint.file
      : (M.terrainPaint
          ? M.terrainPaint.file
          : (M.colormap ? M.colormap.file : null));
    const usesDetailPatchColor = !!(detailPaint && detailColorSrc === detailPaint.file);
    
    const [detailTexPng, detailColorRaw] = await Promise.all([
      getCachedTexturePromise('detailHeightmapPng', `${DATA}/${D.file}`),
      detailColorSrc ? getCachedTexturePromise('detailColor', `${DATA}/${detailColorSrc}`) : null,
    ]);
    const detailTex = detailTexPng;
    
    if (!detailTex) {
      console.warn('Detail heightmap failed to load');
      return;
    }
    
    detailTex.minFilter = THREE.LinearFilter;
    detailTex.magFilter = THREE.LinearFilter;

    // Compute UV parameters
    const dW = detailGeo.parameters.width;
    const dH = detailGeo.parameters.height;
    const uvRepeatX = dW / planeSize;
    const uvRepeatY = dH / planeSize;
    const uvOffsetX = (detPosX - dW / 2 + planeSize / 2) / planeSize;
    const uvOffsetY = (planeSize / 2 - detPosZ - dH / 2) / planeSize;

    // Setup color texture
    let detailColorTex = null;
    if (usesDetailPatchColor) {
      detailColorTex = detailColorRaw;
      if (detailColorTex) {
        detailColorTex.colorSpace = THREE.SRGBColorSpace;
        detailColorTex.minFilter = THREE.LinearMipmapLinearFilter;
        detailColorTex.anisotropy = S.renderer.capabilities.getMaxAnisotropy();
      }
    } else if (detailColorRaw) {
      detailColorTex = detailColorRaw;
      detailColorTex.colorSpace = THREE.SRGBColorSpace;
      detailColorTex.minFilter = THREE.LinearMipmapLinearFilter;
      detailColorTex.anisotropy = S.renderer.capabilities.getMaxAnisotropy();
      detailColorTex.offset.set(uvOffsetX, uvOffsetY);
      detailColorTex.repeat.set(uvRepeatX, uvRepeatY);
    } else {
      detailColorTex = detailTex.clone();
      detailColorTex.colorSpace = THREE.SRGBColorSpace;
    }

    // HM2 visual displacement is intentionally disabled here.
    // We reuse the main LR displacement in the detail region to keep
    // terrain, objects and tool raycasts vertically consistent.
    S.detailMesh = null;

    // Detail texture mesh
    // detail.hasAlpha is false when the Rust pipeline outputs a JPEG (no alpha channel)
    const detailHasAlpha = !!(detailPaint && detailPaint.file && detailPaint.hasAlpha !== false);
    const detailTexMatOpts = {
      map: detailColorTex,
      roughness: 0.85, metalness: 0, side: THREE.DoubleSide,
      transparent: detailHasAlpha, alphaTest: detailHasAlpha ? 0.01 : 0,
      displacementScale: defaultHeight,
      displacementBias: 0,
      polygonOffset: true, polygonOffsetFactor: -3,
    };

    // Detail mesh displacement: prefer the native-resolution HM2 heightmap
    // (`heightmap_detail.png`, typically 2 m/px) over the cropped base
    // heightmap (32 m/px on 64 km maps like avg_tunisia_desert). The HM2
    // float buffer was already overlaid into the base LR map upstream, so
    // hover/raycast values stay numerically consistent with the visual
    // mesh inside the HM2 region.
    if (detailTex) {
      detailTexMatOpts.displacementMap = detailTex;
      detailTexMatOpts.displacementScale = detailScale;
      detailTexMatOpts.displacementBias = detailBias;
    } else if (baseHeightmapTex) {
      const detDisp = baseHeightmapTex.clone();
      detDisp.minFilter = THREE.LinearFilter;
      detDisp.magFilter = THREE.LinearFilter;
      detDisp.offset.set(uvOffsetX, uvOffsetY);
      detDisp.repeat.set(uvRepeatX, uvRepeatY);
      detDisp.wrapS = detDisp.wrapT = THREE.ClampToEdgeWrapping;
      detailTexMatOpts.displacementMap = detDisp;
    }
    
    // Use globally-aligned normal map for HM2 detail color continuity.
    // detail normal maps can introduce noticeable shading mismatch against base terrain.
    if (normalMapTex) {
      const detNormalTex = normalMapTex.clone();
      detNormalTex.offset.set(uvOffsetX, uvOffsetY);
      detNormalTex.repeat.set(uvRepeatX, uvRepeatY);
      detailTexMatOpts.normalMap = detNormalTex;
      detailTexMatOpts.normalScale = new THREE.Vector2(0.9, 0.9);
    }
    
    S.detailTexMesh = new THREE.Mesh(detailGeo, new THREE.MeshStandardMaterial(detailTexMatOpts));
    S.detailTexMesh.rotation.x = -Math.PI / 2;
    S.detailTexMesh.position.x = detPosX;
    S.detailTexMesh.position.z = detPosZ;
    // Tag mesh so setHeightScale uses HM2 scale/bias, not LR full-range.
    if (params.hm2Meta) {
      S.detailTexMesh.userData.hm2 = params.hm2Meta;
    }
    // Prevent HM2 patch from visually popping early. Keep it hidden until
    // the main terrain intro is complete.
    if (!S.terrainIntroDone) {
      S.detailTexMesh.visible = false;
      const _showDetailAfterIntro = () => {
        if (S.detailTexMesh) {
          S.detailTexMesh.visible = true;
          S.needsRender = true;
        }
      };
      window.addEventListener('terrain-intro-complete', _showDetailAfterIntro, { once: true });
    }
    S.scene.add(S.detailTexMesh);
    S.displacedMeshes.push(S.detailTexMesh);

    S.detailRaycastMesh = null;
    S.detailRaycastNorms = null;
    S.hm2PixelW = 0;
    S.hm2PixelH = 0;
    S.hm2PixelData = null;
    
    console.log('Detail heightmap loaded successfully');
    S.needsRender = true;
  } catch (err) {
    console.error('Failed to lazy-load detail heightmap:', err);
  }
}

export async function loadTerrain() {
  progress(5, 'Loading manifest...');
  const res = await fetch(`${DATA}/manifest.json`, { cache: 'no-store' });
  S.M = await res.json();
  const M = S.M;
  applySunSettings(M.sun || { azimuth: 30, elevation: 50, strength: 0.5 });

  // WebGPU compute initialization
  S.gpuAvailable = await initWebGPU();
  { const b = document.getElementById('gpu-badge'); if (b) { b.textContent = S.gpuAvailable ? 'WebGPU' : 'CPU'; b.classList.toggle('active', S.gpuAvailable); } }

  const frame = getMapFrame(M);
  const mapW = frame.mcW;
  const mapH = frame.mcH;
  const planeSize = frame.planeSize;
  const frameCoord0 = frame.mc0;
  const frameCoord1 = frame.mc1;
  const heightMin = (M.heightmap && M.heightmap.height_min_m != null) ? M.heightmap.height_min_m : 0;
  const heightMax = (M.heightmap && M.heightmap.height_max_m != null) ? M.heightmap.height_max_m : planeSize * 0.01;
  const heightRange = heightMax - heightMin;
  S.maxHeightScale = heightRange || planeSize * 0.01;

  const segments = S.gpuAvailable ? 1024 : 512;
  const terrainGeo = new THREE.PlaneGeometry(planeSize, planeSize, segments, segments);

  // Parallel-load the primary textures (heightmap, normal map, colormap, terrain paint, tile grid)
  progress(10, 'Loading terrain textures...');
  const [heightmapTex, normalMapTexRaw, colormapTex, paintTex, tileGridTex] = await Promise.all([
    M.heightmap      ? loadTexture(`${DATA}/${M.heightmap.file}`)      : null,
    M.normalmap      ? loadTexture(`${DATA}/${M.normalmap.file}`)      : null,
    M.colormap       ? loadTexture(`${DATA}/${M.colormap.file}`)       : null,
    M.terrainPaint   ? loadTexture(`${DATA}/${M.terrainPaint.file}`)
                     : (M.tileGrid ? loadTexture(`${DATA}/${M.tileGrid.file}`) : null),
    M.tileGrid       ? loadTexture(`${DATA}/${M.tileGrid.file}`)       : null,
  ]);
  if (heightmapTex) {
    heightmapTex.minFilter = THREE.LinearFilter;
    heightmapTex.magFilter = THREE.LinearFilter;
    applyWorldExtentUV(heightmapTex, M.heightmap?.world_extent, frameCoord0, frameCoord1);
  }
  let normalMapTex = null;
  if (normalMapTexRaw) {
    normalMapTex = normalMapTexRaw;
    normalMapTex.minFilter = THREE.LinearMipmapLinearFilter;
    normalMapTex.magFilter = THREE.LinearFilter;
    normalMapTex.anisotropy = S.renderer.capabilities.getMaxAnisotropy();
    applyWorldExtentUV(normalMapTex, M.heightmap?.world_extent, frameCoord0, frameCoord1);
  }

  const defaultHeightPct = 100;
  const defaultHeight = (defaultHeightPct / 100) * S.maxHeightScale;

  // -- Overview colormap terrain --
  if (M.colormap && colormapTex) {
    progress(20, 'Preparing overview map...');
    const tex = colormapTex;
    tex.colorSpace = THREE.SRGBColorSpace;
    tex.minFilter = THREE.LinearMipmapLinearFilter;
    tex.magFilter = THREE.LinearFilter;
    tex.anisotropy = S.renderer.capabilities.getMaxAnisotropy();

    applyWorldExtentUV(tex, M.heightmap?.world_extent, frameCoord0, frameCoord1);

    const matOpts = {
      map: tex, roughness: 0.9, metalness: 0, side: THREE.DoubleSide,
      transparent: true, opacity: 0.8, depthWrite: false,
      polygonOffset: true, polygonOffsetFactor: -1,
    };
    if (normalMapTex) {
      const nmClone = normalMapTex.clone();
      nmClone.repeat.copy(tex.repeat);
      nmClone.offset.copy(tex.offset);
      nmClone.wrapS = nmClone.wrapT = tex.wrapS;
      matOpts.normalMap = nmClone;
      matOpts.normalScale = new THREE.Vector2(1.0, 1.0);
    }
    if (heightmapTex) {
      matOpts.displacementMap = heightmapTex;
      matOpts.displacementScale = defaultHeight;
    }
    const mat = new THREE.MeshStandardMaterial(matOpts);
    S.terrainMesh = new THREE.Mesh(terrainGeo, mat);
    S.terrainMesh.rotation.x = -Math.PI / 2;
    S.terrainMesh.visible = false;
    S.scene.add(S.terrainMesh);
    S.displacedMeshes.push(S.terrainMesh);
    // Keep overview mesh available for fallback paths, but do not expose it in viewer controls.
    document.getElementById('row-colormap').style.display = 'none';
  }

  // -- Terrain paint --
  const hasPaintedTerrain = !!M.terrainPaint;
  if (paintTex) {
    progress(40, 'Preparing terrain paint...');
    const tex = paintTex;
    tex.colorSpace = THREE.SRGBColorSpace;
    tex.minFilter = THREE.LinearMipmapLinearFilter;
    tex.anisotropy = S.renderer.capabilities.getMaxAnisotropy();

    const tileWorldExtent = M.tileGrid && M.tileGrid.world_extent;
    applyWorldExtentUV(tex, tileWorldExtent || M.heightmap?.world_extent, frameCoord0, frameCoord1);

    const matOpts = {
      map: tex, roughness: 0.85, metalness: 0, side: THREE.DoubleSide,
      transparent: hasPaintedTerrain,
    };
    if (normalMapTex) {
      const nm = normalMapTex.clone();
      matOpts.normalMap = nm;
      matOpts.normalScale = new THREE.Vector2(1.0, 1.0);
    }
    if (heightmapTex) {
      matOpts.displacementMap = heightmapTex;
      matOpts.displacementScale = defaultHeight;
    }
    S.tileGridMesh = new THREE.Mesh(terrainGeo, new THREE.MeshStandardMaterial(matOpts));
    S.tileGridMesh.rotation.x = -Math.PI / 2;
    S.scene.add(S.tileGridMesh);
    S.displacedMeshes.push(S.tileGridMesh);
    document.getElementById('row-tilegrid').style.display = 'flex';
  }

  // -- Splatmap layer --
  if (M.tileGrid && M.tileGrid.file && tileGridTex) {
    const sgTex = tileGridTex;
    sgTex.colorSpace = THREE.SRGBColorSpace;
    sgTex.minFilter = THREE.LinearMipmapLinearFilter;
    sgTex.anisotropy = S.renderer.capabilities.getMaxAnisotropy();
    applyWorldExtentUV(sgTex, M.tileGrid?.world_extent, frameCoord0, frameCoord1);
    const sgMatOpts = {
      map: sgTex, roughness: 0.85, metalness: 0, side: THREE.DoubleSide,
      transparent: true, opacity: 1.0, depthWrite: false,
      polygonOffset: true, polygonOffsetFactor: -3,
    };
    if (heightmapTex) {
      sgMatOpts.displacementMap = heightmapTex;
      sgMatOpts.displacementScale = defaultHeight;
    }
    S.splatmapMesh = new THREE.Mesh(terrainGeo, new THREE.MeshStandardMaterial(sgMatOpts));
    S.splatmapMesh.rotation.x = -Math.PI / 2;
    S.splatmapMesh.visible = false;
    S.scene.add(S.splatmapMesh);
    S.displacedMeshes.push(S.splatmapMesh);
    document.getElementById('row-splatmap').style.display = 'flex';
  }

  // -- Heightmap view layer --
  if (heightmapTex) {
    const hmViewTex = heightmapTex.clone();
    hmViewTex.colorSpace = THREE.SRGBColorSpace;
    const hmMatOpts = {
      map: hmViewTex, roughness: 0.9, metalness: 0, side: THREE.DoubleSide,
      transparent: true, opacity: 0.8, depthWrite: false,
      polygonOffset: true, polygonOffsetFactor: -2,
    };
    if (heightmapTex) {
      hmMatOpts.displacementMap = heightmapTex;
      hmMatOpts.displacementScale = defaultHeight;
    }
    S.heightmapMesh = new THREE.Mesh(terrainGeo, new THREE.MeshStandardMaterial(hmMatOpts));
    S.heightmapMesh.rotation.x = -Math.PI / 2;
    S.heightmapMesh.visible = false;
    S.scene.add(S.heightmapMesh);
    S.displacedMeshes.push(S.heightmapMesh);
    document.getElementById('row-heightmap').style.display = 'flex';
  }

  // -- HM2 detail heightmap (TIER 3: Lazy load in background) --
  if (M.heightmapDetail && M.heightmap) {
    // Store parameters for lazy loading, start loading in background
    const detailParams = {
      manifest: M,
      planeSize,
      defaultHeight,
      terrainGeo: null,
      detailGeo: null,
    };

    // Calculate geometry parameters now (needed later)
    const D = M.heightmapDetail;
    // Use mapCoord (not world_extent) — the terrain plane is sized/centred on mapCoord
    const lrXmin = frameCoord0[0], lrZmin = frameCoord0[1];
    const lrXmax = frameCoord1[0], lrZmax = frameCoord1[1];
    const lrW = lrXmax - lrXmin;
    const lrH = lrZmax - lrZmin;
    const scaleX = planeSize / lrW;
    const scaleZ = planeSize / lrH;

    const hx0 = Math.min(D.world_x0, D.world_x1);
    const hx1 = Math.max(D.world_x0, D.world_x1);
    const hz0 = Math.min(D.world_z0, D.world_z1);
    const hz1 = Math.max(D.world_z0, D.world_z1);

    const dW = (hx1 - hx0) * scaleX;
    const dH = (hz1 - hz0) * scaleZ;

    const detSegs = Math.min(
      S.gpuAvailable ? 768 : 320,
      Math.max(128, Math.floor(Math.max(D.width, D.height) / 4)),
    );
    detailParams.detailGeo = new THREE.PlaneGeometry(dW, dH, detSegs, detSegs);

    const lrMin = M.heightmap.height_min_m || 0;
    const lrMax = M.heightmap.height_max_m || 1;
    const lrRange = lrMax - lrMin || 1;
    const hm2Range = D.height_max_m - D.height_min_m;
    const detailScale = (hm2Range / lrRange) * defaultHeight;
    const detailBias = ((D.height_min_m - lrMin) / lrRange) * defaultHeight;

    const hm2CenterX = (hx0 + hx1) / 2;
    const hm2CenterZ = (hz0 + hz1) / 2;
    const detCenter = worldToSceneXZ(M, hm2CenterX, hm2CenterZ, planeSize);
    const detPosX = detCenter.x;
    const detPosZ = detCenter.z;

    detailParams.detailScale = detailScale;
    detailParams.detailBias = detailBias;
    detailParams.detPosX = detPosX;
    detailParams.detPosZ = detPosZ;
    detailParams.normalMapTex = normalMapTex;
    detailParams.baseHeightmapTex = heightmapTex;
    // Carry HM2/LR ranges so setHeightScale can scale the HM2-normalized
    // displacementMap correctly. Without this the detail mesh gets the
    // full LR scale applied to a 0..1 HM2 texture and pops into the sky.
    detailParams.hm2Meta = {
      lrRange,
      hm2Range,
      hm2MinOffset: D.height_min_m - lrMin,
    };

    // Start lazy loading in background (non-blocking)
    // Don't await this - let it load while user sees main terrain
    progress(50, 'Preparing terrain...');
    
    detailLoadingPromise = lazyLoadDetailHeightmap(detailParams);
    
    // Store for cleanup/reference if needed
    S.detailLoadingPromise = detailLoadingPromise;
    // Guard: these rows may not exist in all HTML builds
    const _elRowDetail = document.getElementById('row-detail');
    if (_elRowDetail) _elRowDetail.style.display = 'none';
    const _elRowDetailTex = document.getElementById('row-detail-tex');
    if (_elRowDetailTex) _elRowDetailTex.style.display = 'flex';
  }

  // Upload heightmap data to GPU for compute shaders
  if (S.gpuAvailable) gpuUploadHeightmaps();

  // -- Water --
  progress(60, 'Creating water...');
  {
    // Water mesh is exactly the map extent — no oversized plane spilling
    // beyond the terrain footprint.
    const waterGeo = new THREE.PlaneGeometry(planeSize, planeSize);
    const waterMat = new THREE.MeshBasicMaterial({
      color: 0x0d4f8b, transparent: true, opacity: 0.92,
      side: THREE.FrontSide, depthWrite: false,
    });
    S.waterMesh = new THREE.Mesh(waterGeo, waterMat);
    S.waterMesh.renderOrder = 1;  // render after terrain — eliminates z-fighting
    S.waterMesh.rotation.x = -Math.PI / 2;
    // Default: ocean ON. Enable whenever we have any plausible water level.
    S.waterLikelyOcean = true;
    if (M.heightmap && M.heightmap.height_min_m != null && M.heightmap.height_max_m != null) {
      const hMin = M.heightmap.height_min_m;
      const hMax = M.heightmap.height_max_m;
      const hRange = hMax - hMin;
      const wl = M.waterLevel;
      let wf = hRange > 0 && wl != null ? (wl - hMin) / hRange : 0;
      if (wf < 0) wf = 0;
      if (wf > 1) wf = 1;
      S.waterFraction = wf;
    } else {
      S.waterFraction = 0;
    }
    // Tiny offset (0.01 % of height range) lifts water above the terrain-minimum
    // displacement so ocean pixels never z-fight with the terrain floor.
    S.waterMesh.position.y = defaultHeight * S.waterFraction + defaultHeight * 0.0001;
    S.waterMesh.visible = S.waterLikelyOcean;
    S.scene.add(S.waterMesh);
  }

  // -- Tank zone wireframe --
  if (M.tankZone && M.tankZone.coord0 && M.tankZone.coord1) {
    const tz = M.tankZone;
    const p0 = worldToSceneXZ(M, tz.coord0[0], tz.coord0[1], planeSize);
    const p1 = worldToSceneXZ(M, tz.coord1[0], tz.coord1[1], planeSize);
    const x0 = Math.min(p0.x, p1.x);
    const x1 = Math.max(p0.x, p1.x);
    const z0 = Math.min(p0.z, p1.z);
    const z1 = Math.max(p0.z, p1.z);
    let yBotFrac = 0, yTopFrac = 1;
    if (M.heightmap && M.heightmap.height_min_m != null) {
      const _lrMin = M.heightmap.height_min_m;
      const _lrRange = (M.heightmap.height_max_m - _lrMin) || 1;
      yBotFrac = (0 - _lrMin) / _lrRange;
      yTopFrac = (3000 - _lrMin) / _lrRange;
    }
    const yBot = yBotFrac * defaultHeight;
    const yTop = yTopFrac * defaultHeight;
    const tzBuf = new THREE.BufferGeometry();
    tzBuf.setAttribute('position', new THREE.BufferAttribute(buildBoxWireframeVerts(x0, x1, yBot, yTop, z0, z1), 3));
    const tzWire = new THREE.LineSegments(tzBuf, new THREE.LineBasicMaterial({ color: 0xff8800, depthTest: false }));
    const tzSolid = new THREE.Mesh(
      new THREE.BoxGeometry(Math.abs(x1 - x0), yTop - yBot, Math.abs(z1 - z0)),
      new THREE.MeshBasicMaterial({ color: 0xff8800, transparent: true, opacity: 0.08, side: THREE.DoubleSide, depthWrite: false })
    );
    tzSolid.position.set((x0 + x1) / 2, (yBot + yTop) / 2, (z0 + z1) / 2);
    S.tankZoneMesh = new THREE.Group();
    S.tankZoneMesh.add(tzWire);
    S.tankZoneMesh.add(tzSolid);
    S.tankZoneMesh.renderOrder = 999;
    S.tankZoneMesh.userData = { x0, x1, z0, z1, yBotFrac, yTopFrac };
    // Keep hidden until intro animation reveals it.
    S.tankZoneMesh.visible = false;
    S.scene.add(S.tankZoneMesh);
  }

  // -- Pre-extract heightmap pixel data --
  if (heightmapTex) {
    const _hmImg = heightmapTex.image;
    S.hmPixelW = _hmImg.naturalWidth || _hmImg.width;
    S.hmPixelH = _hmImg.naturalHeight || _hmImg.height;
    const _hmCv = document.createElement('canvas');
    _hmCv.width = S.hmPixelW; _hmCv.height = S.hmPixelH;
    const _hmCtx = _hmCv.getContext('2d');
    _hmCtx.drawImage(_hmImg, 0, 0);
    S.hmPixelData = _hmCtx.getImageData(0, 0, S.hmPixelW, S.hmPixelH).data;
  }
  const hmPixelData = S.hmPixelData, hmPixelW = S.hmPixelW, hmPixelH = S.hmPixelH;

  // -- Render instances --
  if (M.rendinst && M.rendinst.file) {
    progress(65, 'Loading render instances...');
    try {
      const riRes = await fetch(`${DATA}/${M.rendinst.file}`);
      const riBuf = await riRes.arrayBuffer();
      const riCount = M.rendinst.instanceCount;
      const riPools = M.rendinst.pools || [];
      // Use mapCoord for positioning — the terrain plane is sized/centred on mapCoord
      const lrXmin = frameCoord0[0], lrZmin = frameCoord0[1];
      const lrXmax = frameCoord1[0], lrZmax = frameCoord1[1];
      const lrW = lrXmax - lrXmin;
      const lrH = lrZmax - lrZmin;
      const lrCX = (lrXmin + lrXmax) / 2;
      const lrCZ = (lrZmin + lrZmax) / 2;
      const sX = planeSize / lrW;
      const sZ = planeSize / lrH;

      const styleBuckets = {};
      const dv = new DataView(riBuf);
      // Stride 14 = pool_idx(u16) + wx(f32) + wy(f32) + wz(f32). Stride 10 = legacy (no Y).
      const riStride = (M.rendinst.stride === 14) ? 14 : 10;
      const _hMin = M.heightmap ? (M.heightmap.height_min_m || 0) : 0;
      const _hMax = M.heightmap ? (M.heightmap.height_max_m || 1) : 1;
      const _hRange = (_hMax - _hMin) || 1;

      // Occupancy grid init
      const _occHMin = _hMin;
      const _occHRange = _hRange;
      const _occHm2 = S.hm2PixelData && M.heightmapDetail ? M.heightmapDetail : null;
      const _occHm2X0 = _occHm2 ? Math.min(_occHm2.world_x0, _occHm2.world_x1) : 0;
      const _occHm2X1 = _occHm2 ? Math.max(_occHm2.world_x0, _occHm2.world_x1) : 0;
      const _occHm2Z0 = _occHm2 ? Math.min(_occHm2.world_z0, _occHm2.world_z1) : 0;
      const _occHm2Z1 = _occHm2 ? Math.max(_occHm2.world_z0, _occHm2.world_z1) : 0;
      const _occHm2Min = _occHm2 ? (_occHm2.height_min_m || 0) : 0;
      const _occHm2Max = _occHm2 ? (_occHm2.height_max_m || 0) : 0;
      const _occHm2Range = (_occHm2Max - _occHm2Min) || 1;
      const _OCC_H = { building: 16, infrastructure: 14, debris: 10, earthwork: 8, other: 8 };
      const _occSamples = [];
      let _occMinX = Infinity, _occMinZ = Infinity, _occMaxX = -Infinity, _occMaxZ = -Infinity;

      for (let i = 0; i < riCount; i++) {
        const off = i * riStride;
        const poolIdx = dv.getUint16(off, true);
        const wx = dv.getFloat32(off + 2, true);
        // stride-14: precise float Y from binary; stride-10: fall back to heightmap PNG
        let wy = null, wz;
        if (riStride === 14) {
          wy = dv.getFloat32(off + 6, true);
          wz = dv.getFloat32(off + 10, true);
        } else {
          wz = dv.getFloat32(off + 6, true);
        }
        const pool = (poolIdx < riPools.length) ? riPools[poolIdx] : null;
        const cat = pool ? pool.category : 'other';
        const styleKey = getRendInstStyle(pool);
        if (!styleBuckets[styleKey]) styleBuckets[styleKey] = [];
        const sx =  (wx - lrCX) * sX;
        const sz = -(wz - lrCZ) * sZ;
        let hNorm;
        if (wy !== null) {
          // Precise height from stored float — no PNG quantization error.
          // Some maps carry inconsistent RI Y; fall back to sampled terrain in that case.
          if (hmPixelData) {
            const u = (wx - lrXmin) / lrW;
            const v = (wz - lrZmin) / lrH;
            const px = Math.min(Math.max(Math.floor(u * hmPixelW), 0), hmPixelW - 1);
            const py = Math.min(Math.max(Math.floor(v * hmPixelH), 0), hmPixelH - 1);
            const hmNorm = hmPixelData[(py * hmPixelW + px) * 4] / 255;
            const hmY = _hMin + hmNorm * _hRange;
            if (Math.abs(wy - hmY) > 120) {
              wy = hmY;
              hNorm = hmNorm;
            } else {
              hNorm = Math.max(0, Math.min(1, (wy - _hMin) / _hRange));
            }
          } else {
            hNorm = Math.max(0, Math.min(1, (wy - _hMin) / _hRange));
          }
        } else if (hmPixelData) {
          const u = (wx - lrXmin) / lrW;
          const v = (wz - lrZmin) / lrH;
          const px = Math.min(Math.max(Math.floor(u * hmPixelW), 0), hmPixelW - 1);
          const py = Math.min(Math.max(Math.floor(v * hmPixelH), 0), hmPixelH - 1);
          hNorm = hmPixelData[(py * hmPixelW + px) * 4] / 255;
          wy = _hMin + hNorm * _hRange;  // synthetic for occupancy
        } else {
          hNorm = 0; wy = _hMin;
        }
        styleBuckets[styleKey].push(sx, sz, hNorm);

        if (!isStructureOccluder(pool, styleKey)) continue;
        const occH = _OCC_H[cat] || 6;
        // Use precise stored Y for the base terrain height; fall back to HM2 pixel lookup
        // only when HM2 pixel data is already available (lazy-loaded)
        let terrH = wy;
        if (_occHm2 && wx >= _occHm2X0 && wx <= _occHm2X1 && wz >= _occHm2Z0 && wz <= _occHm2Z1) {
          const u2 = (wx - _occHm2X0) / ((_occHm2X1 - _occHm2X0) || 1);
          const v2 = (wz - _occHm2Z0) / ((_occHm2Z1 - _occHm2Z0) || 1);
          const px2 = Math.min(S.hm2PixelW - 1, Math.max(0, Math.round(u2 * (S.hm2PixelW - 1))));
          const py2 = Math.min(S.hm2PixelH - 1, Math.max(0, Math.round(v2 * (S.hm2PixelH - 1))));
          terrH = _occHm2Min + (S.hm2PixelData[(py2 * S.hm2PixelW + px2) * 4] / 255) * _occHm2Range;
        }
        const topH = terrH + occH;
        const occRadius = styleKey === 'building' ? 4.0 : 3.0;
        _occSamples.push({ wx, wz, topH, occRadius });
        if (wx < _occMinX) _occMinX = wx;
        if (wz < _occMinZ) _occMinZ = wz;
        if (wx > _occMaxX) _occMaxX = wx;
        if (wz > _occMaxZ) _occMaxZ = wz;
      }

      if (_occSamples.length > 0) {
        const occMargin = 8;
        S.occGridX0 = _occMinX - occMargin;
        S.occGridZ0 = _occMinZ - occMargin;
        const occX1 = _occMaxX + occMargin;
        const occZ1 = _occMaxZ + occMargin;
        const occWm = Math.max(1, occX1 - S.occGridX0);
        const occHm = Math.max(1, occZ1 - S.occGridZ0);
        S.occGridCellSize = Math.max(1, occWm / 4096, occHm / 4096);
        S.occGridW = Math.max(1, Math.ceil(occWm / S.occGridCellSize));
        S.occGridH = Math.max(1, Math.ceil(occHm / S.occGridCellSize));
        S.occGrid = new Float32Array(S.occGridW * S.occGridH);

        for (const s of _occSamples) {
          const gcx = Math.floor((s.wx - S.occGridX0) / S.occGridCellSize);
          const gcz = Math.floor((s.wz - S.occGridZ0) / S.occGridCellSize);
          const rCells = Math.max(1, Math.ceil(s.occRadius / S.occGridCellSize));
          for (let dz = -rCells; dz <= rCells; dz++) {
            for (let dx = -rCells; dx <= rCells; dx++) {
              if ((dx * dx + dz * dz) > (rCells * rCells)) continue;
              const gx = gcx + dx;
              const gz = gcz + dz;
              if (gx < 0 || gx >= S.occGridW || gz < 0 || gz >= S.occGridH) continue;
              const idx = gz * S.occGridW + gx;
              if (s.topH > S.occGrid[idx]) S.occGrid[idx] = s.topH;
            }
          }
        }
      } else {
        S.occGrid = null;
        S.occGridW = 0;
        S.occGridH = 0;
      }

      // Upload occupancy grid to GPU
      if (S.gpuAvailable && S.gpuDevice && S.occGrid) {
        S.gpuOccBuf = S.gpuDevice.createBuffer({
          size: S.occGrid.byteLength,
          usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST
        });
        S.gpuDevice.queue.writeBuffer(S.gpuOccBuf, 0, S.occGrid);
        // Occupancy uploaded successfully; keep console output quiet in normal viewer usage.
      } else {
        S.gpuOccBuf = null;
      }

      // Define 3D shapes per category
      const SHAPES = {
        tree:           { geo: new THREE.ConeGeometry(9, 30, 5),         color: 0x228B22, yOff: 15 },
        bush:           { geo: new THREE.DodecahedronGeometry(6, 0),     color: 0x6B8E23, yOff: 6 },
        vegetation:     { geo: new THREE.SphereGeometry(3.75, 4, 3),     color: 0x90EE90, yOff: 3.75 },
        building:       { geo: new THREE.BoxGeometry(27, 18, 27),        color: 0x808080, yOff: 9 },
        infrastructure: { geo: new THREE.BoxGeometry(15, 0.2, 15),      color: 0xC8B898, yOff: 0.1 },
        road:           { geo: new THREE.BoxGeometry(14, 0.35, 14),      color: 0x2f2f2f, yOff: 0.2 },
        rock:           { geo: new THREE.IcosahedronGeometry(7.5, 0),    color: 0x708090, yOff: 3.75 },
        debris:         { geo: new THREE.BoxGeometry(6, 3, 6),           color: 0x8B0000, yOff: 1.5 },
        earthwork:      { geo: new THREE.BoxGeometry(8, 1.2, 8),         color: 0x7a3320, yOff: 0.6 },
        other:          { geo: new THREE.OctahedronGeometry(4.5, 0),     color: 0xCCCCCC, yOff: 4.5 },
      };

      S.rendinstGroup = new THREE.Group();
      S.rendinstGroup.renderOrder = 1000;
      S.rendinstCategoryData = {};

      for (const [cat, flat] of Object.entries(styleBuckets)) {
        const count = flat.length / 3;
        const shape = SHAPES[cat] || SHAPES.other;
        const mat = new THREE.MeshLambertMaterial({ color: shape.color });
        const mesh = new THREE.InstancedMesh(shape.geo, mat, count);
        mesh.frustumCulled = false;
        const arr = mesh.instanceMatrix.array;
        for (let i = 0; i < count; i++) {
          const off = i * 16;
          const fi = i * 3;
          arr[off]    = 1; arr[off+5]  = 1; arr[off+10] = 1; arr[off+15] = 1;
          arr[off+12] = flat[fi];
          arr[off+13] = flat[fi+2] * defaultHeight + shape.yOff;
          arr[off+14] = flat[fi+1];
        }
        mesh.instanceMatrix.needsUpdate = true;
        S.rendinstCategoryData[cat] = { mesh, flat: new Float32Array(flat), yOff: shape.yOff };
        //if (cat === 'infrastructure') mesh.visible = false;
        S.rendinstGroup.add(mesh);
      }

      S.rendinstGroup.visible = false;
      S.scene.add(S.rendinstGroup);
    } catch (e) {
      console.warn('RendInst load failed:', e);
    }
  }

  // Animate the visible terrain meshes in (radial wireframe expansion ->
  // radial texture expansion -> water/object reveal). This runs after
  // tank zone + objects are created so they can stay hidden from frame 0.
  playTerrainIntro([S.tileGridMesh, S.terrainMesh].filter(Boolean));

  // -- Tile hover highlight --
  {
    const _thlFill = new THREE.Mesh(
      new THREE.PlaneGeometry(1, 1),
      new THREE.MeshBasicMaterial({ color: 0xbbffcc, transparent: true, opacity: 0.25,
        depthTest: false, side: THREE.DoubleSide, depthWrite: false })
    );
    _thlFill.rotation.x = -Math.PI / 2;
    const _thlBorderPts = [
      new THREE.Vector3(-0.5, 0, -0.5), new THREE.Vector3( 0.5, 0, -0.5),
      new THREE.Vector3( 0.5, 0,  0.5), new THREE.Vector3(-0.5, 0,  0.5),
    ];
    const _thlBorder = new THREE.LineLoop(
      new THREE.BufferGeometry().setFromPoints(_thlBorderPts),
      new THREE.LineBasicMaterial({ color: 0x66ffaa, depthTest: false })
    );
    S.tileHighlightGroup = new THREE.Group();
    S.tileHighlightGroup.add(_thlFill);
    S.tileHighlightGroup.add(_thlBorder);
    S.tileHighlightGroup.visible = false;
    S.tileHighlightGroup.renderOrder = 998;
    S.scene.add(S.tileHighlightGroup);
  }

  // -- CPU-displaced mesh for hover raycasting --
  {
    // Use 256 segments (not display-quality) for fast raycasting
    const _rcSegs = 512;
    const raycastGeo = new THREE.PlaneGeometry(planeSize, planeSize, _rcSegs, _rcSegs);
    const positions = raycastGeo.attributes.position;
    const uvs = raycastGeo.attributes.uv;
    S.raycastHeightNorms = new Float32Array(positions.count);

    if (hmPixelData) {
      // Rule §3 (docs/ORIENTATION.md): PNG is 1:1 with the §5 float buffer,
      // so sampling is non-flipped: px = u*W, py = v*H. Any `(1 - u)` or
      // `(1 - v)` inversion is a legacy bug (see §12).
      for (let i = 0; i < positions.count; i++) {
        const u = uvs.getX(i);
        const v = uvs.getY(i);
        const px = Math.min(Math.floor(u * hmPixelW), hmPixelW - 1);
        const py = Math.min(Math.floor(v * hmPixelH), hmPixelH - 1);
        const h = hmPixelData[(py * hmPixelW + px) * 4] / 255;
        S.raycastHeightNorms[i] = h;
        positions.setZ(i, h * defaultHeight);
      }
      positions.needsUpdate = true;
    }

    raycastGeo.computeBoundingSphere();
    S.raycastMesh = new THREE.Mesh(raycastGeo, new THREE.MeshBasicMaterial({ visible: false }));
    S.raycastMesh.rotation.x = -Math.PI / 2;
    S.scene.add(S.raycastMesh);
  }

  // Camera
  const dist = planeSize * 0.7;
  // Camera starts on the scene +Z (world −wz = south) side looking toward
  // −Z (world +wz = north), so the user sees north at the top of the screen.
  S.camera.position.set(0, dist * 0.5, dist * 0.6);
  S.camera.lookAt(0, defaultHeight * S.waterFraction, 0);
  S.controls.target.set(0, defaultHeight * S.waterFraction, 0);
  S.controls.minDistance = 5;
  S.controls.maxDistance = planeSize * 3;
  S.controls.update();
  
  // Mark startup as complete - detail assets loading in background if available
  progress(100, 'Ready for interaction!');
  if (detailLoadingPromise) {
    console.info('Detail heightmap loading in background...');
  }
}
