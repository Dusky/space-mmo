# Avalonia client test

The Avalonia half of the client test in `docs/design.md`. It started with fake data only; it now also plays the real server, so the slice can be played by a person.

The window only draws snapshots and sends command lines. Networking lives in `../../client-core`, which is plain C# and works for a Godot C# client too. Offline mode uses `FakeSession`, which sends the same snapshots, so the UI can't tell the difference.

## Run it

Download the build for your system from the latest CI run on GitHub (Actions → CI → Artifacts → `client-test-avalonia-*`), unzip it and run `SpaceMmoSpike`. On macOS the app is unsigned, so right-click → Open the first time. With no options it runs offline with fake data.

To play a real server (one started with `--dev`):

```sh
SpaceMmoSpike --server HOST:7777 --account yourname --ca path/to/dev-certs/ca.pem
```

The server writes `dev-certs/ca.pem` each time it starts with `--dev`, and the client must have that copy to trust it.

To build from source, install the .NET 10 SDK and run `dotnet run -c Release --` with the same options in this folder.

| Key or command | Does |
| --- | --- |
| F1, F2, … | Station services while docked, in the order the station lists them |
| Esc | Focus the command line |
| Ctrl+F | Focus the overview filter |
| Tab, ↑ and ↓ | Complete commands and arguments from the snapshot, walk history |
| Overview row, then Target buttons | Approach, Mine, Dock or Jump, depending on what's selected |
| Ship buttons | Warp to sites in the system, Stop |
| Any server verb | `undock`, `warp belt-1`, `approach rock-3`, `mine rock-3`, `dock`, `sell ferrite all`, `buy ship sparrow`, `jump exit` |
| `theme amber`, `theme standard` | Swap theme at runtime (client only) |
| `glow off`, `glow fake`, `glow blur` | Radar glow styles (client only) |

`--screenshots DIR` renders without a display. Offline it captures the fake-data screens and times radar frames. With `--server` it plays the whole tutorial through the window's own command line and captures each stage: an end-to-end test of client and server together.

## Results so far

Measured on the Linux server with no GPU, so rendering was in software. A real desktop with a GPU should do much better.

| Item | Result |
| --- | --- |
| Station view with services and hotkeys | Built, from the station's own service list |
| Radar: rings, sweep, 20 contacts moving every second | Built; scales to the grid and shows laser range |
| Overview: 200 rows, sorting, filters | Built; selection drives the radar and Target panel |
| Command line with history and tab completion | Built; completes sites and contact ids from the snapshot |
| Theme swap at runtime | Built; themes are JSON colour tokens, validated on load |
| Crisp monospace text at 11 to 14 px | Crisp in screenshots, using the mockup's IBM Plex Mono. Needs a real screen. |
| Glow without hurting frame rate | Software: none 38 ms/frame, fake about the same, real blur about 92 ms. Needs a GPU test. |
| Low CPU while docked | Software: 7% of one core docked, 210 MB RAM; a full core in space at 60 fps. Needs a GPU test. |
| Generated protobuf client connects | Passed, and the client plays the whole tutorial into Kesh on the real server |
| Steam overlay (Shift+Tab) | Not tested: needs Steamworks.NET and a desktop with Steam |
| Sound, high-DPI and ultrawide, non-Latin input | Not tested yet |

Controller and Steam Deck support are not goals.

## Notes for the comparison

- Building screens: XAML layout plus C# was quick. The radar is a hand-drawn custom control of about 250 lines.
- Avalonia 12 changes from 11 cost a couple of compile fixes (`Bitmap.Save` now takes encoder options).
- A UI framework installs its own synchronisation context, so library code must use `ConfigureAwait(false)` or a blocking call deadlocks. `client-core` does; a Godot client would hit the same rule.
- Text, theming and list virtualisation came for free. Animation is manual: a timer redraws the radar, and stops while docked.
