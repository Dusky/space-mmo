# Space MMO

A text and menu-based space MMO. See `docs/design.md` for the design and `CLAUDE.md` for the architecture rules.

Needs Rust from [rustup](https://rustup.rs). The headless bot plays the game, and so can the Avalonia test client in `spikes/avalonia`, which needs the .NET 10 SDK. For where the work stands and what's next, see `docs/handoff.md`.

```sh
cargo test --workspace            # Includes the whole slice, sped up, over real TLS

# Run it for real: start the server, then a bot in a second terminal
cargo run -p server -- --dev
cargo run -p bot -- --account alice
```

`--dev` lets anyone who can reach the port log in as any account, so keep it on localhost. Without it, the server needs `--cert` and `--key`, and refuses every login until Steam tickets are implemented.

The world lives in memory and is saved to Postgres every 5 seconds, and once more on Ctrl-C. The server reads `DATABASE_URL` from the environment or from `.env` (git-ignored), and creates its tables on first start. `--no-db` runs without saving. Storage tests make and drop their own throwaway database, so they never touch real data; they are skipped when no database is configured.

On the dev machine, Postgres runs in Docker from `~/.docker/space-mmo-db`, listening on 127.0.0.1:5433 only.

| Path | What it holds |
| --- | --- |
| `proto/` | Protocol schema, the source of truth |
| `protocol/` | Rust code generated from the schema |
| `server/` | Authoritative server. `sim.rs` is the game, `net.rs` the networking |
| `bot/` | Headless client and slice tests |
| `config/` | Tuning numbers and world content. `ASSUMED` marks values the design doc doesn't specify yet |
| `client-core/` | C# connection and protocol library, shared by the client tests |
| `spikes/` | Client tests (Avalonia now, Godot next) and ProtoCheck |
| `docs/` | Design doc, handoff notes and mockups |
