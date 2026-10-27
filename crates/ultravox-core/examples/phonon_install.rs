//! `cargo run --release --example phonon_install -- <models_dir>`: download, verify and unpack Phonon-2.
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let models = std::path::PathBuf::from(std::env::args().nth(1).expect("models dir"));
    let started = std::time::Instant::now();
    let watcher = tokio::spawn(async {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            eprintln!("progress {:.0}%", ultravox_core::phonon::install_progress() * 100.0);
        }
    });
    ultravox_core::phonon::install(models.clone()).await.expect("install");
    watcher.abort();
    eprintln!(
        "installed={} in {:.1}s",
        ultravox_core::phonon::is_installed(&models),
        started.elapsed().as_secs_f64()
    );
}
