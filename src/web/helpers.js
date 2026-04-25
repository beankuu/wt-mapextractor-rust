import * as THREE from 'three';

const urlParams = new URLSearchParams(window.location.search);
export const mapName = urlParams.get('map');
export const ASSET_PREFIX = window.location.pathname.includes('/src/') ? '..' : '.';
export const DATA = mapName ? `${ASSET_PREFIX}/maps/${mapName}` : `${ASSET_PREFIX}`;

if (mapName) document.getElementById('back-btn').style.display = 'block';

export function buildBoxWireframeVerts(x0, x1, yBot, yTop, z0, z1) {
  return new Float32Array([
    x0,yBot,z0, x1,yBot,z0,  x1,yBot,z0, x1,yBot,z1,
    x1,yBot,z1, x0,yBot,z1,  x0,yBot,z1, x0,yBot,z0,
    x0,yTop,z0, x1,yTop,z0,  x1,yTop,z0, x1,yTop,z1,
    x1,yTop,z1, x0,yTop,z1,  x0,yTop,z1, x0,yTop,z0,
    x0,yBot,z0, x0,yTop,z0,  x1,yBot,z0, x1,yTop,z0,
    x1,yBot,z1, x1,yTop,z1,  x0,yBot,z1, x0,yTop,z1,
  ]);
}

export function loadTexture(url) {
  return new Promise((resolve, reject) => {
    new THREE.TextureLoader().load(url, tex => {
      tex.flipY = false;
      tex.minFilter = THREE.LinearFilter;
      tex.magFilter = THREE.LinearFilter;
      tex.anisotropy = 4;
      resolve(tex);
    }, undefined, reject);
  });
}

const progressBar = document.getElementById('progress-bar');
const loadText = document.getElementById('load-text');

export function progress(pct, msg) {
  progressBar.style.width = pct + '%';
  if (msg) loadText.textContent = msg;
}
