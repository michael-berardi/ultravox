//! One hidden native window in the production application; no separate transport.
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::Write,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::Manager;
pub const LABEL: &str = "native-mirror";
const MAX_FRAME: usize = 64 * 1024;

static CONTROL: Mutex<()> = Mutex::new(());
static SOCKET: Mutex<Option<PathBuf>> = Mutex::new(None);
type EvalSender = mpsc::SyncSender<Value>;
static EVAL: OnceLock<Mutex<Option<(String, String, EvalSender)>>> = OnceLock::new();
static GENERATION: Mutex<Option<String>> = Mutex::new(None);
fn now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
pub fn invoke_allowed(label: &str, command: &str) -> bool {
    label != LABEL
        || matches!(
            command,
            "vs_read_audio"
                | "vs_list_voices"
                | "vs_corpus_candidates"
                | "vs_create_voice"
                | "vs_delete_voice"
                | "vs_rename_voice"
                | "vs_speak"
                | "vs_tts_status"
                | "mirror_eval_result"
                | "get_app_info"
                | "get_app_status"
                | "get_pro_status"
                | "get_media_state"
                | "voice_generations_list"
                | "get_settings"
                | "get_permission_status"
                | "get_app_telemetry_status"
                | "get_update_preferences"
                | "list_recordings"
                | "search_recordings"
                | "get_recording"
                | "get_shortcut_settings"
                | "get_transcription_status"
                | "get_model_catalog"
                | "get_downloads"
                | "get_download_progress"
                | "get_model_progress"
                | "is_model_downloaded"
                | "get_audio_devices"
                | "get_audio_input_config"
                | "plugin:event|listen"
                | "plugin:event|unlisten"
        )
}
fn nonce_matches(expected: &str, label: &str, nonce: &str, caller: &str) -> bool {
    expected == nonce && label == caller
}
#[tauri::command]
pub fn mirror_eval_result(
    window: tauri::WebviewWindow,
    nonce: String,
    result: Value,
) -> Result<(), String> {
    if serde_json::to_vec(&result)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_FRAME
    {
        return Err("eval result too large".into());
    }
    let mut slot = EVAL
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "eval lock")?;
    if !slot
        .as_ref()
        .is_some_and(|(n, label, _)| nonce_matches(n, label, &nonce, window.label()))
    {
        return Err("unknown eval nonce".into());
    }
    if let Some((_, _, tx)) = slot.take() {
        let _ = tx.try_send(result);
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase", deny_unknown_fields)]
enum Control {
    Open { seconds: u64 },
    State,
    Eval { script: String },
    Capture { path: String },
    Resize { width: u32, height: u32 },
    Close,
    Refresh { revision: String },
    Apply,
    Rollback,
}
fn parse_control(v: Value) -> Result<Control, String> {
    let object = v.as_object().ok_or("control must be an object")?;
    if matches!(v["cmd"].as_str(), Some("state" | "close")) && object.len() != 1 {
        return Err("unexpected control fields".into());
    }
    let r: Control = serde_json::from_value(v).map_err(|e| e.to_string())?;
    if matches!(&r,Control::Eval{script} if script.is_empty()||script.len()>32*1024) {
        return Err("eval script must be 1..32768 bytes".into());
    }
    if matches!(&r, Control::Resize { width, height } if !(400..=3840).contains(width) || !(500..=2160).contains(height))
    {
        return Err(
            "mirror viewport must be within the app window bounds (400..3840 × 500..2160)".into(),
        );
    }
    if matches!(&r, Control::Open { seconds } if !(1..=3600).contains(seconds)) {
        return Err("seconds must be 1..3600".into());
    }
    Ok(r)
}
fn settle_script(label: &str) -> &'static str {
    if label != LABEL {
        return "";
    }
    // Hidden WebKit pauses finite layout transitions as well as display frames.
    // Complete them only in this disposable renderer; never mutate the primary.
    // Hidden WebKit can suspend timers. Settle finite animation styles without
    // waiting on a timer that can prevent even a read-only eval from replying.
    r#"for(let pass=0;pass<2;pass++){let changed=false;for(const a of document.getAnimations?.()??[]){if(a.playState!=="finished" && Number.isFinite(a.effect?.getComputedTiming().endTime)){try{a.finish();changed=true}catch{}}}if(!changed)break;document.documentElement.getBoundingClientRect();}"#
}
fn evaluate(window: &tauri::WebviewWindow, script: &str) -> Result<Value, String> {
    let nonce = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::sync_channel(1);
    {
        let mut pending = EVAL
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "eval lock")?;
        if pending.is_some() {
            return Err("eval busy".into());
        }
        *pending = Some((nonce.clone(), window.label().to_string(), tx));
    }
    let js=format!("(async()=>{{const nonce={};let result;try{{result={{ok:true,value:await (async()=>{{ {} }})()}};JSON.stringify(result)}}catch(e){{result={{ok:false,error:String(e)}}}};try{{await window.__TAURI_INTERNALS__.invoke('mirror_eval_result',{{nonce,result}})}}catch(e){{await window.__TAURI_INTERNALS__.invoke('mirror_eval_result',{{nonce,result:{{ok:false,error:String(e)}}}})}}}})()",serde_json::to_string(&nonce).unwrap(),format!("{}\n{}", settle_script(window.label()), script));
    let answer = window.eval(&js).map_err(|e| e.to_string()).and_then(|_| {
        rx.recv_timeout(Duration::from_secs(8))
            .map_err(|_| "hidden window eval timed out".into())
    });
    *EVAL.get().unwrap().lock().map_err(|_| "eval lock")? = None;
    answer
}
fn screenshot_path(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if p.parent() != Some(Path::new("/tmp/screenshots"))
        || p.extension().and_then(|s| s.to_str()) != Some("png")
        || p.file_name().is_none()
    {
        return Err("capture requires /tmp/screenshots/<name>.png".into());
    }
    Ok(p)
}
fn capture(window: &tauri::WebviewWindow, path: &str) -> Result<Value, String> {
    let path = screenshot_path(path)?;
    // The screenshot directory may be shared, but never follow a supplied symlink.
    if !Path::new("/tmp/screenshots").exists() {
        std::fs::create_dir("/tmp/screenshots").map_err(|e| e.to_string())?;
    }
    let m = std::fs::symlink_metadata("/tmp/screenshots").map_err(|e| e.to_string())?;
    if !m.is_dir() || m.file_type().is_symlink() || m.uid() != unsafe { libc::geteuid() } {
        return Err("unsafe screenshots directory".into());
    }
    let settled = evaluate(window, "return true")?;
    if settled["ok"] != true {
        return Err("mirror layout did not settle".into());
    }
    let (tx, rx) = mpsc::sync_channel(1);
    window
        .with_webview(move |webview| unsafe { snapshot::take(webview.inner() as *mut _, tx) })
        .map_err(|e| e.to_string())?;
    let bytes=rx.recv_timeout(Duration::from_secs(10)).map_err(|_|"hidden WKWebView snapshot timed out; window remains invisible. Use state/eval DOM inspection; no screen capture fallback.")??;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    Ok(json!({"path":path,"bytes":bytes.len(),"renderer":"WKWebView.takeSnapshot","visible":false}))
}

fn build_mirror(
    app: &tauri::AppHandle,
    seed: &Value,
    path: &str,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let mut initialization = format!(
        "window.__ULTRAVOX_MIRROR_SEED__ = {};",
        serde_json::to_string(seed).map_err(|e| e.to_string())?
    );
    // JS fetch guards alone do not cover images, beacons, WebSockets or forms.
    // Install the policy as the parser creates <head>, before the app mounts.
    initialization.push_str(r#"
        (() => {
          const lock = () => {
            if (!document.head) return false;
            const policy = document.createElement('meta');
            policy.httpEquiv = 'Content-Security-Policy';
            policy.content = "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src ipc: http://ipc.localhost; media-src blob:; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'self'";
            document.head.prepend(policy);
            return true;
          };
          if (!lock()) {
            const observer = new MutationObserver(() => { if (lock()) observer.disconnect(); });
            observer.observe(document, {childList:true, subtree:true});
          }
        })();
    "#);
    let source_url = app
        .get_webview_window("main")
        .ok_or("main unavailable")?
        .url()
        .map_err(|e| e.to_string())?;
    let scheme = source_url.scheme().to_owned();
    let host = source_url.host_str().map(str::to_owned);
    let port = source_url.port();
    let window = tauri::WebviewWindowBuilder::new(app, LABEL, tauri::WebviewUrl::App(path.into()))
        .title("UltraVox Native Mirror")
        .inner_size(width, height)
        .decorations(false)
        .visible(false)
        .focused(false)
        .initialization_script(&initialization)
        .on_navigation(move |url| {
            url.scheme() == scheme
                && url.host_str() == host.as_deref()
                && url.port() == port
                && url.query() == Some("native-mirror=1")
        })
        .build()
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    window
        .with_webview(|webview| unsafe {
            snapshot::keep_scheduling(webview.inner() as *mut _);
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}
static HOT_STATE: Mutex<(Option<String>, Option<String>)> = Mutex::new((None, None));
fn loaded(window: &tauri::WebviewWindow) -> Option<String> {
    let url = window.url().ok()?;
    Some(
        url.path()
            .strip_prefix("/__hot/")?
            .split('/')
            .next()?
            .to_owned(),
    )
}
fn refresh(app: &tauri::AppHandle, revision: &str) -> Result<Value, String> {
    let main = app.get_webview_window("main").ok_or("main unavailable")?;
    let origin = main.url().map_err(|e| e.to_string())?;
    if origin.scheme() != "tauri"
        && !(origin.scheme() == "http" && origin.host_str() == Some("tauri.localhost"))
    {
        return Err("hot assets require embedded custom-protocol debug build, not devUrl".into());
    }
    if revision != "embedded" {
        crate::hot_assets::load(revision)?;
    }
    let window = app.get_webview_window(LABEL).ok_or("mirror is closed")?;
    // The adapter keeps mirror storage in memory. Copy it, never into primary storage.
    let snapshot = evaluate(&window, "const seed = window.__ULTRAVOX_MIRROR_SNAPSHOT__?.() ?? window.__ULTRAVOX_MIRROR_SEED__; if (!seed) throw Error('mirror seed unavailable'); return {...seed, storage: Object.fromEntries(Array.from({length: localStorage.length}, (_, i) => localStorage.key(i)).filter(k => k !== null).map(k => [k, localStorage.getItem(k)]))};")?;
    if snapshot["ok"] != true || !snapshot["value"]["storage"].is_object() {
        return Err("mirror snapshot failed".into());
    }
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let old = Some(loaded(&window).unwrap_or_else(|| "embedded".into()));
    window.destroy().map_err(|e| e.to_string())?;
    build_mirror(
        app,
        &snapshot["value"],
        &if revision == "embedded" {
            "index.html?native-mirror=1".into()
        } else {
            format!("__hot/{revision}/index.html?native-mirror=1")
        },
        size.width as f64 / scale,
        size.height as f64 / scale,
    )?;
    // Keep the original generation/TTL: refresh cannot extend the deadline.
    let mut state = HOT_STATE.lock().map_err(|_| "hot state lock")?;
    if old.as_deref() != Some(revision) {
        state.1 = old;
    }
    state.0 = Some(revision.to_owned());
    Ok(
        json!({"requestedRevision":revision,"primaryUntouched":true,"verify":"state then eval/capture; navigation is asynchronous"}),
    )
}
pub fn open(app: &tauri::AppHandle, seconds: u64) -> Result<Value, String> {
    if !(1..=3600).contains(&seconds) {
        return Err("seconds must be 1..3600".into());
    }
    if app.get_webview_window(LABEL).is_some() {
        return Err("mirror already open".into());
    }
    let main = app
        .get_webview_window("main")
        .ok_or("main renderer unavailable")?;
    let result = evaluate(&main, "return window.__ULTRAVOX_MIRROR_SNAPSHOT__?.()")?;
    let seed = &result["value"];
    if result["ok"] != true || !seed["storage"].is_object() {
        return Err("main mirror snapshot not ready".into());
    }
    sandbox()?;
    build_mirror(app, seed, "index.html?native-mirror=1", 450.0, 650.0)?;
    let generation = uuid::Uuid::new_v4().to_string();
    *GENERATION.lock().map_err(|_| "generation lock")? = Some(generation.clone());
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(seconds));
        let _control = CONTROL.lock().unwrap();
        let mut current = GENERATION.lock().unwrap();
        if current.as_ref() == Some(&generation) {
            if let Some(window) = app.get_webview_window(LABEL) {
                let _ = window.destroy();
            }
            *current = None;
            cleanup_sandbox();
        }
    });
    Ok(json!({"label":LABEL,"visible":false,"seconds":seconds}))
}
pub fn control(app: &tauri::AppHandle, value: Value) -> Result<Value, String> {
    let request = parse_control(value)?;
    let _guard = CONTROL.try_lock().map_err(|_| "mirror control busy")?;
    match &request {
        Control::Refresh { revision } => return refresh(app, revision),
        Control::Rollback => {
            let previous = HOT_STATE
                .lock()
                .map_err(|_| "hot state lock")?
                .1
                .clone()
                .ok_or(
                    "no previous staged mirror revision; refresh a retained revision explicitly",
                )?;
            return refresh(app, &previous);
        }
        Control::Apply => {
            let mirror = app.get_webview_window(LABEL).ok_or("mirror is closed")?;
            let revision = loaded(&mirror).unwrap_or_else(|| "embedded".into());
            let requested = HOT_STATE.lock().map_err(|_| "hot state lock")?.0.clone();
            if requested.as_deref() != Some(&revision) {
                return Err("mirror revision not current".into());
            }
            let main = app.get_webview_window("main").ok_or("main unavailable")?;
            let mut url = main.url().map_err(|e| e.to_string())?;
            url.set_path(&if revision == "embedded" {
                "/index.html".into()
            } else {
                format!("/__hot/{revision}/index.html")
            });
            url.set_query(None);
            url.set_fragment(None);
            if let Err(error) = main.navigate(url) {
                let _ = main.eval("sessionStorage.removeItem('ultravox.hot-ui-state')");
                return Err(error.to_string());
            }
            return Ok(
                json!({"requestedPrimaryRevision":revision,"navigationOnly":true,"verify":"state reports loaded URLs; eval/capture verifies render"}),
            );
        }
        _ => {}
    }
    if let Control::Open { seconds } = request {
        return open(app, seconds);
    }
    if matches!(request, Control::State) && app.get_webview_window(LABEL).is_none() {
        return Ok(json!({"label":LABEL,"open":false,"hotRefresh":true}));
    }
    let window = app.get_webview_window(LABEL).ok_or("mirror is closed")?;
    match request {
        Control::State => Ok(
            json!({"label":LABEL,"open":true,"hotRefresh":true,"visible":window.is_visible().unwrap_or(false),"focused":window.is_focused().unwrap_or(false),"native":true,"url":window.url().ok().map(|u|u.to_string()),"primaryUrl":app.get_webview_window("main").and_then(|w|w.url().ok()).map(|u|u.to_string()),"loadedRevision":window.url().ok().and_then(|u|u.path().strip_prefix("/__hot/").map(|p|p.split('/').next().unwrap_or("").to_owned()))}),
        ),
        Control::Eval { script } => evaluate(&window, &script),
        Control::Capture { path } => capture(&window, &path),
        Control::Resize { width, height } => {
            window
                .set_size(tauri::LogicalSize::new(width as f64, height as f64))
                .map_err(|e| e.to_string())?;
            Ok(json!({"width":width,"height":height}))
        }
        Control::Close => {
            window.destroy().map_err(|e| e.to_string())?;
            *GENERATION.lock().map_err(|_| "generation lock")? = None;
            cleanup_sandbox();
            Ok(json!({"closing":true}))
        }
        Control::Open { .. } => unreachable!(),
        Control::Refresh { .. } | Control::Apply | Control::Rollback => unreachable!(),
    }
}
/// Objective-C block ABI with explicit Arc copy/dispose ownership. WK retains the
/// completion block until callback; a timed-out receiver cannot leave a dangling pointer.
#[cfg(target_os = "macos")]
mod snapshot {
    use super::*;
    use std::{
        ffi::{c_char, c_void, CStr},
        mem,
    };
    type Id = *mut c_void;
    type Sender = mpsc::SyncSender<Result<Vec<u8>, String>>;
    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Id;
        fn objc_msgSend();
    }
    #[link(name = "AppKit", kind = "framework")]
    extern "C" {}
    #[link(name = "WebKit", kind = "framework")]
    extern "C" {}
    extern "C" {
        static _NSConcreteStackBlock: c_void;
        fn _Block_copy(block: *const c_void) -> *mut c_void;
        fn _Block_release(block: *const c_void);
    }
    #[repr(C)]
    struct Descriptor {
        reserved: usize,
        size: usize,
        copy: unsafe extern "C" fn(*mut Block, *const Block),
        dispose: unsafe extern "C" fn(*mut Block),
    }
    #[repr(C)]
    struct Block {
        isa: *const c_void,
        flags: i32,
        reserved: i32,
        invoke: unsafe extern "C" fn(*mut Block, Id, Id),
        descriptor: *const Descriptor,
        sender: *const Sender,
    }
    unsafe extern "C" fn copy(dst: *mut Block, src: *const Block) {
        Arc::increment_strong_count((*src).sender);
        (*dst).sender = (*src).sender;
    }
    unsafe extern "C" fn dispose(block: *mut Block) {
        Arc::decrement_strong_count((*block).sender);
    }
    static DESCRIPTOR: Descriptor = Descriptor {
        reserved: 0,
        size: mem::size_of::<Block>(),
        copy,
        dispose,
    };
    unsafe fn sel(s: &'static [u8]) -> Id {
        sel_registerName(s.as_ptr().cast())
    }
    unsafe fn class(s: &'static [u8]) -> Id {
        objc_getClass(s.as_ptr().cast())
    }
    unsafe fn get(obj: Id, s: &'static [u8]) -> Id {
        let f: unsafe extern "C" fn(Id, Id) -> Id = mem::transmute(objc_msgSend as *const ());
        f(obj, sel(s))
    }
    // Public WebKit API, available on macOS 14+. Only the disposable mirror's
    // WKPreferences are changed; neither NSWindow visibility nor focus changes.
    pub unsafe fn keep_scheduling(webview: Id) {
        let preferences = get(get(webview, b"configuration\0"), b"preferences\0");
        let responds: unsafe extern "C" fn(Id, Id, Id) -> bool =
            mem::transmute(objc_msgSend as *const ());
        let setter = sel(b"setInactiveSchedulingPolicy:\0");
        if !preferences.is_null() && responds(preferences, sel(b"respondsToSelector:\0"), setter) {
            let set: unsafe extern "C" fn(Id, Id, isize) =
                mem::transmute(objc_msgSend as *const ());
            set(preferences, setter, 2); // WKInactiveSchedulingPolicyNone
        }
    }
    unsafe extern "C" fn complete(block: *mut Block, image: Id, error: Id) {
        let result = (|| -> Result<Vec<u8>, String> {
            if !error.is_null() || image.is_null() {
                let detail = if error.is_null() {
                    "nil image".into()
                } else {
                    let text = get(get(error, b"localizedDescription\0"), b"UTF8String\0");
                    if text.is_null() {
                        "WK error".into()
                    } else {
                        CStr::from_ptr(text.cast()).to_string_lossy().into_owned()
                    }
                };
                return Err(format!("hidden WKWebView snapshot unavailable: {detail}; never shown. Safe alternative: state/eval DOM inspection."));
            }
            let tiff = get(image, b"TIFFRepresentation\0");
            if tiff.is_null() {
                return Err("hidden snapshot has no TIFF pixels".into());
            }
            let one: unsafe extern "C" fn(Id, Id, Id) -> Id =
                mem::transmute(objc_msgSend as *const ());
            let rep = one(
                class(b"NSBitmapImageRep\0"),
                sel(b"imageRepWithData:\0"),
                tiff,
            );
            let png: unsafe extern "C" fn(Id, Id, usize, Id) -> Id =
                mem::transmute(objc_msgSend as *const ());
            let data = png(
                rep,
                sel(b"representationUsingType:properties:\0"),
                4,
                get(class(b"NSDictionary\0"), b"dictionary\0"),
            );
            if data.is_null() {
                return Err("WK snapshot PNG conversion failed".into());
            }
            let length: unsafe extern "C" fn(Id, Id) -> usize =
                mem::transmute(objc_msgSend as *const ());
            let len = length(data, sel(b"length\0"));
            if len == 0 || len > 32 * 1024 * 1024 {
                return Err("WK snapshot PNG outside byte bound".into());
            }
            let bytes = get(data, b"bytes\0");
            if bytes.is_null() {
                return Err("WK snapshot has no PNG data".into());
            }
            let bytes = std::slice::from_raw_parts(bytes.cast::<u8>(), len).to_vec();
            if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                return Err("WK snapshot did not produce PNG".into());
            }
            Ok(bytes)
        })();
        let _ = (*(*block).sender).try_send(result);
    }
    pub unsafe fn take(webview: Id, sender: Sender) {
        let responds: unsafe extern "C" fn(Id, Id, Id) -> bool =
            mem::transmute(objc_msgSend as *const ());
        let method = sel(b"takeSnapshotWithConfiguration:completionHandler:\0");
        if !responds(webview, sel(b"respondsToSelector:\0"), method) {
            let _ = sender.try_send(Err(
                "WKWebView takeSnapshot unsupported; use DOM eval; never show window".into(),
            ));
            return;
        }
        let sender = Arc::into_raw(Arc::new(sender));
        let block = Block {
            isa: &_NSConcreteStackBlock,
            flags: 1 << 25,
            reserved: 0,
            invoke: complete,
            descriptor: &DESCRIPTOR,
            sender,
        };
        let copied = _Block_copy(&block as *const _ as *const c_void);
        Arc::decrement_strong_count(sender);
        let call: unsafe extern "C" fn(Id, Id, Id, *mut c_void) =
            mem::transmute(objc_msgSend as *const ());
        let config = get(class(b"WKSnapshotConfiguration\0"), b"new\0");
        // Do not wait for a screen update: the view is deliberately never onscreen.
        let set: unsafe extern "C" fn(Id, Id, bool) = mem::transmute(objc_msgSend as *const ());
        set(config, sel(b"setAfterScreenUpdates:\0"), false);
        call(webview, method, config, copied);
        get(config, b"release\0");
        _Block_release(copied);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_settling_never_waits_for_a_suspended_timer() {
        let script = settle_script(LABEL);
        assert!(script.contains("getComputedTiming"));
        assert!(!script.contains("await"));
        assert!(!script.contains("setTimeout"));
        assert_eq!(settle_script("main"), "");
    }
    #[test]
    fn command_boundary() {
        for command in [
            "terminate_app",
            "write_to_session",
            "close_session",
            "mirror",
            "plugin:window|close",
            "mirror_mode",
        ] {
            assert!(!invoke_allowed(LABEL, command));
            assert!(invoke_allowed("main", command));
        }
        for command in ["vs_create_voice", "vs_speak", "mirror_eval_result"] {
            assert!(invoke_allowed(LABEL, command));
        }
    }
    #[test]
    fn bounded_controls() {
        for value in [
            json!({"cmd":"open","seconds":0}),
            json!({"cmd":"open","seconds":3601}),
            json!({"cmd":"resize","width":100,"height":1080}),
            json!({"cmd":"resize","width":1920,"height":2161}),
            json!({"cmd":"close","label":"main"}),
            json!({"cmd":"quit"}),
            json!({"cmd":"eval","script":""}),
        ] {
            assert!(parse_control(value).is_err());
        }
        assert!(parse_control(json!({"cmd":"open","seconds":1})).is_ok());
        assert!(parse_control(json!({"cmd":"resize","width":1920,"height":1080})).is_ok());
    }
    #[test]
    fn stale_and_cross_window_nonce_denied() {
        assert!(!nonce_matches("new", LABEL, "old", LABEL));
        assert!(!nonce_matches("new", LABEL, "new", "main"));
        assert!(!nonce_matches("seed", "main", "seed", LABEL));
        assert!(nonce_matches("seed", "main", "seed", "main"));
    }
    #[test]
    fn capture_scope() {
        assert!(screenshot_path("/tmp/screenshots/x.png").is_ok());
        assert!(screenshot_path("/tmp/screenshots/../x.png").is_err());
    }
}

// No process-global data-dir override: primary and CLI always retain their store.
static SANDBOX: Mutex<Option<PathBuf>> = Mutex::new(None);
pub fn sandbox() -> Result<PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt;
    let mut slot = SANDBOX.lock().map_err(|_| "sandbox lock")?;
    if let Some(path) = slot.as_ref() {
        return Ok(path.clone());
    }
    let path = std::env::temp_dir().join(format!("ultravox-mirror-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .map_err(|e| e.to_string())?;
    *slot = Some(path.clone());
    Ok(path)
}
fn cleanup_sandbox() {
    if let Ok(mut slot) = SANDBOX.lock() {
        if let Some(path) = slot.take() {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
pub fn start(app: tauri::AppHandle) -> Result<(), String> {
    use std::io::{BufRead, BufReader, Read};
    use std::os::unix::{
        fs::{DirBuilderExt, PermissionsExt},
        net::UnixListener,
    };
    let root = crate::state::data_dir(&app)?.join("mirror-control");
    if !root.exists() {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .map_err(|e| e.to_string())?;
    }
    let m = std::fs::symlink_metadata(&root).map_err(|e| e.to_string())?;
    if !m.is_dir()
        || m.file_type().is_symlink()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
    {
        return Err("unsafe mirror directory".into());
    }
    let path = root.join("mirror.sock");
    // Never unlink someone else's live or stale endpoint automatically.
    let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
    *SOCKET.lock().map_err(|_| "socket lock")? = Some(path.clone());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            let result = (|| -> Result<Value, String> {
                BufReader::new((&stream).take((MAX_FRAME + 1) as u64))
                    .read_line(&mut line)
                    .map_err(|e| e.to_string())?;
                if line.len() > MAX_FRAME || !line.ends_with('\n') {
                    return Err("invalid frame".into());
                }
                control(
                    &app,
                    serde_json::from_str(&line).map_err(|e| e.to_string())?,
                )
            })();
            let response = match result {
                Ok(value) => json!({"ok":true,"value":value}),
                Err(error) => json!({"ok":false,"error":error}),
            };
            let _ = writeln!(stream, "{}", response);
        }
    });
    Ok(())
}
pub fn cleanup() {
    cleanup_sandbox();
    if let Ok(mut slot) = SOCKET.lock() {
        if let Some(path) = slot.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}
