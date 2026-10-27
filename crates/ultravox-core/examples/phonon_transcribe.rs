//! `cargo run --release --example phonon_transcribe -- <models_dir> <audio.wav>`
use std::time::Instant;
fn main() {
    let mut args = std::env::args().skip(1);
    let models = std::path::PathBuf::from(args.next().expect("models dir"));
    let wav = std::path::PathBuf::from(args.next().expect("wav"));
    let t = Instant::now();
    ultravox_core::phonon::prepare(&models).expect("load");
    eprintln!("loaded in {:.2}s", t.elapsed().as_secs_f64());
    let t = Instant::now();
    let text = ultravox_core::phonon::transcribe_file(&models, &wav, None).expect("transcribe");
    eprintln!("transcribed in {:.2}s", t.elapsed().as_secs_f64());
    println!("{text}");
}
