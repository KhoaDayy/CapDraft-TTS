//! Parse a file once (for profilers): `parse_once <file.json>`.
fn main() {
    let Some(path) = std::env::args().nth(1) else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let v = capdraft_core::json::parse(&text);
    std::hint::black_box(v.is_ok());
}
