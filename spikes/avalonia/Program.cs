using Avalonia;

namespace SpaceMmo.Spike;

public static class Options
{
    public static bool StartUndocked { get; set; }
}

public static class Program
{
    [STAThread]
    public static int Main(string[] args)
    {
        if (args is ["--screenshots", var folder])
            return Screenshots.Run(folder);
        Options.StartUndocked = args.Contains("--undocked");
        BuildAvaloniaApp().StartWithClassicDesktopLifetime(args);
        return 0;
    }

    public static AppBuilder BuildAvaloniaApp() =>
        AppBuilder.Configure<App>().UsePlatformDetect().LogToTrace();
}
