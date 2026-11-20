//! A live TTS round trip with the Rust client (needs internet and CapCut's API):
//!
//! ```sh
//! cargo run --release --example tts_live -- [legacy|upstream] ["text 1" "text 2" ...]
//! ```
//!
//! Uses `external/capcut-tts-api/device.json`, writes audio into a temporary
//! cache folder and prints each result.

use capdraft_core::capcut_client::DeviceProfile;
use capdraft_core::tts::{CapCutTts, TtsSettings, UreqTransport};
use std::sync::Arc;

fn main() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let profile = match args.first().map(String::as_str) {
        Some("legacy") => {
            args.remove(0);
            DeviceProfile::Legacy
        }
        Some("upstream") => {
            args.remove(0);
            DeviceProfile::Upstream
        }
        _ => DeviceProfile::Upstream,
    };
    if args.is_empty() {
        args = vec![
            "Xin chào, đây là CapDraft TTS.".into(),
            "Hello from the Rust port.".into(),
            "Thử nghiệm ba.".into(),
        ];
    }
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let work = std::env::temp_dir().join("capdraft-tts-live");
    let settings = TtsSettings {
        chunk_size: 25,
        parallel_chunks: 4,
        poll_interval_sec: 1.0,
        download_workers: 8,
        default_voice: "BV074_streaming".into(),
        default_resource_id: "7102355709945188865".into(),
        default_rate: 1.0,
        cache_dir: work.join("cache"),
        projects_dir: work.join("projects"),
        device_json_path: repo.join("external/capcut-tts-api/device.json"),
        ffprobe_path: "ffprobe".into(),
        device_profile: profile,
    };
    let tts = CapCutTts::new(settings, "", Arc::new(UreqTransport))?;
    let items: Vec<(i64, String)> = args
        .iter()
        .enumerate()
        .map(|(i, t)| (i as i64 + 1, t.clone()))
        .collect();
    let never: capdraft_core::tts::CancelCallback = Arc::new(|| false);
    let started = std::time::Instant::now();
    let res = tts.generate_tts_batch(&items, None, None, None, &never, &None, false);
    for (idx, r) in res.iter() {
        println!(
            "#{idx} {} {:?}s {} {}",
            r.status,
            r.duration,
            r.audio_path,
            r.error.as_deref().unwrap_or("")
        );
    }
    let missing = items.iter().filter(|(i, _)| !res.contains(*i)).count();
    println!(
        "{} result(s), {missing} without a result (failed chunk), {:.1}s, profile {profile:?}",
        res.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
