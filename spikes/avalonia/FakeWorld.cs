namespace SpaceMmo.Spike;

public enum ContactKind { Station, Gate, Asteroid, PlayerShip, NpcShip, Anomaly }

public sealed class Contact
{
    public required string Id { get; init; }
    public required string Name { get; init; }
    public required ContactKind Kind { get; init; }
    public double X { get; set; }
    public double Y { get; set; }
    public double Vx { get; set; }
    public double Vy { get; set; }
    public string Activity { get; set; } = "";
    public double Distance => Math.Sqrt(X * X + Y * Y);
}

/// <summary>
/// Stand-in for the server, per the client test plan: no networking, just
/// plausible data. 200 contacts, 20 of them ships that move every second.
/// </summary>
public sealed class FakeWorld
{
    readonly Random _rng = new(42);

    public List<Contact> Contacts { get; } = [];
    public bool Docked { get; private set; } = true;
    public long Credits { get; private set; } = 4_000;
    public string ShipState { get; private set; } = "Docked";
    public string SystemName => "Kesh";
    public double Security => 1.0;
    public string[] Sites { get; } = ["station", "belt-1", "belt-2"];
    public List<string> Events { get; } = [];

    public FakeWorld()
    {
        Contacts.Add(new Contact { Id = "station", Name = "Kesh Station", Kind = ContactKind.Station, X = -42, Y = 31 });
        Contacts.Add(new Contact { Id = "gate-vael", Name = "Gate → Vael", Kind = ContactKind.Gate, X = 71, Y = 68 });
        Contacts.Add(new Contact { Id = "rock-vein", Name = "Rich Ferrite vein", Kind = ContactKind.Anomaly, X = 18, Y = -36 });
        for (var i = 1; i <= 158; i++)
        {
            var (cx, cy) = (i % 3) switch { 0 => (30.0, -20.0), 1 => (-60.0, -55.0), _ => (140.0, 90.0) };
            Contacts.Add(new Contact
            {
                Id = $"rock-{i}",
                Name = $"Ferrite asteroid {i}",
                Kind = ContactKind.Asteroid,
                X = cx + _rng.NextDouble() * 30 - 15,
                Y = cy + _rng.NextDouble() * 30 - 15,
            });
        }
        string[] npcNames = ["Hauler Oskin", "Prospector Mel", "Prospector Dray", "Kesh Patrol", "Ilsa Varn", "Tomas Reik", "Juno", "Sable", "Orrin", "Vex"];
        for (var i = 1; i <= 20; i++)
        {
            var npc = i % 2 == 0;
            Contacts.Add(new Contact
            {
                Id = $"ship-{i}",
                Name = npc ? npcNames[i / 2 - 1] : $"Pilot {i:00}",
                Kind = npc ? ContactKind.NpcShip : ContactKind.PlayerShip,
                X = _rng.NextDouble() * 160 - 80,
                Y = _rng.NextDouble() * 160 - 80,
                Vx = _rng.NextDouble() * 4 - 2,
                Vy = _rng.NextDouble() * 4 - 2,
                Activity = i % 3 == 0 ? "mining" : "moving",
            });
        }
        for (var i = 1; i <= 19; i++)
        {
            Contacts.Add(new Contact
            {
                Id = $"wreck-{i}",
                Name = $"Distant signal {i}",
                Kind = ContactKind.Anomaly,
                X = _rng.NextDouble() * 600 - 300,
                Y = 150 + _rng.NextDouble() * 150,
            });
        }
    }

    /// <summary>One server tick: the 20 ships move and bounce inside 95 km.</summary>
    public void Tick()
    {
        foreach (var c in Contacts.Where(c => c.Vx != 0 || c.Vy != 0))
        {
            c.X += c.Vx;
            c.Y += c.Vy;
            if (Math.Abs(c.X) > 95) c.Vx = -c.Vx;
            if (Math.Abs(c.Y) > 95) c.Vy = -c.Vy;
        }
    }

    /// <summary>Fake answers to the real command verbs.</summary>
    public string Execute(string verb, string[] args)
    {
        string Log(string message) { Events.Add(message); return message; }
        switch (verb)
        {
            case "undock" when Docked:
                Docked = false; ShipState = "Idle";
                return Log("Undocked onto the Kesh Station grid.");
            case "undock":
                return Log("Refused: you are not docked.");
            case "dock" when !Docked:
                Docked = true; ShipState = "Docked";
                return Log("Docked at Kesh Station.");
            case "dock":
                return Log("Refused: you are already docked.");
            case "warp" when args.Length == 1 && Sites.Contains(args[0]):
                ShipState = "Aligning";
                return Log($"Aligning to {args[0]} (fake: nothing moves).");
            case "approach" or "mine" when args.Length == 1:
                var target = Contacts.FirstOrDefault(c => c.Id == args[0]);
                if (target is null) return Log($"Refused: nothing called {args[0]} on this grid.");
                ShipState = verb == "mine" ? "Mining" : "Approaching";
                return Log($"{(verb == "mine" ? "Mining" : "Approaching")} {target.Name}.");
            case "stop":
                ShipState = "Idle";
                return Log("Stopped.");
            case "sell" when args is ["ferrite", _]:
                Credits += 4_500;
                return Log("Sold 500 m³ of Ferrite for 4500 cr.");
            case "buy" when args is ["ship", "sparrow"]:
                return Log("Refused: you already own a ship.");
            default:
                return Log($"Unknown or malformed command: {verb} {string.Join(' ', args)}".TrimEnd());
        }
    }
}
