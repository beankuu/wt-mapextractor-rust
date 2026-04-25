// Builds the line-of-sight visualization meshes from a precomputed
// ray-grid (positions + per-sample visibility flags) and adds them to
// the supplied THREE.Group.
//
// Inputs (caller-provided):
//   THREE      - the THREE namespace
//   group      - target THREE.Group to attach the resulting meshes
//   gridPos    - Float32Array(nRays * nSteps * 3) of scene-space points
//   gridVis    - Uint8Array(nRays * nSteps) visibility flag (1 = visible)
//   nRays      - ray count
//   nSteps     - sample count along each ray
//   eyePt      - THREE.Vector3 of the observer position in scene space
//   planeSize  - world-plane size used to scale the eye marker
//   waterY     - scene-space water surface height; magenta frontier tris
//                are suppressed under water (defaults to null = no clamp)
//
// Output meshes (added to group):
//   - cyan visible-region surface (0x00ffcc, opacity 0.22)
//   - magenta frontier band marking the visibility transition
//     (0xff3388, opacity 0.18, DoubleSide) - only for the first quad
//     where visibility flips off, and skipped underwater
//   - yellow eye-position marker sphere
export function buildLosMesh({
  THREE,
  group,
  gridPos,
  gridVis,
  nRays,
  nSteps,
  eyePt,
  planeSize,
  waterY = null,
}) {
  const cyanVerts = [];
  const magVerts = [];

  function gp(r, s) {
    const i = (r * nSteps + s) * 3;
    return new THREE.Vector3(gridPos[i], gridPos[i + 1], gridPos[i + 2]);
  }
  function gv(r, s) { return gridVis[r * nSteps + s]; }
  function pushTri(arr, a, b, c) {
    arr.push(a.x, a.y, a.z, b.x, b.y, b.z, c.x, c.y, c.z);
  }
  function aboveWater(...pts) {
    if (waterY == null) return true;
    return pts.every((p) => p.y > waterY + 0.25);
  }

  for (let r = 0; r < nRays; r++) {
    const rn = (r + 1) % nRays;

    // Inner cap: triangle from eye to first-step ring.
    {
      const a = gp(r, 0), b = gp(rn, 0);
      const vis = gv(r, 0) && gv(rn, 0);
      if (vis) pushTri(cyanVerts, eyePt, a, b);
    }

    // Ribbon between adjacent rays, step by step outward.
    for (let s = 0; s < nSteps - 1; s++) {
      const a = gp(r, s), b = gp(rn, s);
      const c = gp(r, s + 1), d = gp(rn, s + 1);
      const visPrev = gv(r, s) && gv(rn, s);
      const vis = gv(r, s + 1) && gv(rn, s + 1);
      if (vis) {
        pushTri(cyanVerts, a, c, b);
        pushTri(cyanVerts, b, c, d);
      } else if (visPrev && aboveWater(a, b, c, d)) {
        // Frontier band: only the first quad where visibility drops off.
        pushTri(magVerts, a, c, b);
        pushTri(magVerts, b, c, d);
      }
    }
  }

  if (cyanVerts.length > 0) {
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.Float32BufferAttribute(cyanVerts, 3));
    geo.computeVertexNormals();
    const mesh = new THREE.Mesh(geo, new THREE.MeshBasicMaterial({
      color: 0x00ffcc, transparent: true, opacity: 0.22,
      depthTest: false, depthWrite: false, side: THREE.DoubleSide,
    }));
    mesh.renderOrder = 5000;
    group.add(mesh);
  }

  if (magVerts.length > 0) {
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.Float32BufferAttribute(magVerts, 3));
    geo.computeVertexNormals();
    const mesh = new THREE.Mesh(geo, new THREE.MeshBasicMaterial({
      color: 0xff3388, transparent: true, opacity: 0.18,
      depthTest: false, depthWrite: false, side: THREE.DoubleSide,
    }));
    mesh.renderOrder = 5000;
    group.add(mesh);
  }

  const mk = new THREE.Mesh(
    new THREE.SphereGeometry(planeSize * 0.002, 12, 8),
    new THREE.MeshBasicMaterial({ color: 0xffff00, depthTest: false })
  );
  mk.position.copy(eyePt);
  mk.renderOrder = 5001;
  group.add(mk);
}
