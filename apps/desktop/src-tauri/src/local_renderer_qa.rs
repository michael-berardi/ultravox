//! PRIVATE local QA: fixed read-only observations of the existing visible main renderer.
//! No app-state, DOM/style, focus, visibility, recorder or permission mutations.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{mpsc, Mutex};
use std::time::Duration;
use tauri::Manager;

const MAX_RESULT: usize = 64 * 1024;
static CONTROL: Mutex<()> = Mutex::new(());
type Sender = mpsc::SyncSender<Result<Observation, String>>;
static PENDING: Mutex<Option<(String, Sender)>> = Mutex::new(None);

fn env_enabled(value: Option<&std::ffi::OsStr>) -> bool {
    value == Some(std::ffi::OsStr::new("1"))
}
pub fn require_enabled() -> Result<(), String> {
    if !env_enabled(std::env::var_os("ULTRAVOX_LOCAL_QA").as_deref()) {
        return Err("local QA disabled: requires ULTRAVOX_LOCAL_QA=1".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HeroRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl HeroRect {
    fn validate(&self) -> Result<(), String> {
        if [self.x, self.y, self.width, self.height]
            .iter()
            .any(|v| !v.is_finite())
            || self.x < 0.0
            || self.y < 0.0
            || self.width < 1.0
            || self.height < 1.0
            || self.x + self.width > 4096.0
            || self.y + self.height > 4096.0
            || self.width * self.height > 4_000_000.0
        {
            return Err("hero geometry outside bounds".into());
        }
        Ok(())
    }
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Beam {
    // Fixed order: level,cx,mw,bh,bendA,cy,glow,h,w,x0..x6,l0..l6,y0..y6.
    variables: Vec<Option<f64>>,
    // Pixel dimensions only; never canvas pixels or data URLs.
    canvases: Vec<[u32; 2]>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    interval_ms: f64,
    beams: Vec<Beam>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RasterSample {
    width: u32,
    height: u32,
    hash: u32,
    non_transparent: u32,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    visibility: String,
    theme: String,
    phase: String,
    status: String,
    hero: HeroRect,
    viewport: [f64; 2],
    beam_count: usize,
    reduced_motion: bool,
    frames: Vec<Frame>,
    raster_before: Vec<RasterSample>,
    raster_after: Vec<RasterSample>,
}
impl Observation {
    fn validate(&self) -> Result<(), String> {
        self.hero.validate()?;
        if self
            .viewport
            .iter()
            .any(|v| !v.is_finite() || !(1.0..=4096.0).contains(v))
            || self.hero.x + self.hero.width > self.viewport[0]
            || self.hero.y + self.hero.height > self.viewport[1]
        {
            return Err("invalid renderer viewport".into());
        }
        if self.visibility != "visible"
            || ![
                "midnight",
                "frutiger-aero",
                "frutiger-dark",
                "winamp",
                "winamp-mmd3",
                "winamp-bento",
                "winamp-hifi",
                "pioneer",
                "olive",
                "nord-frost",
                "solar-dusk",
                "vapor",
                "obsidian-rite",
                "crystal",
            ]
            .contains(&self.theme.as_str())
            || !["idle", "listening", "processing"].contains(&self.phase.as_str())
            || ![
                "ready",
                "loading",
                "unavailable",
                "recording",
                "meeting",
                "transcribing",
            ]
            .contains(&self.status.as_str())
            || self.beam_count > 2
            || ![1, 16].contains(&self.frames.len())
        {
            return Err("unexpected renderer diagnostics".into());
        }
        if self.raster_before.len() != self.raster_after.len() || self.raster_before.len() > 4 {
            return Err("invalid decorative canvas sample count".into());
        }
        for sample in self.raster_before.iter().chain(self.raster_after.iter()) {
            if !(1..=256).contains(&sample.width)
                || !(1..=192).contains(&sample.height)
                || sample.non_transparent > sample.width * sample.height
            {
                return Err("invalid bounded decorative canvas sample".into());
            }
        }
        for frame in &self.frames {
            if !frame.interval_ms.is_finite()
                || !(0.0..=3000.0).contains(&frame.interval_ms)
                || frame.beams.len() != self.beam_count
            {
                return Err("invalid observer frame".into());
            }
            for beam in &frame.beams {
                if beam.variables.len() != 30
                    || beam.canvases.len() > 2
                    || beam
                        .variables
                        .iter()
                        .flatten()
                        .any(|v| !v.is_finite() || v.abs() > 1_000_000.0)
                    || beam.canvases.iter().flatten().any(|v| *v > 8192)
                {
                    return Err("invalid bounded beam diagnostics".into());
                }
            }
        }
        Ok(())
    }
}
fn nonce_matches(expected: &str, nonce: &str, label: &str) -> bool {
    label == "main" && expected == nonce
}
fn decode_result(result: Value) -> Result<Observation, String> {
    let object = result.as_object().ok_or("invalid QA result")?;
    if object.len() == 1 && object.contains_key("error") {
        return Err(match result["error"].as_str() {
            Some("not_visible") => "document is not visible",
            Some("hero_unavailable") => "recorder data/geometry unavailable",
            Some("hero_missing") => "recording panel is not mounted (setup/model view)",
            Some("recorder_hidden") => "recording panel is hidden by another application view",
            Some("dialog_open") => "a dialog blocks the recorder view",
            Some("hero_structure") => "recorder contains unexpected content; diagnostics refused",
            Some("hero_outside") => "recording panel is outside the viewport",
            Some("history_overlap") => "history overlaps the recording panel",
            Some("hero_occluded") => "another renderer element covers the recording panel",
            Some("status_unknown") => "recorder status class is not recognized",
            Some("observer_timeout") => "visible renderer rAF observation timed out",
            Some("layout_changed") => "hero layout or renderer state changed during observation",
            _ => "fixed renderer observation failed",
        }
        .into());
    }
    let observation: Observation =
        serde_json::from_value(result).map_err(|_| "unexpected QA result fields or types")?;
    observation.validate()?;
    Ok(observation)
}
#[tauri::command]
pub fn local_renderer_qa_result(
    window: tauri::WebviewWindow,
    nonce: String,
    result: Value,
) -> Result<(), String> {
    require_enabled()?;
    if window.label() != "main" || nonce.len() > 36 {
        return Err("QA callback requires main and a pending nonce".into());
    }
    if serde_json::to_vec(&result)
        .map_err(|_| "QA result serialization failed")?
        .len()
        > MAX_RESULT
    {
        return Err("QA result exceeds 64KiB".into());
    }
    let mut pending = PENDING.lock().map_err(|_| "QA pending lock")?;
    if !pending
        .as_ref()
        .is_some_and(|(n, _)| nonce_matches(n, &nonce, window.label()))
    {
        return Err("unknown QA nonce".into());
    }
    if let Some((_, tx)) = pending.take() {
        let _ = tx.try_send(decode_result(result));
    }
    Ok(())
}

// Only diagnostic-owned rAF/timer handles are written to this private transient slot.
// It never touches application state, library callbacks, DOM, styles, inputs or storage.
// Both JS finally and native timeout cleanup cancel these handles; nothing persists.
const SCRIPT: &str = r#"
(() => {
  const nonce = NONCE, count = COUNT;
  let raf = 0, timer = 0, done = false;
  const slot = '__ULTRAVOX_LOCAL_QA_OBSERVER__';
  const cleanup = () => { cancelAnimationFrame(raf); clearTimeout(timer);
    if (window[slot]?.nonce === nonce) delete window[slot]; };
  const finish = result => { if (done) return; done = true; cleanup();
    void window.__TAURI_INTERNALS__.invoke('local_renderer_qa_result', {nonce, result}).catch(() => {}); };
  if (window[slot]) { finish({error:'observer_failed'}); return; }
  window[slot] = {nonce, cancel: () => { done = true; cleanup(); }};
  const read = () => {
    if (document.visibilityState !== 'visible') throw 'not_visible';
    const heroes = document.querySelectorAll('.hero');
    if (heroes.length !== 1) throw 'hero_missing';
    const hero = heroes[0], r = hero.getBoundingClientRect();
    if (!hero.getClientRects().length || hero.closest('[hidden],[inert],[aria-hidden="true"]')) throw 'recorder_hidden';
    if (document.querySelector('[role="dialog"],.popover,.url-import-popover')) throw 'dialog_open';
    // Disallow user-content, error notices, popovers and intersecting history.
    if (hero.querySelector('input,textarea,[contenteditable],.activity-error,.pro-lock-prompt') ||
        Array.from(hero.children).some(e => !e.matches('.recording-surround,#activity-status,.record-button,.secondary-actions')))
      throw 'hero_structure';
    if (r.x < 0 || r.y < 0 || r.width < 1 || r.height < 1 || r.right > innerWidth || r.bottom > innerHeight) throw 'hero_outside';
    for (const h of document.querySelectorAll('.history-section')) {
      const hr = h.getBoundingClientRect();
      if (hr.left < r.right && hr.right > r.left && hr.top < r.bottom && hr.bottom > r.top) throw 'history_overlap';
    }
    for (const x of [.02,.5,.98]) for (const y of [.02,.5,.98]) {
      const e = document.elementFromPoint(r.x+r.width*x, r.y+r.height*y);
      if (!e || !hero.contains(e)) throw 'hero_occluded';
    }
    const status = ['ready','loading','unavailable','recording','meeting','transcribing'].find(s => hero.querySelector('#activity-status')?.classList.contains(s));
    const beams = Array.from(hero.querySelectorAll('[data-voice-beam]'));
    if (!status || beams.length > 2) throw 'status_unknown';
    return {visibility:document.visibilityState,theme:document.documentElement.dataset.theme,
      phase:hero.dataset.voicePhase,status,hero:{x:r.x,y:r.y,width:r.width,height:r.height},
      viewport:[innerWidth,innerHeight],beam_count:beams.length,reduced_motion:matchMedia('(prefers-reduced-motion: reduce)').matches,
      beams:beams.map(el => {
        const id = el.getAttribute('data-voice-beam');
        if (!id || !/^[a-zA-Z0-9_-]{1,80}$/.test(id)) throw 'hero_unavailable';
        const css = getComputedStyle(el);
        const keys = ['level','cx','mw','bh','bendA','cy','glow','h','w',
          ...Array.from({length:7},(_,i)=>'x'+i),...Array.from({length:7},(_,i)=>'l'+i),...Array.from({length:7},(_,i)=>'y'+i)];
        return {variables:keys.map(k => {
          const s=css.getPropertyValue('--vb-'+k+'-'+id).trim();
          if (!s) return null;
          if (!/^-?(?:\d+(?:\.\d+)?|\.\d+)(?:px|deg)?$/.test(s)) throw 'hero_unavailable';
          const n=parseFloat(s); if (!Number.isFinite(n) || Math.abs(n)>1000000) throw 'hero_unavailable'; return n;
        }),canvases:Array.from(el.querySelectorAll('canvas[data-voice-beam-band],canvas[data-voice-beam-band-halo]')).map(c=>[c.width,c.height])};
      })};
  };
  // Only the library's isolated decorative Canvas2D buffers are sampled.
  // They draw curves/colors from RMS, never DOM, desktop, text or user images.
  // Export bounded numeric fingerprints, never pixels or data URLs. Readback
  // occurs before/after the finite frame observer, not inside its timing loop.
  const raster = () => Array.from(document.querySelectorAll('.hero [data-voice-beam] canvas[data-voice-beam-band],.hero [data-voice-beam] canvas[data-voice-beam-band-halo]')).map(c => {
    if (c.width < 1 || c.height < 1) throw 'hero_unavailable';
    const width=Math.min(256,c.width),height=Math.min(192,c.height);
    const pixels=c.getContext('2d').getImageData(Math.floor((c.width-width)/2),c.height-height,width,height).data;
    let hash=2166136261,non_transparent=0;
    for(let i=0;i<pixels.length;i++){hash=Math.imul(hash^pixels[i],16777619);if(i%4===3&&pixels[i]>0)non_transparent++;}
    return {width,height,hash:hash>>>0,non_transparent};
  });
  let first, rasterBefore, previous, frames = [];
  try { first=read(); rasterBefore=raster(); } catch(e) { finish({error:typeof e==='string'?e:'observer_failed'}); return; }
  const sample = t => { try {
    const s=read();
    if (JSON.stringify([s.hero,s.phase,s.theme,s.status,s.beam_count]) !== JSON.stringify([first.hero,first.phase,first.theme,first.status,first.beam_count])) throw 'layout_changed';
    frames.push({interval_ms:previous===undefined?0:t-previous,beams:s.beams}); previous=t;
    if (frames.length===count) { const {beams,...metadata}=s; finish({...metadata,frames,raster_before:rasterBefore,raster_after:raster()}); }
    else raf=requestAnimationFrame(sample);
  } catch(e) { finish({error:typeof e==='string'?e:'observer_failed'}); } };
  timer=setTimeout(()=>finish({error:'observer_timeout'}),2500);
  if (count===1) sample(performance.now()); else raf=requestAnimationFrame(sample);
})()
"#;

struct PendingGuard {
    nonce: String,
    window: tauri::WebviewWindow,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        if let Ok(mut pending) = PENDING.lock() {
            if pending.as_ref().is_some_and(|(n, _)| n == &self.nonce) {
                pending.take();
            }
        }
        // Fixed cleanup, never a caller-provided script. Also runs after eval errors/timeouts.
        let nonce = serde_json::to_string(&self.nonce).unwrap();
        let _ = self.window.eval(&format!("(()=>{{const s=window.__ULTRAVOX_LOCAL_QA_OBSERVER__;if(s?.nonce==={nonce})s.cancel();}})()"));
    }
}
fn metadata(window: &tauri::WebviewWindow) -> Result<Value, String> {
    let visible = window
        .is_visible()
        .map_err(|_| "native visibility unavailable")?;
    let minimized = window
        .is_minimized()
        .map_err(|_| "native minimized state unavailable")?;
    let onscreen = crate::native_qa_visibility::is_onscreen(window)?;
    if !visible || minimized || !onscreen {
        return Err("main is not visible (or is minimized); QA never shows or focuses it".into());
    }
    Ok(
        json!({"visible":visible,"minimized":minimized,"onscreen":onscreen,"focused":window.is_focused().map_err(|_| "native focus unavailable")?}),
    )
}
fn observe(window: &tauri::WebviewWindow, frames: usize) -> Result<Observation, String> {
    require_enabled()?;
    metadata(window)?;
    let nonce = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::sync_channel(1);
    {
        let mut pending = PENDING.lock().map_err(|_| "QA pending lock")?;
        if pending.is_some() {
            return Err("QA observer busy".into());
        }
        *pending = Some((nonce.clone(), tx));
    }
    let _guard = PendingGuard {
        nonce: nonce.clone(),
        window: window.clone(),
    };
    let script = SCRIPT
        .replace("NONCE", &serde_json::to_string(&nonce).unwrap())
        .replace("COUNT", &frames.to_string());
    window
        .eval(&script)
        .map_err(|_| "fixed QA script could not be scheduled")?;
    rx.recv_timeout(Duration::from_secs(4)).map_err(|e| {
        match e {
            mpsc::RecvTimeoutError::Timeout => {
                "main renderer callback timed out (hidden/throttled/unresponsive); no evidence"
            }
            mpsc::RecvTimeoutError::Disconnected => "main renderer callback disconnected",
        }
        .to_string()
    })?
}
fn motion_summary(observation: &Observation) -> Value {
    let intervals: Vec<f64> = observation
        .frames
        .iter()
        .skip(1)
        .map(|f| f.interval_ms)
        .collect();
    let mut sorted = intervals.clone();
    sorted.sort_by(f64::total_cmp);
    let p95 = (!sorted.is_empty()).then(|| sorted[(sorted.len() * 95).div_ceil(100) - 1]);
    let mut changes = 0;
    for pair in observation.frames.windows(2) {
        for (a, b) in pair[0].beams.iter().zip(&pair[1].beams) {
            changes += a
                .variables
                .iter()
                .zip(&b.variables)
                .filter(|(a, b)| a != b)
                .count();
        }
    }
    let raster_changes = observation
        .raster_before
        .iter()
        .zip(&observation.raster_after)
        .filter(|(a, b)| a.hash != b.hash)
        .count();
    let nontransparent = observation
        .raster_after
        .iter()
        .filter(|s| s.non_transparent > 0)
        .count();
    json!({"sampledLibraryVariableChanges":changes,"libraryCanvasRasterChanges":raster_changes,
        "nonTransparentCanvasCount":nontransparent,"rasterScope":"bounded numeric fingerprints of decorative library canvases only; no DOM/window pixels","observerIntervalCount":intervals.len(),
        "observerMeanIntervalMs":if intervals.is_empty() { None } else { Some(intervals.iter().sum::<f64>() / intervals.len() as f64) },
        "observerP95IntervalMs":p95,
        "libraryCallbackCount":null,"libraryCallbacksInstrumented":false})
}
fn activity(app: &tauri::AppHandle) -> Value {
    let state = app.state::<crate::state::AppState>();
    json!({"recordingActive":state.session.try_lock().ok().map(|s| s.is_some()),
        "meetingActive":state.meeting_session.try_lock().ok().map(|s| s.is_some()),
        "transcriptionActive":state.active_transcription.try_lock().ok().map(|s| s.is_some()),
        "meaning":"point-in-time native state booleans; null means lock busy; not an atomic idle/restart guarantee"})
}
pub fn health(app: &tauri::AppHandle) -> Result<Value, String> {
    require_enabled()?;
    let state = app.state::<crate::state::AppState>();
    let english_model_installed = state.models_dir().ok().map(|path| ultravox_core::phonon::is_installed(&path));
    Ok(json!({"nativeActivity":activity(app),"permissions":crate::commands::get_permission_status(),
        "englishModelInstalledAtConfiguredRoot":english_model_installed,
        "evidence":"read-only native activity/permission/model metadata; no renderer, input or control mutation"}))
}

pub fn run(app: &tauri::AppHandle, capture: bool, _path: Option<String>) -> Result<Value, String> {
    require_enabled()?;
    // No primary pixels: an asynchronous crop is not a privacy boundary.
    // Keep only the fixed numeric observer; captures use the effect-denied mirror.
    if capture {
        return Err("renderer-capture unavailable: primary pixels could expose transient private content; use the native mirror".into());
    }
    let _control = CONTROL.try_lock().map_err(|_| "local QA busy")?;
    let window = app
        .get_webview_window("main")
        .ok_or("main renderer unavailable")?;
    let before = metadata(&window)?;
    let observation = observe(&window, 16)?;
    let after = metadata(&window)?;
    Ok(
        json!({"nativeBefore":before,"nativeAfter":after,"nativeActivity":activity(app),
        "motion":motion_summary(&observation),"diagnostics":observation,
        "observerFrames":observation.frames.len(),"libraryCallbackCount":null,
        "evidence":"actual visible-renderer observer intervals, library CSS variables and decorative Canvas2D fingerprints; NOT library callback counts, full-window pixels or GPU efficiency proof"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn env_requires_exact_one() {
        for value in [None, Some(""), Some("0"), Some("true"), Some("01")] {
            assert!(!env_enabled(value.map(std::ffi::OsStr::new)));
        }
        assert!(env_enabled(Some(std::ffi::OsStr::new("1"))));
    }
    #[test]
    fn nonce_and_main_required() {
        assert!(nonce_matches("n", "n", "main"));
        assert!(!nonce_matches("n", "other", "main"));
        assert!(!nonce_matches("n", "n", "native-mirror"));
    }
    #[test]
    fn unexpected_fields_rejected() {
        assert!(decode_result(json!({"error":"not_visible","text":"secret"})).is_err());
        assert!(serde_json::from_value::<HeroRect>(
            json!({"x":0,"y":0,"width":10,"height":10,"script":"x"})
        )
        .is_err());
    }
    #[test]
    fn cleanup_is_nonce_scoped_and_read_only() {
        assert!(SCRIPT.contains("cancelAnimationFrame(raf); clearTimeout(timer)"));
        assert!(SCRIPT.contains("canvas[data-voice-beam-band]"));
        assert!(SCRIPT.contains("Math.min(256,c.width)"));
        assert!(!SCRIPT.contains("toDataURL"));
        assert!(!SCRIPT.contains("drawImage"));
        for forbidden in [
            "localStorage",
            "textContent",
            "innerText",
            ".finish(",
            "setProperty",
            ".focus(",
        ] {
            assert!(!SCRIPT.contains(forbidden));
        }
    }
}
