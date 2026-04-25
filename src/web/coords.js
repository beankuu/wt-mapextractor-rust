export function getMapFrame(M, _planeSize = null) {
  const mc0 = M.mapCoord0;
  const mc1 = M.mapCoord1;

  const mcW = mc1[0] - mc0[0];
  const mcH = mc1[1] - mc0[1];
  const resolvedPlaneSize = Math.max(mcW, mcH);
  const cx = (mc0[0] + mc1[0]) / 2;
  const cz = (mc0[1] + mc1[1]) / 2;
  const sX = resolvedPlaneSize / mcW;
  const sZ = resolvedPlaneSize / mcH;
  return { mc0, mc1, mcW, mcH, cx, cz, sX, sZ, planeSize: resolvedPlaneSize };
}

export function worldToSceneXZ(M, wx, wz, planeSize = null) {
  const f = getMapFrame(M, planeSize);
  return {
    x: (wx - f.cx) * f.sX,
    z: -(wz - f.cz) * f.sZ,
  };
}

export function sceneToWorldXZ(M, sx, sz, planeSize = null) {
  const f = getMapFrame(M, planeSize);
  return {
    x: f.cx + (sx / f.sX),
    z: f.cz - (sz / f.sZ),
  };
}
