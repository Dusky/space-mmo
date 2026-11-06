# Space MMO (working title)

A text/menu-based space MMO inspired by EVE Online and Elite Dangerous, aimed at a Steam release on Windows, macOS and Linux, played with keyboard and mouse. Steam Deck and controller support are not goals. No Electron or web runtime: the client is native.

The full design for the current milestone is in `docs/design.md`. Read it before starting any feature. The radar screen reference mockup is `docs/mockups/radar-standard.html` (open it in a browser). For where the work stands and what's next, read `docs/handoff.md`.

## Working agreements

- Ask before commits, pushes, creating containers, and anything that costs money.
- Never print secrets, such as `.env` contents.

## Current milestone: the vertical slice

One complete loop: tutorial pocket (instanced, one per player) → station view → buy the Sparrow → undock → warp to belt → mine → warp back → dock → sell → one-way exit jump into Kesh, the home system. Everything else is out of scope until the slice works end to end.

## Architecture rules

- **The server is authoritative.** It runs a 1-second tick and owns all game state. The client sends commands and draws what the server tells it; it never decides outcomes.
- **Command verbs are the API.** Every player action is a named command (`undock`, `warp belt-1`, `approach <id>`, `mine <id>`, `dock`, `sell ferrite all`, `buy ship sparrow`, `jump exit`). The client UI, the command line and the future public automation API all send the same commands. Never add an action that bypasses this.
- **Every command validates the ship state first** (see the state diagram in the design doc) and returns a clear refusal reason the client can show.
- **Radar only receives what is on the player's own grid.** Never send the client information the player could not see.
- **Space is grids.** Each system has grids (station, belts, gates); a ship on a grid has a position in kilometres; warp moves a ship between grids.
- **Keep data separate from presentation.** UI layouts and themes will become swappable data files later. Themes are data only, never scripts.
- **Numbers live in config**, not code. Values in the design doc's "Starting numbers" table are tuning starting points.

## Stack

- Server: Rust with tokio, one process. Postgres via sqlx; the world lives in memory and is saved every few ticks.
- Protocol: Protobuf over TLS. The `.proto` schema is the single source of truth; generate server and client code from it.
- Client: Godot 4 (C#) or Avalonia 12 (C#), not decided yet. See "Client test" in the design doc. Until it's decided, don't write client code beyond the test spikes and the engine-agnostic `client-core/` library they share.
- Steam: Steamworks.NET. Login uses Steam session tickets validated by the server; no passwords stored.
- Tests and load: a headless Rust bot client that speaks the protocol.
- CI: GitHub Actions building all three platforms.

## Naming

Game content is our own, not EVE's. The common ore is **Ferrite**; the rich ore is **Siltane**. Don't use EVE names (Veldspar, Jita, etc.) anywhere.

## Repo layout

```
proto/        # .proto schema (source of truth)
protocol/     # Rust code generated from the schema
server/       # Rust server
bot/          # Rust headless client (tests, load, API reference)
config/       # Tuning numbers and world content
client-core/  # C# connection and protocol library, shared by both client tests
spikes/       # Avalonia and (next) Godot test clients, plus ProtoCheck
client/       # The chosen client, after the client test
docs/         # design.md, handoff.md, mockups
```
