# CapDraft TTS: Rust port

Beta, shipped beside the Python build. See [MAP.md](MAP.md) for what is ported and [DECISIONS.md](DECISIONS.md) for why.

```sh
cargo test     # unit tests + replay of every case recorded from the Python original
```

```sh
cargo run --release -p capdraft-app   # the desktop app (crates/capdraft-app, Slint)
```
