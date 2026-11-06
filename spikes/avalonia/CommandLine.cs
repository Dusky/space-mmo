namespace SpaceMmo.Spike;

/// <summary>Up and Down walk previous commands, like a shell.</summary>
public sealed class CommandHistory
{
    readonly List<string> _lines = [];
    int _cursor;
    string _draft = "";

    public void Add(string line)
    {
        if (line.Length > 0 && (_lines.Count == 0 || _lines[^1] != line))
            _lines.Add(line);
        _cursor = _lines.Count;
        _draft = "";
    }

    public string Previous(string current)
    {
        if (_cursor == _lines.Count) _draft = current;
        if (_cursor > 0) _cursor--;
        return _lines.Count == 0 ? current : _lines[_cursor];
    }

    public string Next()
    {
        if (_cursor < _lines.Count) _cursor++;
        return _cursor == _lines.Count ? _draft : _lines[_cursor];
    }
}

/// <summary>
/// Tab completion over the command verbs and their arguments. Completes the
/// shared prefix; when several remain, returns them so the UI can list them.
/// </summary>
public static class Completer
{
    public static readonly string[] Verbs =
        ["approach", "buy", "dock", "glow", "help", "mine", "sell", "stop", "theme", "undock", "warp"];

    public static (string Text, IReadOnlyList<string> Options) Complete(
        string text, Func<string, IReadOnlyList<string>> argumentsFor)
    {
        var endsWithSpace = text.EndsWith(' ');
        var words = text.Split(' ', StringSplitOptions.RemoveEmptyEntries).ToList();
        if (endsWithSpace || words.Count == 0) words.Add("");

        var candidates = words.Count == 1 ? Verbs : argumentsFor(words[0].ToLowerInvariant());
        var partial = words[^1];
        var matches = candidates
            .Where(c => c.StartsWith(partial, StringComparison.OrdinalIgnoreCase))
            .ToList();
        if (matches.Count == 0) return (text, []);

        var prefix = matches.Aggregate(SharedPrefix);
        words[^1] = prefix;
        var completed = string.Join(' ', words);
        if (matches.Count == 1) return (completed + " ", []);
        return (completed, matches);
    }

    static string SharedPrefix(string a, string b)
    {
        var n = 0;
        while (n < a.Length && n < b.Length && char.ToLowerInvariant(a[n]) == char.ToLowerInvariant(b[n])) n++;
        return a[..n];
    }
}
