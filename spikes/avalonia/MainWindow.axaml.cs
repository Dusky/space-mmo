using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.Layout;
using Avalonia.Threading;
using SpaceMmo.ClientCore;
using SpaceMmo.V1;

namespace SpaceMmo.Spike;

public sealed record OverviewRow(string Id, string Name, string Type, string Distance, double DistanceKm);
public sealed record LogLine(string Text, bool Warning);

/// <summary>
/// Draws whatever the latest snapshot says and sends every action as a command
/// line. It holds no game rules: the server decides and the window shows it.
/// </summary>
public partial class MainWindow : Window
{
    const int LogLines = 5;

    readonly IGameSession _session;
    readonly CommandHistory _history = new();
    readonly List<LogLine> _log = [];
    readonly DispatcherTimer _frameTimer = new() { Interval = TimeSpan.FromMilliseconds(1000.0 / 60) };
    readonly DispatcherTimer _fpsTimer = new() { Interval = TimeSpan.FromSeconds(1) };

    enum SortColumn { Name, Type, Distance }
    SortColumn _sort = SortColumn.Distance;
    bool _ascending = true;
    int _lastFrames;
    string? _selectedId;
    bool _wasDocked;
    string _service = "";
    string _stationKey = "", _targetKey = "", _shipKey = "";

    public Snapshot? Current { get; private set; }
    public RadarControl RadarView => Radar;

    public MainWindow() : this(new FakeSession(live: true), "offline: fake data") { }

    public MainWindow(IGameSession session, string connectionLabel)
    {
        InitializeComponent();
        _session = session;
        ConnectionText.Text = connectionLabel;
        _session.SnapshotReceived += s => Dispatcher.UIThread.Post(() => Apply(s));
        _session.Closed += reason => Dispatcher.UIThread.Post(() =>
        {
            ConnectionText.Text = "disconnected";
            Log(reason, true);
        });

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
            if (Overview.SelectedItem is OverviewRow row && row.Id != _selectedId)
                Select(row.Id);
        };

        // The radar only animates in space. Docked, nothing redraws, so the app idles.
        _frameTimer.Tick += (_, _) => Radar.InvalidateVisual();
        _fpsTimer.Tick += (_, _) =>
        {
            FpsText.Text = Radar.IsVisible ? $"{Radar.FramesDrawn - _lastFrames} fps" : "";
            _lastFrames = Radar.FramesDrawn;
        };
        _fpsTimer.Start();
        Opened += (_, _) => CommandBox.Focus();
        Closed += async (_, _) => await _session.DisposeAsync();
    }

    // ---- Commands ---------------------------------------------------------

    /// <summary>Runs one command line, as typed or sent by a button or hotkey.</summary>
    public async Task RunAsync(string line)
    {
        line = line.Trim();
        if (line.Length == 0) return;
        _history.Add(line);
        var words = line.Split(' ', StringSplitOptions.RemoveEmptyEntries);
        var verb = words[0].ToLowerInvariant();

        // Client-only commands: presentation, never game state.
        switch (verb)
        {
            case "theme" when words.Length == 2:
                Log(Themes.Apply(words[1].ToLowerInvariant()) ?? $"Theme: {Themes.CurrentName}", false);
                return;
            case "glow" when words.Length == 2 && Enum.TryParse<GlowMode>(words[1], true, out var glow):
                Radar.Glow = glow;
                Log($"Radar glow: {glow}", false);
                return;
            case "help":
                Log("Commands: " + string.Join(", ", Completer.Verbs), false);
                return;
        }

        if (verb is "approach" or "mine" && words.Length == 2)
            Select(words[1].ToLowerInvariant());
        try
        {
            var result = await _session.SendAsync(line);
            Log(result.Message, !result.Ok);
        }
        catch (Exception e) when (e is IOException or OperationCanceledException)
        {
            Log($"Not sent: {e.Message}", true);
        }
    }

    void Run(string line) => _ = RunAsync(line);

    IReadOnlyList<string> ArgumentsFor(string verb)
    {
        var s = Current;
        return verb switch
        {
            "warp" => s?.Sites.Select(x => x.Id).ToList() ?? [],
            "approach" => s?.Contacts.Where(c => c.Kind is not (ContactKind.PlayerShip or ContactKind.NpcShip)).Select(c => c.Id).ToList() ?? [],
            "mine" => s?.Contacts.Where(c => c.Kind is ContactKind.Asteroid or ContactKind.RichVein).Select(c => c.Id).ToList() ?? [],
            "sell" => s?.Ship?.Hold.Select(h => h.Ore).ToList() ?? [],
            "buy" => ["ship", "module"],
            "jump" => ["exit"],
            "theme" => Themes.Available(),
            "glow" => ["off", "fake", "blur"],
            _ => [],
        };
    }

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
            FilterBox.Focus();
        else if (e.Key == Key.Escape)
            CommandBox.Focus();
        else if (Current?.Station is { } station && e.Key is >= Key.F1 and <= Key.F12
                 && e.Key - Key.F1 < station.Services.Count)
            OpenService(station.Services[e.Key - Key.F1]);
        else
            return;
        e.Handled = true;
    }

    // ---- Snapshot to screen -----------------------------------------------

    void Apply(Snapshot s)
    {
        Current = s;
        foreach (var e in s.Events) Log(e, false);

        var ship = s.Ship;
        var docked = s.Station is not null;
        SystemText.Text = $"{s.System} · {s.Security:0.0}";
        var timer = ship is { TimerTicks: > 0 } ? $" · {Math.Ceiling(ship.TimerTicks * s.TickMs / 1000.0)} s" : "";
        StateText.Text = ship is null ? "No ship" : $"{Presentation.Title(ship.TypeId)} · {Presentation.State(ship.State)}{timer}";
        HoldText.Text = ship is null ? "" : $"{ship.HoldUsedM3}/{ship.HoldCapacityM3} m³";
        CreditsText.Text = $"{s.Credits:N0} cr";

        var hint = Presentation.TutorialHint(s.TutorialStep);
        TutorialBanner.IsVisible = hint is not null;
        TutorialText.Text = hint;

        StationView.IsVisible = docked;
        Radar.IsVisible = !docked;
        Radar.Snapshot = s;
        Radar.SelectedId = _selectedId;
        if (docked) _frameTimer.Stop(); else _frameTimer.Start();

        // Each time you dock, start on the service you most likely came for.
        if (docked && !_wasDocked) _service = "";
        _wasDocked = docked;
        if (docked) RefreshStation(s.Station!, s);
        RefreshOverview();
        RefreshTarget();
        RefreshShip(s);
    }

    void Log(string text, bool warning)
    {
        if (text.Length == 0) return;
        _log.Add(new LogLine(text, warning));
        if (_log.Count > 200) _log.RemoveRange(0, 100);
        EventLog.ItemsSource = _log.TakeLast(LogLines).ToList();
    }

    void OpenService(string service)
    {
        if (service == "undock")
        {
            Run("undock");
            return;
        }
        _service = service;
        _stationKey = "";
        if (Current?.Station is { } station) RefreshStation(station, Current);
    }

    void RefreshStation(StationView station, Snapshot s)
    {
        if (_service.Length == 0 || !station.Services.Contains(_service))
            _service = s.TutorialStep == TutorialStep.Sell || s.Ship?.HoldUsedM3 > 0 ? "market" : "shipyard";

        // Rebuild only when something changed, so buttons aren't recreated under the mouse.
        var key = $"{_service}|{station}|{s.Ship}|{s.Credits}";
        if (key == _stationKey) return;
        _stationKey = key;

        StationTitle.Text = station.Name.ToUpperInvariant();
        StationSubtitle.Text = $"{s.System} · security {s.Security:0.0} · " +
            (s.Ship is null ? "hangar empty" : $"active ship: {Presentation.Title(s.Ship.TypeId)}");

        Services.Children.Clear();
        for (var i = 0; i < station.Services.Count; i++)
        {
            var service = station.Services[i];
            var button = new Button { Content = $"[F{i + 1}] {Presentation.Title(service)}", Classes = { "service" }, Width = 170 };
            if (service == _service) button.Classes.Add("current");
            button.Click += (_, _) => OpenService(service);
            Services.Children.Add(button);
        }

        ServicePanel.Children.Clear();
        void Heading(string text) => ServicePanel.Children.Add(new TextBlock { Text = text, Classes = { "heading" } });
        void Line(string text, string cls = "dim") => ServicePanel.Children.Add(new TextBlock { Text = text, Classes = { cls }, TextWrapping = Avalonia.Media.TextWrapping.Wrap });
        void Action(string label, string command)
        {
            var button = new Button { Content = label, Classes = { "action" }, HorizontalAlignment = HorizontalAlignment.Left };
            button.Click += (_, _) => Run(command);
            ServicePanel.Children.Add(button);
        }

        switch (_service)
        {
            case "shipyard":
                Heading("SHIPYARD");
                if (station.Shipyard.Count == 0) Line("Nothing for sale here.");
                foreach (var item in station.Shipyard)
                {
                    Line($"{item.Name} · {item.Price:N0} cr", "normal");
                    Line(item.Summary);
                    Action($"Buy  ({item.Command})", item.Command);
                }
                break;
            case "market":
                Heading("MARKET");
                foreach (var order in station.BuyOrders)
                    Line($"NPC buy order · {Presentation.Title(order.Ore)} · {order.PricePerM3} cr/m³ · {order.VolumeLeftM3:N0} m³ wanted", "normal");
                var hold = s.Ship?.Hold ?? [];
                if (hold.Count == 0) Line("Your ore hold is empty.");
                foreach (var item in hold)
                {
                    var price = station.BuyOrders.FirstOrDefault(o => o.Ore == item.Ore)?.PricePerM3 ?? 0;
                    Action($"Sell all {Presentation.Title(item.Ore)}: {item.VolumeM3} m³ for about {item.VolumeM3 * price:N0} cr",
                        $"sell {item.Ore} all");
                }
                break;
            case "hangar":
                Heading("HANGAR");
                if (s.Ship is { } ship)
                {
                    Line($"{Presentation.Title(ship.TypeId)} (active) · {ship.HoldCapacityM3} m³ ore hold", "normal");
                    Line($"Fitted: {ship.Laser} · {ship.LaserYieldM3} m³ per cycle · {ship.LaserRangeKm:0} km range");
                    Line($"Hold: {ship.HoldUsedM3} m³ used");
                }
                else
                {
                    Line("No ship. Buy one at the Shipyard.");
                }
                break;
        }
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

    void RefreshOverview()
    {
        var s = Current;
        if (s is null || s.Station is not null)
        {
            Overview.ItemsSource = null;
            OverviewCount.Text = "Docked: nothing on radar";
            return;
        }
        if (s.Ship is { Grid.Length: 0 })
        {
            Overview.ItemsSource = null;
            OverviewCount.Text = "In warp: nothing on radar";
            return;
        }
        var filter = FilterBox.Text?.Trim() ?? "";
        bool Shown(Contact c) => c.Kind switch
        {
            ContactKind.PlayerShip or ContactKind.NpcShip => ShipsToggle.IsChecked == true,
            ContactKind.Asteroid or ContactKind.RichVein => RocksToggle.IsChecked == true,
            _ => OtherToggle.IsChecked == true,
        };
        var rows = s.Contacts
            .Where(Shown)
            .Select(c => new OverviewRow(
                c.Id,
                c.Kind switch
                {
                    ContactKind.Asteroid => $"{c.Id} · {c.VolumeLeftM3:N0} m³",
                    ContactKind.PlayerShip or ContactKind.NpcShip when c.Activity.Length > 0 => $"{c.Name} · {c.Activity}",
                    _ => c.Name,
                },
                Presentation.Kind(c.Kind),
                Presentation.Distance(c.DistanceKm),
                c.DistanceKm))
            .Where(r => filter.Length == 0 || r.Name.Contains(filter, StringComparison.OrdinalIgnoreCase));
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
        Overview.ItemsSource = list;
        Overview.SelectedItem = list.FirstOrDefault(r => r.Id == _selectedId);
        OverviewCount.Text = $"{list.Count} of {s.Contacts.Count} contacts";
    }

    void Select(string id)
    {
        _selectedId = id;
        Radar.SelectedId = id;
        Radar.InvalidateVisual();
        RefreshTarget();
    }

    void RefreshTarget()
    {
        var s = Current;
        var c = s?.Contacts.FirstOrDefault(x => x.Id == _selectedId);
        var key = c is null ? "" : $"{c.Id}|{c.Kind}|{s?.Ship?.State}";
        TargetText.Text = c is null
            ? "Select a contact in the overview."
            : $"{c.Name} · {Presentation.Kind(c.Kind)} · {Presentation.Distance(c.DistanceKm)}" +
              (c.VolumeLeftM3 > 0 ? $" · {c.VolumeLeftM3:N0} m³ {c.Ore}" : "") +
              (c.Activity.Length > 0 ? $" · {c.Activity}" : "");
        if (key == _targetKey) return;
        _targetKey = key;
        TargetActions.Children.Clear();
        if (c is null) return;
        if (c.Kind is not (ContactKind.PlayerShip or ContactKind.NpcShip))
            AddAction(TargetActions, "Approach", $"approach {c.Id}");
        if (c.Kind is ContactKind.Asteroid or ContactKind.RichVein)
            AddAction(TargetActions, "Mine", $"mine {c.Id}");
        if (c.Kind == ContactKind.Station)
            AddAction(TargetActions, "Dock", "dock");
        if (c.Kind == ContactKind.Gate)
            AddAction(TargetActions, "Jump", "jump exit");
    }

    void RefreshShip(Snapshot s)
    {
        var ship = s.Ship;
        HoldBar.Value = ship is null || ship.HoldCapacityM3 == 0 ? 0 : (double)ship.HoldUsedM3 / ship.HoldCapacityM3;
        HoldBarText.Text = ship is null ? "" : $"{ship.HoldUsedM3}/{ship.HoldCapacityM3} m³";
        if (ship is null)
        {
            ShipText.Text = "No ship. Buy one at the Shipyard.";
        }
        else
        {
            var timer = ship.TimerTicks > 0 ? $" · {Math.Ceiling(ship.TimerTicks * s.TickMs / 1000.0)} s" : "";
            var target = ship.Target.Length > 0 ? $" → {ship.Target}" : "";
            ShipText.Text = $"{Presentation.State(ship.State)}{target}{timer}\n" +
                            $"{ship.Laser} · {ship.LaserYieldM3} m³ / cycle · {ship.LaserRangeKm:0} km";
        }

        var key = $"{ship?.State}|{ship?.Grid}|{string.Join(',', s.Sites.Select(x => x.Id))}";
        if (key == _shipKey) return;
        _shipKey = key;
        ShipActions.Children.Clear();
        if (ship is null) return;
        if (ship.State == ShipState.Idle)
            foreach (var site in s.Sites.Where(x => x.Id != ship.Grid))
                AddAction(ShipActions, $"Warp: {site.Name}", $"warp {site.Id}");
        if (ship.State is ShipState.Mining or ShipState.Approaching)
            AddAction(ShipActions, "Stop", "stop");
    }

    void AddAction(Panel panel, string label, string command)
    {
        var button = new Button { Content = label, Classes = { "action" }, Margin = new Avalonia.Thickness(0, 0, 6, 6) };
        ToolTip.SetTip(button, command);
        button.Click += (_, _) => Run(command);
        panel.Children.Add(button);
    }
}
