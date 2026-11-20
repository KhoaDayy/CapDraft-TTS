//! New ids and the clock, overridable per thread for replay tests (ADR-012).

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

thread_local! {
    static SEQ: Cell<Option<u128>> = const { Cell::new(None) };
    static CLOCK: Cell<Option<f64>> = const { Cell::new(None) };
}

/// `str(uuid.uuid4()).upper()`.
#[must_use]
pub fn new_uuid_upper() -> String {
    SEQ.with(|seq| match seq.get() {
        Some(n) => {
            seq.set(Some(n + 1));
            let hex = format!("{:032X}", n + 1);
            format!(
                "{}-{}-{}-{}-{}",
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..]
            )
        }
        None => uuid::Uuid::new_v4().to_string().to_uppercase(),
    })
}

/// `time.time()`.
#[must_use]
pub fn now_unix() -> f64 {
    CLOCK.with(Cell::get).unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64())
    })
}

/// `datetime.now()` (local time), or the fixed clock read as a naive time.
#[must_use]
pub fn local_now() -> chrono::NaiveDateTime {
    match CLOCK.with(Cell::get) {
        Some(t) => chrono::DateTime::from_timestamp(t.trunc() as i64, ((t.fract()) * 1e9) as u32)
            .map(|d| d.naive_utc())
            .unwrap_or_default(),
        None => chrono::Local::now().naive_local(),
    }
}

/// Ids become `00000000-0000-0000-0000-000000000001`, `…02`, … and the clock
/// reads `clock` on this thread until [`reset`].
pub fn deterministic(clock: f64) {
    SEQ.with(|s| s.set(Some(0)));
    CLOCK.with(|c| c.set(Some(clock)));
}

/// Back to random ids and the system clock.
pub fn reset() {
    SEQ.with(|s| s.set(None));
    CLOCK.with(|c| c.set(None));
}
