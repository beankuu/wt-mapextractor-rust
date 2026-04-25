import { ASSET_PREFIX } from './helpers.js';

// Wikimedia Commons "World Map" (Robinson projection). Shipped locally
// under src/World_Map.svg (2754×1398, H/W ≈ 0.5076 ≈ Robinson 0.5072).
const _WORLD_MAP_SOURCE_URL =
  'https://upload.wikimedia.org/wikipedia/commons/8/83/Equirectangular_projection_SW.jpg';
// 1.0 = whole world visible inside the vignette. Raise to zoom into the mark.
const _GLOBE_ZOOM = 9.0;

// --- Robinson projection -------------------------------------------------
// Tabulated horizontal scale (X) and meridian offset (Y) at 5° intervals
// from 0° to 90° latitude (Robinson 1974). Y is normalised so that the
// pole is at ±1.0 and the equator is at 0.0; X is the parallel length
// relative to the equator.
const _ROBINSON_X = [
  1.0000, 0.9986, 0.9954, 0.9900, 0.9822, 0.9730, 0.9600, 0.9427, 0.9216,
  0.8962, 0.8679, 0.8350, 0.7986, 0.7597, 0.7186, 0.6732, 0.6213, 0.5722,
  0.5322,
];
const _ROBINSON_Y = [
  0.0000, 0.0620, 0.1240, 0.1860, 0.2480, 0.3100, 0.3720, 0.4340, 0.4958,
  0.5571, 0.6176, 0.6769, 0.7346, 0.7903, 0.8435, 0.8936, 0.9394, 0.9761,
  1.0000,
];
// Robinson content height-to-width ratio (≈0.5072). The shipped
// World_Map.svg has H/W ≈ 0.5076, so the content fills almost the entire
// SVG canvas with negligible padding.
const _ROBINSON_ASPECT = 0.5072;

// The shipped World_Map.svg is not perfectly centred on the prime
// meridian: its lon=0° column sits at ~nx 0.4725 of the canvas width
// (about 75 px left of centre, ~2.75 % of the image). Calibrated against
// 25°S 34°E and 10°N 106°E test points. Without this correction every
// placed marker appears ~10° east of its true longitude.
const _SVG_PRIME_NX = 0.4725;

function _projectNorm(latDeg, lonDeg) {
  const lat = Math.max(-90, Math.min(90, latDeg));
  const lon = Math.max(-180, Math.min(180, lonDeg));
  const absLat = Math.abs(lat);
  let i = Math.floor(absLat / 5);
  if (i >= 18) i = 17;
  const frac = (absLat - i * 5) / 5;
  const X = _ROBINSON_X[i] + frac * (_ROBINSON_X[i + 1] - _ROBINSON_X[i]);
  const Yabs = _ROBINSON_Y[i] + frac * (_ROBINSON_Y[i + 1] - _ROBINSON_Y[i]);
  const Y = lat >= 0 ? Yabs : -Yabs;
  const nx = _SVG_PRIME_NX + X * (lon / 180) * 0.5;
  const ny = 0.5 - Y * 0.5; // 0 = top, 1 = bottom
  return { nx, ny };
}
const _CONTENT_ASPECT = _ROBINSON_ASPECT;
// -------------------------------------------------------------------------

export function initEarthGlobe(lat, lon) {
  const container = document.getElementById('earth-globe-container');
  const globeCanvas = document.getElementById('earth-globe');
  if (!container || !globeCanvas) return;
  container.style.display = 'block';
  const ctx = globeCanvas.getContext('2d');
  if (!ctx) return;

  const img = new Image();
  img.crossOrigin = 'anonymous';

  const draw = () => {
    const w = globeCanvas.width;
    const h = globeCanvas.height;
    const cx = w * 0.5;
    const cy = h * 0.5;
    const r = Math.min(w, h) * 0.48;

    // Circular clipped mini-map with gradient background
    ctx.clearRect(0, 0, w, h);
    ctx.save();
    ctx.beginPath();
    ctx.arc(cx, cy, r, 0, Math.PI * 2);
    ctx.closePath();
    ctx.clip();

    // Gradient background from sea blue to slightly lighter
    const grad = ctx.createRadialGradient(cx, cy, 0, cx, cy, r);
    grad.addColorStop(0, '#1a3a5c');
    grad.addColorStop(1, '#0f223c');
    ctx.fillStyle = grad;
    ctx.fillRect(0, 0, w, h);

    // Draw the world map image if successfully loaded, with plate-carrée
    // (equirectangular) projected lat/lon → pixel mapping so the red marker
    // lands on the correct continent regardless of location.
    if (img.naturalWidth > 0) {
      // Display size: the full map width fits across ~92% of the vignette
      // diameter at zoom=1. Preserve the SVG's native aspect ratio so the
      // plate-carrée content occupies its correct pixel region within the image.
      const svgW = img.naturalWidth;
      const svgH = img.naturalHeight;
      const svgAspect = svgH / svgW; // ~0.556 for the Wikimedia blank world
      const baseMapW = r * 2 * 0.92;
      const mapW = baseMapW * _GLOBE_ZOOM;
      const mapH = mapW * svgAspect;

      // Normalised projection (0..1) inside the content rectangle.
      const { nx, ny } = _projectNorm(Number(lat) || 0, Number(lon) || 0);
      const contentFrac = _CONTENT_ASPECT / svgAspect; // content height / svg height
      const pyFrac = 0.5 + (ny - 0.5) * contentFrac;

      // Draw the map so that (nx, pyFrac) in image-normalised coords lands
      // at the canvas centre. Draw wrapped copies left and right so the
      // world wraps seamlessly near ±180°.
      const mapX = cx - nx * mapW;
      const mapY = cy - pyFrac * mapH;
      try {
        ctx.drawImage(img, mapX - mapW, mapY, mapW, mapH);
        ctx.drawImage(img, mapX, mapY, mapW, mapH);
        ctx.drawImage(img, mapX + mapW, mapY, mapW, mapH);
      } catch (_) {
        /* draw failure — fall through to marker-only */
      }

      // Graticule overlay — draws 30° lat/lon grid lines using the same
      // projection math as the marker. If the lines don't align with the
      // continent outlines, the SVG projection differs from plate carrée
      // and needs calibration.
      const projectGlobe = (latDeg, lonDeg) => {
        const p = _projectNorm(latDeg, lonDeg);
        const py = 0.5 + (p.ny - 0.5) * contentFrac;
        return { x: mapX + p.nx * mapW, y: mapY + py * mapH };
      };
      ctx.save();
      ctx.strokeStyle = 'rgba(255, 255, 255, 0.18)';
      ctx.lineWidth = 0.6;
      ctx.beginPath();
      // Meridians every 30°
      for (let ln = -180; ln <= 180; ln += 30) {
        const a = projectGlobe(-90, ln);
        const b = projectGlobe(90, ln);
        ctx.moveTo(a.x, a.y);
        ctx.lineTo(b.x, b.y);
      }
      // Parallels every 30°
      for (let la = -60; la <= 60; la += 30) {
        const a = projectGlobe(la, -180);
        const b = projectGlobe(la, 180);
        ctx.moveTo(a.x, a.y);
        ctx.lineTo(b.x, b.y);
      }
      ctx.stroke();
      // Emphasise equator and prime meridian
      ctx.strokeStyle = 'rgba(255, 255, 255, 0.35)';
      ctx.lineWidth = 0.8;
      ctx.beginPath();
      const eqA = projectGlobe(0, -180);
      const eqB = projectGlobe(0, 180);
      ctx.moveTo(eqA.x, eqA.y);
      ctx.lineTo(eqB.x, eqB.y);
      const pmA = projectGlobe(-90, 0);
      const pmB = projectGlobe(90, 0);
      ctx.moveTo(pmA.x, pmA.y);
      ctx.lineTo(pmB.x, pmB.y);
      ctx.stroke();
      ctx.restore();
    }

    const mx = cx;
    const my = cy;

    // Enhanced location marker with glow effect
    ctx.shadowColor = 'rgba(255, 100, 100, 0.8)';
    ctx.shadowBlur = 12;
    ctx.shadowOffsetX = 0;
    ctx.shadowOffsetY = 0;
    ctx.strokeStyle = '#ff5555';
    ctx.lineWidth = 2.5;
    ctx.beginPath();
    ctx.arc(mx, my, 7, 0, Math.PI * 2);
    ctx.stroke();

    ctx.fillStyle = '#ff2d2d';
    ctx.beginPath();
    ctx.arc(mx, my, 4, 0, Math.PI * 2);
    ctx.fill();

    ctx.shadowColor = 'transparent';
    ctx.restore();

    // Enhanced border with gradient
    const borderGrad = ctx.createLinearGradient(cx - r, cy, cx + r, cy);
    borderGrad.addColorStop(0, '#4dabf7');
    borderGrad.addColorStop(0.5, '#2d9cff');
    borderGrad.addColorStop(1, '#4dabf7');
    ctx.strokeStyle = borderGrad;
    ctx.lineWidth = 2.5;
    ctx.beginPath();
    ctx.arc(cx, cy, r, 0, Math.PI * 2);
    ctx.stroke();
  };

  img.onload = () => {
    draw();
  };

  img.onerror = () => {
    // Fallback: draw vignette + marker only
    draw();
  };

  // Path fallback list. We ship the Robinson-projection SVG locally; the
  // remote URL only runs if every local path fails.
  const localPaths = [
    '../../src/World_Map.svg',
    '../src/World_Map.svg',
    `${ASSET_PREFIX}/src/World_Map.svg`,
    '/src/World_Map.svg',
  ];

  const attemptLoad = (paths, index) => {
    if (index >= paths.length) {
      img.src = _WORLD_MAP_SOURCE_URL;
      return;
    }
    img.onerror = () => attemptLoad(paths, index + 1);
    img.src = paths[index];
  };

  attemptLoad(localPaths, 0);

  // Safety: draw once even if nothing loads within 500ms.
  setTimeout(() => {
    if (globeCanvas.width > 0 && globeCanvas.height > 0) {
      draw();
    }
  }, 500);

  container.setAttribute('data-map-source', _WORLD_MAP_SOURCE_URL);
}
