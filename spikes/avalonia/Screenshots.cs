using System.Diagnostics;
using Avalonia;
using Avalonia.Headless;
using Avalonia.Media.Imaging;
using Avalonia.Threading;
using SpaceMmo.ClientCore;
using SpaceMmo.V1;

namespace SpaceMmo.Spike;

/// <summary>
/// Renders the app without a display, through the real Skia renderer. Offline,
/// it captures the fake-data screens and times radar frames. With a server, it
/// plays the tutorial through the window's own command line and captures each
/// stage, which doubles as an end-to-end test of the client.
/// </summary>
public static class Screenshots
{
    public static int Run(string folder, ConnectOptions? connect)
    {
        Directory.CreateDirectory(folder);
        AppBuilder.Configure<App>()
            .UseSkia()
            .UseHeadless(new AvaloniaHeadlessPlatformOptions { UseHeadlessDrawing = false })
            .SetupWithoutStarting();
        try
        {
            return connect is null ? Offline(folder) : Live(folder, connect);
        }
        catch (Exception e)
        {
            Console.Error.WriteLine($"FAILED: {e.Message}");
            return 1;
        }
    }

    static int Offline(string folder)
    {
        var fake = new FakeSession(live: false);
        var window = new MainWindow(fake, "offline: fake data") { Width = 1440, Height = 900 };
        window.Show();
        fake.Tick();
        Pump();
        Save(window, folder, "offline-1-station");

        Do(window, "undock");
        for (var i = 0; i < 3; i++) fake.Tick();
        Do(window, "approach ship-4");
        Save(window, folder, "offline-2-radar");
        Do(window, "theme amber");
        Save(window, folder, "offline-3-amber");
        Do(window, "theme standard");

        foreach (var mode in new[] { "off", "fake", "blur" })
        {
            Do(window, $"glow {mode}");
            var watch = Stopwatch.StartNew();
            const int frames = 120;
            for (var i = 0; i < frames; i++)
            {
                window.RadarView.InvalidateVisual();
                Pump();
            }
            Console.WriteLine($"glow {mode}: {watch.Elapsed.TotalMilliseconds / frames:0.0} ms per frame (software, 1440x900)");
        }
        return 0;
    }

    /// <summary>The tutorial, played only through the UI's command line.</summary>
    static int Live(string folder, ConnectOptions connect)
    {
        var session = GameConnection.ConnectAsync(connect).GetAwaiter().GetResult();
        var window = new MainWindow(session, $"{connect.Account} (headless)") { Width = 1440, Height = 900 };
        window.Show();
        WaitFor(window, "first snapshot", s => true);
        Save(window, folder, "live-1-new-player");

        Do(window, "buy ship sparrow");
        WaitFor(window, "the ship", s => s.Ship is not null);
        Do(window, "undock");
        WaitFor(window, "undock", s => s.Ship?.State == ShipState.Idle);
        Do(window, "warp belt-1");
        WaitFor(window, "arrival at the belt", s => s.Ship?.State == ShipState.Idle && s.Ship.Grid == "belt-1");
        Save(window, folder, "live-2-belt");

        // Mine to a full hold, switching to the rich vein when it surfaces.
        var shotVein = false;
        while (Snap(window).Ship!.HoldUsedM3 < Snap(window).Ship!.HoldCapacityM3)
        {
            var s = Snap(window);
            var vein = s.Contacts.FirstOrDefault(c => c.Kind == ContactKind.RichVein);
            var rock = vein ?? s.Contacts.Where(c => c.Kind == ContactKind.Asteroid && c.VolumeLeftM3 > 0).MinBy(c => c.DistanceKm)!;
            if (rock.DistanceKm > s.Ship!.LaserRangeKm)
            {
                Do(window, $"approach {rock.Id}");
                WaitFor(window, "approach", x => x.Ship?.State == ShipState.Idle);
                continue;
            }
            Do(window, $"mine {rock.Id}");
            var before = s.Ship.HoldUsedM3;
            WaitFor(window, "mining progress", x =>
                x.Ship!.State != ShipState.Mining
                || x.Ship.HoldUsedM3 >= x.Ship.HoldCapacityM3
                || (vein is null && x.Contacts.Any(c => c.Kind == ContactKind.RichVein))
                || (vein is not null && x.Ship.HoldUsedM3 > before));
            if (!shotVein && Snap(window).Contacts.Any(c => c.Kind == ContactKind.RichVein))
            {
                window.RadarView.SelectedId = Snap(window).Contacts.First(c => c.Kind == ContactKind.RichVein).Id;
                Save(window, folder, "live-3-rich-vein");
                shotVein = true;
            }
            if (vein is not null && Snap(window).Ship!.State == ShipState.Mining) Do(window, "stop");
        }
        Do(window, "warp station");
        WaitFor(window, "arrival at the station", s => s.Ship?.State == ShipState.Idle && s.Ship.Grid == "station");
        Do(window, "dock");
        WaitFor(window, "docking", s => s.Station is not null);
        Save(window, folder, "live-4-market");
        Do(window, "sell ferrite all");
        WaitFor(window, "the sale", s => s.Ship!.HoldUsedM3 == 0);

        Do(window, "undock");
        WaitFor(window, "undock", s => s.Ship?.State == ShipState.Idle);
        Do(window, "warp exit-gate");
        WaitFor(window, "arrival at the gate", s => s.Ship?.State == ShipState.Idle && s.Ship.Grid == "exit-gate");
        Do(window, "jump exit");
        WaitFor(window, "Kesh", s => s.System == "Kesh");
        WaitFor(window, "a moment in Kesh", s => true);
        Save(window, folder, "live-5-kesh");
        Console.WriteLine($"tutorial complete through the UI: {Snap(window).Credits} cr, in {Snap(window).System}");
        window.Close();
        return 0;
    }

    static Snapshot Snap(MainWindow window) => window.Current ?? throw new InvalidOperationException("no snapshot yet");

    static void Do(MainWindow window, string line)
    {
        var task = window.RunAsync(line);
        while (!task.IsCompleted)
        {
            Pump();
            Thread.Sleep(2);
        }
        task.GetAwaiter().GetResult();
        Pump();
    }

    /// <summary>Pumps the UI until a newer snapshot satisfies the condition.</summary>
    static void WaitFor(MainWindow window, string what, Func<Snapshot, bool> done)
    {
        var start = window.Current?.Tick ?? 0;
        var clock = Stopwatch.StartNew();
        while (window.Current is null || window.Current.Tick <= start || !done(window.Current))
        {
            if (clock.Elapsed > TimeSpan.FromSeconds(60))
                throw new TimeoutException($"timed out waiting for {what}");
            Pump();
            Thread.Sleep(2);
        }
    }

    static void Pump()
    {
        Dispatcher.UIThread.RunJobs();
        AvaloniaHeadlessPlatform.ForceRenderTimerTick();
        Dispatcher.UIThread.RunJobs();
    }

    static void Save(MainWindow window, string folder, string name)
    {
        Pump();
        var frame = window.CaptureRenderedFrame() ?? throw new InvalidOperationException("no frame rendered");
        var path = Path.Combine(folder, name + ".png");
        frame.Save(path, PngBitmapEncoderOptions.Default);
        Console.WriteLine($"saved {path}");
    }
}
