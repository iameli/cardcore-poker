/**
 * Device-local user settings, persisted in localStorage. Not synced anywhere
 * — sound preferences and the like belong to the device, not the identity.
 */
const KEY = "cardcore_settings";

function readAll() {
  try {
    return JSON.parse(localStorage.getItem(KEY) || "{}") || {};
  } catch {
    return {};
  }
}

export function getSetting(name, fallback) {
  const all = readAll();
  return name in all ? all[name] : fallback;
}

export function setSetting(name, value) {
  const all = readAll();
  all[name] = value;
  try {
    localStorage.setItem(KEY, JSON.stringify(all));
  } catch {
    // Private browsing / quota — the setting just won't persist.
  }
}
