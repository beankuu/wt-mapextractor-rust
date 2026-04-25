import * as THREE from 'three';
import { S } from './state.js';

export function setupGizmo() {
  const gizmoCanvas = document.getElementById('gizmo-canvas');
  S.gizmoRenderer = new THREE.WebGLRenderer({ canvas: gizmoCanvas, alpha: true, antialias: true });
  S.gizmoRenderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  S.gizmoRenderer.setSize(100, 100);
  S.gizmoRenderer.setClearColor(0x000000, 0);

  S.gizmoScene = new THREE.Scene();
  S.gizmoCamera = new THREE.PerspectiveCamera(50, 1, 0.1, 10);
  S.gizmoCamera.position.set(0, 0, 3.2);

  const axisColors = { x: 0xe05050, y: 0x50c050, z: 0x5080e0 };
  const axisLabels = ['X', 'H', 'Z'];
  const axisDirs = [
    new THREE.Vector3(1, 0, 0),
    new THREE.Vector3(0, 1, 0),
    new THREE.Vector3(0, 0, 1),
  ];
  const colorArr = [axisColors.x, axisColors.y, axisColors.z];

  function makeAxisArrow(dir, color) {
    const group = new THREE.Group();
    const shaftGeo = new THREE.CylinderGeometry(0.055, 0.055, 0.82, 8);
    shaftGeo.translate(0, 0.41, 0);
    const shaftMat = new THREE.MeshBasicMaterial({ color, depthTest: false });
    const shaft = new THREE.Mesh(shaftGeo, shaftMat);
    group.add(shaft);
    const coneGeo = new THREE.ConeGeometry(0.14, 0.26, 10);
    coneGeo.translate(0, 0.95, 0);
    const cone = new THREE.Mesh(coneGeo, shaftMat);
    group.add(cone);
    group.quaternion.setFromUnitVectors(new THREE.Vector3(0, 1, 0), dir);
    return group;
  }

  function makeAxisLabel(text, color) {
    const sz = 64;
    const cv = document.createElement('canvas');
    cv.width = sz; cv.height = sz;
    const ctx = cv.getContext('2d');
    ctx.font = 'bold 40px Segoe UI, system-ui, sans-serif';
    ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
    ctx.fillStyle = '#' + color.toString(16).padStart(6, '0');
    ctx.fillText(text, sz / 2, sz / 2);
    const tex = new THREE.CanvasTexture(cv);
    tex.minFilter = THREE.LinearFilter;
    const mat = new THREE.SpriteMaterial({ map: tex, depthTest: false, transparent: true });
    const sprite = new THREE.Sprite(mat);
    sprite.scale.set(0.35, 0.35, 1);
    return sprite;
  }

  S.gizmoRoot = new THREE.Group();
  const gizmoArrows = [];
  const gizmoLabelSprites = [];
  for (let i = 0; i < 3; i++) {
    const arrow = makeAxisArrow(axisDirs[i], colorArr[i]);
    S.gizmoRoot.add(arrow);
    gizmoArrows.push(arrow);
    const label = makeAxisLabel(axisLabels[i], colorArr[i]);
    label.position.copy(axisDirs[i].clone().multiplyScalar(1.3));
    S.gizmoRoot.add(label);
    gizmoLabelSprites.push(label);
  }
  const dotGeo = new THREE.SphereGeometry(0.08, 8, 8);
  const dotMat = new THREE.MeshBasicMaterial({ color: 0x888888, depthTest: false });
  S.gizmoRoot.add(new THREE.Mesh(dotGeo, dotMat));

  S.gizmoScene.add(S.gizmoRoot);

  // Click-to-view
  const gizmoRaycaster = new THREE.Raycaster();
  gizmoCanvas.style.cursor = 'pointer';
  gizmoCanvas.addEventListener('click', (e) => {
    const rect = gizmoCanvas.getBoundingClientRect();
    const mouse = new THREE.Vector2(
      ((e.clientX - rect.left) / rect.width) * 2 - 1,
      -((e.clientY - rect.top) / rect.height) * 2 + 1
    );
    gizmoRaycaster.setFromCamera(mouse, S.gizmoCamera);

    let bestAxis = -1;
    let bestDist = Infinity;
    for (let i = 0; i < 3; i++) {
      const hits = gizmoRaycaster.intersectObject(gizmoArrows[i], true);
      if (hits.length > 0 && hits[0].distance < bestDist) {
        bestDist = hits[0].distance;
        bestAxis = i;
      }
      const labelHits = gizmoRaycaster.intersectObject(gizmoLabelSprites[i], true);
      if (labelHits.length > 0 && labelHits[0].distance < bestDist) {
        bestDist = labelHits[0].distance;
        bestAxis = i;
      }
    }
    if (bestAxis < 0) return;

    const dist = S.camera.position.distanceTo(S.controls.target);
    const viewDir = axisDirs[bestAxis].clone().multiplyScalar(dist);
    const newPos = S.controls.target.clone().add(viewDir);
    const startPos = S.camera.position.clone();
    const startTime = performance.now();
    const duration = 400;
    function animateSnap(now) {
      const t = Math.min(1, (now - startTime) / duration);
      const ease = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
      S.camera.position.lerpVectors(startPos, newPos, ease);
      S.camera.lookAt(S.controls.target);
      S.controls.update();
      if (t < 1) requestAnimationFrame(animateSnap);
    }
    requestAnimationFrame(animateSnap);
  });
}
