//! The Phonon-2 native CPU libraries (encoder + TDT decoder), loaded over a C ABI.
//!
//! Pinned binaries live in `third_party/phonon2` (Apache-2.0, Fermion Research) and are
//! embedded per platform. On macOS a hardened, Developer-ID-signed app refuses libraries
//! signed by another team, so the release script copies them into `Contents/Frameworks`
//! and signs them with the app; that copy is preferred. Everywhere else (and for
//! development builds) the pinned bytes are verified against `SHA256SUMS.txt` and
//! extracted under the models directory.

use super::PhononError;
use sha2::{Digest, Sha256};
use std::ffi::c_void;
use std::path::{Path, PathBuf};

const PINS: &str = include_str!("../../../../third_party/phonon2/SHA256SUMS.txt");

macro_rules! embed {
    ($name:literal) => {
        (
            $name,
            include_bytes!(concat!("../../../../third_party/phonon2/", $name)) as &[u8],
        )
    };
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const LIBS: Option<[(&str, &[u8]); 2]> = Some([
    embed!("libphonon2_enc-macos-arm64.dylib"),
    embed!("libphonon2_tdt-macos-arm64.dylib"),
]);
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const LIBS: Option<[(&str, &[u8]); 2]> = Some([
    embed!("libphonon2_enc-linux-x86_64.so"),
    embed!("libphonon2_tdt-linux-x86_64.so"),
]);
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const LIBS: Option<[(&str, &[u8]); 2]> = Some([
    embed!("phonon2_enc-windows-x64.dll"),
    embed!("phonon2_tdt-windows-x64.dll"),
]);
#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
const LIBS: Option<[(&str, &[u8]); 2]> = None;

fn pinned(name: &str) -> Option<&'static str> {
    PINS.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        (parts.next()? == name).then_some(hash)
    })
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Locate (or extract) the two libraries and return `(encoder, decoder)` paths.
pub(crate) fn library_paths(runtime_dir: &Path) -> Result<(PathBuf, PathBuf), PhononError> {
    let libs = LIBS.ok_or_else(|| {
        PhononError::Unsupported(format!(
            "Phonon-2 has no native library for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })?;
    #[cfg(target_os = "macos")]
    // Resolve symlinks first: `ultravox-control` on a PATH is a link to the app's copy, and the
    // bundled libraries sit next to the real binary, not next to the link.
    if let Some(dir) = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("../Frameworks")))
    {
        let (enc, tdt) = (dir.join(libs[0].0), dir.join(libs[1].0));
        if enc.is_file() && tdt.is_file() {
            return Ok((enc, tdt));
        }
    }
    let mut out = Vec::new();
    for (name, bytes) in libs {
        let expected = pinned(name)
            .ok_or_else(|| PhononError::Native(format!("no pinned sha256 for {name}")))?;
        let actual = sha256_hex(bytes);
        if actual != expected {
            return Err(PhononError::Native(format!(
                "embedded {name} sha256 {actual} does not match its pin {expected}"
            )));
        }
        let dir = runtime_dir.join(&expected[..12]);
        std::fs::create_dir_all(&dir)
            .map_err(|e| PhononError::Native(format!("create {}: {e}", dir.display())))?;
        let path = dir.join(name);
        let current = std::fs::read(&path)
            .map(|existing| sha256_hex(&existing) == expected)
            .unwrap_or(false);
        if !current {
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, bytes)
                .and_then(|_| std::fs::rename(&tmp, &path))
                .map_err(|e| PhononError::Native(format!("write {}: {e}", path.display())))?;
        }
        out.push(path);
    }
    Ok((out.remove(0), out.remove(0)))
}

type P = *mut c_void;

/// Resolved C entry points. The `Library` handles are kept alive beside them.
pub(crate) struct Api {
    _enc: libloading::Library,
    _tdt: libloading::Library,
    pub cpu_abi_version: unsafe extern "C" fn() -> i32,
    pub enc_abi_version: unsafe extern "C" fn() -> i32,
    pub cpu_set_threads: unsafe extern "C" fn(i32),
    pub cpu_create: unsafe extern "C" fn(i32, i32, P, P, P, P, i32) -> P,
    pub cpu_destroy: unsafe extern "C" fn(P),
    pub cpu_enable_onedot: unsafe extern "C" fn(P) -> f32,
    pub enc_create: unsafe extern "C" fn(i32, i32, i32, i32, i32, i32, i32) -> P,
    #[allow(clippy::type_complexity)]
    pub enc_set_subsampling:
        unsafe extern "C" fn(P, P, P, P, P, P, P, P, P, P, P, P, P, P) -> i32,
    pub enc_set_layer: unsafe extern "C" fn(P, i32, *const P, *const P) -> i32,
    pub enc_out_len: unsafe extern "C" fn(P, i32) -> i32,
    pub enc_forward: unsafe extern "C" fn(P, *const f32, i32, *mut f32) -> i32,
    pub enc_destroy: unsafe extern "C" fn(P),
    pub tdt_abi_version: unsafe extern "C" fn() -> i32,
    #[allow(clippy::type_complexity)]
    pub tdt_create: unsafe extern "C" fn(
        i32,
        i32,
        i32,
        i32,
        i32,
        *const i32,
        i32,
        i32,
        P,
        P,
        P,
        P,
        P,
        P,
        P,
        P,
        P,
        P,
        P,
        P,
        P,
        P,
        P,
        P,
        P,
        P,
        P,
        P,
    ) -> P,
    pub tdt_decode: unsafe extern "C" fn(P, *const f32, i32, *mut i32, i32) -> i32,
    pub tdt_set_threads: unsafe extern "C" fn(i32),
    pub tdt_destroy: unsafe extern "C" fn(P),
}

impl Api {
    pub fn load(runtime_dir: &Path) -> Result<Self, PhononError> {
        let (enc_path, tdt_path) = library_paths(runtime_dir)?;
        let open = |p: &Path| unsafe {
            libloading::Library::new(p)
                .map_err(|e| PhononError::Native(format!("load {}: {e}", p.display())))
        };
        let enc = open(&enc_path)?;
        let tdt = open(&tdt_path)?;
        macro_rules! sym {
            ($lib:expr, $name:literal) => {
                unsafe {
                    *$lib
                        .get(concat!($name, "\0").as_bytes())
                        .map_err(|e| PhononError::Native(format!("symbol {}: {e}", $name)))?
                }
            };
        }
        let api = Api {
            cpu_abi_version: sym!(enc, "phonon2_cpu_abi_version"),
            enc_abi_version: sym!(enc, "phonon2_enc_abi_version"),
            cpu_set_threads: sym!(enc, "phonon2_cpu_set_threads"),
            cpu_create: sym!(enc, "phonon2_cpu_create"),
            cpu_destroy: sym!(enc, "phonon2_cpu_destroy"),
            cpu_enable_onedot: sym!(enc, "phonon2_cpu_enable_onedot"),
            enc_create: sym!(enc, "phonon2_enc_create"),
            enc_set_subsampling: sym!(enc, "phonon2_enc_set_subsampling"),
            enc_set_layer: sym!(enc, "phonon2_enc_set_layer"),
            enc_out_len: sym!(enc, "phonon2_enc_out_len"),
            enc_forward: sym!(enc, "phonon2_enc_forward"),
            enc_destroy: sym!(enc, "phonon2_enc_destroy"),
            tdt_abi_version: sym!(tdt, "phonon2_tdt_abi_version"),
            tdt_create: sym!(tdt, "phonon2_tdt_create"),
            tdt_decode: sym!(tdt, "phonon2_tdt_decode"),
            tdt_set_threads: sym!(tdt, "phonon2_tdt_set_threads"),
            tdt_destroy: sym!(tdt, "phonon2_tdt_destroy"),
            _enc: enc,
            _tdt: tdt,
        };
        let (cpu, enc_v, tdt_v) = unsafe {
            (
                (api.cpu_abi_version)(),
                (api.enc_abi_version)(),
                (api.tdt_abi_version)(),
            )
        };
        if cpu != 1 || enc_v != 1 || tdt_v != 1 {
            return Err(PhononError::Native(format!(
                "unexpected ABI versions (gemm {cpu}, encoder {enc_v}, decoder {tdt_v}); expected 1/1/1"
            )));
        }
        Ok(api)
    }
}
