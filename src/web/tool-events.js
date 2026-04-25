import * as THREE from 'three';
import { Line2 } from 'three/addons/lines/Line2.js';
import { LineMaterial } from 'three/addons/lines/LineMaterial.js';
import { LineGeometry } from 'three/addons/lines/LineGeometry.js';
import { S } from './state.js';
import { sceneToWorld } from './hover.js';
import { computeLoS, clearLoS } from './webgpu-los.js';
import { createRulerTool } from './tools/ruler.js';
import { applySunSettings } from './scene-setup.js';

export let rulerTool = null;

export function setupToolEvents() {
  rulerTool = createRulerTool({
    THREE,
    Line2,
    LineMaterial,
    LineGeometry,
    scene: S.scene,
    camera: S.camera,
    getMap: () => S.M,
    getDisplacedMeshes: () => S.displacedMeshes,
    getRaycastMeshes: () => ({ detailRaycastMesh: S.detailRaycastMesh, raycastMesh: S.raycastMesh }),
    sceneToWorld,
  });

  // --- Tool activation ---
  function _setTool(tool) {
    if (S.activeTool === tool) {
      S.activeTool = null;
    } else {
      S.activeTool = tool;
    }
    document.getElementById('btn-ruler').classList.toggle('active', S.activeTool === 'ruler');
    document.getElementById('btn-los').classList.toggle('active', S.activeTool === 'los');

    // Cancel pending ruler point + preview line
    if (S.activeTool !== 'ruler' && S.rulerPending) {
      S.scene.remove(S.rulerPending.sphereA);
      S.rulerPending.sphereA.geometry.dispose();
      S.rulerPending.sphereA.material.dispose();
      if (S.rulerPending.previewLine) {
        S.scene.remove(S.rulerPending.previewLine);
        S.rulerPending.previewLine.geometry.dispose();
        S.rulerPending.previewLine.material.dispose();
      }
      S.rulerPending = null;
    }

    // Cursor style
    S.canvas.style.cursor = S.activeTool ? 'crosshair' : '';
    // Re-enable orbit when no tool
    S.controls.enableRotate = !S.activeTool || S.activeTool === null;
    S.controls.enablePan = !S.activeTool || S.activeTool === null;
  }

  document.getElementById('btn-ruler').addEventListener('click', () => _setTool('ruler'));
  document.getElementById('btn-los').addEventListener('click', () => _setTool('los'));
  document.getElementById('btn-tile').addEventListener('click', () => {
    S.tileToolActive = !S.tileToolActive;
    document.getElementById('btn-tile').classList.toggle('active', S.tileToolActive);
    if (!S.tileToolActive && S.tileHighlightGroup) S.tileHighlightGroup.visible = false;
  });

  // --- Double-click to remove rulers / LoS ---
  S.canvas.addEventListener('dblclick', (e) => {
    const near = rulerTool.findNearestRulerHandle(e.clientX, e.clientY);
    if (near) {
      rulerTool.removeRuler(near.rulerIdx);
      return;
    }
    if (S.losGroup) {
      clearLoS();
    }
  });

  // --- Mouse events for tools ---
  S.canvas.addEventListener('pointerdown', (e) => {
    if (e.button !== 0 || !S.activeTool) return;

    if (S.activeTool === 'ruler') {
      const near = rulerTool.findNearestRulerHandle(e.clientX, e.clientY);
      if (near) {
        S.rulerDragging = near;
        S.controls.enableRotate = false;
        S.controls.enablePan = false;
        return;
      }
      const hit = rulerTool.hitTerrain(e.clientX, e.clientY);
      if (!hit) return;

      if (!S.rulerPending) {
        const planeSize = Math.max(S.M.mapSize[0], S.M.mapSize[1]);
        const handleSize = planeSize * 0.0003;
        const sphere = rulerTool.createHandle(0xff4444);
        sphere.position.copy(hit);
        sphere.scale.setScalar(handleSize);
        S.scene.add(sphere);
        const prevMat = new LineMaterial({ color: 0x4dabf7, linewidth: 3, depthTest: false, transparent: true, opacity: 0.6, dashed: true, dashSize: 3, gapSize: 2, worldUnits: false });
        prevMat.resolution.set(window.innerWidth, window.innerHeight);
        const prevGeo = new LineGeometry();
        prevGeo.setPositions([hit.x, hit.y, hit.z, hit.x, hit.y, hit.z]);
        const prevLine = new Line2(prevGeo, prevMat);
        prevLine.computeLineDistances();
        prevLine.renderOrder = 1000;
        prevLine.frustumCulled = false;
        S.scene.add(prevLine);
        S.rulerPending = { pt: hit, sphereA: sphere, previewLine: prevLine };
      } else {
        S.scene.remove(S.rulerPending.sphereA);
        S.rulerPending.sphereA.geometry.dispose();
        S.rulerPending.sphereA.material.dispose();
        if (S.rulerPending.previewLine) {
          S.scene.remove(S.rulerPending.previewLine);
          S.rulerPending.previewLine.geometry.dispose();
          S.rulerPending.previewLine.material.dispose();
        }
        rulerTool.makeRuler(S.rulerPending.pt, hit);
        S.rulerPending = null;
      }
    } else if (S.activeTool === 'los') {
      const hit = rulerTool.hitTerrain(e.clientX, e.clientY);
      if (hit) computeLoS(hit);
    }
  });

  S.canvas.addEventListener('pointermove', (e) => {
    if (S.rulerDragging) {
      const hit = rulerTool.hitTerrain(e.clientX, e.clientY);
      if (hit) rulerTool.updateRulerEndpoint(S.rulerDragging.ruler, S.rulerDragging.handle, hit);
    }
    if (S.activeTool === 'ruler' && S.rulerPending && S.rulerPending.previewLine) {
      const hit = rulerTool.hitTerrain(e.clientX, e.clientY);
      if (hit) {
        const startPt = S.rulerPending.pt;
        S.rulerPending.previewLine.geometry.setPositions([
          startPt.x, startPt.y, startPt.z,
          hit.x, hit.y, hit.z
        ]);
        S.rulerPending.previewLine.computeLineDistances();
      }
    }
  });

  S.canvas.addEventListener('pointerup', (e) => {
    if (S.rulerDragging) {
      S.rulerDragging = null;
      if (S.activeTool === 'ruler') {
        S.controls.enableRotate = false;
        S.controls.enablePan = false;
      }
    }
  });

  // ESC to deactivate tool
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      _setTool(null);
      S.controls.enableRotate = true;
      S.controls.enablePan = true;
    }
  });

  // Panel toggle
  const panel = document.getElementById('panel');
  const toggleBtn = document.getElementById('toggle-panel');
  const sunWidget = document.getElementById('sun-widget');
  toggleBtn.addEventListener('click', () => {
    panel.classList.toggle('collapsed');
    toggleBtn.classList.toggle('shifted');
    if (sunWidget) sunWidget.classList.toggle('shifted');
  });

  setupSunWidget();
}

function setupSunWidget() {
  const canvas = document.getElementById('sun-hemisphere');
  const slider = document.getElementById('sl-sun-strength');
  const values = document.getElementById('sun-values');
  if (!canvas || !slider || !values) return;

  const M = S.M || {};
  const sunCfg = M.sun || { azimuth: 30, elevation: 50, strength: 0.5 };
  let az = Number.isFinite(sunCfg.azimuth) ? sunCfg.azimuth : 30;
  let el = Number.isFinite(sunCfg.elevation) ? sunCfg.elevation : 50;
  let st = Number.isFinite(sunCfg.strength) ? sunCfg.strength : 0.5;

  const ctx = canvas.getContext('2d');
  let dragging = false;

  function clamp(v, min, max) {
    return Math.max(min, Math.min(max, v));
  }

  function syncUI() {
    slider.value = Math.round(clamp(st, 0, 1) * 100);
    values.textContent = `Az ${Math.round(((az % 360) + 360) % 360)}° · El ${Math.round(clamp(el, 0, 90))}° · Str ${st.toFixed(2)}`;
  }

  function draw() {
    const w = canvas.width;
    const h = canvas.height;
    const cx = w * 0.5;
    const cy = h - 10;
    const r = Math.min(w * 0.42, h - 16);

    ctx.clearRect(0, 0, w, h);

    // Dome fill and rim
    const domeGrad = ctx.createLinearGradient(0, cy - r, 0, cy + 2);
    domeGrad.addColorStop(0, 'rgba(112, 165, 229, 0.18)');
    domeGrad.addColorStop(1, 'rgba(43, 76, 114, 0.10)');
    ctx.beginPath();
    ctx.moveTo(cx - r, cy);
    ctx.arc(cx, cy, r, Math.PI, 0, false);
    ctx.closePath();
    ctx.fillStyle = domeGrad;
    ctx.fill();
    ctx.strokeStyle = 'rgba(110, 160, 220, 0.75)';
    ctx.lineWidth = 1.2;
    ctx.beginPath();
    ctx.arc(cx, cy, r, Math.PI, 0, false);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(cx - r, cy);
    ctx.lineTo(cx + r, cy);
    ctx.strokeStyle = 'rgba(110, 160, 220, 0.35)';
    ctx.stroke();

    // Helper meridian lines
    ctx.strokeStyle = 'rgba(110, 160, 220, 0.22)';
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.lineTo(cx, cy - r);
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(cx, cy, r * 0.62, Math.PI, 0, false);
    ctx.stroke();

    // Globe-like meridians
    for (const m of [-0.6, -0.3, 0.3, 0.6]) {
      ctx.beginPath();
      for (let i = 0; i <= 40; i += 1) {
        const t = i / 40;
        const yy = t;
        const xx = m * Math.sqrt(Math.max(0, 1 - yy * yy));
        const gx = cx + xx * r;
        const gy = cy - yy * r;
        if (i === 0) ctx.moveTo(gx, gy);
        else ctx.lineTo(gx, gy);
      }
      ctx.stroke();
    }

    // Dot mapping via directional unit vector projected onto hemisphere.
    const azRad = (((az % 360) + 360) % 360) * Math.PI / 180;
    const elRad = clamp(el, 0, 90) * Math.PI / 180;
    const vx = Math.sin(azRad) * Math.cos(elRad);
    const vy = Math.sin(elRad);
    const px = cx + vx * r;
    const py = cy - vy * r;
    const sunGlow = ctx.createRadialGradient(px, py, 0, px, py, 7);
    sunGlow.addColorStop(0, 'rgba(255, 208, 112, 0.95)');
    sunGlow.addColorStop(1, 'rgba(255, 165, 68, 0.18)');
    ctx.fillStyle = sunGlow;
    ctx.beginPath();
    ctx.arc(px, py, 7, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = 'rgba(255, 181, 84, 1)';
    ctx.beginPath();
    ctx.arc(px, py, 3.3, 0, Math.PI * 2);
    ctx.fill();
  }

  function applySun() {
    applySunSettings({ azimuth: az, elevation: el, strength: st });
    S.needsRender = true;
    syncUI();
    draw();
  }

  function updateFromPointer(evt) {
    const rect = canvas.getBoundingClientRect();
    const w = canvas.width;
    const h = canvas.height;
    const cx = w * 0.5;
    const cy = h - 10;
    const r = Math.min(w * 0.42, h - 16);

    let lx = ((evt.clientX - rect.left) / rect.width) * w;
    let ly = ((evt.clientY - rect.top) / rect.height) * h;

    // Clamp pointer to dome area (upper hemisphere only).
    ly = clamp(ly, 0, cy);
    let dx = lx - cx;
    let dy = cy - ly;
    const len = Math.sqrt(dx * dx + dy * dy);
    if (len > r) {
      dx = (dx / len) * r;
      dy = (dy / len) * r;
    }

    const nx = dx / r;
    const ny = clamp(dy / r, 0, 1);
    // Orthographic hemisphere inversion.
    const nz = Math.sqrt(Math.max(0, 1 - nx * nx - ny * ny));

    az = (Math.atan2(nx, nz) * 180 / Math.PI + 360) % 360;
    el = Math.asin(ny) * 180 / Math.PI;
    applySun();
  }

  canvas.addEventListener('pointerdown', (evt) => {
    dragging = true;
    canvas.setPointerCapture(evt.pointerId);
    updateFromPointer(evt);
  });
  canvas.addEventListener('pointermove', (evt) => {
    if (!dragging) return;
    updateFromPointer(evt);
  });
  canvas.addEventListener('pointerup', (evt) => {
    dragging = false;
    canvas.releasePointerCapture(evt.pointerId);
  });

  slider.addEventListener('input', () => {
    st = clamp((Number(slider.value) || 0) / 100, 0, 1);
    applySun();
  });

  // Ensure widget position mirrors current panel state.
  const panel = document.getElementById('panel');
  const sunWidget = document.getElementById('sun-widget');
  if (panel && sunWidget && !panel.classList.contains('collapsed')) {
    sunWidget.classList.add('shifted');
  }

  applySun();
}
