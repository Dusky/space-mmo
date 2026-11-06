using System.Text.Json;
using Avalonia;
using Avalonia.Media;

namespace SpaceMmo.Spike;

/// <summary>
/// Themes are JSON data files holding colour tokens, never code. Applying one
/// replaces the brushes that every view looks up with DynamicResource.
/// </summary>
public static class Themes
{
    public static readonly string[] Tokens =
    [
        "background", "panel", "line", "input", "text", "textDim", "textFaint",
        "accent", "warning", "friendly", "selection",
    ];

    public static string CurrentName { get; private set; } = "";
    public static IReadOnlyDictionary<string, Color> Colors { get; private set; } =
        new Dictionary<string, Color>();

    public static event Action? Changed;

    static string Folder => Path.Combine(AppContext.BaseDirectory, "themes");

    public static IReadOnlyList<string> Available() =>
        Directory.GetFiles(Folder, "*.json")
            .Select(Path.GetFileNameWithoutExtension)
            .OfType<string>()
            .Order()
            .ToList();

    /// <summary>Loads and applies a theme, or returns why it couldn't.</summary>
    public static string? Apply(string id)
    {
        if (!Available().Contains(id))
            return $"No theme called {id}. Available: {string.Join(", ", Available())}";

        ThemeFile? file;
        try
        {
            file = JsonSerializer.Deserialize<ThemeFile>(File.ReadAllText(Path.Combine(Folder, id + ".json")));
        }
        catch (JsonException e)
        {
            return $"Theme {id} is not valid JSON: {e.Message}";
        }
        if (file?.Colors is null)
            return $"Theme {id} has no colors.";

        var parsed = new Dictionary<string, Color>();
        foreach (var token in Tokens)
        {
            if (!file.Colors.TryGetValue(token, out var hex) || !Color.TryParse(hex, out var color))
                return $"Theme {id} is missing a valid colour for {token}.";
            parsed[token] = color;
        }
        var unknown = file.Colors.Keys.Except(Tokens).ToList();
        if (unknown.Count > 0)
            return $"Theme {id} has unknown tokens: {string.Join(", ", unknown)}";

        var resources = Application.Current!.Resources;
        foreach (var (token, color) in parsed)
            resources[BrushKey(token)] = new SolidColorBrush(color);
        Colors = parsed;
        CurrentName = file.Name ?? id;
        Changed?.Invoke();
        return null;
    }

    /// <summary>"textDim" becomes "TextDimBrush", the key used in XAML.</summary>
    public static string BrushKey(string token) => char.ToUpperInvariant(token[0]) + token[1..] + "Brush";

    sealed class ThemeFile
    {
        [System.Text.Json.Serialization.JsonPropertyName("name")]
        public string? Name { get; set; }

        [System.Text.Json.Serialization.JsonPropertyName("colors")]
        public Dictionary<string, string>? Colors { get; set; }
    }
}
