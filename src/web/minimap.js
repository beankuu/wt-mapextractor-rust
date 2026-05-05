import { S } from './state.js';
import { DATA } from './helpers.js';
import { sceneToWorldXZ } from './coords.js';

const imageCache = new Map();
const imageDataCache = new Map();
const FALLBACK_MINIMAP_GRID = 10;
const DOWNLOAD_SIZE = 2048;

function loadImageCached(src) {
  if (!src) return Promise.resolve(null);
  if (imageCache.has(src)) return imageCache.get(src);
  const promise = new Promise((resolve) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => resolve(null);
    img.src = src;
  });
  imageCache.set(src, promise);
  return promise;
}

async function loadImageDataCached(src) {
  if (!src) return null;
  if (imageDataCache.has(src)) return imageDataCache.get(src);
  const img = await loadImageCached(src);
  if (!img) return null;
  const canvas = document.createElement('canvas');
  canvas.width = img.naturalWidth || img.width;
  canvas.height = img.naturalHeight || img.height;
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  ctx.drawImage(img, 0, 0);
  const data = {
    width: canvas.width,
    height: canvas.height,
    pixels: ctx.getImageData(0, 0, canvas.width, canvas.height).data,
  };
  imageDataCache.set(src, data);
  return data;
}

function rectFromExtent(extent, fallback0, fallback1) {
  const x0 = extent && extent.length === 4 ? extent[0] : fallback0[0];
  const z0 = extent && extent.length === 4 ? extent[1] : fallback0[1];
  const x1 = extent && extent.length === 4 ? extent[2] : fallback1[0];
  const z1 = extent && extent.length === 4 ? extent[3] : fallback1[1];
  return {
    x0: Math.min(x0, x1),
    z0: Math.min(z0, z1),
    x1: Math.max(x0, x1),
    z1: Math.max(z0, z1),
  };
}

function imageRectForWorld(worldRect, sourceRect, img) {
  const iw = img.naturalWidth || img.width;
  const ih = img.naturalHeight || img.height;
  const sw = Math.max(sourceRect.x1 - sourceRect.x0, 0.001);
  const sh = Math.max(sourceRect.z1 - sourceRect.z0, 0.001);
  const x0 = ((worldRect.x0 - sourceRect.x0) / sw) * iw;
  const x1 = ((worldRect.x1 - sourceRect.x0) / sw) * iw;
  const y0 = ((worldRect.z0 - sourceRect.z0) / sh) * ih;
  const y1 = ((worldRect.z1 - sourceRect.z0) / sh) * ih;
  const sx = Math.max(0, Math.min(iw, Math.min(x0, x1)));
  const sy = Math.max(0, Math.min(ih, Math.min(y0, y1)));
  const ex = Math.max(0, Math.min(iw, Math.max(x0, x1)));
  const ey = Math.max(0, Math.min(ih, Math.max(y0, y1)));
  return { sx, sy, sw: Math.max(1, ex - sx), sh: Math.max(1, ey - sy) };
}

function getMainSource(M) {
  if (M.terrainPaint && M.terrainPaint.file) {
    return {
      src: `${DATA}/${M.terrainPaint.file}`,
      rect: rectFromExtent(M.tileGrid && M.tileGrid.world_extent, M.mapCoord0, M.mapCoord1),
    };
  }
  if (M.colormap && M.colormap.file) {
    return {
      src: `${DATA}/${M.colormap.file}`,
      rect: rectFromExtent(M.heightmap && M.heightmap.world_extent, M.mapCoord0, M.mapCoord1),
    };
  }
  if (M.tileGrid && M.tileGrid.file) {
    return {
      src: `${DATA}/${M.tileGrid.file}`,
      rect: rectFromExtent(M.tileGrid.world_extent, M.mapCoord0, M.mapCoord1),
    };
  }
  return null;
}

function getHeightmapSource(M) {
  if (!M.heightmap || !M.heightmap.file) return getMainSource(M);
  return {
    src: `${DATA}/${M.heightmap.file}`,
    rect: rectFromExtent(M.heightmap.world_extent, M.mapCoord0, M.mapCoord1),
  };
}

function getTankZoneRect(M) {
  const tz = M.tankZone;
  if (!tz || !tz.coord0 || !tz.coord1) return null;
  return rectFromExtent([tz.coord0[0], tz.coord0[1], tz.coord1[0], tz.coord1[1]], M.mapCoord0, M.mapCoord1);
}

function getGridCount(M) {
  const gridSize = M?.tankZone?.gridSize;
  if (Number.isFinite(gridSize) && gridSize >= 2 && gridSize <= 64) return Math.round(gridSize);
  return FALLBACK_MINIMAP_GRID;
}

function rowLabel(i) {
  let n = i;
  let label = '';
  do {
    label = String.fromCharCode(65 + (n % 26)) + label;
    n = Math.floor(n / 26) - 1;
  } while (n >= 0);
  return label;
}

function drawGrid(ctx, size, grid, viewRect) {
  const fontSize = Math.max(9, Math.round(size / 58));
  const scaleLabelFont = Math.max(8, Math.round(fontSize * 0.78));
  const pad = Math.max(4, Math.round(size / 96));
  ctx.save();
  ctx.strokeStyle = 'rgba(230,238,255,0.55)';
  ctx.lineWidth = 1;
  ctx.fillStyle = 'rgba(245,248,255,0.9)';
  ctx.font = `600 ${fontSize}px sans-serif`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'top';
  for (let i = 0; i <= grid; i++) {
    const p = Math.round((i / grid) * size) + 0.5;
    ctx.beginPath();
    ctx.moveTo(p, 0);
    ctx.lineTo(p, size);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(0, p);
    ctx.lineTo(size, p);
    ctx.stroke();
  }

  for (let i = 0; i < grid; i++) {
    const center = ((i + 0.5) / grid) * size;
    ctx.fillText(String(i + 1), center, pad);
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.fillText(rowLabel(i), pad, center);
    ctx.textAlign = 'center';
    ctx.textBaseline = 'top';
  }

  const gridMeters = (viewRect.x1 - viewRect.x0) / grid;
  const label = `1 grid = ${formatMeters(gridMeters)}`;
  ctx.font = `600 ${scaleLabelFont}px sans-serif`;
  ctx.textAlign = 'right';
  ctx.textBaseline = 'bottom';
  const x = size - pad;
  const y = size - pad;
  const metrics = ctx.measureText(label);
  ctx.fillStyle = 'rgba(7,16,29,0.72)';
  ctx.fillRect(
    x - metrics.width - pad,
    y - scaleLabelFont - pad,
    metrics.width + pad * 2,
    scaleLabelFont + pad * 1.5,
  );
  ctx.fillStyle = 'rgba(245,248,255,0.96)';
  ctx.fillText(label, x, y);
  ctx.restore();
}

function drawBattleRect(ctx, viewRect, battleRect, size, stroke = '#ff9f2e', fill = 'rgba(255,159,46,0.12)') {
  const vw = Math.max(viewRect.x1 - viewRect.x0, 0.001);
  const vh = Math.max(viewRect.z1 - viewRect.z0, 0.001);
  const x0 = ((battleRect.x0 - viewRect.x0) / vw) * size;
  const x1 = ((battleRect.x1 - viewRect.x0) / vw) * size;
  const y0 = size - ((battleRect.z1 - viewRect.z0) / vh) * size;
  const y1 = size - ((battleRect.z0 - viewRect.z0) / vh) * size;
  ctx.save();
  ctx.strokeStyle = stroke;
  ctx.fillStyle = fill;
  ctx.lineWidth = 2;
  ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
  ctx.strokeRect(x0 + 1, y0 + 1, x1 - x0 - 2, y1 - y0 - 2);
  ctx.restore();
}

function worldToCanvas(viewRect, wx, wz, size) {
  const vw = Math.max(viewRect.x1 - viewRect.x0, 0.001);
  const vh = Math.max(viewRect.z1 - viewRect.z0, 0.001);
  return {
    x: ((wx - viewRect.x0) / vw) * size,
    y: size - ((wz - viewRect.z0) / vh) * size,
  };
}

function intersectsView(viewRect, x0, z0, x1, z1) {
  return !(x1 < viewRect.x0 || x0 > viewRect.x1 || z1 < viewRect.z0 || z0 > viewRect.z1);
}

async function drawOceanOverlay(ctx, viewRect, size) {
  const M = S.M;
  const waterOn = document.getElementById('cb-water')?.checked;
  if (!waterOn || !M?.heightmap?.file || M.waterLevel == null) return;
  const hMin = M.heightmap.height_min_m;
  const hMax = M.heightmap.height_max_m;
  if (!Number.isFinite(hMin) || !Number.isFinite(hMax) || hMax <= hMin) return;

  const hm = await loadImageDataCached(`${DATA}/${M.heightmap.file}`);
  if (!hm) return;
  const hmRect = rectFromExtent(M.heightmap.world_extent, M.mapCoord0, M.mapCoord1);
  const hmW = Math.max(hmRect.x1 - hmRect.x0, 0.001);
  const hmH = Math.max(hmRect.z1 - hmRect.z0, 0.001);
  const viewW = Math.max(viewRect.x1 - viewRect.x0, 0.001);
  const viewH = Math.max(viewRect.z1 - viewRect.z0, 0.001);
  const step = size >= 1024 ? 2 : 1;
  const threshold = M.waterLevel - ((hMax - hMin) / 255.0);

  ctx.save();
  ctx.fillStyle = 'rgba(13, 79, 139, 0.88)';
  for (let y = 0; y < size; y += step) {
    const wz = viewRect.z1 - ((y + 0.5) / size) * viewH;
    const py = Math.max(0, Math.min(hm.height - 1, Math.floor(((wz - hmRect.z0) / hmH) * hm.height)));
    for (let x = 0; x < size; x += step) {
      const wx = viewRect.x0 + ((x + 0.5) / size) * viewW;
      const px = Math.max(0, Math.min(hm.width - 1, Math.floor(((wx - hmRect.x0) / hmW) * hm.width)));
      const v = hm.pixels[(py * hm.width + px) * 4];
      const height = hMin + (v / 255) * (hMax - hMin);
      if (height <= threshold) ctx.fillRect(x, y, step, step);
    }
  }
  ctx.restore();
}

function effectiveMissionMode(items) {
  if ((items || []).some((x) => x.mode === 'arcade')) return 'arcade';
  if ((items || []).some((x) => x.mode === 'hardcore')) return 'hardcore';
  return 'arcade';
}

function selectedMission() {
  const selected = document.querySelector('input[name="mission"]:checked');
  if (!selected || selected.value === '' || !S.missionData?.missions) return null;
  const idx = parseInt(selected.value, 10);
  return Number.isFinite(idx) ? S.missionData.missions[idx] : null;
}

function drawMissionOverlays(ctx, viewRect, size) {
  const mission = selectedMission();
  if (!mission) return;
  const captureMode = effectiveMissionMode(mission.captureZones);
  const spawnMode = effectiveMissionMode(mission.spawns);
  const spawnZoneMode = effectiveMissionMode(mission.spawnZones);
  const scale = size / Math.max(viewRect.x1 - viewRect.x0, 0.001);

  ctx.save();
  for (const cap of mission.captureZones || []) {
    if (cap.mode !== captureMode) continue;
    const p = worldToCanvas(viewRect, cap.pos[0], cap.pos[1], size);
    const r = Math.max(2, cap.radius * scale);
    ctx.fillStyle = 'rgba(255, 196, 0, 0.25)';
    ctx.strokeStyle = '#ffc400';
    ctx.lineWidth = Math.max(1, size / 512);
    ctx.beginPath();
    ctx.arc(p.x, p.y, r, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
  }

  const drawSpawn = (item, large) => {
    const p = worldToCanvas(viewRect, item.pos[0], item.pos[1], size);
    if (p.x < -8 || p.y < -8 || p.x > size + 8 || p.y > size + 8) return;
    const color = item.team === 2 ? '#ff2b2b' : item.team === 1 ? '#00a2ff' : '#88ff88';
    ctx.fillStyle = color;
    ctx.strokeStyle = 'rgba(0,0,0,0.7)';
    ctx.lineWidth = Math.max(1, size / 768);
    ctx.beginPath();
    ctx.arc(p.x, p.y, Math.max(2, size * (large ? 0.007 : 0.004)), 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
  };
  for (const sp of mission.spawns || []) if (sp.mode === spawnMode) drawSpawn(sp, true);
  for (const sz of mission.spawnZones || []) if (sz.mode === spawnZoneMode) drawSpawn(sz, false);
  ctx.restore();
}

function drawObjectOverlays(ctx, viewRect, size) {
  if (!document.getElementById('cb-rendinst')?.checked || !S.rendinstCategoryData) return;
  ctx.save();
  ctx.fillStyle = 'rgba(230, 238, 255, 0.88)';
  const pixel = Math.max(1, Math.round(size / 1024));
  for (const data of Object.values(S.rendinstCategoryData)) {
    if (!data.mesh?.visible || !data.flat) continue;
    for (let i = 0; i + 1 < data.flat.length; i += 3) {
      const world = sceneToWorldXZ(S.M, data.flat[i], data.flat[i + 1]);
      if (world.x < viewRect.x0 || world.x > viewRect.x1 || world.z < viewRect.z0 || world.z > viewRect.z1) continue;
      const p = worldToCanvas(viewRect, world.x, world.z, size);
      ctx.fillRect(p.x, p.y, pixel, pixel);
    }
  }
  ctx.restore();
}

function formatMeters(v) {
  if (!Number.isFinite(v) || v <= 0) return '-- m';
  if (v >= 1000) return `${(v / 1000).toFixed(2)} km`;
  return `${Math.round(v)} m`;
}

async function renderMinimapTo(canvas, mode, updateRuler) {
  const M = S.M;
  const rulerEl = document.getElementById('minimap-ruler');
  if (!M || !canvas) return;

  const ctx = canvas.getContext('2d');
  const size = canvas.width;
  const battleRect = getTankZoneRect(M);
  const battleSelected = !!(document.getElementById('cb-tankzone')?.checked && battleRect);
  const heightmapOnly = mode === 'heightmap';

  let source = mode === 'heightmap' ? getHeightmapSource(M) : getMainSource(M);
  let viewRect = source ? source.rect : rectFromExtent(null, M.mapCoord0, M.mapCoord1);
  if (mode === 'battle' && battleSelected) {
    source = getMainSource(M) || getHeightmapSource(M);
    viewRect = battleRect;
  }

  if (updateRuler && rulerEl) {
    if (heightmapOnly) {
      rulerEl.textContent = 'Heightmap only';
    } else {
      const grid = getGridCount(M);
      const gridMeters = (viewRect.x1 - viewRect.x0) / grid;
      rulerEl.textContent = `1 grid = ${formatMeters(gridMeters)}`;
    }
  }

  ctx.clearRect(0, 0, size, size);
  ctx.fillStyle = '#07101d';
  ctx.fillRect(0, 0, size, size);

  const img = await loadImageCached(source && source.src);
  if (img && source) {
    const srcRect = imageRectForWorld(viewRect, source.rect, img);
    ctx.save();
    ctx.translate(0, size);
    ctx.scale(1, -1);
    ctx.drawImage(img, srcRect.sx, srcRect.sy, srcRect.sw, srcRect.sh, 0, 0, size, size);
    ctx.restore();
  }

  if (heightmapOnly) return;

  await drawOceanOverlay(ctx, viewRect, size);

  if (mode !== 'battle' && battleSelected) {
    drawBattleRect(ctx, viewRect, battleRect, size);
  }
  drawObjectOverlays(ctx, viewRect, size);
  drawMissionOverlays(ctx, viewRect, size);
  drawGrid(ctx, size, getGridCount(M), viewRect);
}

export function renderMinimap() {
  const canvas = document.getElementById('minimap-canvas');
  const select = document.getElementById('minimap-type');
  if (!canvas || !select) return;
  renderMinimapTo(canvas, select.value, true);
}

export function refreshMinimapOptions() {
  const M = S.M;
  const widget = document.getElementById('minimap-widget');
  const select = document.getElementById('minimap-type');
  if (!M || !widget || !select) return;

  const hasAnySource = !!(getMainSource(M) || getHeightmapSource(M));
  widget.style.display = hasAnySource ? 'block' : 'none';

  const battleOption = select.querySelector('option[value="battle"]');
  const battleAvailable = !!(getTankZoneRect(M) && document.getElementById('cb-tankzone')?.checked);
  if (battleOption) battleOption.hidden = !battleAvailable;
  if (!battleAvailable && select.value === 'battle') select.value = 'main';
  renderMinimap();
}

export function initMinimap() {
  const select = document.getElementById('minimap-type');
  const canvas = document.getElementById('minimap-canvas');
  const dl = document.getElementById('minimap-download');
  if (!select) return;
  select.addEventListener('change', renderMinimap);
  if (dl && canvas) {
    dl.addEventListener('click', async () => {
      const mapName = (S.M && S.M.mapName) ? S.M.mapName : 'map';
      const mode = select.value || 'main';
      const out = document.createElement('canvas');
      out.width = DOWNLOAD_SIZE;
      out.height = DOWNLOAD_SIZE;
      await renderMinimapTo(out, mode, false);
      const a = document.createElement('a');
      a.href = out.toDataURL('image/png');
      a.download = `${mapName}_minimap_${mode}.png`;
      a.click();
    });
  }
  refreshMinimapOptions();
}
