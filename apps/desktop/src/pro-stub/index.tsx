import type { AppConfig, ProStatus } from "../ipc";
import { openDownloadPage, openStorefront } from "../lib/storefront";

/**
 * Open-source stand-in for the Pro module. Pro code is not linked into this
 * build, so the surfaces render nothing and the Pro settings explain where Pro
 * lives: the official UltraVox download.
 */

export type ProLicensePanelProps = {
  pro: ProStatus;
  onRefresh: () => Promise<void>;
};

export function ProDownloadCard({ copy }: { copy: string }) {
  return (
    <section className="settings-card buy-hero">
      <div className="settings-card-heading">
        <h3>Get UltraVox Pro</h3>
        <p>{copy}</p>
      </div>
      <div className="theme-lock-actions">
        <button type="button" className="btn btn-primary" onClick={() => void openDownloadPage()}>
          Get the official UltraVox download
        </button>
        <button type="button" className="btn" onClick={() => void openStorefront()}>
          Buy Pro — $25
        </button>
      </div>
      <p className="capture-note">
        Pro is available in the official UltraVox build: https://implosecybernetics.com/projects/ultravox/
      </p>
    </section>
  );
}

export function ProLicensePanel(_props: ProLicensePanelProps) {
  return (
    <ProDownloadCard copy="UltraVox is free and open source. Pro is an optional license that unlocks Meeting mode, Lecture mode, Voice Studio, custom dictionaries, the media console and signature themes in the official UltraVox build." />
  );
}

export function VoiceStudioPage(_props: {
  active: boolean;
  onClose: () => void;
  onBlockingChange: (blocked: boolean) => void;
}) {
  return null;
}

export function RetexDictionarySettings(_props: { config: AppConfig }) {
  return null;
}
