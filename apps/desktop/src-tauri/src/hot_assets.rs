//! Private development-only immutable frontend revisions. No disk IO in asset requests.
#![cfg(feature = "mirror-debug")]
use std::{
    borrow::Cow,
    collections::BTreeMap,
    fs,
    os::unix::fs::MetadataExt,
    path::Path,
    sync::{Mutex, OnceLock},
};
use tauri::{
    utils::assets::{AssetKey, AssetsIter, CspHash},
    Assets, Wry,
};
static REVISIONS: OnceLock<Mutex<BTreeMap<String, BTreeMap<String, Vec<u8>>>>> = OnceLock::new();
struct Empty;
impl Assets<Wry> for Empty {
    fn get(&self, _: &AssetKey) -> Option<Cow<'_, [u8]>> {
        None
    }
    fn iter(&self) -> Box<AssetsIter<'_>> {
        Box::new(std::iter::empty())
    }
    fn csp_hashes(&self, _: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        Box::new(std::iter::empty())
    }
}
struct Hot(Box<dyn Assets<Wry>>);
impl Assets<Wry> for Hot {
    fn setup(&self, app: &tauri::App<Wry>) {
        self.0.setup(app)
    }
    fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        if let Some(path) = key.as_ref().strip_prefix("/__hot/") {
            let (revision, file) = path.split_once('/')?;
            return REVISIONS
                .get()?
                .lock()
                .ok()?
                .get(revision)?
                .get(file)
                .cloned()
                .map(Cow::Owned);
        }
        self.0.get(key)
    }
    fn iter(&self) -> Box<AssetsIter<'_>> {
        self.0.iter()
    }
    fn csp_hashes(&self, key: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        if key.as_ref().starts_with("/__hot/") {
            Box::new(std::iter::empty())
        } else {
            self.0.csp_hashes(key)
        }
    }
}
pub fn install_assets(context: &mut tauri::Context<Wry>) {
    let embedded = context.set_assets(Box::new(Empty));
    context.set_assets(Box::new(Hot(embedded)));
}
fn safe_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 240
        && s.split('/').all(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && p.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        })
}
fn checked(path: &Path, directory: bool) -> Result<(), String> {
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
        || m.file_type().is_symlink()
        || (if directory {
            !m.is_dir()
        } else {
            !m.is_file() || m.nlink() != 1
        })
    {
        return Err("unsafe staged asset ownership/type/mode".into());
    }
    Ok(())
}
pub fn load(revision: &str) -> Result<(), String> {
    if revision.len() != 64
        || !revision
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err("invalid revision".into());
    }
    let revisions = REVISIONS.get_or_init(Default::default);
    let mut revisions = revisions.lock().map_err(|_| "asset lock")?;
    if revisions.contains_key(revision) {
        return Ok(());
    }
    if revisions.len() >= 8 {
        return Err("eight revision runtime limit; retain current/previous, rebuild later".into());
    }
    let home = std::env::var_os("HOME").ok_or("HOME unavailable")?;
    let root = Path::new(&home).join(".ultravox");
    checked(&root, true)?;
    let root = root.join("hot-assets");
    checked(&root, true)?;
    let root = root.join(revision);
    checked(&root, true)?;
    let manifest = root.join("manifest.json");
    checked(&manifest, false)?;
    if fs::metadata(&manifest).map_err(|e| e.to_string())?.len() > 1024 * 1024 {
        return Err("manifest too large".into());
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(manifest).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if manifest["revision"] != revision {
        return Err("revision mismatch".into());
    }
    let files = manifest["files"].as_object().ok_or("invalid manifest")?;
    if files.len() > 4096 || !files.contains_key("index.html") {
        return Err("invalid build".into());
    }
    let mut assets = BTreeMap::new();
    let mut total = 0u64;
    for (name, spec) in files {
        if !safe_name(name) || name == "manifest.json" {
            return Err("invalid asset path".into());
        }
        let mut path = root.clone();
        let parts: Vec<_> = name.split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            path.push(part);
            checked(&path, i + 1 != parts.len())?;
        }
        let size = fs::metadata(&path).map_err(|e| e.to_string())?.len();
        total += size;
        if size > 16 * 1024 * 1024
            || total > 64 * 1024 * 1024
            || spec["size"].as_u64() != Some(size)
        {
            return Err("asset size bound/mismatch".into());
        }
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ *b as u64).wrapping_mul(0x100000001b3)
        });
        if spec["fnv64"].as_str() != Some(format!("{hash:016x}").as_str()) {
            return Err("asset digest mismatch".into());
        }
        assets.insert(name.clone(), bytes);
    }
    revisions.insert(revision.into(), assets);
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn paths() {
        for p in ["../x", "/x", "a/../x", "a//x", "a\\x", "%2e/x"] {
            assert!(!super::safe_name(p));
        }
        assert!(super::safe_name("assets/app-x.js"));
    }
}
