import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { S } from './state.js';
import { DATA, ASSET_PREFIX } from './helpers.js';
import { setHeightScale } from './height-scale.js';
import { initEarthGlobe } from './earth-globe.js';
import { refreshMinimapOptions } from './minimap.js';

function _titleCaseType(name) {
  return String(name || 'other').replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
}

function _buildObjectTypesUI() {
  const panel = document.getElementById('object-types-panel');
  const list = document.getElementById('object-types-list');
  const btnEnableAll = document.getElementById('btn-obj-enable-all');
  const btnDisableAll = document.getElementById('btn-obj-disable-all');
  const btnRealObjects = document.getElementById('btn-real-objects');
  if (!panel || !list || !btnEnableAll || !btnDisableAll || !btnRealObjects) return;

  // Initially hidden; can be expanded
  panel.style.display = 'none';
  list.innerHTML = '';
  if (!S.rendinstCategoryData) return;

  // Add collapse/expand toggle
  let expanded = false;
  const row = document.getElementById('row-rendinst');
  if (row && !row.querySelector('.obj-toggle-btn')) {
    const toggleBtn = document.createElement('button');
    toggleBtn.className = 'obj-toggle-btn';
    toggleBtn.style.cssText = 'margin-top:4px;width:100%;background:#0f1f35;border:1px solid #1e3a5f;color:#4dabf7;border-radius:4px;padding:4px 6px;cursor:pointer;font-size:10px;font-weight:600;transition:background 0.15s;';
    toggleBtn.textContent = '▸ Object Types';
    toggleBtn.addEventListener('mouseover', () => { toggleBtn.style.background = 'rgba(30, 58, 95, 0.5)'; });
    toggleBtn.addEventListener('mouseout', () => { toggleBtn.style.background = '#0f1f35'; });
    toggleBtn.addEventListener('click', () => {
      expanded = !expanded;
      toggleBtn.textContent = (expanded ? '▾' : '▸') + ' Object Types';
      panel.style.display = expanded ? 'block' : 'none';
    });
    row.appendChild(toggleBtn);
  }

  const keys = Object.keys(S.rendinstCategoryData).sort((a, b) => a.localeCompare(b));
  for (const key of keys) {
    const data = S.rendinstCategoryData[key];
    const row = document.createElement('div');
    row.className = 'ctrl-row';
    row.style.padding = '1px 0';
    row.style.fontSize = '10px';
    const count = Math.floor((data.flat?.length || 0) / 3);
    row.innerHTML = `
      <input type="checkbox" id="objtype-${key}" ${data.mesh.visible ? 'checked' : ''}>
      <label for="objtype-${key}" style="color:#9fb3cc;">${_titleCaseType(key)} (${count.toLocaleString()})</label>
    `;
    const cb = row.querySelector('input');
    cb.addEventListener('change', () => {
      data.mesh.visible = cb.checked;
      refreshMinimapOptions();
    });
    list.appendChild(row);
  }

  btnEnableAll.onclick = () => {
    Object.values(S.rendinstCategoryData).forEach(d => { d.mesh.visible = true; });
    list.querySelectorAll('input[type="checkbox"]').forEach(cb => { cb.checked = true; });
    refreshMinimapOptions();
  };
  btnDisableAll.onclick = () => {
    Object.values(S.rendinstCategoryData).forEach(d => { d.mesh.visible = false; });
    list.querySelectorAll('input[type="checkbox"]').forEach(cb => { cb.checked = false; });
    refreshMinimapOptions();
  };

  btnRealObjects.textContent = S.rendinstRealLoaded
    ? 'Real 3D Objects Loaded'
    : 'Load Real 3D Objects (experimental)';
  btnRealObjects.disabled = S.rendinstRealLoaded;
  btnRealObjects.onclick = _loadRealObjectsExperimental;
}

async function _loadRealObjectsExperimental() {
  if (S.rendinstRealLoaded || !S.rendinstGroup || !S.rendinstCategoryData) return;
  const btn = document.getElementById('btn-real-objects');
  if (btn) {
    btn.disabled = true;
    btn.textContent = 'Loading real objects...';
  }
  try {
    const res = await fetch(`${DATA}/rendinst_models.json`);
    if (!res.ok) {
      if (btn) {
        btn.disabled = false;
        btn.textContent = 'Load Real 3D Objects (experimental)';
      }
      console.warn('Real 3D objects manifest not found at viewer_data/rendinst_models.json');
      return;
    }

    const manifest = await res.json();
    const models = manifest && manifest.models ? manifest.models : manifest;
    const loader = new GLTFLoader();

    for (const [cat, data] of Object.entries(S.rendinstCategoryData)) {
      const entry = models ? models[cat] : null;
      if (!entry) continue;
      const rel = typeof entry === 'string' ? entry : entry.file;
      if (!rel) continue;
      const yOff = (entry && Number.isFinite(entry.yOff)) ? entry.yOff : data.yOff;

      const gltf = await new Promise((resolve, reject) => {
        loader.load(`${DATA}/${rel}`, resolve, undefined, reject);
      });
      let srcMesh = null;
      gltf.scene.traverse((n) => {
        if (!srcMesh && n.isMesh && n.geometry) srcMesh = n;
      });
      if (!srcMesh) continue;

      const count = Math.floor((data.flat?.length || 0) / 3);
      const mat = srcMesh.material && srcMesh.material.clone ? srcMesh.material.clone() : new THREE.MeshLambertMaterial({ color: 0xaaaaaa });
      const inst = new THREE.InstancedMesh(srcMesh.geometry, mat, count);
      inst.frustumCulled = false;
      const arr = inst.instanceMatrix.array;
      for (let i = 0; i < count; i++) {
        const off = i * 16;
        const fi = i * 3;
        arr[off] = 1; arr[off + 5] = 1; arr[off + 10] = 1; arr[off + 15] = 1;
        arr[off + 12] = data.flat[fi];
        arr[off + 13] = data.flat[fi + 2] * (S.displacedMeshes[0]?.material?.displacementScale || 1) + yOff;
        arr[off + 14] = data.flat[fi + 1];
      }
      inst.instanceMatrix.needsUpdate = true;
      inst.visible = data.mesh.visible;

      S.rendinstGroup.remove(data.mesh);
      if (data.mesh.geometry) data.mesh.geometry.dispose();
      if (data.mesh.material) data.mesh.material.dispose();
      data.mesh = inst;
      data.yOff = yOff;
      S.rendinstGroup.add(inst);
    }

    S.rendinstRealLoaded = true;
    _buildObjectTypesUI();
    console.info('Real 3D objects loaded for available categories.');
  } catch (e) {
    console.warn('Real 3D object loading failed:', e);
    if (btn) {
      btn.disabled = false;
      btn.textContent = 'Load Real 3D Objects (experimental)';
    }
  }
}

export function buildUI() {
  const M = S.M;
  // Title
  const title = M.mapName.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
  document.title = `War Thunder Map Viewer \u2014 ${title}`;
  document.getElementById('map-title').textContent = title;

  const loc = M.location;
  if (loc && loc.latitude != null && loc.longitude != null) {
    const lat = parseFloat(loc.latitude), lon = parseFloat(loc.longitude);
    const latStr = lat >= 0 ? `${Math.abs(lat).toFixed(0)}\u00b0N` : `${Math.abs(lat).toFixed(0)}\u00b0S`;
    const lonStr = lon >= 0 ? `${Math.abs(lon).toFixed(0)}\u00b0E` : `${Math.abs(lon).toFixed(0)}\u00b0W`;
    document.getElementById('map-subtitle').textContent = `${latStr} ${lonStr}`;
    initEarthGlobe(lat, lon);
  }

  // Coord info
  document.getElementById('coord-info').innerHTML = `
    <div>World: <span class="v">${M.mapCoord0[0]}, ${M.mapCoord0[1]}</span>
    to <span class="v">${M.mapCoord1[0]}, ${M.mapCoord1[1]}</span></div>
    <div>Size: <span class="v">${M.mapSize[0]} &times; ${M.mapSize[1]}</span> units</div>
    <div>Water: <span class="v">${M.waterLevel}</span>${M.averageGroundLevel != null ? ` (ground: ${M.averageGroundLevel})` : ''}</div>
    ${M.customLevelMap ? `<div>Tac-map: <span class="v">${M.customLevelMap}</span></div>` : ''}
    ${M.rendinst ? `<div>Objects: <span class="v">${M.rendinst.instanceCount.toLocaleString()} (${M.rendinst.poolCount} types)</span></div>` : ''}
  `;

  // Landclass list with hover tooltip
  const lcHeader = document.getElementById('h2-landclasses');
  const lcDiv = document.getElementById('landclasses');
  const lcTooltip = document.getElementById('lc-tooltip');
  lcHeader.style.display = 'none';
  lcDiv.style.display = 'none';
  lcTooltip.style.display = 'none';

  const _allMats = M.materials || [];
  function findMatImg(pattern) {
    if (!pattern || _allMats.length === 0) return null;
    const p = pattern.replace(/\*$/, '').replace(/_tex_[dr]$/, '').toLowerCase();
    if (!p) return null;
    const m = _allMats.find(x => x.file.toLowerCase().includes(p));
    return m ? `${DATA}/${m.file}` : null;
  }

  if (M.landclasses && M.landclasses.length > 0) {
    // Check if any landclass has a thumbnail available
    let hasAnyThumbnail = false;
    for (const lc of M.landclasses) {
      let thumbSrc = findMatImg(lc.texture);
      if (!thumbSrc) thumbSrc = findMatImg(lc.name);
      if (!thumbSrc) thumbSrc = lc.thumbnailPatch ? `${DATA}/${lc.thumbnailPatch}` : null;
      if (thumbSrc) {
        hasAnyThumbnail = true;
        break;
      }
    }

    // Only show LC panel if thumbnails are available
    if (hasAnyThumbnail) {
      lcHeader.style.display = '';
      lcDiv.style.cssText = 'display:flex;flex-direction:column;gap:5px;max-height:45vh;overflow-y:auto;';
    const lcSorted = [...M.landclasses].sort((a, b) => (a.name || '').localeCompare(b.name || ''));
    lcSorted.forEach((lc, lcIdx) => {
      const el = document.createElement('div');
      el.style.cssText = `display:flex;align-items:center;gap:8px;font-size:11px;padding:6px 8px;
        background:rgba(30,58,95,0.2);border-radius:4px;cursor:default;
        border:1px solid #1e3a5f;transition:background 0.15s;`;
      const name = lc.name.replace(/_/g, ' ');
      const details = lc.details || {};
      const compKeys = Object.keys(details).filter(k => !k.endsWith('_r'));
      let thumbSrc = findMatImg(lc.texture);
      if (!thumbSrc) thumbSrc = findMatImg(lc.name);
      if (!thumbSrc) thumbSrc = lc.thumbnailPatch ? `${DATA}/${lc.thumbnailPatch}` : null;
      const thumbHtml = thumbSrc
        ? `<img src="${thumbSrc}" style="width:48px;height:48px;min-width:48px;flex-shrink:0;border-radius:4px;border:1px solid #1e3a5f;object-fit:cover;">`
        : `<div style="width:48px;height:48px;min-width:48px;flex-shrink:0;border-radius:4px;border:1px solid #1e3a5f;background:rgba(30,58,95,0.4);display:flex;align-items:center;justify-content:center;">
             <span style="font-size:16px;color:#3a5a7a;">&#9632;</span></div>`;
      el.innerHTML = `${thumbHtml}<div style="flex:1;min-width:0;overflow:hidden;">
        <div style="display:flex;align-items:center;gap:6px;min-width:0;">
          <span style="font-size:9px;color:#667;flex-shrink:0;">#${lcIdx + 1}</span>
          <div style="color:#aac;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;">${name}</div>
        </div>
        <div style="font-size:9px;color:#667;margin-top:2px;">${compKeys.length} detail component${compKeys.length !== 1 ? 's' : ''}</div>
      </div>`;
      el.addEventListener('mouseenter', () => {
        el.style.background = 'rgba(40,75,120,0.45)';
        let html = '';
        if (thumbSrc) html += `<img src="${thumbSrc}" class="tt-img" alt="">`;
        html += `<div class="tt-title">${name}</div>`;
        if (lc.texture) html += `<div class="tt-tex">${lc.texture}</div>`;
        if (lc.mainTexture) html += `<div style="font-size:10px;color:#6a8;margin-top:2px;">Main: ${lc.mainTexture}</div>`;
        if (lc.detailTexture && lc.detailTexture !== lc.mainTexture) html += `<div style="font-size:10px;color:#8ad;">Detail: ${lc.detailTexture}</div>`;
        if (lc.size) html += `<div style="font-size:10px;color:#667;margin-bottom:6px;">Tile size: ${lc.size[0]}\u00d7${lc.size[1]}m</div>`;
        if (compKeys.length > 0) {
          html += '<hr class="tt-sep">';
          html += '<div style="font-size:11px;color:#889;margin-bottom:6px;">Detail components</div>';
          for (const k of compKeys) {
            const texName = details[k];
            const shortName = texName.replace(/_tex_d$/, '').replace(/^detail_/, '');
            const compImg = findMatImg(texName) || findMatImg(shortName);
            html += `<div class="tt-comp" style="margin-bottom:5px;">`;
            if (compImg) {
              html += `<img src="${compImg}" style="width:40px;height:40px;border-radius:4px;border:1px solid #1e3a5f;object-fit:cover;flex-shrink:0;">`;
            }
            html += `<span class="tt-cn" style="${compImg ? '' : 'color:#778;'}">${shortName}</span>`;
            html += `</div>`;
          }
        }
        if (lc.detailSizes && lc.detailSizes.length > 0) {
          html += `<div style="font-size:10px;color:#667;margin-top:8px;">Detail scale: ${lc.detailSizes.join(' / ')}m</div>`;
        }
        lcTooltip.innerHTML = html;
        lcTooltip.style.display = 'block';
        const rect = el.getBoundingClientRect();
        const ttH = lcTooltip.offsetHeight;
        const maxTop = window.innerHeight - ttH - 16;
        lcTooltip.style.top = Math.max(8, Math.min(rect.top, maxTop)) + 'px';
      });
      el.addEventListener('mouseleave', () => {
        el.style.background = 'rgba(30,58,95,0.2)';
        lcTooltip.style.display = 'none';
      });
      lcDiv.appendChild(el);
    });
    }
  }

  // Materials as thumbnail gallery
  const matHeader = document.getElementById('h2-materials');
  const matDiv = document.getElementById('materials');
  const matTooltip = document.getElementById('mat-tooltip');
  matHeader.style.display = 'none';
  matDiv.style.display = 'none';
  matTooltip.style.display = 'none';
  if (M.materials && M.materials.length > 0) {
    matHeader.style.display = '';
    matDiv.style.cssText = 'display:flex;flex-wrap:wrap;gap:6px;';
    M.materials.forEach((mInfo) => {
      const thumb = document.createElement('div');
      thumb.style.cssText = 'width:56px;text-align:center;cursor:pointer;';
      thumb.title = `${mInfo.name}\n${mInfo.width}x${mInfo.height}`;
      thumb.innerHTML = `
        <img src="${DATA}/${mInfo.file}" style="width:56px;height:56px;border-radius:4px;border:1px solid #1e3a5f;object-fit:cover;">
        <div style="font-size:9px;color:#667;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:56px;">${mInfo.name}</div>
      `;
      thumb.addEventListener('mouseenter', () => {
        matTooltip.innerHTML = `
          <img src="${DATA}/${mInfo.file}" alt="">
          <div class="mt-name">${mInfo.name}</div>
          <div class="mt-dim">${mInfo.width}\u00d7${mInfo.height}</div>
        `;
        const rect = thumb.getBoundingClientRect();
        matTooltip.style.top = Math.max(8, Math.min(rect.top, window.innerHeight - 300)) + 'px';
        matTooltip.style.display = 'block';
      });
      thumb.addEventListener('mouseleave', () => { matTooltip.style.display = 'none'; });
      matDiv.appendChild(thumb);
    });
  }

  // Layer toggles
  if (S.terrainMesh) {
    const cb = document.getElementById('cb-colormap');
    const sl = document.getElementById('sl-colormap');
    cb.addEventListener('change', () => { S.terrainMesh.visible = cb.checked; });
    sl.addEventListener('input', () => { S.terrainMesh.material.opacity = sl.value / 100; });
  }

  if (S.tileGridMesh) {
    const cb = document.getElementById('cb-tilegrid');
    const sl = document.getElementById('sl-tilegrid');
    cb.addEventListener('change', () => {
      S.tileGridMesh.visible = cb.checked;
      if (S.detailTexMesh) {
        const splatOn = S.splatmapMesh && document.getElementById('cb-splatmap').checked;
        S.detailTexMesh.visible = cb.checked && !splatOn;
      }
    });
    sl.addEventListener('input', () => {
      S.tileGridMesh.material.opacity = sl.value / 100;
      if (S.detailTexMesh) S.detailTexMesh.material.opacity = sl.value / 100;
    });
  }

  if (S.heightmapMesh) {
    const cb = document.getElementById('cb-heightmap');
    const sl = document.getElementById('sl-heightmap');
    cb.addEventListener('change', () => {
      S.heightmapMesh.visible = cb.checked;
      if (S.detailMesh) S.detailMesh.visible = cb.checked;
    });
    sl.addEventListener('input', () => {
      S.heightmapMesh.material.opacity = sl.value / 100;
      if (S.detailMesh) S.detailMesh.material.opacity = sl.value / 100;
    });
  }

  {
    const cb = document.getElementById('cb-water');
    const sl = document.getElementById('sl-water');
    cb.checked = !!S.waterLikelyOcean;
    cb.addEventListener('change', () => {
      S.waterMesh.visible = cb.checked;
      refreshMinimapOptions();
      S.needsRender = true;
    });
    sl.addEventListener('input', () => {
      S.waterMesh.material.opacity = sl.value / 100;
      refreshMinimapOptions();
      S.needsRender = true;
    });
    S.waterMesh.visible = cb.checked;
  }

  if (S.tankZoneMesh) {
    document.getElementById('row-tankzone').style.display = '';
    const cb = document.getElementById('cb-tankzone');
    cb.addEventListener('change', () => {
      S.tankZoneMesh.visible = cb.checked;
      refreshMinimapOptions();
    });
  }

  if (S.splatmapMesh) {
    const cb = document.getElementById('cb-splatmap');
    const sl = document.getElementById('sl-splatmap');
    cb.addEventListener('change', () => {
      S.splatmapMesh.visible = cb.checked;
      if (S.detailTexMesh) {
        const texOn = S.tileGridMesh && document.getElementById('cb-tilegrid').checked;
        S.detailTexMesh.visible = texOn && !cb.checked;
      }
    });
    sl.addEventListener('input', () => { S.splatmapMesh.material.opacity = sl.value / 100; });
  }

  if (S.rendinstGroup) {
    document.getElementById('row-rendinst').style.display = '';
    const cb = document.getElementById('cb-rendinst');
    const sl = document.getElementById('sl-rendinst');
    cb.addEventListener('change', () => {
      S.rendinstGroup.visible = cb.checked;
      refreshMinimapOptions();
    });
    sl.addEventListener('input', () => {
      const op = sl.value / 100;
      S.rendinstGroup.children.forEach(m => {
        m.material.opacity = op;
        m.material.transparent = op < 1;
        m.material.needsUpdate = true;
      });
      refreshMinimapOptions();
    });
    _buildObjectTypesUI();
  }

  // Height scale
  {
    const sl = document.getElementById('sl-height');
    const val = document.getElementById('val-height');
    sl.addEventListener('input', () => {
      val.textContent = sl.value + '%';
      setHeightScale(parseFloat(sl.value));
    });
  }
}
