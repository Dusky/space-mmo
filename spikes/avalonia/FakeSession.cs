using SpaceMmo.ClientCore;
using SpaceMmo.V1;

namespace SpaceMmo.Spike;

/// <summary>
/// Offline stand-in for the server, for the client test's fake-data checks:
/// 200 contacts, 20 of them ships moving every second. It speaks in the same
/// snapshots and command results as the real server, so the UI is identical.
/// </summary>
public sealed class FakeSession : IGameSession
{
    sealed class Body
    {
        public required string Id;
        public required string Name;
        public required ContactKind Kind;
        public double X, Y, Vx, Vy;
        public string Activity = "";
        public uint Volume;
    }

    readonly Random _rng = new(42);
    readonly List<Body> _bodies = [];
    readonly List<string> _events = [];
    readonly Timer? _timer;
    readonly object _lock = new();
    bool _docked = true;
    ShipState _state = ShipState.Docked;
    string _target = "";
    ulong _tick;
    ulong _credits = 4_000;

    public event Action<Snapshot>? SnapshotReceived;
    public event Action<string>? Closed;

    /// <param name="live">Tick once a second by itself. Off for scripted screenshots.</param>
    public FakeSession(bool live)
    {
        _bodies.Add(new Body { Id = "station", Name = "Kesh Station", Kind = ContactKind.Station, X = -42, Y = 31 });
        _bodies.Add(new Body { Id = "gate-vael", Name = "Gate → Vael", Kind = ContactKind.Gate, X = 71, Y = 68 });
        _bodies.Add(new Body { Id = "rock-vein", Name = "Rich Ferrite vein", Kind = ContactKind.RichVein, X = 18, Y = -36, Volume = 500 });
        for (var i = 1; i <= 158; i++)
        {
            var (cx, cy) = (i % 3) switch { 0 => (30.0, -20.0), 1 => (-60.0, -55.0), _ => (140.0, 90.0) };
            _bodies.Add(new Body
            {
                Id = $"rock-{i}", Name = "Ferrite asteroid", Kind = ContactKind.Asteroid,
                X = cx + _rng.NextDouble() * 30 - 15, Y = cy + _rng.NextDouble() * 30 - 15,
                Volume = (uint)_rng.Next(2000, 8000),
            });
        }
        string[] npcs = ["Hauler Oskin", "Prospector Mel", "Prospector Dray", "Kesh Patrol", "Ilsa Varn", "Tomas Reik", "Juno", "Sable", "Orrin", "Vex"];
        for (var i = 1; i <= 20; i++)
        {
            var npc = i % 2 == 0;
            _bodies.Add(new Body
            {
                Id = $"ship-{i}", Name = npc ? npcs[i / 2 - 1] : $"Pilot {i:00}",
                Kind = npc ? ContactKind.NpcShip : ContactKind.PlayerShip,
                X = _rng.NextDouble() * 160 - 80, Y = _rng.NextDouble() * 160 - 80,
                Vx = _rng.NextDouble() * 4 - 2, Vy = _rng.NextDouble() * 4 - 2,
                Activity = i % 3 == 0 ? "mining" : "moving",
            });
        }
        for (var i = 1; i <= 19; i++)
        {
            _bodies.Add(new Body
            {
                Id = $"signal-{i}", Name = $"Distant signal {i}", Kind = ContactKind.RichVein,
                X = _rng.NextDouble() * 600 - 300, Y = 150 + _rng.NextDouble() * 150,
            });
        }
        if (live) _timer = new Timer(_ => Tick(), null, TimeSpan.FromSeconds(1), TimeSpan.FromSeconds(1));
    }

    /// <summary>One fake server tick: ships move and bounce inside 95 km.</summary>
    public void Tick()
    {
        Snapshot snapshot;
        lock (_lock)
        {
            _tick++;
            foreach (var b in _bodies.Where(b => b.Vx != 0 || b.Vy != 0))
            {
                b.X += b.Vx;
                b.Y += b.Vy;
                if (Math.Abs(b.X) > 95) b.Vx = -b.Vx;
                if (Math.Abs(b.Y) > 95) b.Vy = -b.Vy;
            }
            snapshot = Build();
        }
        SnapshotReceived?.Invoke(snapshot);
    }

    public Task<CommandResult> SendAsync(string line, CancellationToken cancellationToken = default)
    {
        CommandResult result;
        Snapshot snapshot;
        lock (_lock)
        {
            result = Execute(line.Trim().ToLowerInvariant().Split(' ', StringSplitOptions.RemoveEmptyEntries));
            snapshot = Build();
        }
        SnapshotReceived?.Invoke(snapshot);
        return Task.FromResult(result);
    }

    CommandResult Execute(string[] words)
    {
        static CommandResult Ok(string m) => new() { Ok = true, Message = m };
        static CommandResult No(Refusal r, string m) => new() { Ok = false, Refusal = r, Message = m };
        switch (words)
        {
            case ["undock"] when _docked:
                _docked = false; _state = ShipState.Idle;
                return Ok("Undocked onto the Kesh Station grid.");
            case ["undock"]:
                return No(Refusal.WrongState, "You are not docked.");
            case ["dock"] when !_docked:
                _docked = true; _state = ShipState.Docked; _target = "";
                return Ok("Docked at Kesh Station.");
            case ["dock"]:
                return No(Refusal.WrongState, "You are already docked.");
            case ["warp", var site] when new[] { "station", "belt-1", "belt-2" }.Contains(site) && !_docked:
                _state = ShipState.Aligning; _target = site;
                return Ok($"Aligning to {site} (offline: nothing moves).");
            case ["approach" or "mine", var id] when !_docked:
                if (_bodies.All(b => b.Id != id)) return No(Refusal.NotFound, $"Nothing called {id} on this grid.");
                _state = words[0] == "mine" ? ShipState.Mining : ShipState.Approaching; _target = id;
                return Ok($"{(words[0] == "mine" ? "Mining" : "Approaching")} {id}.");
            case ["stop"]:
                _state = _docked ? ShipState.Docked : ShipState.Idle; _target = "";
                return Ok("Stopped.");
            case ["sell", "ferrite", _] when _docked:
                _credits += 4_500;
                return Ok("Sold 500 m³ of Ferrite for 4500 cr.");
            case ["buy", "ship", "sparrow"]:
                return No(Refusal.AlreadyOwned, "You already own a ship.");
            case ["buy", "module", _]:
                return No(Refusal.MissingSkill, "Mining Laser II requires Mining V, Laser Tuning V.");
            case []:
                return No(Refusal.UnknownCommand, "Enter a command.");
            default:
                return No(Refusal.UnknownCommand, $"Unknown command (offline mode): {string.Join(' ', words)}");
        }
    }

    Snapshot Build()
    {
        var s = new Snapshot
        {
            Tick = _tick, TickMs = 1000, Character = "offline", Credits = _credits,
            System = "Kesh", Security = 1.0, TutorialStep = TutorialStep.None,
            Ship = new OwnShip
            {
                TypeId = "sparrow", State = _state, Grid = _docked ? "" : "station", Target = _target,
                HoldUsedM3 = 175, HoldCapacityM3 = 500, Laser = "mining-laser-1", LaserYieldM3 = 25, LaserRangeKm = 10,
                TimerTicks = _state == ShipState.Mining ? (uint)(30 - _tick % 30) : 0,
            },
        };
        s.Ship.Hold.Add(new HoldItem { Ore = "ferrite", VolumeM3 = 175 });
        s.Sites.Add(new Site { Id = "station", Name = "Kesh Station" });
        s.Sites.Add(new Site { Id = "belt-1", Name = "Kesh Belt 1" });
        s.Sites.Add(new Site { Id = "belt-2", Name = "Kesh Belt 2" });
        if (_docked)
        {
            s.Station = new StationView { Id = "station", Name = "Kesh Station" };
            s.Station.Services.AddRange(["shipyard", "market", "hangar", "undock"]);
            s.Station.BuyOrders.Add(new BuyOrder { Ore = "ferrite", PricePerM3 = 9, VolumeLeftM3 = 4_812 });
            s.Station.Shipyard.Add(new ShipyardItem
            {
                Command = "buy module mining-laser-2", Name = "Mining Laser II", Price = 1_000_000,
                Summary = "35 m³ per 30 s cycle. Requires Laser Tuning V, Mining V",
            });
        }
        else
        {
            foreach (var b in _bodies)
            {
                s.Contacts.Add(new Contact
                {
                    Id = b.Id, Kind = b.Kind, Name = b.Name, XKm = b.X, YKm = b.Y,
                    DistanceKm = Math.Sqrt(b.X * b.X + b.Y * b.Y), Activity = b.Activity,
                    Ore = b.Kind is ContactKind.Asteroid or ContactKind.RichVein ? "ferrite" : "",
                    VolumeLeftM3 = b.Volume,
                });
            }
        }
        s.Events.AddRange(_events);
        _events.Clear();
        return s;
    }

    public ValueTask DisposeAsync()
    {
        _timer?.Dispose();
        Closed?.Invoke("Offline session closed.");
        return ValueTask.CompletedTask;
    }
}
