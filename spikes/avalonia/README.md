# Avalonia client test

The Avalonia half of the client test in `docs/design.md`: the same screens the Godot test will build, using fake data and no server. `../ProtoCheck` covers the networking check separately.

## Run it

Download the build for your system from the latest CI run on GitHub (Actions → CI → Artifacts → `client-test-avalonia-*`), unzip it and run `SpaceMmoSpike`. On macOS the app is unsigned, so right-click → Open the first time.

To build from source instead, install the .NET 10 SDK and run `dotnet run -c Release` in this folder. Add `--undocked` to start in space.

| Key or command | Does |
| --- | --- |
| F1 to F4 | Station services while docked (F4 undocks) |
| Esc | Focus the command line |
| Ctrl+F | Focus the overview filter |
| Tab, ↑ and ↓ | Complete commands, walk history |
| `undock`, `dock`, `warp belt-1`, `approach ship-4`, `mine rock-3` | Fake versions of the real verbs |
| `theme amber`, `theme standard` | Swap theme at runtime |
| `glow off`, `glow fake`, `glow blur` | Radar glow styles, for the frame rate check |

`--screenshots DIR` renders the screens without a display and times radar frames.

## Results so far

Measured on the Linux server with no GPU, so rendering was in software. Numbers on a real desktop with a GPU should be much better.

| Item | Result |
| --- | --- |
| Station view with services and hotkeys | Built |
| Radar: rings, sweep, 20 contacts moving every second | Built |
| Overview: 200 rows, sorting, filters | Built; the selected row highlights on radar |
| Command line with history and tab completion | Built |
| Theme swap at runtime | Built; themes are JSON colour tokens, validated on load |
| Crisp monospace text at 11 to 14 px | Looks crisp in screenshots, using the mockup's IBM Plex Mono. Needs a look on a real screen. |
| Glow without hurting frame rate | Software: no glow 38 ms/frame, fake glow (double-drawn lines) about the same, real blur about 100 ms. Needs a GPU test. |
| Low CPU while docked | Software: 7% of one core docked, 210 MB RAM. In space at 60 fps, a full core. Needs a GPU test. |
| Generated protobuf client connects | Passed: ProtoCheck logged in over TLS 1.3 and bought a ship on the real server |
| Steam overlay (Shift+Tab) | Not tested: needs Steamworks.NET and a desktop with Steam |
| Sound, high-DPI and ultrawide, non-Latin input | Not tested yet |

Controller and Steam Deck support are not goals, so Avalonia's lack of built-in gamepad input doesn't count against it.

## Notes for the comparison

- Building screens: XAML layout plus C# was quick. The radar is a custom control drawn by hand, roughly 200 lines.
- Avalonia 12 changes from 11 cost a couple of compile fixes (`Bitmap.Save` now takes encoder options).
- Text, theming and list virtualisation came for free. Animation is manual: a timer redraws the radar, and stops while docked.
