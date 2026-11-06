using SpaceMmo.V1;

namespace SpaceMmo.ClientCore;

/// <summary>
/// What a client UI talks to: snapshots in, command lines out. The real server
/// connection and offline fakes both implement it, so the UI can't tell them
/// apart and never decides outcomes itself.
/// </summary>
public interface IGameSession : IAsyncDisposable
{
    /// <summary>Raised on a background thread; marshal to the UI thread yourself.</summary>
    event Action<Snapshot>? SnapshotReceived;

    /// <summary>Raised once, on a background thread, when the session ends.</summary>
    event Action<string>? Closed;

    /// <summary>Sends one verb line, such as "warp belt-1", and returns the server's answer.</summary>
    Task<CommandResult> SendAsync(string line, CancellationToken cancellationToken = default);
}
