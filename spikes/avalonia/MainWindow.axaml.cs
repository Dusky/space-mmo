using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.Threading;

namespace SpaceMmo.Spike;

public sealed record OverviewRow(string Id, string Name, string Type, string Distance, double DistanceKm);

public partial class MainWindow : Window
{
    readonly FakeWorld _world = new();
    readonly CommandHistory _history = new();
    readonly DispatcherTimer _worldTimer = new() { Interval = TimeSpan.FromSeconds(1) };
    readonly DispatcherTimer _frameTimer = new() { Interval = TimeSpan.FromMilliseconds(1000.0 / 60) };

    enum SortColumn { Name, Type, Distance }
    SortColumn _sort = SortColumn.Distance;
    bool _ascending = true;
    int _lastFrames;

    static readonly (string Key, Key Hotkey, string Name, string Detail)[] ServiceList =
    [
        ("F1", Key.F1, "Shipyard", "SHIPYARD\nSparrow mining frigate · 6,000 cr · 500 m³ hold, fitted with Mining Laser I\n> buy ship sparrow"),
        ("F2", Key.F2, "Market", "MARKET\nFerrite · NPC buy order · 9 cr/m³ · 4,812 m³ wanted\n> sell ferrite all"),
        ("F3", Key.F3, "Hangar", "HANGAR\nSparrow (active) · Mining Laser I · hold empty"),
        ("F4", Key.F4, "Undock", "Undocking puts you on the station grid.\n> undock"),
    ];

    public MainWindow()
    {
        InitializeComponent();
        Radar.World = _world;

        foreach (var (key, hotkey, name, _) in ServiceList)
        {
            var button = new Button { Content = $"[{key}]  {name}", Classes = { "service" } };
            button.Click += (_, _) => OpenService(hotkey);
            Services.Children.Add(button);
        }

        CommandBox.AddHandler(KeyDownEvent, OnCommandKey, RoutingStrategies.Tunnel);
        KeyDown += OnWindowKey;
        FilterBox.TextChanged += (_, _) => RefreshOverview();
        foreach (var toggle in new[] { ShipsToggle, RocksToggle, OtherToggle })
            toggle.IsCheckedChanged += (_, _) => RefreshOverview();
        SortName.Click += (_, _) => SortBy(SortColumn.Name);
        SortType.Click += (_, _) => SortBy(SortColumn.Type);
        SortDistance.Click += (_, _) => SortBy(SortColumn.Distance);
        Overview.SelectionChanged += (_, _) =>
        {
            Radar.SelectedId = (Overview.SelectedItem as OverviewRow)?.Id;
            Radar.InvalidateVisual();
        };

        _worldTimer.Tick += (_, _) => WorldTick();
        // Only runs in space. Docked, nothing animates, so the app should idle.
        _frameTimer.Tick += (_, _) => Radar.InvalidateVisual();
        _worldTimer.Start();

        Opened += (_, _) => CommandBox.Focus();
        if (Options.StartUndocked) Run("undock");
        RefreshAll();
    }

    public FakeWorld World => _world;

    void WorldTick()
    {
        _world.Tick();
        if (!_world.Docked) RefreshOverview();
        FpsText.Text = _world.Docked ? "idle" : $"{Radar.FramesDrawn - _lastFrames} fps";
        _lastFrames = Radar.FramesDrawn;
    }

    /// <summary>Runs one command line, as typed or sent by a hotkey.</summary>
    public void Run(string line)
    {
        line = line.Trim();
        if (line.Length == 0) return;
        _history.Add(line);
        var words = line.Split(' ', StringSplitOptions.RemoveEmptyEntries);
        var verb = words[0].ToLowerInvariant();
        var args = words[1..].Select(w => w.ToLowerInvariant()).ToArray();
        switch (verb)
        {
            case "theme" when args.Length == 1:
                _world.Events.Add(Themes.Apply(args[0]) ?? $"Theme: {Themes.CurrentName}");
                break;
            case "glow" when args.Length == 1 && Enum.TryParse<GlowMode>(args[0], true, out var glow):
                Radar.Glow = glow;
                _world.Events.Add($"Radar glow: {glow}");
                break;
            case "help":
                _world.Events.Add("Commands: " + string.Join(", ", Completer.Verbs));
                break;
            default:
                _world.Execute(verb, args);
                // Commands that target a contact select it on radar and overview.
                if (verb is "approach" or "mine" && args.Length == 1 && _world.Contacts.Any(c => c.Id == args[0]))
                    Radar.SelectedId = args[0];
                break;
        }
        RefreshAll();
    }

    IReadOnlyList<string> ArgumentsFor(string verb) => verb switch
    {
        "warp" => _world.Sites,
        "approach" or "mine" => _world.Contacts.Select(c => c.Id).ToList(),
        "theme" => Themes.Available(),
        "glow" => ["off", "fake", "blur"],
        "buy" => ["ship"],
        "sell" => ["ferrite"],
        _ => [],
    };

    void OnCommandKey(object? sender, KeyEventArgs e)
    {
        switch (e.Key)
        {
            case Key.Enter:
                Run(CommandBox.Text ?? "");
                CommandBox.Text = "";
                break;
            case Key.Up:
                SetCommandText(_history.Previous(CommandBox.Text ?? ""));
                break;
            case Key.Down:
                SetCommandText(_history.Next());
                break;
            case Key.Tab:
                var (text, options) = Completer.Complete(CommandBox.Text ?? "", ArgumentsFor);
                SetCommandText(text);
                CompletionText.Text = options.Count > 0
                    ? string.Join("  ", options.Take(12)) + (options.Count > 12 ? $"  … {options.Count - 12} more" : "")
                    : "Tab completes · ↑ history · same commands as the automation API";
                break;
            default:
                return;
        }
        e.Handled = true;
    }

    void SetCommandText(string text)
    {
        CommandBox.Text = text;
        CommandBox.CaretIndex = text.Length;
    }

    void OnWindowKey(object? sender, KeyEventArgs e)
    {
        if (e.Key == Key.F && e.KeyModifiers.HasFlag(KeyModifiers.Control))
        {
            FilterBox.Focus();
            e.Handled = true;
        }
        else if (e.Key == Key.Escape)
        {
            CommandBox.Focus();
            e.Handled = true;
        }
        else if (_world.Docked && ServiceList.Any(s => s.Hotkey == e.Key))
        {
            OpenService(e.Key);
            e.Handled = true;
        }
    }

    void OpenService(Key hotkey)
    {
        var service = ServiceList.First(s => s.Hotkey == hotkey);
        if (hotkey == Key.F4)
        {
            Run("undock");
            return;
        }
        ServiceDetail.Text = service.Detail;
    }

    void SortBy(SortColumn column)
    {
        _ascending = _sort != column || !_ascending;
        _sort = column;
        var arrow = _ascending ? " ▲" : " ▼";
        SortName.Content = "CONTACT" + (column == SortColumn.Name ? arrow : "");
        SortType.Content = "TYPE" + (column == SortColumn.Type ? arrow : "");
        SortDistance.Content = "DIST" + (column == SortColumn.Distance ? arrow : "");
        RefreshOverview();
    }

    void RefreshAll()
    {
        var docked = _world.Docked;
        StationView.IsVisible = docked;
        Radar.IsVisible = !docked;
        if (docked) _frameTimer.Stop(); else _frameTimer.Start();
        SystemText.Text = $"{_world.SystemName} · {_world.Security:0.0}";
        StateText.Text = $"Sparrow · {_world.ShipState}";
        CreditsText.Text = $"{_world.Credits:N0} cr";
        EventLog.ItemsSource = _world.Events.TakeLast(5).ToList();
        RefreshOverview();
    }

    void RefreshOverview()
    {
        if (_world.Docked)
        {
            Overview.ItemsSource = null;
            OverviewCount.Text = "Docked: nothing on radar";
            return;
        }
        var filter = FilterBox.Text?.Trim() ?? "";
        bool Shown(Contact c) => c.Kind switch
        {
            ContactKind.PlayerShip or ContactKind.NpcShip => ShipsToggle.IsChecked == true,
            ContactKind.Asteroid => RocksToggle.IsChecked == true,
            _ => OtherToggle.IsChecked == true,
        };
        var rows = _world.Contacts
            .Where(Shown)
            .Where(c => filter.Length == 0 || c.Name.Contains(filter, StringComparison.OrdinalIgnoreCase))
            .Select(c => new OverviewRow(c.Id, c.Name, TypeName(c.Kind), $"{c.Distance:0} km", c.Distance));
        rows = (_sort, _ascending) switch
        {
            (SortColumn.Name, true) => rows.OrderBy(r => r.Name),
            (SortColumn.Name, false) => rows.OrderByDescending(r => r.Name),
            (SortColumn.Type, true) => rows.OrderBy(r => r.Type).ThenBy(r => r.DistanceKm),
            (SortColumn.Type, false) => rows.OrderByDescending(r => r.Type).ThenBy(r => r.DistanceKm),
            (_, true) => rows.OrderBy(r => r.DistanceKm),
            (_, false) => rows.OrderByDescending(r => r.DistanceKm),
        };
        var list = rows.ToList();
        var selectedId = Radar.SelectedId;
        Overview.ItemsSource = list;
        Overview.SelectedItem = list.FirstOrDefault(r => r.Id == selectedId);
        OverviewCount.Text = $"{list.Count} of {_world.Contacts.Count} contacts";
    }

    static string TypeName(ContactKind kind) => kind switch
    {
        ContactKind.Station => "Station",
        ContactKind.Gate => "Gate",
        ContactKind.Asteroid => "Asteroid",
        ContactKind.PlayerShip => "Pilot",
        ContactKind.NpcShip => "NPC",
        _ => "Anomaly",
    };
}
