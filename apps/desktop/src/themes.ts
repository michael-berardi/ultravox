import { getSettings, onSettingsChanged, setThemeMaterial } from "./ipc";
export type ThemeDefinition = { id: string; name: string; tagline: string; swatch: [string,string,string] };
export const DEFAULT_THEME = "midnight";
export const THEMES: ThemeDefinition[] = [
  { id: "midnight", name: "Midnight", tagline: "Deep-space blues where a faint nebula breathes.", swatch: ["#0b0d12", "#8eb7ff", "#b794ff"] },
  { id: "winamp-hifi", name: "Silver Rack", tagline: "Brushed-silver faceplate with a heartbeat standby LED.", swatch: ["#b9bdc6", "#101014", "#ff3b30"] },
  { id: "nord-frost", name: "Nord Frost", tagline: "Arctic daylight: pale ice beneath a drifting polar aurora.", swatch: ["#e5eaf3", "#5e81ac", "#88c0d0"] },
  { id: "vapor", name: "Vapor", tagline: "Neon grid nights with a horizon that never stops arriving.", swatch: ["#1a1030", "#01cdfe", "#ff71ce"] },
  { id: "obsidian-rite", name: "Obsidian Rite", tagline: "Alien liturgy in violet plasma: cut hull plating, breathing light.", swatch: ["#050309", "#a06bff", "#8fe8ff"] },
];
const KNOWN_THEME_IDS = Object.fromEntries(THEMES.map(theme => [theme.id, true]));
export function applyTheme(themeId: string | null | undefined): void {
  const id = themeId && KNOWN_THEME_IDS[themeId] ? themeId : DEFAULT_THEME;
  document.documentElement.dataset.theme = id;
  void setThemeMaterial(id).catch(error => console.error("Failed to set window material:", error));
}
export async function initTheme(): Promise<void> {
  try { applyTheme((await getSettings()).theme); } catch (error) { console.error("Failed to load theme:", error); applyTheme(DEFAULT_THEME); }
  await onSettingsChanged(payload => applyTheme(payload.config.theme));
}
