import { S } from './state.js';
import { DATA, progress } from './helpers.js';
import { setupScene, syncZoomSlider } from './scene-setup.js';
import { loadTerrain, clampCameraAboveTerrain } from './terrain-init.js';
import { buildUI } from './ui.js';
import { buildMissionsUI } from './mission-overlay.js';
import { setupHover, updateHoverCoords } from './hover.js';
import { setupGizmo } from './gizmo.js';
import { rulerTool, setupToolEvents } from './tool-events.js';
import { updateLoSVisuals } from './webgpu-los.js';
import { initMinimap } from './minimap.js';

async function init() {
  setupScene();

  window.addEventListener('terrain-intro-complete', () => {
    document.body.classList.add('ui-ready');
    const panel = document.getElementById('panel');
    const toggleBtn = document.getElementById('toggle-panel');
    const sunWidget = document.getElementById('sun-widget');
    if (panel && panel.classList.contains('collapsed')) {
      panel.classList.remove('collapsed');
      if (toggleBtn) toggleBtn.classList.add('shifted');
      if (sunWidget) sunWidget.classList.add('shifted');
    }
    S.needsRender = true;
  }, { once: true });

  // Mark a render needed whenever the scene changes; also clamp camera above terrain
  S.needsRender = true;
  S.controls.addEventListener('change', () => {
    S.needsRender = true;
    clampCameraAboveTerrain();
  });

  // Temporarily reduce pixel ratio while dragging to cut rasterization cost
  S.canvas.addEventListener('pointerdown', () => {
    S.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1));
  });
  document.addEventListener('pointerup', () => {
    S.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 1.5));
    S.needsRender = true;
  });
  // Tool clicks (ruler, LoS) add meshes to the scene
  S.canvas.addEventListener('click', () => { S.needsRender = true; });
  S.canvas.addEventListener('dblclick', () => { S.needsRender = true; });

  await loadTerrain();
  syncZoomSlider();
  buildUI();
  initMinimap();

  try {
    if (S.M && S.M.hasMissions) {
      const res = await fetch(`${DATA}/missions.json`);
      if (res.ok) {
        S.missionData = await res.json();
        buildMissionsUI();
      }
    }
  } catch { /* no missions available */ }

  setupHover();
  setupToolEvents();
  setupGizmo();

  progress(100, 'Done');
  setTimeout(() => {
    document.getElementById('loading').style.display = 'none';
  }, 400);

  animate();
}

let _hoverTick = 0;

function animate() {
  requestAnimationFrame(animate);
  const controlsMoved = S.controls.update();
  if (controlsMoved) S.needsRender = true;
  syncZoomSlider();
  if (!controlsMoved) {
    // Hover raycasts are expensive on dense terrain; half-rate is enough for UX.
    _hoverTick = (_hoverTick + 1) & 1;
    if (_hoverTick === 0) updateHoverCoords();
  }
  if (rulerTool) rulerTool.updateRulerLabels();
  updateLoSVisuals();

  if (!S.needsRender) return;
  S.needsRender = false;

  // Gizmo camera sync
  S.gizmoCamera.position
    .copy(S.camera.position)
    .sub(S.controls.target)
    .normalize()
    .multiplyScalar(3.2);
  S.gizmoCamera.lookAt(0, 0, 0);
  S.gizmoRoot.quaternion.identity();

  S.renderer.render(S.scene, S.camera);
  S.gizmoRenderer.render(S.gizmoScene, S.gizmoCamera);
}

window.addEventListener('resize', () => {
  const w = window.innerWidth, h = window.innerHeight;
  S.camera.aspect = w / h;
  S.camera.updateProjectionMatrix();
  S.renderer.setSize(w, h);
  if (rulerTool) rulerTool.updateLineMaterialResolution(w, h);
  if (S.rulerPending && S.rulerPending.previewLine && S.rulerPending.previewLine.material.resolution) {
    S.rulerPending.previewLine.material.resolution.set(w, h);
  }
  S.needsRender = true;
});

init();
