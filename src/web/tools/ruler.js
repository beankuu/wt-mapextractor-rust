export function createRulerTool({
  THREE,
  Line2,
  LineMaterial,
  LineGeometry,
  scene,
  camera,
  getMap,
  getDisplacedMeshes,
  getRaycastMeshes,
  sceneToWorld,
}) {
  const rulers = [];
  const raycaster = new THREE.Raycaster();

  function hitTerrain(mx, my) {
    const ndc = new THREE.Vector2(
      (mx / window.innerWidth) * 2 - 1,
      -(my / window.innerHeight) * 2 + 1
    );
    raycaster.setFromCamera(ndc, camera);
    const targets = [];
    const meshes = getRaycastMeshes();
    if (meshes.detailRaycastMesh) targets.push(meshes.detailRaycastMesh);
    if (meshes.raycastMesh) targets.push(meshes.raycastMesh);
    const hits = raycaster.intersectObjects(targets, false);
    return hits.length > 0 ? hits[0].point.clone() : null;
  }

  function createHandle(color) {
    const geo = new THREE.SphereGeometry(0.5, 12, 8);
    const mat = new THREE.MeshBasicMaterial({ color, depthTest: false, transparent: true, opacity: 0.9 });
    const mesh = new THREE.Mesh(geo, mat);
    mesh.renderOrder = 1001;
    return mesh;
  }

  function updateRulerLabel(ruler) {
    const d = ruler.distM;
    ruler.labelDiv.textContent = d >= 1000 ? (d / 1000).toFixed(2) + ' km' : d.toFixed(0) + ' m';
  }

  function makeRuler(pA, pB) {
    const lineMat = new LineMaterial({ color: 0x4dabf7, linewidth: 3, depthTest: false, transparent: true, opacity: 0.95, worldUnits: false });
    lineMat.resolution.set(window.innerWidth, window.innerHeight);
    const lineGeo = new LineGeometry();
    lineGeo.setPositions([pA.x, pA.y, pA.z, pB.x, pB.y, pB.z]);
    const lineObj = new Line2(lineGeo, lineMat);
    lineObj.computeLineDistances();
    lineObj.renderOrder = 1000;
    lineObj.frustumCulled = false;
    scene.add(lineObj);

    const M = getMap();
    const planeSize = Math.max(M.mapSize[0], M.mapSize[1]);
    const handleSize = planeSize * 0.0003;
    const sphereA = createHandle(0xff4444);
    sphereA.position.copy(pA);
    sphereA.scale.setScalar(handleSize);
    scene.add(sphereA);

    const sphereB = createHandle(0x44ff44);
    sphereB.position.copy(pB);
    sphereB.scale.setScalar(handleSize);
    scene.add(sphereB);

    const labelDiv = document.createElement('div');
    labelDiv.className = 'ruler-label';
    document.body.appendChild(labelDiv);

    const wA = sceneToWorld(pA);
    const wB = sceneToWorld(pB);
    const dx = wA.x - wB.x, dz = wA.z - wB.z, dh = wA.h - wB.h;
    const distM = Math.sqrt(dx * dx + dz * dz + dh * dh);

    const ruler = { lineObj, labelDiv, sphereA, sphereB, ptA: pA.clone(), ptB: pB.clone(), worldA: wA, worldB: wB, distM };
    rulers.push(ruler);
    updateRulerLabel(ruler);
    return ruler;
  }

  function updateRulerEndpoint(ruler, handle, newPt) {
    if (handle === 'A') {
      ruler.ptA.copy(newPt);
      ruler.sphereA.position.copy(newPt);
      ruler.worldA = sceneToWorld(newPt);
    } else {
      ruler.ptB.copy(newPt);
      ruler.sphereB.position.copy(newPt);
      ruler.worldB = sceneToWorld(newPt);
    }
    ruler.lineObj.geometry.setPositions([
      ruler.ptA.x, ruler.ptA.y, ruler.ptA.z,
      ruler.ptB.x, ruler.ptB.y, ruler.ptB.z
    ]);
    ruler.lineObj.computeLineDistances();
    const dx = ruler.worldA.x - ruler.worldB.x;
    const dz = ruler.worldA.z - ruler.worldB.z;
    const dh = ruler.worldA.h - ruler.worldB.h;
    ruler.distM = Math.sqrt(dx * dx + dz * dz + dh * dh);
    updateRulerLabel(ruler);
  }

  function updateRulerLabels() {
    for (const ruler of rulers) {
      const mid = new THREE.Vector3().addVectors(ruler.ptA, ruler.ptB).multiplyScalar(0.5);
      mid.project(camera);
      if (mid.z > 1) {
        ruler.labelDiv.style.display = 'none';
        continue;
      }
      const x = (mid.x * 0.5 + 0.5) * window.innerWidth;
      const y = (-mid.y * 0.5 + 0.5) * window.innerHeight;
      ruler.labelDiv.style.display = 'block';
      ruler.labelDiv.style.left = x + 'px';
      ruler.labelDiv.style.top = (y - 20) + 'px';
    }
  }

  function removeRuler(idx) {
    if (idx < 0 || idx >= rulers.length) return;
    const r = rulers[idx];
    scene.remove(r.lineObj);
    r.lineObj.geometry.dispose();
    r.lineObj.material.dispose();
    scene.remove(r.sphereA);
    r.sphereA.geometry.dispose();
    r.sphereA.material.dispose();
    scene.remove(r.sphereB);
    r.sphereB.geometry.dispose();
    r.sphereB.material.dispose();
    if (r.labelDiv.parentNode) r.labelDiv.parentNode.removeChild(r.labelDiv);
    rulers.splice(idx, 1);
  }

  function removeAllRulers() {
    while (rulers.length > 0) removeRuler(0);
  }

  function findNearestRulerHandle(mx, my) {
    const ndc = new THREE.Vector2(
      (mx / window.innerWidth) * 2 - 1,
      -(my / window.innerHeight) * 2 + 1
    );
    raycaster.setFromCamera(ndc, camera);
    let best = null;
    let bestDist = Infinity;
    for (let i = 0; i < rulers.length; i++) {
      const r = rulers[i];
      for (const [handle, sphere] of [['A', r.sphereA], ['B', r.sphereB]]) {
        const hits = raycaster.intersectObject(sphere, false);
        if (hits.length > 0 && hits[0].distance < bestDist) {
          bestDist = hits[0].distance;
          best = { ruler: r, rulerIdx: i, handle };
        }
      }
    }
    return best;
  }

  function updateLineMaterialResolution(width, height) {
    const res = new THREE.Vector2(width, height);
    for (const r of rulers) {
      if (r.lineObj.material.resolution) r.lineObj.material.resolution.copy(res);
    }
  }

  return {
    rulers,
    hitTerrain,
    createHandle,
    makeRuler,
    updateRulerEndpoint,
    updateRulerLabels,
    removeRuler,
    removeAllRulers,
    findNearestRulerHandle,
    updateLineMaterialResolution,
  };
}
