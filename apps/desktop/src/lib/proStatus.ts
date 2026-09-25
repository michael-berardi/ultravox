/** Contract types and pure helpers for the server-signed Pro entitlement.
 *
 * The renderer never unlocks anything by itself: `ProStatus` is whatever the
 * Pro module verified and reported over IPC. These helpers only normalize the
 * payload and recognise `pro-locked:` command errors so the UI can answer with
 * a lock prompt instead of a raw error string.
 */

export type ProState = "paid" | "trial" | "expired" | "none" | "unavailable";

export type ProStatus = {
  available: boolean;
  unlocked: boolean;
  state: ProState;
  trialUntil: string | null;
  updatesUntil: string | null;
  plan: string | null;
  label: string | null;
  error: string | null;
};

/** Event name carrying a fresh `ProStatus` whenever it changes. */
export const PRO_STATUS_EVENT = "pro://status-changed";

/** Locked Pro commands reject with an error string starting with this prefix. */
export const PRO_LOCKED_PREFIX = "pro-locked:";

const PRO_STATES: readonly ProState[] = ["paid", "trial", "expired", "none", "unavailable"];

function nullableString(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

/** Clamp an untrusted `get_pro_status` payload to the contract shape. */
export function mapProStatus(raw: unknown): ProStatus {
  const record =
    typeof raw === "object" && raw !== null ? (raw as Record<string, unknown>) : {};
  const available = record.available === true;
  const state =
    typeof record.state === "string" && (PRO_STATES as readonly string[]).includes(record.state)
      ? (record.state as ProState)
      : available
        ? "none"
        : "unavailable";
  return {
    available,
    unlocked: available && record.unlocked === true,
    state: available ? state : "unavailable",
    trialUntil: nullableString(record.trialUntil),
    updatesUntil: nullableString(record.updatesUntil),
    plan: nullableString(record.plan),
    label: nullableString(record.label),
    // "Not activated" is a state, not an error; the summary line already says it.
    error: available && state === "none" ? null : readableProError(record.error),
  };
}

/** Strip the IPC `pro-locked:` prefix and sentence-case the remaining reason. */
function readableProError(value: unknown): string | null {
  const raw = nullableString(value);
  if (!raw || !raw.trimStart().startsWith(PRO_LOCKED_PREFIX)) return raw;
  const reason = raw.trimStart().slice(PRO_LOCKED_PREFIX.length).trim();
  return reason ? reason.charAt(0).toUpperCase() + reason.slice(1) : null;
}

/** Friendly replacement copy for a raw `pro-locked:` error. */
export function proLockedMessage(feature: string): string {
  return `${feature} is part of UltraVox Pro. Open Settings → Pro to activate it or start a free trial.`;
}

/** True when a command failed because Pro is locked, not because it broke. */
export function isProLockedError(cause: unknown): boolean {
  const message = typeof cause === "string" ? cause : cause instanceof Error ? cause.message : "";
  return message.trimStart().startsWith(PRO_LOCKED_PREFIX);
}

/** Plain one-line state copy for the Pro settings panel. */
export function proStatusSummary(status: ProStatus): string {
  switch (status.state) {
    case "paid":
      return "Licensed on this device";
    case "trial":
      return status.trialUntil
        ? `Free trial until ${new Date(status.trialUntil).toLocaleDateString(undefined, { month: "long", day: "numeric" })}`
        : "Free trial active";
    case "expired":
      return status.plan === "trial" ? "Free trial ended" : "License expired";
    case "none":
      return "Not activated on this device";
    default:
      return "Pro is not available in this build";
  }
}
