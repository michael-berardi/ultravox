//! Log-mel front end for Parakeet: pre-emphasis 0.97, 512-point STFT (hop 160, 400-sample
//! symmetric Hann centred in the frame), Slaney mel 128, `log(x + 2^-24)`, per-feature
//! normalisation over the utterance.

use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;

pub(crate) const N_FFT: usize = 512;
pub(crate) const HOP: usize = 160;
pub(crate) const WIN: usize = 400;
pub(crate) const N_MELS: usize = 128;
const BINS: usize = N_FFT / 2 + 1;
const PREEMPH: f32 = 0.97;
const LOG_GUARD: f32 = 5.960_464_5e-8; // 2^-24

fn hz_to_mel(f: f64) -> f64 {
    let (f_sp, min_log_hz) = (200.0 / 3.0, 1000.0);
    let min_log_mel = min_log_hz / f_sp;
    let logstep = 6.4f64.ln() / 27.0;
    if f >= min_log_hz {
        min_log_mel + (f / min_log_hz).ln() / logstep
    } else {
        f / f_sp
    }
}

fn mel_to_hz(m: f64) -> f64 {
    let (f_sp, min_log_hz) = (200.0 / 3.0, 1000.0);
    let min_log_mel = min_log_hz / f_sp;
    let logstep = 6.4f64.ln() / 27.0;
    if m >= min_log_mel {
        min_log_hz * (logstep * (m - min_log_mel)).exp()
    } else {
        f_sp * m
    }
}

fn mel_filters(sr: f64) -> Vec<f32> {
    let fft_freqs: Vec<f64> = (0..BINS)
        .map(|i| i as f64 * (sr / 2.0) / (BINS - 1) as f64)
        .collect();
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(sr / 2.0));
    let mel_f: Vec<f64> = (0..N_MELS + 2)
        .map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (N_MELS + 1) as f64))
        .collect();
    let mut weights = vec![0f32; N_MELS * BINS];
    for i in 0..N_MELS {
        let (f0, f1, f2) = (mel_f[i], mel_f[i + 1], mel_f[i + 2]);
        let enorm = 2.0 / (f2 - f0);
        for (j, &f) in fft_freqs.iter().enumerate() {
            let lower = (f - f0) / (f1 - f0);
            let upper = (f2 - f) / (f2 - f1);
            weights[i * BINS + j] = (lower.min(upper).max(0.0) * enorm) as f32;
        }
    }
    weights
}

pub(crate) struct Mel {
    filters: Vec<f32>,
    window: Vec<f32>, // WIN taps, placed at offset (N_FFT - WIN) / 2 inside each frame
    fft: Arc<dyn RealToComplex<f32>>,
}

impl Mel {
    pub fn new() -> Self {
        let window = (0..WIN)
            .map(|n| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / (WIN - 1) as f64).cos()) as f32)
            .collect();
        Self {
            filters: mel_filters(16_000.0),
            window,
            fft: RealFftPlanner::<f32>::new().plan_fft_forward(N_FFT),
        }
    }

    /// `[frames][N_MELS]` row-major features for one utterance of 16 kHz mono samples.
    pub fn log_mel(&self, wave: &[f32]) -> Vec<f32> {
        let n = wave.len();
        let mut x = Vec::with_capacity(n);
        for (i, &v) in wave.iter().enumerate() {
            x.push(if i == 0 { v } else { v - PREEMPH * wave[i - 1] });
        }
        let frames = 1 + n / HOP;
        let pad = N_FFT / 2;
        let offset = (N_FFT - WIN) / 2;
        let mut input = self.fft.make_input_vec();
        let mut spectrum = self.fft.make_output_vec();
        let mut power = vec![0f32; BINS];
        let mut feats = vec![0f32; frames * N_MELS];
        for t in 0..frames {
            input.iter_mut().for_each(|v| *v = 0.0);
            for j in 0..WIN {
                // Padded index of window tap j, minus the zero padding of `pad` samples.
                let idx = (t * HOP + offset + j) as isize - pad as isize;
                if idx >= 0 && (idx as usize) < n {
                    input[offset + j] = x[idx as usize] * self.window[j];
                }
            }
            self.fft
                .process(&mut input, &mut spectrum)
                .expect("fft sizes are fixed");
            for (p, c) in power.iter_mut().zip(&spectrum) {
                *p = c.re * c.re + c.im * c.im;
            }
            for m in 0..N_MELS {
                let row = &self.filters[m * BINS..(m + 1) * BINS];
                let acc: f32 = row.iter().zip(&power).map(|(a, b)| a * b).sum();
                feats[t * N_MELS + m] = (acc + LOG_GUARD).ln();
            }
        }
        if frames > 1 {
            for m in 0..N_MELS {
                let mean = (0..frames).map(|t| feats[t * N_MELS + m] as f64).sum::<f64>() / frames as f64;
                let var = (0..frames)
                    .map(|t| (feats[t * N_MELS + m] as f64 - mean).powi(2))
                    .sum::<f64>()
                    / (frames - 1) as f64;
                let denom = var.sqrt() as f32 + 1e-5;
                for t in 0..frames {
                    feats[t * N_MELS + m] = (feats[t * N_MELS + m] - mean as f32) / denom;
                }
            }
        }
        feats
    }
}
