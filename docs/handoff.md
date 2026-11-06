# Handoff

Where the project stands, for whoever picks it up next. Written 2026-11-05, when development moved from the takoyaki server to Matt's PC. Update or trim this file as things change; `CLAUDE.md` and `docs/design.md` stay the source of truth for rules and design.

## State

The vertical slice is playable end to end, by the bot and by a person through the Avalonia client.

| Part | State |
| --- | --- |
| Server (`server/`) | Done for the slice. Tutorial pocket, Kesh, shipyard, market, mining, warp, dock, exit jump, skills data. 1 tick = 1 game second. |
| Persistence | World in memory, saved to Postgres every 5 s and on Ctrl-C. Restore refuses to load a save that no longer matches config or content. |
| Protocol (`proto/`, `protocol/`) | Protobuf over TLS, 4-byte big-endian length-prefixed frames. Rust code via prost and protox, so no `protoc` install is needed. |
| Bot (`bot/`) | Plays the whole slice. `cargo test --workspace` runs it sped up over real TLS. |
| Client core (`client-core/`) | Plain C# library: TLS connect, dev login, commands, snapshots. No engine code, so Godot C# can use it as is. |
| Avalonia client (`spikes/avalonia/`) | Every slice screen, playable against the real server or offline with fake data. A headless run plays the tutorial into Kesh. Only tested headless on Linux so far. |
| Godot client | Not started. Matt is building it on the PC. |
| CI | Green. Rust on Windows, macOS and Linux; a Postgres job; Avalonia builds for win-x64, osx-arm64 and linux-x64 as artifacts. |

## Getting going on a new machine

You need Rust from rustup and the .NET 10 SDK. Postgres is optional.

```sh
cargo test --workspace                      # Storage tests skip without DATABASE_URL
cargo run -p server -- --dev --no-db        # Local server, world not saved
cd spikes/avalonia
dotnet run -c Release -- --server 127.0.0.1:7777 --account matt --ca ../../dev-certs/ca.pem
```

The server writes a new dev CA to `dev-certs/` (relative to where it runs) on every `--dev` start, so restart the client after restarting the server. Run the server from the repo root so the paths line up.

To save the world locally, run any Postgres and put `DATABASE_URL=postgres://...` in a git-ignored `.env` in the repo root. Tables are created on first start. On takoyaki, the dev database is a Docker container on 127.0.0.1:5433 (compose in `~/.docker/space-mmo-db`); it is only reachable from that machine.

## Next steps, roughly in order

1. **Play the Avalonia client by hand** on a real desktop. Nobody has used it with a real screen, mouse and keyboard yet. Note what feels slow or confusing.
2. **Build the Godot prototype** with the same list as the Avalonia test (design doc, "Client test"). Reuse `client-core` and copy the shape of `FakeSession` for offline mode. Watch for:
   - `client-core` targets `net10.0`. If the Godot version in use needs an older .NET, multi-target it (for example `net8.0;net10.0`) rather than forking it.
   - Network events arrive on a background thread. Marshal them to the main thread (`CallDeferred` in Godot), as the Avalonia client does with `Dispatcher.UIThread.Post`.
   - Library code keeps `ConfigureAwait(false)` on every await, or a blocking call from the UI deadlocks.
3. **Finish the client-test checks** for both engines: Steam overlay (Shift+Tab, needs Steamworks.NET and Steam running), sound, high-DPI and ultrawide, non-Latin input, frame time with a GPU, macOS notarization.
4. **Decide the engine.** Overlay is pass or fail; otherwise pick whichever made screens faster to build. Then move the winner from `spikes/` to `client/`.
5. **Server work not yet done:**
   - Keep the dev CA between restarts, so a client only needs `ca.pem` copied once.
   - Rate limits on connections and commands.
   - Skill training. Costs and book rules are already in config and `server/src/skills.rs`; nothing trains yet.
   - Steam session ticket login. Without `--dev` the server currently refuses every login.
6. **Open design questions** are listed in the design doc, under "Screens and open questions".

## Things learned the hard way

- **Dev TLS:** the CA and server certificate must have different names. When they shared one, OpenSSL-based clients (.NET on Linux) treated the leaf as self-signed and failed with NotSignatureValid. Fixed in `server/src/dev_tls.rs`; keep it that way.
- **Headless Avalonia** (`--screenshots DIR`) needs `DOTNET_ROOT` set when the .NET SDK isn't installed system-wide, or the apphost can't find the runtime. On a desktop install this isn't an issue.
- **Software-rendered numbers** in `spikes/avalonia/README.md` came from a Linux server with no GPU. Don't compare them with GPU numbers from the Godot test.
- **Avalonia 12:** `Bitmap.Save` takes encoder options (`PngBitmapEncoderOptions.Default`).
- **Clippy:** the protocol crate allows `clippy::large_enum_variant`, because prost generates it.

## How Matt likes to work

- Ask before commits, pushes, creating containers, and anything that costs money.
- Never print secrets, such as the contents of `.env`.
- Game content is our own: no EVE names (Veldspar, Jita, and so on).
- Pacing is EVE-like and slow: Mining Laser II takes a dedicated miner about a month. Basic skills are free; advanced skills need bought books.
- Controller and Steam Deck support are not goals. The target is Steam on Windows, macOS and Linux, keyboard and mouse.
