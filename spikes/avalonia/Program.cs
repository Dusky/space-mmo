using Avalonia;
using SpaceMmo.ClientCore;

namespace SpaceMmo.Spike;

public static class Options
{
    /// <summary>A live server session made before the UI starts, or null for offline fake data.</summary>
    public static IGameSession? Session { get; set; }
    public static string ConnectionLabel { get; set; } = "";
}

public static class Program
{
    const string Usage = """
        Usage: SpaceMmoSpike [options]
          (no options)            Offline, with fake data
          --server HOST:PORT      Play on a real server
          --account NAME          Account to log in as (dev login)
          --ca FILE               CA certificate to trust (default dev-certs/ca.pem)
          --screenshots DIR       Render screenshots without a display; with --server,
                                  plays the tutorial through the UI and captures it
        """;

    [STAThread]
    public static int Main(string[] args)
    {
        string? server = null, account = null, screenshots = null;
        var ca = "dev-certs/ca.pem";
        for (var i = 0; i < args.Length; i++)
        {
            string Value() => i + 1 < args.Length ? args[++i] : throw new ArgumentException($"{args[i]} needs a value");
            switch (args[i])
            {
                case "--server": server = Value(); break;
                case "--account": account = Value(); break;
                case "--ca": ca = Value(); break;
                case "--screenshots": screenshots = Value(); break;
                case "--help" or "-h": Console.WriteLine(Usage); return 0;
                default: Console.Error.WriteLine($"Unknown option {args[i]}\n{Usage}"); return 2;
            }
        }

        ConnectOptions? connect = null;
        if (server is not null)
        {
            if (account is null)
            {
                Console.Error.WriteLine("--server needs --account");
                return 2;
            }
            var split = server.LastIndexOf(':');
            connect = new ConnectOptions(server[..split], int.Parse(server[(split + 1)..]), account, File.ReadAllText(ca));
        }

        if (screenshots is not null)
            return Screenshots.Run(screenshots, connect);

        if (connect is not null)
        {
            try
            {
                Options.Session = GameConnection.ConnectAsync(connect).GetAwaiter().GetResult();
                Options.ConnectionLabel = $"{connect.Account} @ {server}";
            }
            catch (Exception e)
            {
                Console.Error.WriteLine($"Could not connect to {server}: {e.Message}");
                return 1;
            }
        }
        BuildAvaloniaApp().StartWithClassicDesktopLifetime(args);
        return 0;
    }

    public static AppBuilder BuildAvaloniaApp() =>
        AppBuilder.Configure<App>().UsePlatformDetect().LogToTrace();
}
