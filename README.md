# Space MMO

A text and menu-based space MMO. See `docs/design.md` for the design and `CLAUDE.md` for the architecture rules.

Needs Rust from [rustup](https://rustup.rs). There is no client yet, so the headless bot plays the game.

```sh
cargo test --workspace            # Includes the whole slice, sped up, over real TLS

# Run it for real: start the server, then a bot in a second terminal
cargo run -p server -- --dev
cargo run -p bot -- --account alice
```

`--dev` lets anyone who can reach the port log in as any account, so keep it on localhost. Without it, the server needs `--cert` and `--key`, and refuses every login until Steam tickets are implemented.

Game state is in memory for now, and is lost when the server stops.

| Path | What it holds |
| --- | --- |
| `proto/` | Protocol schema, the source of truth |
| `protocol/` | Rust code generated from the schema |
| `server/` | Authoritative server. `sim.rs` is the game, `net.rs` the networking |
| `bot/` | Headless client and slice tests |
| `config/` | Tuning numbers and world content. `ASSUMED` marks values the design doc doesn't specify yet |
| `docs/` | Design doc and mockups |
