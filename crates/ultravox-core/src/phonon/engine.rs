//! Phonon-2 inference: log-mel -> packed five-value FastConformer encoder (C) -> projector -> greedy TDT decode (C).

use super::container::{Container, Record};
use super::mel::{Mel, N_MELS};
use super::native::Api;
use super::PhononError;
use std::ffi::c_void;
use std::path::Path;

pub(crate) const SAMPLE_RATE: usize = 16_000;
const LAYERS: usize = 24;
const D_MODEL: i32 = 1024;
const FF_DIM: i32 = 4096;
const HEADS: i32 = 8;
const CONV_KERNEL: i32 = 9;
const SUB_CHANNELS: i32 = 256;
const PROJ_OUT: usize = 640;
const DURATIONS: [i32; 5] = [0, 1, 2, 3, 4];
const BLANK: i32 = 8192;
const MAX_SYMBOLS: i32 = 10;

const LAYER_MATRICES: [&str; 11] = [
    "feed_forward1.linear1",
    "feed_forward1.linear2",
    "self_attn.q_proj",
    "self_attn.k_proj",
    "self_attn.v_proj",
    "self_attn.o_proj",
    "self_attn.relative_k_proj",
    "conv.pointwise_conv1",
    "conv.pointwise_conv2",
    "feed_forward2.linear1",
    "feed_forward2.linear2",
];

const LAYER_VECTORS: [&str; 17] = [
    "norm_feed_forward1.weight",
    "norm_feed_forward1.bias",
    "norm_self_att.weight",
    "norm_self_att.bias",
    "norm_conv.weight",
    "norm_conv.bias",
    "norm_feed_forward2.weight",
    "norm_feed_forward2.bias",
    "norm_out.weight",
    "norm_out.bias",
    "self_attn.bias_u",
    "self_attn.bias_v",
    "conv.depthwise_conv.weight",
    "conv.norm.weight",
    "conv.norm.bias",
    "conv.norm.running_mean",
    "conv.norm.running_var",
];

pub(crate) struct Engine {
    api: Api,
    mel: Mel,
    encoder: *mut c_void,
    decoder: *mut c_void,
    gemms: Vec<*mut c_void>,
    proj_w: Vec<f32>, // [PROJ_OUT][D_MODEL]
    proj_b: Vec<f32>,
    vocab: Vec<String>,
    /// Buffers the native handles may reference for the engine's lifetime.
    _keep: Vec<Vec<u8>>,
}

// The native handles are only used behind the engine mutex in `mod.rs`.
unsafe impl Send for Engine {}

fn threads() -> i32 {
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("sysctl")
        .args(["-n", "hw.perflevel0.logicalcpu"])
        .output()
    {
        if let Ok(n) = String::from_utf8_lossy(&out.stdout).trim().parse::<i32>() {
            if n > 0 {
                return n;
            }
        }
    }
    let logical = std::thread::available_parallelism().map_or(4, |n| n.get()) as i32;
    // SMT siblings share one int8 unit; from 12 logical CPUs up, use roughly one thread per core.
    (if logical >= 12 { logical / 2 } else { logical }).clamp(1, 16)
}

fn vocabulary(config: &Path) -> Result<Vec<String>, PhononError> {
    let text = std::fs::read_to_string(config)
        .map_err(|e| PhononError::Model(format!("{}: {e}", config.display())))?;
    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| PhononError::Model(format!("{}: {e}", config.display())))?;
    let list = json["vocabulary"]
        .as_array()
        .or_else(|| json["joint"]["vocabulary"].as_array())
        .ok_or_else(|| PhononError::Model("config.json has no vocabulary".into()))?;
    Ok(list.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect())
}

fn ptr<T>(v: &[T]) -> *mut c_void {
    v.as_ptr() as *mut c_void
}

impl Engine {
    pub fn load(model_dir: &Path, runtime_dir: &Path) -> Result<Self, PhononError> {
        let api = Api::load(runtime_dir)?;
        let container = Container::read(&model_dir.join("model.fermion"))?;
        let vocab = vocabulary(&model_dir.join("config.json"))?;
        let n_threads = threads();
        unsafe {
            (api.cpu_set_threads)(n_threads);
            (api.tdt_set_threads)(n_threads);
        }

        let proj_w = container.f32("encoder_projector.weight")?;
        let proj_b = container.f32("encoder_projector.bias")?;
        let mut keep: Vec<Vec<u8>> = Vec::new();
        let mut gemms: Vec<*mut c_void> = Vec::new();
        let mut gemm_for = |name: &str, api: &Api| -> Result<*mut c_void, PhononError> {
            let Some(Record::FiveValue { rows, cols, plane_a, plane_b, lo, hi }) =
                container.records.get(name)
            else {
                return Err(PhononError::Model(format!("model is missing matrix {name}")));
            };
            let handle = unsafe {
                (api.cpu_create)(
                    *rows as i32,
                    *cols as i32,
                    ptr(plane_a),
                    ptr(plane_b),
                    ptr(lo),
                    ptr(hi),
                    0,
                )
            };
            if handle.is_null() {
                return Err(PhononError::Native(format!("phonon2_cpu_create failed for {name}")));
            }
            unsafe { (api.cpu_enable_onedot)(handle) };
            gemms.push(handle);
            Ok(handle)
        };

        let encoder = unsafe {
            (api.enc_create)(
                LAYERS as i32,
                D_MODEL,
                FF_DIM,
                HEADS,
                CONV_KERNEL,
                N_MELS as i32,
                SUB_CHANNELS,
            )
        };
        if encoder.is_null() {
            return Err(PhononError::Native("phonon2_enc_create failed".into()));
        }

        // Subsampling stack: conv layers 0,2,3,5,6, the linear, and the rotary inverse frequencies.
        let sub = |n: &str| container.f32(&format!("encoder.subsampling.{n}"));
        let sub_tensors = [
            sub("layers.0.weight")?,
            sub("layers.0.bias")?,
            sub("layers.2.weight")?,
            sub("layers.2.bias")?,
            sub("layers.3.weight")?,
            sub("layers.3.bias")?,
            sub("layers.5.weight")?,
            sub("layers.5.bias")?,
            sub("layers.6.weight")?,
            sub("layers.6.bias")?,
            sub("linear.weight")?,
            sub("linear.bias")?,
        ];
        let inv_freq: Vec<f32> = (0..D_MODEL as usize / 2)
            .map(|i| (1.0 / 10000f64.powf((2 * i) as f64 / D_MODEL as f64)) as f32)
            .collect();
        let rc = unsafe {
            (api.enc_set_subsampling)(
                encoder,
                ptr(&sub_tensors[0]),
                ptr(&sub_tensors[1]),
                ptr(&sub_tensors[2]),
                ptr(&sub_tensors[3]),
                ptr(&sub_tensors[4]),
                ptr(&sub_tensors[5]),
                ptr(&sub_tensors[6]),
                ptr(&sub_tensors[7]),
                ptr(&sub_tensors[8]),
                ptr(&sub_tensors[9]),
                ptr(&sub_tensors[10]),
                ptr(&sub_tensors[11]),
                ptr(&inv_freq),
            )
        };
        if rc != 0 {
            return Err(PhononError::Native(format!("phonon2_enc_set_subsampling rc={rc}")));
        }

        for layer in 0..LAYERS {
            let mut handles = [std::ptr::null_mut::<c_void>(); 11];
            for (slot, suffix) in handles.iter_mut().zip(LAYER_MATRICES) {
                *slot = gemm_for(&format!("encoder.layers.{layer}.{suffix}"), &api)?;
            }
            let vectors = LAYER_VECTORS
                .iter()
                .map(|v| container.f32(&format!("encoder.layers.{layer}.{v}")))
                .collect::<Result<Vec<_>, _>>()?;
            let vector_ptrs: Vec<*mut c_void> = vectors.iter().map(|v| ptr(v)).collect();
            let rc = unsafe {
                (api.enc_set_layer)(encoder, layer as i32, handles.as_ptr(), vector_ptrs.as_ptr().cast())
            };
            if rc != 0 {
                return Err(PhononError::Native(format!("phonon2_enc_set_layer({layer}) rc={rc}")));
            }
        }

        // Transducer decoder: int tables stay integer; biases stay fp16.
        let int_table = |name: &str| -> Result<(Vec<u8>, Vec<u8>, usize, usize), PhononError> {
            match container.records.get(name) {
                Some(Record::IntN { rows, cols, q, scales }) => {
                    Ok((q.iter().map(|&v| v as u8).collect(), scales.clone(), *rows, *cols))
                }
                _ => Err(PhononError::Model(format!("model is missing integer table {name}"))),
            }
        };
        let half = |name: &str| -> Result<Vec<u8>, PhononError> {
            match container.records.get(name) {
                Some(Record::Fp16 { data }) => Ok(data.clone()),
                _ => Err(PhononError::Model(format!("model is missing tensor {name}"))),
            }
        };
        let (emb_q, emb_s, emb_rows, emb_cols) = int_table("decoder.embedding.weight")?;
        let (ih0, ih0s, ..) = int_table("decoder.lstm.weight_ih_l0")?;
        let (hh0, hh0s, _, hidden) = int_table("decoder.lstm.weight_hh_l0")?;
        let (ih1, ih1s, ..) = int_table("decoder.lstm.weight_ih_l1")?;
        let (hh1, hh1s, ..) = int_table("decoder.lstm.weight_hh_l1")?;
        let (pq, ps, ..) = int_table("decoder.decoder_projector.weight")?;
        let (hq, hs, head_rows, _) = int_table("joint.head.weight")?;
        let bih0 = half("decoder.lstm.bias_ih_l0")?;
        let bhh0 = half("decoder.lstm.bias_hh_l0")?;
        let bih1 = half("decoder.lstm.bias_ih_l1")?;
        let bhh1 = half("decoder.lstm.bias_hh_l1")?;
        let pb = half("decoder.decoder_projector.bias")?;
        let hb = half("joint.head.bias")?;
        let decoder = unsafe {
            (api.tdt_create)(
                emb_rows as i32,
                emb_cols as i32,
                hidden as i32,
                head_rows as i32,
                DURATIONS.len() as i32,
                DURATIONS.as_ptr(),
                BLANK,
                MAX_SYMBOLS,
                ptr(&emb_q),
                ptr(&emb_s),
                ptr(&ih0),
                ptr(&ih0s),
                ptr(&bih0),
                ptr(&hh0),
                ptr(&hh0s),
                ptr(&bhh0),
                ptr(&ih1),
                ptr(&ih1s),
                ptr(&bih1),
                ptr(&hh1),
                ptr(&hh1s),
                ptr(&bhh1),
                ptr(&pq),
                ptr(&ps),
                ptr(&pb),
                ptr(&hq),
                ptr(&hs),
                ptr(&hb),
            )
        };
        if decoder.is_null() {
            return Err(PhononError::Native("phonon2_tdt_create failed".into()));
        }
        keep.extend([
            emb_q, emb_s, ih0, ih0s, bih0, hh0, hh0s, bhh0, ih1, ih1s, bih1, hh1, hh1s, bhh1, pq, ps,
            pb, hq, hs, hb,
        ]);
        // The packed planes were handed to the library by pointer; keep them alive with the engine.
        // (Moving the container's vectors keeps their heap addresses stable.)
        for (_, record) in container.records {
            if let Record::FiveValue { plane_a, plane_b, lo, hi, .. } = record {
                keep.extend([plane_a, plane_b, lo, hi]);
            }
        }

        Ok(Self {
            api,
            mel: Mel::new(),
            encoder,
            decoder,
            gemms,
            proj_w,
            proj_b,
            vocab,
            _keep: keep,
        })
    }

    /// Decode one utterance (<= ~35 s) of 16 kHz mono samples to text.
    pub fn decode(&self, wave: &[f32]) -> Result<String, PhononError> {
        let feats = self.mel.log_mel(wave);
        let frames = (feats.len() / N_MELS) as i32;
        let out_len = unsafe { (self.api.enc_out_len)(self.encoder, frames) };
        if out_len <= 0 {
            return Ok(String::new());
        }
        let d = D_MODEL as usize;
        let mut enc = vec![0f32; out_len as usize * d];
        let rc = unsafe { (self.api.enc_forward)(self.encoder, feats.as_ptr(), frames, enc.as_mut_ptr()) };
        if rc != 0 {
            return Err(PhononError::Native(format!("phonon2_enc_forward rc={rc}")));
        }
        let t = out_len as usize;
        let mut projected = vec![0f32; t * PROJ_OUT];
        for (row, out) in enc.chunks_exact(d).zip(projected.chunks_exact_mut(PROJ_OUT)) {
            for (o, (w, b)) in out
                .iter_mut()
                .zip(self.proj_w.chunks_exact(d).zip(&self.proj_b))
            {
                *o = b + w.iter().zip(row).map(|(a, b)| a * b).sum::<f32>();
            }
        }
        let mut ids = vec![0i32; (12 * t).max(64)];
        let n = unsafe {
            (self.api.tdt_decode)(self.decoder, projected.as_ptr(), t as i32, ids.as_mut_ptr(), ids.len() as i32)
        };
        if n < 0 {
            return Err(PhononError::Native(format!("phonon2_tdt_decode rc={n}")));
        }
        let mut text = String::new();
        for &id in &ids[..n as usize] {
            let Some(piece) = self.vocab.get(id as usize) else { continue };
            let special = (piece.starts_with("<|") && piece.ends_with("|>"))
                || piece == "<unk>"
                || piece == "<pad>";
            if !special {
                text.push_str(&piece.replace('\u{2581}', " "));
            }
        }
        Ok(text.trim().to_string())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            (self.api.tdt_destroy)(self.decoder);
            (self.api.enc_destroy)(self.encoder);
            for &g in &self.gemms {
                (self.api.cpu_destroy)(g);
            }
        }
    }
}
