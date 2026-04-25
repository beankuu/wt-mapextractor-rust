import * as THREE from 'three';
import { S } from './state.js';
import { getMapFrame, sceneToWorldXZ, worldToSceneXZ } from './coords.js';

const raycaster = new THREE.Raycaster();
raycaster.firstHitOnly = true;
const mouse = new THREE.Vector2();
let _mouseDirty = false;
let _mouseClientX = 0;
let _mouseClientY = 0;

function _lcSwatchColor(idx) {
  const hue = (idx * 137.508) % 360;
  return `hsl(${hue}deg 78% 56%)`;
}

export function sceneToWorld(p) {
  const M = S.M;
  if (!M) return { x: 0, z: 0, h: 0 };
  const w = sceneToWorldXZ(M, p.x, p.z);
  let heightM = p.y;
  if (M.heightmap && M.heightmap.height_min_m != null && S.displacedMeshes.length > 0) {
    const curScale = S.displacedMeshes[0].material.displacementScale || 1;
    const norm = curScale > 0 ? p.y / curScale : 0;
    heightM = M.heightmap.height_min_m + norm * (M.heightmap.height_max_m - M.heightmap.height_min_m);
  }
  return { x: w.x, z: w.z, h: heightM };
}

export function setupHover() {
  const hoverEl = document.getElementById('hover-coords');
  const tilePopupEl = document.getElementById('tile-lc-popup');

  S.canvas.addEventListener('mousemove', (e) => {
    _mouseClientX = e.clientX;
    _mouseClientY = e.clientY;
    mouse.x = (e.clientX / window.innerWidth) * 2 - 1;
    mouse.y = -(e.clientY / window.innerHeight) * 2 + 1;
    _mouseDirty = true;
    S.needsRender = true;
  });

  S.canvas.addEventListener('mouseleave', () => {
    hoverEl.style.display = 'none';
    if (tilePopupEl) tilePopupEl.style.display = 'none';
    if (S.tileHighlightGroup) S.tileHighlightGroup.visible = false;
    _mouseDirty = false;
  });
}

export function updateHoverCoords() {
  const M = S.M;
  const hoverEl = document.getElementById('hover-coords');
  const tilePopupEl = document.getElementById('tile-lc-popup');
  if (!_mouseDirty || !M || !S.raycastMesh) return;
  _mouseDirty = false;
  raycaster.setFromCamera(mouse, S.camera);

  let hitDetail = false;
  let p = null;
  if (S.detailRaycastMesh) {
    const detHits = raycaster.intersectObjects([S.detailRaycastMesh], false);
    if (detHits.length > 0) {
      p = detHits[0].point;
      hitDetail = true;
    }
  }

  if (!p) {
    const hits = raycaster.intersectObjects([S.raycastMesh], false);
    if (hits.length === 0) {
      hoverEl.style.display = 'none';
      return;
    }
    p = hits[0].point;
  }

  const planeSize = Math.max(M.mapSize[0], M.mapSize[1]);
  const frame = getMapFrame(M, planeSize);
  const world = sceneToWorldXZ(M, p.x, p.z, planeSize);
  const worldX = world.x;
  const worldZ = world.z;
  let heightM = p.y;
  if (M.heightmap && M.heightmap.height_min_m != null && S.displacedMeshes.length > 0) {
    if (hitDetail && S.detailRaycastMesh && S.detailRaycastMesh.userData.hm2) {
      const hm2 = S.detailRaycastMesh.userData.hm2;
      const curLRScale = S.displacedMeshes[0].material.displacementScale || 1;
      const detScale = (hm2.hm2Range / hm2.lrRange) * curLRScale;
      const detBias = (hm2.hm2MinOffset / hm2.lrRange) * curLRScale;
      const norm = detScale > 0 ? (p.y - detBias) / detScale : 0;
      const D = M.heightmapDetail;
      heightM = D.height_min_m + norm * (D.height_max_m - D.height_min_m);
    } else {
      const curScale = S.displacedMeshes[0].material.displacementScale || 1;
      const norm = curScale > 0 ? p.y / curScale : 0;
      heightM = M.heightmap.height_min_m + norm * (M.heightmap.height_max_m - M.heightmap.height_min_m);
    }
  }

  let tilePopupHtml = '';
  if (S.tileToolActive) {
    const tp = M.terrainPaint;
    const tg = M.tileGrid;
    const gw = tp ? tp.gridW : (tg ? tg.cols : 0);
    const gh = tp ? tp.gridH : (tg ? tg.rows : 0);
    if (gw && gh) {
      const we = (tg && Array.isArray(tg.world_extent) && tg.world_extent.length === 4)
        ? tg.world_extent
        : ((M.heightmap && Array.isArray(M.heightmap.world_extent) && M.heightmap.world_extent.length === 4)
            ? M.heightmap.world_extent
            : null);
      const x0 = we ? Math.min(we[0], we[2]) : frame.mc0[0];
      const x1 = we ? Math.max(we[0], we[2]) : frame.mc1[0];
      const z0 = we ? Math.min(we[1], we[3]) : frame.mc0[1];
      const z1 = we ? Math.max(we[1], we[3]) : frame.mc1[1];
      const tx = Math.floor((worldX - x0) / (Math.max(x1 - x0, 0.001) / gw));
      const tz = Math.floor((worldZ - z0) / (Math.max(z1 - z0, 0.001) / gh));
      const cx = Math.max(0, Math.min(gw - 1, tx));
      const cz = Math.max(0, Math.min(gh - 1, tz));
      tilePopupHtml = `<div class="tl-row"><span class="label">Tile:</span> <span class="val">${cx},${cz}</span></div>`;

      const lcNames = Array.isArray(tp?.lcNames) ? tp.lcNames : null;
      const cellLcIndices = Array.isArray(tp?.cellLcIndices) ? tp.cellLcIndices : null;
      const lcMeta = Array.isArray(M.landclasses) ? M.landclasses : [];
      // Map an LC name to its manifest record (for `details` components).
      const lcByName = new Map();
      for (const lc of lcMeta) {
        if (lc && typeof lc.name === 'string') lcByName.set(lc.name, lc);
      }
      if (lcNames && cellLcIndices) {
        const ci = cz * gw + cx;
        const det = cellLcIndices[ci];
        if (Array.isArray(det)) {
          // det layout (paint.rs blend_tile): [base, R, G, B, +2G, +2R, +2A]
          // where R/G/B/+... are the splat-weight channel slots. We show
          // channel assignment since per-pixel splat weights aren't
          // available client-side (the weights live in per-cell DDSx on
          // disk). The `detailRed/Green/Blue/Black` strings in each LC's
          // `details` object give the named components the LC uses
          // internally (sampled through the R/G/B/K weight channels of
          // its OWN splat).
          const channelLabels = ['Base', 'R', 'G', 'B', 'G2', 'R2', 'A2'];
          // Detail-component keys preferred display order (matching the
          // native blender's primary_detail_slots permutation).
          const detailOrder = ['R', 'G', 'B', 'K'];
          let anyShown = false;
          let html = '';
          for (let slot = 0; slot < det.length && slot < channelLabels.length; slot++) {
            const rawIdx = Number(det[slot]);
            if (!Number.isFinite(rawIdx) || rawIdx === 255) continue;
            if (rawIdx < 0 || rawIdx >= lcNames.length) continue;
            const lcName = lcNames[rawIdx];
            const lc = lcByName.get(lcName);
            if (!anyShown) {
              html += `<div style="height:6px"></div><div class="label">LC channels</div>`;
              anyShown = true;
            }
            html += `<div class="tl-row" style="margin-top:4px"><span class="swatch" style="background:${_lcSwatchColor(rawIdx)}"></span><span class="val"><b>${channelLabels[slot]}</b> \u2192 ${rawIdx}: ${lcName}</span></div>`;
            const details = lc && typeof lc.details === 'object' ? lc.details : null;
            if (details) {
              for (const k of detailOrder) {
                const v = details[k];
                if (typeof v === 'string' && v.length > 0) {
                  const short = v.replace(/_tex_d$/, '').replace(/^detail_/, '');
                  html += `<div class="tl-row" style="margin-left:18px;font-size:10px;color:#89a"><span class="val">\u00b7 ${k}: ${short}</span></div>`;
                }
              }
            }
          }
          if (anyShown) tilePopupHtml += html;
        }
      }

      if (S.tileHighlightGroup) {
        const tileWorldW = frame.mcW / gw;
        const tileWorldH = frame.mcH / gh;
        const tileCenterWX = frame.mc0[0] + (cx + 0.5) * tileWorldW;
        const tileCenterWZ = frame.mc0[1] + (cz + 0.5) * tileWorldH;
        const sceneCenter = worldToSceneXZ(M, tileCenterWX, tileCenterWZ, planeSize);
        const tileSzX = Math.abs(tileWorldW * frame.sX);
        const tileSzZ = Math.abs(tileWorldH * frame.sZ);
        S.tileHighlightGroup.position.set(sceneCenter.x, p.y + 0.3, sceneCenter.z);
        S.tileHighlightGroup.scale.set(tileSzX, 1, tileSzZ);
        S.tileHighlightGroup.visible = true;
      }
    } else if (S.tileHighlightGroup) {
      S.tileHighlightGroup.visible = false;
    }
  } else if (S.tileHighlightGroup) {
    S.tileHighlightGroup.visible = false;
  }
  hoverEl.innerHTML = `<span class="label">X:</span> <span class="val">${worldX.toFixed(0)}</span> &nbsp; <span class="label">Z:</span> <span class="val">${worldZ.toFixed(0)}</span> &nbsp; <span class="label">H:</span> <span class="val">${heightM.toFixed(1)}m</span>`;

  if (tilePopupEl) {
    if (S.tileToolActive && tilePopupHtml) {
      tilePopupEl.innerHTML = tilePopupHtml;
      tilePopupEl.style.display = 'block';
      const pad = 10;
      const rect = tilePopupEl.getBoundingClientRect();
      let left = _mouseClientX + 14;
      let top = _mouseClientY + 14;
      if (left + rect.width + pad > window.innerWidth) left = _mouseClientX - rect.width - 14;
      if (top + rect.height + pad > window.innerHeight) top = _mouseClientY - rect.height - 14;
      left = Math.max(pad, Math.min(left, window.innerWidth - rect.width - pad));
      top = Math.max(pad, Math.min(top, window.innerHeight - rect.height - pad));
      tilePopupEl.style.left = `${left}px`;
      tilePopupEl.style.top = `${top}px`;
    } else {
      tilePopupEl.style.display = 'none';
    }
  }
  hoverEl.style.display = 'block';
}
