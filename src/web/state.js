// Central shared mutable state for the viewer application.
// All modules import S and read/write properties directly.

export const S = {
  // Map manifest
  M: null,

  // Three.js core
  canvas: null,
  renderer: null,
  scene: null,
  camera: null,
  controls: null,

  // Lighting
  sun: null,
  fill: null,
  sunMesh: null,

  // Terrain meshes
  displacedMeshes: [],
  terrainMesh: null,
  tileGridMesh: null,
  heightmapMesh: null,
  detailMesh: null,
  detailTexMesh: null,
  splatmapMesh: null,

  // Environment
  waterMesh: null,
  waterFraction: 0.5,
  waterLikelyOcean: false,
  maxHeightScale: 1,
  tankZoneMesh: null,

  // Raycasting
  raycastMesh: null,
  raycastHeightNorms: null,
  detailRaycastMesh: null,
  detailRaycastNorms: null,
  tileHighlightGroup: null,

  // Render instances
  rendinstGroup: null,
  rendinstCategoryData: null,
  rendinstRealLoaded: false,

  // Mission overlay
  missionGroup: null,
  missionData: null,

  // Heightmap pixels (shared for LoS / hover)
  hmPixelData: null,
  hmPixelW: 0,
  hmPixelH: 0,
  hm2PixelData: null,
  hm2PixelW: 0,
  hm2PixelH: 0,

  // Occupancy grid
  occGrid: null,
  occGridW: 0,
  occGridH: 0,
  occGridX0: 0,
  occGridZ0: 0,
  occGridCellSize: 2,
  gpuOccBuf: null,

  // WebGPU
  gpuDevice: null,
  gpuAvailable: false,
  gpuHmBuf: null,
  gpuHm2Buf: null,
  gpuLosPipeline: null,
  gpuLosBGL: null,

  // LoS
  losGroup: null,
  losOrigin: null,
  losComputing: false,
  losLastScale: null,

  // Tools
  activeTool: null,
  tileToolActive: false,
  rulerPending: null,
  rulerDragging: null,
  rulerTool: null,

  // Gizmo
  gizmoRenderer: null,
  gizmoScene: null,
  gizmoCamera: null,
  gizmoRoot: null,
};
