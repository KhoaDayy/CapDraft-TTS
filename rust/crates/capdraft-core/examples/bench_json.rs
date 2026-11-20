//! `cargo run --release --example bench_json -- <draft_content.json>`: parse,
//! clone and dump timings for the json module (best of 7).

use capdraft_core::json::{dumps_compact, parse};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
use std::time::Instant;

fn best<T>(mut f: impl FnMut() -> T) -> f64 {
    (0..7)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(f());
            t.elapsed().as_secs_f64()
        })
        .fold(f64::MAX, f64::min)
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: bench_json <draft_content.json>");
        return;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    let mb = bytes.len() as f64 / 1e6;
    println!("read file      {:.4}s", best(|| std::fs::read(&path)));
    println!(
        "utf-8 check    {:.4}s",
        best(|| std::str::from_utf8(&bytes).is_ok())
    );
    let Ok(text) = String::from_utf8(bytes) else {
        return;
    };
    let t = best(|| parse(&text));
    println!("parse          {t:.4}s  {:.0} MB/s", mb / t);
    let Ok(v) = parse(&text) else { return };
    println!("clone (COW)    {:.6}s", best(|| v.clone()));
    let t = best(|| dumps_compact(&v));
    println!("dumps_compact  {t:.4}s  {:.0} MB/s", mb / t);
}
