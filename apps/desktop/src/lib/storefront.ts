import { open as openExternal } from "@tauri-apps/plugin-shell";

/** Store page for the optional UltraVox Pro license. */
export const PRO_STORE_URL = "https://paretocybernetics.com/software/?product=ultravox";

/** Public project page (source, releases and install steps) for the official UltraVox download. */
export const PRO_DOWNLOAD_URL = "https://github.com/michael-berardi/ultravox";

async function openLink(url: string): Promise<void> {
  try {
    await openExternal(url);
  } catch {
    window.open(url, "_blank");
  }
}

export async function openStorefront(): Promise<void> {
  await openLink(PRO_STORE_URL);
}

export async function openDownloadPage(): Promise<void> {
  await openLink(PRO_DOWNLOAD_URL);
}
