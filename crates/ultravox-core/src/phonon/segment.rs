//! Long-audio windowing (25-35 s windows cut in the longest pause) and resampling.

const SR: usize = 16_000;
const SINGLE_SHOT_MAX: usize = 35 * SR;
const BAND_MIN: usize = 25 * SR;
const BAND_MAX: usize = 35 * SR;
const TARGET: usize = 30 * SR;
const BLOCK: usize = SR / 20; // 50 ms
const NOISE_FLOOR_RMS: f32 = 0.004;
const GATE_RATIO: f32 = 0.18;

/// Contiguous `(start, end)` sample windows covering `audio`.
pub(crate) fn plan(audio: &[f32]) -> Vec<(usize, usize)> {
    let n = audio.len();
    if n <= SINGLE_SHOT_MAX {
        return vec![(0, n)];
    }
    let n_blocks = n.div_ceil(BLOCK);
    let rms: Vec<f32> = (0..n_blocks)
        .map(|b| {
            let chunk = &audio[b * BLOCK..((b + 1) * BLOCK).min(n)];
            (chunk.iter().map(|v| v * v).sum::<f32>() / BLOCK as f32).sqrt()
        })
        .collect();
    let mut windows = Vec::new();
    let mut start = 0usize;
    while n - start > SINGLE_SHOT_MAX {
        let lo = start + BAND_MIN;
        let hi = start + BAND_MAX;
        let (b0, b1) = (lo.div_ceil(BLOCK), hi / BLOCK);
        let peak = rms[start / BLOCK..b1.min(n_blocks)]
            .iter()
            .fold(0f32, |m, v| m.max(*v));
        let gate = NOISE_FLOOR_RMS.max(GATE_RATIO * peak);
        let quiet: Vec<bool> = (b0..b1.min(n_blocks)).map(|b| rms[b] <= gate).collect();
        let mut cut = start + TARGET;
        let mut best: Option<(usize, isize, usize)> = None; // (length, -distance, mid sample)
        let mut i = 0;
        while i < quiet.len() {
            if !quiet[i] {
                i += 1;
                continue;
            }
            let mut j = i;
            while j < quiet.len() && quiet[j] {
                j += 1;
            }
            let mid = (b0 + i + b0 + j) * BLOCK / 2;
            let cand = (j - i, -((mid as isize - (start + TARGET) as isize).abs()), mid);
            if best.map_or(true, |b| cand > b) {
                best = Some(cand);
            }
            i = j;
        }
        if let Some((_, _, mid)) = best {
            cut = mid;
        }
        cut = cut.min(hi).min(n).max(lo);
        windows.push((start, cut));
        start = cut;
    }
    windows.push((start, n));
    windows
}

/// `audio` with `head_ms` / `tail_ms` of digital silence added at the start / end.
pub(crate) fn pad_edges(audio: &[f32], head_ms: usize, tail_ms: usize) -> Vec<f32> {
    let (head, tail) = (head_ms * SR / 1000, tail_ms * SR / 1000);
    let mut out = Vec::with_capacity(head + audio.len() + tail);
    out.resize(head, 0.0);
    out.extend_from_slice(audio);
    out.resize(head + audio.len() + tail, 0.0);
    out
}

/// Windowed-sinc resampler (Hann window, 16 zero crossings), anti-aliased when downsampling.
pub(crate) fn resample(input: &[f32], from: usize, to: usize) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let cutoff = ratio.min(1.0);
    let half = 16.0 / cutoff;
    let out_len = (input.len() as f64 * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let center = i as f64 / ratio;
        let first = (center - half).ceil().max(0.0) as usize;
        let last = ((center + half).floor() as usize).min(input.len() - 1);
        let mut acc = 0f64;
        for k in first..=last {
            let x = (k as f64 - center) * cutoff;
            let sinc = if x == 0.0 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) };
            let w = 0.5 + 0.5 * (std::f64::consts::PI * (k as f64 - center) / half).cos();
            acc += input[k] as f64 * sinc * w * cutoff;
        }
        out.push(acc as f32);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_audio_is_one_window() {
        assert_eq!(plan(&vec![0.1; 20 * SR]), vec![(0, 20 * SR)]);
    }

    #[test]
    fn long_audio_cuts_in_the_pause_and_covers_everything() {
        let mut audio = vec![0.3f32; 80 * SR];
        // A 600 ms pause centred at 28 s.
        for v in &mut audio[28 * SR - 4_800..28 * SR + 4_800] {
            *v = 0.0;
        }
        let windows = plan(&audio);
        assert_eq!(windows[0].0, 0);
        assert!((windows[0].1 as isize - 28 * SR as isize).abs() < 800, "{windows:?}");
        assert_eq!(windows.last().unwrap().1, audio.len());
        assert!(windows.windows(2).all(|w| w[0].1 == w[1].0));
        assert!(windows.iter().all(|(a, b)| b - a <= SINGLE_SHOT_MAX));
    }

    #[test]
    fn continuous_speech_is_cut_at_thirty_seconds() {
        let windows = plan(&vec![0.3f32; 70 * SR]);
        assert_eq!(windows[0], (0, 30 * SR));
    }

    #[test]
    fn pad_edges_adds_silence_on_both_sides_only() {
        let out = pad_edges(&[0.5; 160], 10, 20);
        assert_eq!(out.len(), 160 + 160 + 320);
        assert!(out[..160].iter().all(|v| *v == 0.0));
        assert!(out[160..320].iter().all(|v| *v == 0.5));
        assert!(out[320..].iter().all(|v| *v == 0.0));
        assert_eq!(pad_edges(&[0.5; 4], 0, 0), vec![0.5; 4]);
    }

    #[test]
    fn resample_changes_length_and_keeps_a_tone() {
        let tone: Vec<f32> = (0..48_000)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin())
            .collect();
        let out = resample(&tone, 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
        let peak = out[1_000..15_000].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!((0.95..1.05).contains(&peak), "peak {peak}");
    }
}
