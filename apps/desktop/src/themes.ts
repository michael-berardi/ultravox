import { getSettings, onSettingsChanged, setThemeMaterial } from "./ipc";

export type ThemeDefinition = {
  id: string;
  name: string;
  tagline: string;
  /** [background, primary accent, secondary accent] preview swatches. */
  swatch: [string, string, string];
  /** Free themes work everywhere; the rest unlock with an UltraVox Pro license. */
  free?: boolean;
};

export const DEFAULT_THEME = "midnight";

const RAW_THEMES: ThemeDefinition[] = [
  {
    id: "midnight",
    name: "Midnight",
    tagline: "The default. Deep-space blues where a faint nebula breathes.",
    swatch: ["#0b0d12", "#8eb7ff", "#b794ff"],
    free: true,
  },
  {
    id: "winamp-hifi",
    name: "Silver Rack",
    tagline: "Brushed-silver faceplate with a heartbeat standby LED.",
    swatch: ["#b9bdc6", "#101014", "#ff3b30"],
    free: true,
  },
  {
    id: "nord-frost",
    name: "Nord Frost",
    tagline: "Arctic daylight: pale ice beneath a drifting polar aurora.",
    swatch: ["#e5eaf3", "#5e81ac", "#88c0d0"],
    free: true,
  },
  {
    id: "vapor",
    name: "Vapor",
    tagline: "Neon grid nights with a horizon that never stops arriving.",
    swatch: ["#1a1030", "#01cdfe", "#ff71ce"],
    free: true,
  },
  {
    id: "obsidian-rite",
    name: "Obsidian Rite",
    tagline: "Alien liturgy in violet plasma: cut hull plating, rotating glyph rings, breathing light.",
    swatch: ["#050309", "#a06bff", "#8fe8ff"],
    free: true,
  },
  {
    id: "frutiger-aero",
    name: "Frutiger Aero",
    tagline: "Vista-era glass: aqua skies, gloss, and slow-rising bubbles.",
    swatch: ["#d6ecf8", "#2f9bff", "#6ee87a"],
  },
  {
    id: "frutiger-dark",
    name: "Frutiger Dark",
    tagline: "Aero gloss on OLED black under a swaying aurora ribbon.",
    swatch: ["#000000", "#35d0ff", "#6ee87a"],
  },
  {
    id: "winamp",
    name: "Phosphor Classic",
    tagline: "Graphite metal, phosphor green, and a crawling CRT refresh line.",
    swatch: ["#121218", "#7eff54", "#ff9f2e"],
  },
  {
    id: "winamp-mmd3",
    name: "Instrument Console",
    tagline: "Silver-and-navy instrumentation with a resting VU needle.",
    swatch: ["#1a2340", "#c8ccd4", "#ffb347"],
  },
  {
    id: "winamp-bento",
    name: "Graphite Stack",
    tagline: "Stacked graphite modules under a soft, breathing top-light.",
    swatch: ["#2a2c30", "#f2f3f5", "#ff8c1a"],
  },
  {
    id: "pioneer",
    name: "OEL Drive",
    tagline: "Blue OEL console whose backlight slowly swells and settles.",
    swatch: ["#050b12", "#00a8ff", "#8ff7ff"],
  },
  {
    id: "olive",
    name: "Olive",
    tagline: "Muted minimalist drab. Flat, quiet, and proudly motionless.",
    swatch: ["#1d1f17", "#a3b86b", "#d96a5f"],
  },
  {
    id: "solar-dusk",
    name: "Solar Dusk",
    tagline: "Warm dusk embers lifting from the horizon, fading as they rise.",
    swatch: ["#221416", "#ffb86b", "#f0719a"],
  },
  {
    id: "crystal",
    name: "Crystal",
    tagline: "Maximally clear glass over warm '70s electronics: amber glow, chrome hairlines, total calm.",
    swatch: ["#e9edf1", "#c47f1f", "#8fa3b8"],
  },
];

/** Free themes first (as shipped), locked Pro themes after. */
export const THEMES: ThemeDefinition[] = [
  ...RAW_THEMES.filter((theme) => theme.free),
  ...RAW_THEMES.filter((theme) => !theme.free),
];

export function isFreeTheme(themeId: string): boolean {
  return RAW_THEMES.find((theme) => theme.id === themeId)?.free === true;
}

const KNOWN_THEME_IDS: Record<string, true> = Object.fromEntries(
  THEMES.map((theme) => [theme.id, true]),
);

export function applyTheme(themeId: string | null | undefined): void {
  // "frutiger-arrow" was the pre-release id of Frutiger Aero.
  const normalized = themeId === "frutiger-arrow" ? "frutiger-aero" : themeId;
  const id = normalized && KNOWN_THEME_IDS[normalized] ? normalized : DEFAULT_THEME;
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
