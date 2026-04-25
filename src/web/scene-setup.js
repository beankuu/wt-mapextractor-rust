import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { S } from './state.js';

export function setupScene() {
  S.canvas = document.getElementById('canvas');

  S.renderer = new THREE.WebGLRenderer({ canvas: S.canvas, antialias: true, logarithmicDepthBuffer: true });
  S.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  S.renderer.setSize(window.innerWidth, window.innerHeight);
  S.renderer.setClearColor(0x0a1628);
  S.renderer.toneMapping = THREE.ACESFilmicToneMapping;
  S.renderer.toneMappingExposure = 1.1;

  S.scene = new THREE.Scene();
  S.camera = new THREE.PerspectiveCamera(50, window.innerWidth / window.innerHeight, 1, 600000);
  S.controls = new OrbitControls(S.camera, S.canvas);
  S.controls.enableDamping = true;
  S.controls.dampingFactor = 0.08;
  S.controls.enableZoom = true;
  S.controls.zoomSpeed = 3.0;
  S.controls.zoomToCursor = true;
  S.controls.maxPolarAngle = Math.PI / 2 - 0.01;

  // Lighting
  S.scene.add(new THREE.AmbientLight(0xffffff, 0.8));
  S.sun = new THREE.DirectionalLight(0xfff5e0, 1.2);
  S.sun.position.set(40000, 80000, 30000);
  S.scene.add(S.sun);
  S.fill = new THREE.DirectionalLight(0x8090c0, 0.3);
  S.fill.position.set(-30000, 40000, -20000);
  S.scene.add(S.fill);
  S.sunMesh = null;
  S.scene.fog = new THREE.FogExp2(0x0a1628, 0.0000025);

  // Zoom slider
  document.getElementById('sl-zoom').addEventListener('input', (e) => {
    const minDist = S.controls.minDistance || 1;
    const maxDist = S.controls.maxDistance || 100000;
    const t = 1 - (e.target.value / 100);
    const newDist = Math.exp(Math.log(minDist) + t * (Math.log(maxDist) - Math.log(minDist)));
    const dir = new THREE.Vector3().subVectors(S.camera.position, S.controls.target).normalize();
    S.camera.position.copy(S.controls.target).addScaledVector(dir, newDist);
    S.controls.update();
  });
}

export function applySunSettings(sunCfg) {
  const az = (sunCfg && Number.isFinite(sunCfg.azimuth)) ? sunCfg.azimuth : 30;
  const el = (sunCfg && Number.isFinite(sunCfg.elevation)) ? sunCfg.elevation : 40;
  const st = (sunCfg && Number.isFinite(sunCfg.strength)) ? sunCfg.strength : 0.6;
  const azRad = az * Math.PI / 180;
  const elRad = Math.max(0, Math.min(90, el)) * Math.PI / 180;
  const horiz = Math.cos(elRad);
  const sx = Math.sin(azRad) * horiz;
  const sz = Math.cos(azRad) * horiz;
  const sy = Math.sin(elRad);
  const dist = 90000;
  S.sun.position.set(sx * dist, sy * dist, sz * dist);
  S.sun.intensity = 1.1 + 1.8 * Math.max(0, Math.min(1, st));
  S.fill.intensity = 0.2 + 0.35 * Math.max(0, Math.min(1, st));
  S.fill.position.set(-sx * 45000, Math.max(12000, sy * 18000), -sz * 45000);
  if (S.sunMesh) {
    S.sunMesh.position.set(sx * (dist * 0.65), sy * (dist * 0.65), sz * (dist * 0.65));
  }
}

export function syncZoomSlider() {
  const sl = document.getElementById('sl-zoom');
  if (!sl) return;
  const minDist = S.controls.minDistance || 1;
  const maxDist = S.controls.maxDistance || 100000;
  const dist = S.camera.position.distanceTo(S.controls.target);
  const t = (Math.log(dist) - Math.log(minDist)) / (Math.log(maxDist) - Math.log(minDist));
  sl.value = Math.round((1 - t) * 100);
}
