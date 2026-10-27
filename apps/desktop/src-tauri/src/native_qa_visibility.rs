//! Private local-QA visibility metadata only. No pixels, focus or window writes.
use std::sync::mpsc;
use std::time::Duration;

pub fn is_onscreen(window: &tauri::WebviewWindow) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = mpsc::sync_channel(1);
        window
            .with_webview(move |webview| {
                let visible = unsafe { native::is_onscreen(webview.inner() as *mut _) };
                let _ = tx.try_send(visible);
            })
            .map_err(|_| "native visibility query could not be scheduled")?;
        rx.recv_timeout(Duration::from_secs(1))
            .map_err(|_| "native visibility query timed out".into())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Err("visible native renderer QA requires macOS WKWebView".into())
    }
}

#[cfg(target_os = "macos")]
mod native {
    use std::ffi::{c_char, c_void};
    type Id = *mut c_void;
    #[link(name = "objc")]
    extern "C" {
        fn sel_registerName(name: *const c_char) -> Id;
        fn objc_msgSend();
    }
    unsafe fn selector(name: &'static [u8]) -> Id {
        sel_registerName(name.as_ptr().cast())
    }
    pub unsafe fn is_onscreen(webview: Id) -> bool {
        let get: unsafe extern "C" fn(Id, Id) -> Id =
            std::mem::transmute(objc_msgSend as *const ());
        let boolean: unsafe extern "C" fn(Id, Id) -> bool =
            std::mem::transmute(objc_msgSend as *const ());
        let integer: unsafe extern "C" fn(Id, Id) -> usize =
            std::mem::transmute(objc_msgSend as *const ());
        let window = get(webview, selector(b"window\0"));
        !window.is_null()
            && boolean(window, selector(b"isVisible\0"))
            && !boolean(window, selector(b"isMiniaturized\0"))
            && integer(window, selector(b"occlusionState\0")) & 2 != 0
    }
}
