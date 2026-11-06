using System.Diagnostics;
using Avalonia;
using Avalonia.Headless;
using Avalonia.Threading;

namespace SpaceMmo.Spike;

/// <summary>
/// Renders the app without a display, through the real Skia renderer, and
/// saves screenshots. Also times radar frames with each glow mode on the CPU.
/// </summary>
public static class Screenshots
{
    public static int Run(string folder)
    {
        Directory.CreateDirectory(folder);
        AppBuilder.Configure<App>()
            .UseSkia()
            .UseHeadless(new AvaloniaHeadlessPlatformOptions { UseHeadlessDrawing = false })
            .SetupWithoutStarting();

        var window = new MainWindow { Width = 1440, Height = 900 };
        window.Show();
        Pump();
        Save(window, folder, "1-station-standard");

        window.Run("undock");
        for (var i = 0; i < 3; i++) window.World.Tick();
        window.Run("approach ship-4");
        Pump();
        Save(window, folder, "2-radar-standard");

        window.Run("glow fake");
        Pump();
        Save(window, folder, "3-radar-fake-glow");

        window.Run("theme amber");
        window.Run("glow off");
        Pump();
        Save(window, folder, "4-radar-amber");

        foreach (var mode in new[] { "off", "fake", "blur" })
        {
            window.Run($"glow {mode}");
            Pump();
            var watch = Stopwatch.StartNew();
            const int frames = 120;
            for (var i = 0; i < frames; i++)
            {
                window.Radar.InvalidateVisual();
                Pump();
            }
            Console.WriteLine($"glow {mode}: {watch.Elapsed.TotalMilliseconds / frames:0.0} ms per frame (software, 1440x900)");
        }
        return 0;
    }

    static void Pump()
    {
        Dispatcher.UIThread.RunJobs();
        AvaloniaHeadlessPlatform.ForceRenderTimerTick();
        Dispatcher.UIThread.RunJobs();
    }

    static void Save(MainWindow window, string folder, string name)
    {
        var frame = window.CaptureRenderedFrame() ?? throw new InvalidOperationException("no frame rendered");
        var path = Path.Combine(folder, name + ".png");
        frame.Save(path, Avalonia.Media.Imaging.PngBitmapEncoderOptions.Default);
        Console.WriteLine($"saved {path}");
    }
}
