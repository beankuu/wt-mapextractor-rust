import * as THREE from 'three';
import { S } from './state.js';
import { buildBoxWireframeVerts } from './helpers.js';
import { renderMission } from './mission-overlay.js';

export function setHeightScale(pct) {
  const M = S.M;
  const scale = (pct / 100) * S.maxHeightScale;
  for (const mesh of S.displacedMeshes) {
    if (mesh.material.displacementMap) {
      if (mesh.userData.hm2) {
        const { lrRange, hm2Range, hm2MinOffset } = mesh.userData.hm2;
        mesh.material.displacementScale = (hm2Range / lrRange) * scale;
        mesh.material.displacementBias = (hm2MinOffset / lrRange) * scale;
      } else {
        mesh.material.displacementScale = scale;
      }
      mesh.material.needsUpdate = true;
    }
  }
  if (S.waterMesh) S.waterMesh.position.y = scale * S.waterFraction;
  S.controls.target.y = scale * S.waterFraction;
  // Update tank zone box
  if (S.tankZoneMesh && S.tankZoneMesh.userData.yBotFrac != null) {
    const { x0, x1, z0, z1, yBotFrac, yTopFrac } = S.tankZoneMesh.userData;
    const yBot = yBotFrac * scale, yTop = yTopFrac * scale;
    const tzWire = S.tankZoneMesh.children[0];
    const pos = tzWire.geometry.attributes.position;
    pos.array.set(buildBoxWireframeVerts(x0, x1, yBot, yTop, z0, z1));
    pos.needsUpdate = true;
    const tzSolid = S.tankZoneMesh.children[1];
    tzSolid.geometry.dispose();
    tzSolid.geometry = new THREE.BoxGeometry(Math.abs(x1 - x0), yTop - yBot, Math.abs(z1 - z0));
    tzSolid.position.y = (yBot + yTop) / 2;
  }
  // Update CPU raycast mesh
  if (S.raycastMesh && S.raycastHeightNorms) {
    const positions = S.raycastMesh.geometry.attributes.position;
    for (let i = 0; i < positions.count; i++) {
      positions.setZ(i, S.raycastHeightNorms[i] * scale);
    }
    positions.needsUpdate = true;
    S.raycastMesh.geometry.boundingSphere = null;
    S.raycastMesh.geometry.computeBoundingSphere();
  }
  // Update CPU detail raycast mesh
  if (S.detailRaycastMesh && S.detailRaycastNorms) {
    const hm2 = S.detailRaycastMesh.userData.hm2;
    const detScale = (hm2.hm2Range / hm2.lrRange) * scale;
    const detBias = (hm2.hm2MinOffset / hm2.lrRange) * scale;
    const positions = S.detailRaycastMesh.geometry.attributes.position;
    for (let i = 0; i < positions.count; i++) {
      positions.setZ(i, S.detailRaycastNorms[i] * detScale + detBias);
    }
    positions.needsUpdate = true;
    S.detailRaycastMesh.geometry.boundingSphere = null;
    S.detailRaycastMesh.geometry.computeBoundingSphere();
  }
  // Update render instance heights
  if (S.rendinstGroup && S.rendinstCategoryData) {
    for (const data of Object.values(S.rendinstCategoryData)) {
      const { mesh, flat, yOff } = data;
      const arr = mesh.instanceMatrix.array;
      const count = flat.length / 3;
      for (let i = 0; i < count; i++) {
        arr[i * 16 + 13] = flat[i * 3 + 2] * scale + yOff;
      }
      mesh.instanceMatrix.needsUpdate = true;
    }
    S.needsRender = true;
  }
  // Re-render active mission overlay
  if (S.missionGroup) {
    const selected = document.querySelector('input[name="mission"]:checked');
    if (selected && selected.value !== '') {
      const idx = parseInt(selected.value);
      const mode = document.querySelector('input[name="mmode"]:checked')?.value || 'arcade';
      if (S.missionData && S.missionData.missions[idx]) {
        renderMission(S.missionData.missions[idx], mode);
      }
    }
  }
}
