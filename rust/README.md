# CapDraft TTS: Rust port

Beta, shipped beside the Python build. See [MAP.md](MAP.md) for what is ported and [DECISIONS.md](DECISIONS.md) for why.

```sh
cargo test     # unit tests + replay of every case recorded from the Python original
```

```sh
cargo run --release -p capdraft-app   # the desktop app (crates/capdraft-app, Slint)
```

CI (`.github/workflows/ci.yml`, on push to `main` and on pull requests): fmt, clippy and every
test on Linux, clippy and a release build of the app on Windows, and the Python test suite.
