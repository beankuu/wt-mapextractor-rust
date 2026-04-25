export function getRendInstStyle(pool) {
  const cat = pool && pool.category ? pool.category : 'other';
  const name = pool && pool.name ? pool.name.toLowerCase() : '';

  if (/(container|warehouse|hangar|building|house|shed|garage|barrack|barracks|port_crane|crane|terminal|tower|bunker|fort|wall|bridge|pillbox|blockhouse|castle|church|mosque|factory|plant|station|depot|yard|dock|pier)/.test(name)) {
    return 'building';
  }

  if (/(^|[^a-z])(road|street|highway|asphalt|pavement|runway|taxiway|track|path)([^a-z]|$)/.test(name)) {
    return 'road';
  }

  if (cat === 'debris') {
    if (/(trench|ditch|foxhole|earthwork|embankment)/.test(name)) return 'earthwork';
    if (/(wreck|ruin|rubble|debris|hedgehog|barbed)/.test(name)) return 'debris';
  }

  return cat;
}

export function isStructureOccluder(pool, styleKey) {
  const cat = pool && pool.category ? pool.category : 'other';
  if (styleKey === 'building') {
    return true;
  }
  if (cat === 'building') {
    return true;
  }
  const name = pool && pool.name ? pool.name.toLowerCase() : '';
  return /(container|warehouse|hangar|building|house|shed|garage|barrack|barracks|tower|fort|bunker|pillbox|blockhouse|castle|church|mosque|factory|plant|station|depot|yard)/.test(name);
}
