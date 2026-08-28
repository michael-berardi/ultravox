import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { MainWindow } from "./pages/MainWindow";
import { SettingsPage, type SettingsTab } from "./pages/SettingsPage";
import { checkForUpdate, getAppStatus, getSettings, getUpdatePreferences, installUpdate, getPermissionStatus, openPermissionSettings, requestPermission, setUpdatePreferences, type AppConfig, type AppStatusResponse, type PermissionKind, type PermissionStatus, type UpdateInfo } from "./ipc";

const SUPPORT_INVITE_KEY = "ultravox-light:last-pro-support-invite";
const SUPPORT_INVITE_INTERVAL_MS = 30 * 24 * 60 * 60 * 1000;

export function supportInviteDue(lastShown: string | null, now = Date.now()): boolean {
  if (lastShown === null) return false;
  const timestamp = Number(lastShown);
  return !Number.isFinite(timestamp) || now - timestamp >= SUPPORT_INVITE_INTERVAL_MS;
}

const PERMISSIONS: Array<{kind: PermissionKind; label: string; copy: string}> = [
  { kind: "microphone", label: "Microphone", copy: "Record speech for local transcription." },
  { kind: "accessibility", label: "Accessibility", copy: "Place transcription in the focused text field." },
];

function PermissionPanel({status, onStatus}: {status: PermissionStatus; onStatus: (status: PermissionStatus) => void}) {
  const [busy, setBusy] = useState<PermissionKind | null>(null);
  const run = async (kind: PermissionKind, action: "request" | "settings") => {
    setBusy(kind);
    try { onStatus(action === "request" ? await requestPermission(kind) : (await openPermissionSettings(kind), await getPermissionStatus())); } finally { setBusy(null); }
  };
  return <div className="compact-modal-backdrop" role="presentation"><section className="compact-modal permission-panel" role="dialog" aria-modal="true" aria-labelledby="permission-title">
    <span className="eyebrow">First-run setup</span><h2 id="permission-title">Allow UltraVox Light to work</h2>
    <p className="compact-modal-copy">UltraVox Light checks each permission at runtime. Grant microphone access to record and Accessibility access to paste into the focused field.</p>
    <div className="permission-list">{PERMISSIONS.map(({kind,label,copy}) => <div className="permission-row" key={kind}><div><strong>{label}</strong><span>{copy}</span></div><span className={`permission-state permission-${status[kind]}`}>{status[kind] === "granted" ? "Granted" : status[kind] === "not_determined" ? "Needs access" : status[kind]}</span>{status[kind] !== "granted" && status[kind] !== "unavailable" && <div className="permission-actions"><button className="btn btn-primary" type="button" disabled={busy !== null} onClick={() => void run(kind,"request")}>{busy === kind ? "Checking…" : "Allow"}</button><button className="btn" type="button" disabled={busy !== null} onClick={() => void run(kind,"settings")}>Open Settings</button></div>}</div>)}</div>
    <p className="permission-footnote">After changing a permission, choose Recheck permissions.</p><button className="btn" type="button" onClick={() => void getPermissionStatus().then(onStatus)}>Recheck permissions</button>
  </section></div>;
}
function UpdatePrompt({info,automatic,busy,error,onAutomaticChange,onInstall,onLater}: {info: UpdateInfo;automatic:boolean;busy:boolean;error:string|null;onAutomaticChange:(enabled:boolean)=>Promise<void>;onInstall:()=>Promise<void>;onLater:()=>void}) { return <div className="compact-modal-backdrop" role="presentation"><section className="compact-modal permission-panel" role="dialog" aria-modal="true" aria-labelledby="update-title"><span className="eyebrow">Update available</span><h2 id="update-title">UltraVox Light {info.latest_version}</h2><p className="compact-modal-copy">The update is downloaded from the public release, then its SHA-256 checksum is verified before installation.</p><label className="settings-row update-automatic-choice"><input type="checkbox" checked={automatic} disabled={busy} onChange={e => void onAutomaticChange(e.target.checked)}/><span><strong>Install updates automatically</strong><small>Opt in to verified public updates.</small></span></label>{error && <p className="settings-error" role="alert">{error}</p>}<div className="compact-modal-actions"><button className="btn" type="button" disabled={busy} onClick={onLater}>Later</button><button className="btn btn-primary" type="button" disabled={busy} onClick={() => void onInstall()}>{busy ? "Verifying…" : "Update now"}</button></div></section></div>; }
export type AppStatus = AppStatusResponse["status"];
export default function App() {
  const [showSettings,setShowSettings]=useState(false),[settingsTab,setSettingsTab]=useState<SettingsTab>("shortcut"),[showSupportInvite,setShowSupportInvite]=useState(false),[settingsConfig,setSettingsConfig]=useState<AppConfig|null>(null),[status,setStatus]=useState<AppStatus>("loading"),[initialRecording,setInitialRecording]=useState(false),[permissionStatus,setPermissionStatus]=useState<PermissionStatus|null>(null),[availableUpdate,setAvailableUpdate]=useState<UpdateInfo|null>(null),[automaticUpdates,setAutomaticUpdates]=useState(false),[updateBusy,setUpdateBusy]=useState(false),[updateError,setUpdateError]=useState<string|null>(null);
  useEffect(() => { let cancelled=false; void Promise.allSettled([getAppStatus(),getSettings(),getPermissionStatus()]).then(([a,c,p])=>{if(cancelled)return;if(a.status==="fulfilled"){setStatus(a.value.status);setInitialRecording(a.value.recording);}if(c.status==="fulfilled")setSettingsConfig(c.value);if(p.status==="fulfilled")setPermissionStatus(p.value);}); return()=>{cancelled=true}; },[]);
  useEffect(() => { let cancelled=false; const check=async()=>{try{const [prefs,candidate]=await Promise.all([getUpdatePreferences(),checkForUpdate()]);if(cancelled)return;setAutomaticUpdates(prefs.automatic);if(!candidate){setAvailableUpdate(null);return;}if(!prefs.automatic){setAvailableUpdate(candidate);return;}setUpdateBusy(true);try{await installUpdate(candidate)}catch(error){if(!cancelled){setAvailableUpdate(candidate);setUpdateError(String(error))}}finally{if(!cancelled)setUpdateBusy(false)}}catch(error){if(!cancelled)console.error("Update check unavailable:",error)}};void check();const interval=window.setInterval(()=>void check(),86400000);return()=>{cancelled=true;window.clearInterval(interval)}},[false]);
  const openSettings=useCallback(async(tab: SettingsTab="shortcut")=>{if(!settingsConfig)setSettingsConfig(await getSettings());setSettingsTab(tab);setShowSettings(true)},[settingsConfig]);
  useEffect(()=>{const p=listen<string>("navigate-to",({payload})=>{if(payload==="settings")void openSettings()});return()=>{void p.then(u=>u())}},[openSettings]);
  const needsPermission=permissionStatus?Object.values(permissionStatus).some(v=>v!=="granted"&&v!=="unavailable"):false;
  useEffect(()=>{if(showSettings||needsPermission||availableUpdate||showSupportInvite)return;const now=Date.now(),lastShown=window.localStorage.getItem(SUPPORT_INVITE_KEY);if(lastShown===null){window.localStorage.setItem(SUPPORT_INVITE_KEY,String(now));return}if(!supportInviteDue(lastShown,now))return;const timer=window.setTimeout(()=>{window.localStorage.setItem(SUPPORT_INVITE_KEY,String(Date.now()));setShowSupportInvite(true)},45000);return()=>window.clearTimeout(timer)},[availableUpdate,needsPermission,showSettings,showSupportInvite]);
  const changeAutomaticUpdates=async(enabled:boolean)=>{setAutomaticUpdates(enabled);try{await setUpdatePreferences({automatic:enabled})}catch(error){setAutomaticUpdates(!enabled);setUpdateError(String(error))}};
  const installAvailableUpdate=async()=>{if(!availableUpdate)return;setUpdateBusy(true);setUpdateError(null);try{await installUpdate(availableUpdate)}catch(error){setUpdateError(String(error));setUpdateBusy(false)}};
  return <div className="app-shell">
    <div className="view-layer" hidden={showSettings||needsPermission} aria-hidden={showSettings||needsPermission}>
      <MainWindow status={status} initialRecording={initialRecording} onOpenSettings={()=>void openSettings()}/>
    </div>
    {settingsConfig&&<div className="view-layer" hidden={!showSettings||needsPermission} aria-hidden={!showSettings||needsPermission}>
      <SettingsPage initialConfig={settingsConfig} initialTab={settingsTab} onClose={()=>setShowSettings(false)}/>
    </div>}
    {!needsPermission&&availableUpdate&&<UpdatePrompt info={availableUpdate} automatic={automaticUpdates} busy={updateBusy} error={updateError} onAutomaticChange={changeAutomaticUpdates} onInstall={installAvailableUpdate} onLater={()=>setAvailableUpdate(null)}/>}
    {needsPermission&&permissionStatus&&<PermissionPanel status={permissionStatus} onStatus={setPermissionStatus}/>}
    {showSupportInvite&&!showSettings&&!needsPermission&&!availableUpdate&&<aside className="support-invitation" aria-labelledby="support-invitation-title">
      <div><span className="eyebrow">Once-a-month note</span><strong id="support-invitation-title">Help keep private transcription independent</strong><p>UltraVox Pro licenses fund signing, cross-platform releases, maintenance, and continued improvements to the free Light edition.</p></div>
      <div className="support-invitation-actions"><button className="btn" type="button" onClick={()=>setShowSupportInvite(false)}>Not now</button><button className="btn btn-primary" type="button" onClick={()=>{setShowSupportInvite(false);void openSettings("support")}}>Why Pro helps</button></div>
    </aside>}
  </div>;
}
