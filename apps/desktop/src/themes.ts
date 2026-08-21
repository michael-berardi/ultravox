import { getSettings, onSettingsChanged, setThemeMaterial } from "./ipc";

export type ThemeDefinition = {
  id: string;
  name: string;
  tagline: string;
  /** [background, primary accent, secondary accent] preview swatches. */
  swatch: [string, string, string];
};

export const DEFAULT_THEME = "midnight";

export const THEMES: ThemeDefinition[] = [
  {
    id: "midnight",
    name: "Midnight",
    tagline: "The default. Deep space blues on true black.",
    swatch: ["#0b0d12", "#8eb7ff", "#b794ff"],
  },
  {
    id: "frutiger-arrow",
    name: "Frutiger Arrow",
    tagline: "Vista-era glass: aqua skies, gloss, and aurora light.",
    swatch: ["#d6ecf8", "#2f9bff", "#6ee87a"],
  },
  {
    id: "frutiger-dark",
    name: "Frutiger Dark",
    tagline: "Aero gloss on true black. OLED-friendly aurora glass.",
    swatch: ["#000000", "#35d0ff", "#6ee87a"],
  },
  {
    id: "winamp",
    name: "Winamp",
    tagline: "Lime-on-charcoal nostalgia with orange EQ bite.",
    swatch: ["#121218", "#7eff54", "#ff9f2e"],
  },
  {
    id: "olive",
    name: "Olive",
    tagline: "Muted minimalist drab. Flat, quiet, zero noise.",
    swatch: ["#1d1f17", "#a3b86b", "#d96a5f"],
  },
  {
    id: "nord-frost",
    name: "Nord Frost",
    tagline: "Arctic daylight: pale ice and steel-blue calm.",
    swatch: ["#e5eaf3", "#5e81ac", "#88c0d0"],
  },
  {
    id: "solar-dusk",
    name: "Solar Dusk",
    tagline: "Warm sunset embers over deep plum night.",
    swatch: ["#221416", "#ffb86b", "#f0719a"],
  },
  {
    id: "vapor",
    name: "Vapor",
    tagline: "Neon grid nights: hot pink, cyan, and indigo.",
    swatch: ["#1a1030", "#01cdfe", "#ff71ce"],
  },
];

const KNOWN_THEME_IDS: Record<string, true> = Object.fromEntries(
  THEMES.map((theme) => [theme.id, true]),
);

export function applyTheme(themeId: string | null | undefined): void {
  const id = themeId && KNOWN_THEME_IDS[themeId] ? themeId : DEFAULT_THEME;
  document.documentElement.dataset.theme = id;
  void setThemeMaterial(id).catch((error) =>
    console.error("Failed to set window material:", error),
  );
}

/**
 * Applies the persisted theme and keeps every window in sync
 * with `settings-changed` events. Fire-and-forget at app start.
 */
export async function initTheme(): Promise<void> {
  try {
    const config = await getSettings();
    applyTheme(config.theme);
  } catch (error) {
    console.error("Failed to load theme:", error);
    applyTheme(DEFAULT_THEME);
  }
  await onSettingsChanged((payload) => applyTheme(payload.config.theme));
}
