//! The whole generate-and-attach pipeline on a project folder against the fake
//! CapCut server (instant), for `rust/tools/check_pipeline.py`.
//!
//! ```sh
//! cargo run --release --example pipeline_bench -- <app_root> <deterministic 0|1>
//! ```
//! `<app_root>` holds `config.json` and `Proj/`. Prints the seconds taken.

#[path = "../tests/common/mod.rs"]
mod common;

use capdraft_core::alignment::NativeAudioAlignmentSettings;
use capdraft_core::config::AppConfig;
use capdraft_core::models::ToneModifyMode;
use capdraft_core::service::{GenerateRequest, ProjectTtsService};
use std::path::PathBuf;
use std::sync::Arc;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(
        args.get(1)
            .ok_or("usage: pipeline_bench <app_root> <deterministic>")?,
    );
    let deterministic = args.get(2).is_some_and(|a| a == "1");
    if deterministic {
        capdraft_core::ids::deterministic(1_700_000_000.25);
    }
    let started = std::time::Instant::now();
    let config = AppConfig::load(&root);
    let mut svc = ProjectTtsService::new(config, Arc::new(common::FakeCapCut::default()), None)
        .with_tts_sleep(Arc::new(|_| {}));
    svc.load_project(&root.join("Proj"))?;
    let ids: Vec<String> = svc
        .get_captions()?
        .iter()
        .map(|c| c.text_segment_id.clone())
        .collect();
    if deterministic {
        capdraft_core::ids::deterministic(1_700_000_000.25);
    }
    let req = GenerateRequest {
        selected_caption_ids: ids,
        voice_type: "BV421_vivn_streaming".into(),
        resource_id: "7252594014782755330".into(),
        voice_display_name: "Nhỏ Ngọt Ngào".into(),
        tts_rate: 1.0,
        capcut_clip_speed: 1.3,
        tone_modify_mode: ToneModifyMode::PreservePitch,
        existing_tts_mode: "replace_existing".into(),
        allow_partial_attach: true,
        use_cache: true,
        alignment: NativeAudioAlignmentSettings::default(),
    };
    let r = svc.generate_and_attach(&req, None, None, None)?;
    println!(
        "{:.3} success={} attached={} failed={}",
        started.elapsed().as_secs_f64(),
        r.success,
        r.attached,
        r.failed
    );
    Ok(())
}
