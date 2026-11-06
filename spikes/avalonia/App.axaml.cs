using Avalonia;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Markup.Xaml;

namespace SpaceMmo.Spike;

public partial class App : Application
{
    public override void Initialize()
    {
        AvaloniaXamlLoader.Load(this);
        if (Themes.Apply("standard") is { } error)
            throw new InvalidOperationException(error);
    }

    public override void OnFrameworkInitializationCompleted()
    {
        if (ApplicationLifetime is IClassicDesktopStyleApplicationLifetime desktop)
        {
            desktop.MainWindow = Options.Session is { } session
                ? new MainWindow(session, Options.ConnectionLabel)
                : new MainWindow();
        }
        base.OnFrameworkInitializationCompleted();
    }
}
