let maps = [];
const ASSET_PREFIX = window.location.pathname.includes('/src/') ? '..' : '.';

function getCategory(name) {
  if (name.startsWith('avg_')) return 'avg';
  if (name.startsWith('air_')) return 'air';
  if (name.startsWith('avn_')) return 'avn';
  if (name.startsWith('arcade_')) return 'arcade';
  return 'other';
}

function displayName(name) {
  return name
    .replace(/^(avg|air|avn|arcade)_/, '')
    .replace(/_/g, ' ')
    .replace(/\b\w/g, c => c.toUpperCase());
}

function render(filter, search) {
  const grid = document.getElementById('grid');
  grid.innerHTML = '';
  let filtered = maps.filter(m => m.success !== false);

  if (filter && filter !== 'all') {
    filtered = filtered.filter(m => getCategory(m.name) === filter);
  }
  if (search) {
    const q = search.toLowerCase();
    filtered = filtered.filter(m => m.name.toLowerCase().includes(q) || displayName(m.name).toLowerCase().includes(q));
  }

  document.getElementById('count').textContent = `${filtered.length} maps`;

  if (filtered.length === 0) {
    grid.innerHTML = '<div class="no-results">No maps found</div>';
    return;
  }

  for (const m of filtered) {
    const cat = getCategory(m.name);
    const catLabels = {avg:'Ground',air:'Air',avn:'Naval',arcade:'Arcade',other:'Classic'};
    const card = document.createElement('div');
    card.className = `card cat-${cat}`;
    card.addEventListener('click', () => {
      window.location.href = `viewer.html?map=${encodeURIComponent(m.name)}`;
    });

    const imgSrc = m.terrainPaint
      ? `${ASSET_PREFIX}/maps/${m.name}/${m.terrainPaint.thumb || m.terrainPaint.file}`
      : m.colormap
        ? `${ASSET_PREFIX}/maps/${m.name}/${m.colormap.file}`
        : m.heightmap
          ? `${ASSET_PREFIX}/maps/${m.name}/${m.heightmap.file}`
          : '';

    let badges = `<span class="badge cat-${cat}">${catLabels[cat]||cat}</span>`;
    if (m.heightmap) badges += '<span class="badge hm">HM</span>';
    if (m.terrainPaint) badges += '<span class="badge tp">Paint</span>';
    if (m.landclasses > 0) badges += `<span class="badge lc">${m.landclasses} LC</span>`;
    if (m.materials > 0) badges += `<span class="badge mat">${m.materials} Mat</span>`;

    const size = m.mapSize ? `${(m.mapSize[0]/1000).toFixed(0)}×${(m.mapSize[1]/1000).toFixed(0)}km` : '';

    card.innerHTML = `
      ${imgSrc ? `<img src="${imgSrc}" loading="lazy" alt="${m.name}">` : '<div style="width:100%;aspect-ratio:1;background:#060e1a;display:flex;align-items:center;justify-content:center;color:#334;font-size:24px;">?</div>'}
      <div class="info">
        <div class="name">${displayName(m.name)}</div>
        <div class="meta">${size}${m.waterLevel != null ? ` · Water: ${m.waterLevel}` : ''}</div>
        <div>${badges}</div>
      </div>
    `;
    grid.appendChild(card);
  }
}

// Multi-map gallery mode (maps/maps_index.json)
async function init() {
  // Try loading multi-map index
  try {
    const res = await fetch(`${ASSET_PREFIX}/maps/maps_index.json`, { cache: 'no-store' });
    if (res.ok) {
      maps = await res.json();
      document.getElementById('loading').style.display = 'none';
      render('all', '');
      setupControls();
      return;
    }
  } catch(e) {}

  document.getElementById('loading').innerHTML = '<div style="color:#ba5d5d">No maps found. Run <code>cargo run -- --all</code> first.</div>';
}

function setupControls() {
  const search = document.getElementById('search');
  const buttons = document.querySelectorAll('.filter-btn');
  let activeFilter = 'all';

  search.addEventListener('input', () => render(activeFilter, search.value));

  buttons.forEach(btn => {
    btn.addEventListener('click', () => {
      buttons.forEach(b => b.classList.remove('active'));
      btn.classList.add('active');
      activeFilter = btn.dataset.filter;
      render(activeFilter, search.value);
    });
  });
}

init();
