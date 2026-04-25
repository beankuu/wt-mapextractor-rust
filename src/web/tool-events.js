import * as THREE from 'three';
import { Line2 } from 'three/addons/lines/Line2.js';
import { LineMaterial } from 'three/addons/lines/LineMaterial.js';
import { LineGeometry } from 'three/addons/lines/LineGeometry.js';
import { S } from './state.js';
import { sceneToWorld } from './hover.js';
import { computeLoS, clearLoS } from './webgpu-los.js';
import { createRulerTool } from './tools/ruler.js';

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
  toggleBtn.addEventListener('click', () => {
    panel.classList.toggle('collapsed');
    toggleBtn.classList.toggle('shifted');
  });
}
