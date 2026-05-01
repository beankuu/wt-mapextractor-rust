import * as THREE from 'three';
import { S } from './state.js';
import { DATA, buildBoxWireframeVerts } from './helpers.js';
import { getMapFrame, worldToSceneXZ } from './coords.js';

// Map mission abbreviations to full names
function expandMissionName(label) {
  if (!label) return 'Mission';
  const raw = String(label).trim();
  const abbrevMap = {
    'bttl': 'Battle',
    'dbttl': 'Double Battle',
    'dbttla': 'Double Battle A',
    'dbttlb': 'Double Battle B',
    'conq': 'Conquest',
    'dom': 'Domination',
    'ctf': 'Capture the Flag',
    'as': 'Assault',
    'ash': 'Ashes',
    'dbldom': 'Double Domination',
  };
  const lower = raw.toLowerCase();
  const token = lower.replace(/^\d+[_\-\s]*/, '');

  if (abbrevMap[token]) {
    return abbrevMap[token];
  }
  const parts = token.split(/[_\-\s]+/).filter(Boolean);
  if (parts.length > 0 && abbrevMap[parts[0]]) {
    return [abbrevMap[parts[0]], ...parts.slice(1)].join(' ');
  }
  return raw;
}

export function missionWorldToScene(wx, wz) {
  const M = S.M;
  const planeSize = Math.max(M.mapSize[0], M.mapSize[1]);
  return worldToSceneXZ(M, wx, wz, planeSize);
}

function _missionSceneY(hMeters) {
  const M = S.M;
  if (!M.heightmap || M.heightmap.height_min_m == null || S.displacedMeshes.length === 0) return 0;
  const curScale = S.displacedMeshes[0].material.displacementScale || 1;
  const hMin = M.heightmap.height_min_m;
  const hRange = (M.heightmap.height_max_m - hMin) || 1;
  return ((hMeters - hMin) / hRange) * curScale;
}

export function clearMissionGroup() {
  if (S.missionGroup) {
    S.scene.remove(S.missionGroup);
    S.missionGroup.traverse(c => { if (c.geometry) c.geometry.dispose(); if (c.material) c.material.dispose(); });
    S.missionGroup = null;
  }
}

  function preferredMissionMode(items, selectedMode) {
    if (selectedMode === 'merged') {
      if (items.some((x) => x.mode === 'arcade')) return 'arcade';
      if (items.some((x) => x.mode === 'hardcore')) return 'hardcore';
      return 'arcade';
    }
    if (selectedMode !== 'hardcore') return selectedMode;
    return items.some((x) => x.mode === 'arcade') ? 'arcade' : 'hardcore';
  }

  function mergedCount(items) {
    const mode = preferredMissionMode(items || [], 'merged');
    return (items || []).filter((x) => x.mode === mode).length;
  }

export function renderMission(mission, mode) {
  clearMissionGroup();
  const M = S.M;
  S.missionGroup = new THREE.Group();
  S.missionGroup.renderOrder = 998;

  const TEAM1_COLOR = 0x00a2ff;
  const TEAM2_COLOR = 0xff2b2b;
  const CAP_COLOR   = 0xcfa400;
  const BA_COLOR    = 0x01f1ff;

    const battleMode = preferredMissionMode(mission.battleAreas || [], mode);
    const captureMode = preferredMissionMode(mission.captureZones || [], mode);
    const spawnMode = preferredMissionMode(mission.spawns || [], mode);
    const spawnZoneMode = preferredMissionMode(mission.spawnZones || [], mode);

  // Battle areas
  for (const ba of mission.battleAreas) {
    if (ba.mode !== battleMode) continue;
    const sc = missionWorldToScene(ba.pos[0], ba.pos[1]);
    const planeSize = Math.max(M.mapSize[0], M.mapSize[1]);
    const frame = getMapFrame(M, planeSize);
    const sX = frame.sX, sZ = frame.sZ;
    const halfSX = ba.halfSize[0] * sX;
    const halfSZ = ba.halfSize[1] * sZ;
    const x0 = sc.x - halfSX, x1 = sc.x + halfSX;
    const z0 = sc.z - halfSZ, z1 = sc.z + halfSZ;
    const yBot = _missionSceneY(0);
    const yTop = _missionSceneY(800);

    const wireGeom = new THREE.BufferGeometry();
    wireGeom.setAttribute('position', new THREE.BufferAttribute(
      buildBoxWireframeVerts(x0, x1, yBot, yTop, z0, z1), 3));
    const wire = new THREE.LineSegments(wireGeom, new THREE.LineBasicMaterial({
      color: BA_COLOR, depthTest: false, transparent: true, opacity: 0.35
    }));
    S.missionGroup.add(wire);

    const solid = new THREE.Mesh(
      new THREE.BoxGeometry(Math.abs(x1 - x0), yTop - yBot, Math.abs(z1 - z0)),
      new THREE.MeshBasicMaterial({
        color: BA_COLOR, transparent: true, opacity: 0.06,
        side: THREE.DoubleSide, depthWrite: false
      })
    );
    solid.position.set((x0 + x1) / 2, (yBot + yTop) / 2, (z0 + z1) / 2);
    S.missionGroup.add(solid);
  }

  // Capture zones
  for (const cap of mission.captureZones) {
    if (cap.mode !== captureMode) continue;
    const sc = missionWorldToScene(cap.pos[0], cap.pos[1]);
    const planeSize = Math.max(M.mapSize[0], M.mapSize[1]);
    const frame = getMapFrame(M, planeSize);
    const scaleR = frame.sX;
    const r = cap.radius * scaleR;
    const yBot = _missionSceneY(-10);
    const yTop = _missionSceneY(100);
    const h = yTop - yBot || 10;

    const cylGeom = new THREE.CylinderGeometry(r, r, h, 24);
    const cylMat = new THREE.MeshBasicMaterial({
      color: CAP_COLOR, transparent: true, opacity: 0.34,
      side: THREE.DoubleSide, depthWrite: false
    });
    const cyl = new THREE.Mesh(cylGeom, cylMat);
    cyl.position.set(sc.x, (yBot + yTop) / 2, sc.z);
    S.missionGroup.add(cyl);

    const ringGeom = new THREE.RingGeometry(r * 0.95, r, 32);
    const ringMat = new THREE.MeshBasicMaterial({
      color: CAP_COLOR, transparent: true, opacity: 0.9,
      side: THREE.DoubleSide, depthWrite: false
    });
    const ring = new THREE.Mesh(ringGeom, ringMat);
    ring.rotation.x = -Math.PI / 2;
    ring.position.set(sc.x, _missionSceneY(5), sc.z);
    S.missionGroup.add(ring);

    if (cap.label) {
      const cvs = document.createElement('canvas');
      cvs.width = 64; cvs.height = 64;
      const ctx = cvs.getContext('2d');
      ctx.fillStyle = 'rgba(0,0,0,0.5)';
      ctx.beginPath(); ctx.arc(32, 32, 28, 0, Math.PI * 2); ctx.fill();
      ctx.strokeStyle = '#ffd700'; ctx.lineWidth = 3;
      ctx.beginPath(); ctx.arc(32, 32, 28, 0, Math.PI * 2); ctx.stroke();
      ctx.fillStyle = '#ffd700';
      ctx.font = 'bold 36px sans-serif';
      ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
      ctx.fillText(cap.label, 32, 34);
      const tex = new THREE.CanvasTexture(cvs);
      const spriteMat = new THREE.SpriteMaterial({ map: tex, depthTest: false, transparent: true });
      const sprite = new THREE.Sprite(spriteMat);
      const spriteScale = r * 1.5 || 20;
      sprite.scale.set(spriteScale, spriteScale, 1);
      sprite.position.set(sc.x, _missionSceneY(50), sc.z);
      sprite.renderOrder = 999;
      S.missionGroup.add(sprite);
    }
  }

  // Spawn squads
  for (const sp of mission.spawns) {
    if (sp.mode !== spawnMode) continue;
    const sc = missionWorldToScene(sp.pos[0], sp.pos[1]);
    const color = sp.team === 1 ? TEAM1_COLOR : sp.team === 2 ? TEAM2_COLOR : 0x88ff88;
    const teamLabel = sp.team === 1 ? '1' : sp.team === 2 ? '2' : '?';

    const cvs = document.createElement('canvas');
    cvs.width = 64; cvs.height = 64;
    const ctx = cvs.getContext('2d');
    const hex = '#' + color.toString(16).padStart(6, '0');
    ctx.fillStyle = 'rgba(0,0,0,0.7)';
    ctx.beginPath(); ctx.arc(32, 28, 22, 0, Math.PI * 2); ctx.fill();
    ctx.fillStyle = hex; ctx.strokeStyle = hex;
    ctx.lineWidth = 3.0;
    ctx.beginPath(); ctx.arc(32, 28, 22, 0, Math.PI * 2); ctx.stroke();
    ctx.font = 'bold 26px sans-serif';
    ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
    ctx.fillText(teamLabel, 32, 30);
    ctx.beginPath(); ctx.moveTo(32, 52); ctx.lineTo(24, 44); ctx.lineTo(40, 44); ctx.closePath(); ctx.fill();

    const tex = new THREE.CanvasTexture(cvs);
    const spriteMat = new THREE.SpriteMaterial({ map: tex, depthTest: false, transparent: true });
    const sprite = new THREE.Sprite(spriteMat);
    const planeSize = Math.max(M.mapSize[0], M.mapSize[1]);
    const spriteScale = planeSize * 0.008;
    sprite.scale.set(spriteScale, spriteScale, 1);
    sprite.position.set(sc.x, _missionSceneY(40), sc.z);
    sprite.renderOrder = 999;
    S.missionGroup.add(sprite);
  }

  // Individual spawn zones
  const planeSize = Math.max(M.mapSize[0], M.mapSize[1]);
  const frame = getMapFrame(M, planeSize);
  const spawnDotR = 7 * frame.sX;
  for (const sz of mission.spawnZones) {
    if (sz.mode !== spawnZoneMode) continue;
    const sc = missionWorldToScene(sz.pos[0], sz.pos[1]);
    const color = sz.team === 1 ? TEAM1_COLOR : sz.team === 2 ? TEAM2_COLOR : 0x88ff88;
    const dotGeom = new THREE.CircleGeometry(spawnDotR, 8);
    const dotMat = new THREE.MeshBasicMaterial({
      color, transparent: true, opacity: 0.96,
      side: THREE.DoubleSide, depthWrite: false
    });
    const dot = new THREE.Mesh(dotGeom, dotMat);
    dot.rotation.x = -Math.PI / 2;
    dot.position.set(sc.x, _missionSceneY(2), sc.z);
    S.missionGroup.add(dot);
  }

  S.scene.add(S.missionGroup);
}

export function buildMissionsUI() {
  const M = S.M;
  if (!S.missionData || !S.missionData.missions || S.missionData.missions.length === 0) return;

  const section = document.getElementById('missions-section');
  const listDiv = document.getElementById('mission-list');
  const modeRow = document.getElementById('mission-mode-row');
  const legendDiv = document.getElementById('mission-legend');
  const missionActiveEl = document.getElementById('mission-active');
  section.style.display = 'block';

  // Collapse/expand toggle
  let listExpanded = false;
  const toggleBtn = document.createElement('button');
  toggleBtn.style.cssText = 'width:100%;background:#0f1f35;border:1px solid #1e3a5f;color:#4dabf7;border-radius:4px;padding:6px;cursor:pointer;font-size:11px;font-weight:600;margin-bottom:6px;transition:background 0.15s;';
  toggleBtn.textContent = '▸ Missions';
  toggleBtn.addEventListener('mouseover', () => { toggleBtn.style.background = 'rgba(30, 58, 95, 0.5)'; });
  toggleBtn.addEventListener('mouseout', () => { toggleBtn.style.background = '#0f1f35'; });
  toggleBtn.addEventListener('click', () => {
    listExpanded = !listExpanded;
    toggleBtn.textContent = (listExpanded ? '▾' : '▸') + ' Missions';
    listDiv.style.display = listExpanded ? 'flex' : 'none';
  });
  
  listDiv.style.display = 'none';
  listDiv.style.flexDirection = 'column';
  listDiv.style.gap = '4px';

  // Insert toggle before mission list
  section.insertBefore(toggleBtn, missionActiveEl);

  const missions = S.missionData.missions;

  function updateMissionActiveText() {
    const selected = document.querySelector('input[name="mission"]:checked');
    const mode = 'merged';
    if (!selected || selected.value === '') {
      missionActiveEl.textContent = 'Active: None';
      return;
    }
    const idx = parseInt(selected.value);
    const label = missions[idx] && missions[idx].label ? expandMissionName(missions[idx].label) : `Mission ${idx + 1}`;
    missionActiveEl.textContent = `Active: ${label}`;
  }

  const noneRow = document.createElement('div');
  noneRow.className = 'mission-radio-row';
  noneRow.innerHTML = `
    <input type="radio" name="mission" id="mission-none" value="" checked>
    <label for="mission-none" style="color:#667;">None</label>
  `;
  listDiv.appendChild(noneRow);

  missions.forEach((m, i) => {
    const row = document.createElement('div');
    row.className = 'mission-radio-row';
    const id = `mission-${i}`;
    const capCount = mergedCount(m.captureZones);
    const missionName = expandMissionName(m.label);
    row.innerHTML = `
      <input type="radio" name="mission" id="${id}" value="${i}">
      <label for="${id}">${missionName}</label>
      <span class="mission-count">${capCount > 0 ? capCount + ' cap' : ''}</span>
    `;
    listDiv.appendChild(row);
  });

  modeRow.style.display = 'none';
  legendDiv.style.display = 'flex';
  legendDiv.innerHTML = '<span style="color:#00a2ff;">●</span>&nbsp;Team 1&nbsp;&nbsp;<span style="color:#ff2b2b;">●</span>&nbsp;Team 2&nbsp;&nbsp;<span style="color:#ffc400;">●</span>&nbsp;Capture&nbsp;&nbsp;<span style="color:#00f0ff;">●</span>&nbsp;Battle Area';

  function updateMissionOverlay() {
    const selected = document.querySelector('input[name="mission"]:checked');
    if (!selected || selected.value === '') {
      clearMissionGroup();
      updateMissionActiveText();
      return;
    }
    const idx = parseInt(selected.value);
    const mode = 'merged';
    renderMission(missions[idx], mode);
    syncMissionSelectionUI();
    updateMissionActiveText();
  }

  function syncMissionSelectionUI() {
    listDiv.querySelectorAll('.mission-radio-row').forEach((row) => {
      const input = row.querySelector('input[name="mission"]');
      row.classList.toggle('selected', !!(input && input.checked && input.value !== ''));
    });
  }

  listDiv.querySelectorAll('input[name="mission"]').forEach(r => {
    r.addEventListener('change', updateMissionOverlay);
  });
  listDiv.querySelectorAll('.mission-radio-row').forEach((row) => {
    row.addEventListener('click', () => {
      const input = row.querySelector('input[name="mission"]');
      if (!input) return;
      input.checked = true;
      updateMissionOverlay();
    });
  });
  syncMissionSelectionUI();
  updateMissionActiveText();
}
