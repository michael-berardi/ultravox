//! Reader for Phonon-2's `fermion-five-value-parakeet-v1` container (`model.fermion`).

use super::PhononError;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

const FORMAT: &str = "fermion-five-value-parakeet-v1";

pub(crate) enum Record {
    /// Encoder linear: two 2-bit planes (row-major, 4 values per byte) and per-row lo/hi magnitudes (fp16).
    FiveValue {
        rows: usize,
        cols: usize,
        plane_a: Vec<u8>,
        plane_b: Vec<u8>,
        lo: Vec<u8>,
        hi: Vec<u8>,
    },
    /// Row-scaled integer table (int6 or int8), values widened to i8, scales fp16.
    IntN {
        rows: usize,
        cols: usize,
        q: Vec<i8>,
        scales: Vec<u8>,
    },
    Fp16 {
        data: Vec<u8>,
    },
}

pub(crate) struct Container {
    pub records: HashMap<String, Record>,
}

pub(crate) fn f16_to_f32(bits: u16) -> f32 {
    let sign = ((bits >> 15) & 1) as u32;
    let exp = ((bits >> 10) & 0x1f) as u32;
    let frac = (bits & 0x3ff) as u32;
    let value = if exp == 0 {
        if frac == 0 {
            sign << 31
        } else {
            // Subnormal half: normalise into an f32.
            let mut e = 127 - 15 + 1;
            let mut f = frac;
            while f & 0x400 == 0 {
                f <<= 1;
                e -= 1;
            }
            (sign << 31) | ((e as u32) << 23) | ((f & 0x3ff) << 13)
        }
    } else if exp == 31 {
        (sign << 31) | (0xff << 23) | (frac << 13)
    } else {
        (sign << 31) | ((exp + 127 - 15) << 23) | (frac << 13)
    };
    f32::from_bits(value)
}

pub(crate) fn fp16_bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(2)
        .map(|c| f16_to_f32(u16::from_le_bytes([c[0], c[1]])))
        .collect()
}

fn five_value(blob: &[u8], rows: usize, cols: usize) -> Result<Record, PhononError> {
    let rb = cols.div_ceil(5);
    let trit_bytes = rows * rb;
    if blob.len() < trit_bytes + 4 * rows || cols % 4 != 0 {
        return Err(PhononError::Model(format!(
            "five-value record {rows}x{cols} is truncated or misaligned"
        )));
    }
    let mut codes = vec![0u8; rows * cols];
    let mut nnz = 0usize;
    for r in 0..rows {
        for b in 0..rb {
            let mut x = blob[r * rb + b];
            for k in 0..5 {
                let col = b * 5 + k;
                let digit = x % 3;
                x /= 3;
                if col < cols {
                    codes[r * cols + col] = digit;
                    nnz += (digit != 1) as usize;
                }
            }
        }
    }
    let rbytes = nnz.div_ceil(8);
    let bits = &blob[trit_bytes..trit_bytes + rbytes];
    let off = trit_bytes + rbytes;
    if off + 4 * rows != blob.len() {
        return Err(PhononError::Model(format!(
            "five-value record {rows}x{cols}: expected {} bytes, found {}",
            off + 4 * rows,
            blob.len()
        )));
    }
    let lo = blob[off..off + 2 * rows].to_vec();
    let hi = blob[off + 2 * rows..off + 4 * rows].to_vec();
    let q4 = cols / 4;
    let mut plane_a = vec![0u8; rows * q4];
    let mut plane_b = vec![0u8; rows * q4];
    let mut bit = 0usize;
    for r in 0..rows {
        for c in 0..cols {
            let code = codes[r * cols + c];
            let is_hi = if code != 1 {
                let v = (bits[bit >> 3] >> (bit & 7)) & 1 == 1;
                bit += 1;
                v
            } else {
                false
            };
            let qa = code;
            let qb = if is_hi { code } else { 1 };
            let shift = (c % 4) * 2;
            plane_a[r * q4 + c / 4] |= qa << shift;
            plane_b[r * q4 + c / 4] |= qb << shift;
        }
    }
    Ok(Record::FiveValue {
        rows,
        cols,
        plane_a,
        plane_b,
        lo,
        hi,
    })
}

fn int_n(blob: &[u8], shape: &[usize], bits: u32) -> Result<Record, PhononError> {
    let rows = shape[0];
    let total: usize = shape.iter().product();
    let cols = total / rows;
    if blob.len() < 2 * rows {
        return Err(PhononError::Model("integer record is truncated".into()));
    }
    let (body, scales) = blob.split_at(blob.len() - 2 * rows);
    let q: Vec<i8> = match bits {
        8 => body.iter().take(total).map(|&b| b as i8).collect(),
        6 => {
            let mut out = Vec::with_capacity(body.len() / 3 * 4);
            for t in body.chunks_exact(3) {
                let packed = t[0] as u32 | (t[1] as u32) << 8 | (t[2] as u32) << 16;
                for s in [0, 6, 12, 18] {
                    out.push(((packed >> s) & 0x3f) as i8 - 32);
                }
            }
            out.truncate(total);
            out
        }
        other => return Err(PhononError::Model(format!("unsupported int{other} record"))),
    };
    if q.len() != total {
        return Err(PhononError::Model(format!(
            "int{bits} record holds {} values, expected {total}",
            q.len()
        )));
    }
    Ok(Record::IntN {
        rows,
        cols,
        q,
        scales: scales.to_vec(),
    })
}

impl Container {
    pub fn read(path: &Path) -> Result<Self, PhononError> {
        let io = |e: std::io::Error| PhononError::Model(format!("{}: {e}", path.display()));
        let mut file = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(path).map_err(io)?);
        let mut len = [0u8; 8];
        file.read_exact(&mut len).map_err(io)?;
        let mut header = vec![0u8; u64::from_le_bytes(len) as usize];
        file.read_exact(&mut header).map_err(io)?;
        let header: serde_json::Value = serde_json::from_slice(&header)
            .map_err(|e| PhononError::Model(format!("container header: {e}")))?;
        if header["format"] != FORMAT {
            return Err(PhononError::Model(format!(
                "unexpected container format {}",
                header["format"]
            )));
        }
        let entries = header["index"]
            .as_array()
            .ok_or_else(|| PhononError::Model("container has no index".into()))?;
        let mut pending: Vec<(String, String, Vec<usize>, Vec<u8>)> = Vec::with_capacity(entries.len());
        for entry in entries {
            let name = entry["n"].as_str().unwrap_or_default().to_string();
            let kind = entry["k"].as_str().unwrap_or_default().to_string();
            let shape: Vec<usize> = entry["shape"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_u64()).map(|v| v as usize).collect())
                .unwrap_or_default();
            let size = entry["b"].as_u64().unwrap_or(0) as usize;
            let mut blob = vec![0u8; size];
            file.read_exact(&mut blob).map_err(io)?;
            pending.push((name, kind, shape, blob));
        }
        // Decoding the 264 five-value matrices dominates load time; spread it across the CPU.
        let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
        let next = std::sync::atomic::AtomicUsize::new(0);
        let decoded: std::sync::Mutex<Vec<(String, Result<Record, PhononError>)>> =
            std::sync::Mutex::new(Vec::with_capacity(pending.len()));
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((name, kind, shape, blob)) = pending.get(i) else { break };
                    let record = match kind.as_str() {
                        "five_value" if shape.len() == 2 => five_value(blob, shape[0], shape[1]),
                        "fp16" => Ok(Record::Fp16 { data: blob.clone() }),
                        k if k.starts_with("int") && !shape.is_empty() => {
                            int_n(blob, shape, k[3..].parse().unwrap_or(0))
                        }
                        other => Err(PhononError::Model(format!(
                            "record {name}: unsupported kind {other}"
                        ))),
                    };
                    decoded.lock().unwrap().push((name.clone(), record));
                });
            }
        });
        let mut records = HashMap::new();
        for (name, record) in decoded.into_inner().unwrap() {
            records.insert(name, record?);
        }
        let mut tail = [0u8; 1];
        if file.read(&mut tail).map_err(io)? != 0 {
            return Err(PhononError::Model("container has trailing bytes".into()));
        }
        Ok(Self { records })
    }

    /// Any tensor as f32 (fp16 widened, integer tables dequantised by their row scale).
    pub fn f32(&self, name: &str) -> Result<Vec<f32>, PhononError> {
        match self.records.get(name) {
            Some(Record::Fp16 { data }) => Ok(fp16_bytes_to_f32(data)),
            Some(Record::IntN {
                rows,
                cols,
                q,
                scales,
            }) => {
                let scale = fp16_bytes_to_f32(scales);
                let mut out = Vec::with_capacity(rows * cols);
                for r in 0..*rows {
                    out.extend(q[r * cols..(r + 1) * cols].iter().map(|&v| v as f32 * scale[r]));
                }
                Ok(out)
            }
            Some(Record::FiveValue { .. }) => Err(PhononError::Model(format!(
                "{name} is a five-value matrix, not a plain tensor"
            ))),
            None => Err(PhononError::Model(format!("model is missing tensor {name}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_precision_conversion() {
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert_eq!(f16_to_f32(0x0000), 0.0);
        assert!((f16_to_f32(0x0001) - 5.960_464_5e-8).abs() < 1e-12); // smallest subnormal
        assert!(f16_to_f32(0x7c00).is_infinite());
    }

    #[test]
    fn five_value_roundtrip() {
        // One row of 8 columns: signs [+, 0, -, +, 0, 0, -, +]; codes are sign + 1.
        let codes = [2u8, 1, 0, 2, 1, 1, 0, 2];
        let mut blob = Vec::new();
        for chunk in codes.chunks(5) {
            let mut byte = 0u8;
            for (k, c) in chunk.iter().enumerate() {
                byte += c * 3u8.pow(k as u32);
            }
            blob.push(byte);
        }
        // Non-zero positions are 0,2,3,6,7; mark 0 and 3 as "hi".
        blob.push(0b0000_1001);
        blob.extend_from_slice(&0x3c00u16.to_le_bytes()); // lo = 1.0
        blob.extend_from_slice(&0x4000u16.to_le_bytes()); // hi = 2.0
        let Record::FiveValue { plane_a, plane_b, lo, hi, .. } = five_value(&blob, 1, 8).unwrap()
        else {
            panic!("expected a five-value record")
        };
        // plane_a stores the code; plane_b stores 1 + sign * is_hi.
        let unpack = |plane: &[u8]| -> Vec<u8> {
            (0..8).map(|c| (plane[c / 4] >> ((c % 4) * 2)) & 3).collect()
        };
        assert_eq!(unpack(&plane_a), codes);
        assert_eq!(unpack(&plane_b), [2, 1, 1, 1, 1, 1, 0, 1]);
        assert_eq!((lo, hi), (vec![0, 0x3c], vec![0, 0x40]));
    }

    #[test]
    fn six_bit_integers_widen_with_row_scale() {
        // Values 0, 63, 32, 1 pack into one 24-bit group; row scale 0.5.
        let packed: u32 = 0 | 63 << 6 | 32 << 12 | 1 << 18;
        let mut blob = vec![packed as u8, (packed >> 8) as u8, (packed >> 16) as u8];
        blob.extend_from_slice(&0x3800u16.to_le_bytes()); // 0.5
        let Record::IntN { q, rows, cols, .. } = int_n(&blob, &[1, 4], 6).unwrap() else {
            panic!("expected an integer record")
        };
        assert_eq!((rows, cols), (1, 4));
        assert_eq!(q, vec![-32, 31, 0, -31]);
    }
}
