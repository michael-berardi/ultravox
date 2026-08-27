import type { CSSProperties } from "react";

export function RecordingMeter({ active, level }: { active: boolean; level: number }) {
  if (!active) return null;
  const normalized = Math.min(1, Math.max(0.04, level));
  return (
    <div className="recording-meter" aria-hidden="true" data-active>
      {Array.from({ length: 11 }, (_, index) => {
        const distance = Math.abs(index - 5) / 5;
        const weight = 1 - distance * 0.48;
        return (
          <span
            key={index}
            className="recording-meter-bar"
            style={{ "--meter-scale": Math.max(0.04, normalized * weight) } as CSSProperties}
          />
        );
      })}
    </div>
  );
}
