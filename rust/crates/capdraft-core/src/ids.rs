//! New ids and the clock, overridable per thread for replay tests (ADR-012).

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

thread_local! {
    static SEQ: Cell<Option<u128>> = const { Cell::new(None) };
    static CLOCK: Cell<Option<f64>> = const { Cell::new(None) };
    static RAND: Cell<Option<u64>> = const { Cell::new(None) };
    static BYTES: Cell<Option<u64>> = const { Cell::new(None) };
}

/// The next UUID: random, or `UUID(int=n)` in deterministic mode.
fn next_uuid() -> uuid::Uuid {
    SEQ.with(|seq| match seq.get() {
        Some(n) => {
            seq.set(Some(n + 1));
            uuid::Uuid::from_u128(n + 1)
        }
        None => uuid::Uuid::new_v4(),
    })
}

/// `str(uuid.uuid4())` (lower case).
#[must_use]
pub fn new_uuid_lower() -> String {
    next_uuid().to_string()
}

/// `uuid.uuid4().hex`.
#[must_use]
pub fn uuid_hex() -> String {
    next_uuid().simple().to_string()
}

/// `random.randint(a, b)`.
#[must_use]
pub fn randint(a: i64, b: i64) -> i64 {
    let span = (b - a + 1).max(1) as u64;
    RAND.with(|r| match r.get() {
        Some(n) => {
            r.set(Some(n + 1));
            a + (((n + 1) * 7) % span) as i64
        }
        None => {
            let mut buf = [0u8; 8];
            let _ = getrandom::fill(&mut buf);
            a + (u64::from_le_bytes(buf) % span) as i64
        }
    })
}

/// `secrets.token_bytes(n)`.
#[must_use]
pub fn random_bytes(n: usize) -> Vec<u8> {
    BYTES.with(|c| match c.get() {
        Some(start) => {
            c.set(Some(start + n as u64));
            (0..n as u64)
                .map(|i| (((start + i) * 73) % 256) as u8)
                .collect()
        }
        None => {
            let mut buf = vec![0u8; n];
            let _ = getrandom::fill(&mut buf);
            buf
        }
    })
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
    RAND.with(|r| r.set(Some(0)));
    BYTES.with(|b| b.set(Some(0)));
}

/// The deterministic counters of this thread, to carry into worker threads.
#[must_use]
pub fn snapshot() -> (Option<u128>, Option<f64>, Option<u64>, Option<u64>) {
    (
        SEQ.with(Cell::get),
        CLOCK.with(Cell::get),
        RAND.with(Cell::get),
        BYTES.with(Cell::get),
    )
}

/// Install counters taken with [`snapshot`] (a worker thread starts where its parent was).
pub fn restore(s: (Option<u128>, Option<f64>, Option<u64>, Option<u64>)) {
    SEQ.with(|c| c.set(s.0));
    CLOCK.with(|c| c.set(s.1));
    RAND.with(|c| c.set(s.2));
    BYTES.with(|c| c.set(s.3));
}

/// Back to random ids and the system clock.
pub fn reset() {
    SEQ.with(|s| s.set(None));
    CLOCK.with(|c| c.set(None));
    RAND.with(|r| r.set(None));
    BYTES.with(|b| b.set(None));
}
