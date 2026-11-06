using System.Buffers.Binary;
using System.Collections.Concurrent;
using System.Net.Security;
using System.Net.Sockets;
using System.Security.Cryptography.X509Certificates;
using Google.Protobuf;
using SpaceMmo.V1;

namespace SpaceMmo.ClientCore;

public sealed record ConnectOptions(
    string Host,
    int Port,
    string Account,
    /// <summary>The CA to trust, as PEM. Only certificates it issued are accepted.</summary>
    string CaPem,
    /// <summary>The name the server certificate must carry.</summary>
    string ServerName = "localhost");

public sealed class LoginRefusedException(string message) : Exception(message);

/// <summary>
/// A session with the real server: TLS, 4-byte big-endian length-prefixed
/// protobuf frames, a dev login, then commands and snapshots.
/// </summary>
public sealed class GameConnection : IGameSession
{
    const int MaxFrameBytes = 256 * 1024;
    static readonly TimeSpan ConnectTimeout = TimeSpan.FromSeconds(10);

    readonly TcpClient _tcp;
    readonly SslStream _tls;
    readonly SemaphoreSlim _writeLock = new(1, 1);
    readonly ConcurrentDictionary<ulong, TaskCompletionSource<CommandResult>> _pending = new();
    readonly CancellationTokenSource _closing = new();
    long _nextId;
    int _closed;

    public event Action<Snapshot>? SnapshotReceived;
    public event Action<string>? Closed;

    GameConnection(TcpClient tcp, SslStream tls)
    {
        _tcp = tcp;
        _tls = tls;
    }

    public static async Task<GameConnection> ConnectAsync(ConnectOptions options, CancellationToken cancellationToken = default)
    {
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeout.CancelAfter(ConnectTimeout);
        var ca = X509Certificate2.CreateFromPem(options.CaPem);

        var tcp = new TcpClient { NoDelay = true };
        try
        {
            // Library code: never resume on the caller's UI context, so a UI that
            // blocks on this call can't deadlock.
            await tcp.ConnectAsync(options.Host, options.Port, timeout.Token).ConfigureAwait(false);
            var tls = new SslStream(tcp.GetStream(), false, (_, cert, _, errors) => Validate(ca, cert, errors));
            await tls.AuthenticateAsClientAsync(
                new SslClientAuthenticationOptions { TargetHost = options.ServerName }, timeout.Token).ConfigureAwait(false);

            var connection = new GameConnection(tcp, tls);
            await connection.WriteAsync(new ClientMessage { Login = new Login { DevAccount = options.Account } }, timeout.Token).ConfigureAwait(false);
            var reply = await connection.ReadAsync(timeout.Token).ConfigureAwait(false);
            if (reply.BodyCase != ServerMessage.BodyOneofCase.LoginResult)
                throw new InvalidDataException("The server didn't answer the login.");
            if (!reply.LoginResult.Ok)
                throw new LoginRefusedException(reply.LoginResult.Message);
            connection.StartReading();
            return connection;
        }
        catch
        {
            tcp.Dispose();
            throw;
        }
    }

    /// <summary>Trust only certificates issued by our CA, with a matching name.</summary>
    static bool Validate(X509Certificate2 ca, X509Certificate? cert, SslPolicyErrors errors)
    {
        if (cert is null || errors.HasFlag(SslPolicyErrors.RemoteCertificateNameMismatch))
            return false;
        using var chain = new X509Chain();
        chain.ChainPolicy.TrustMode = X509ChainTrustMode.CustomRootTrust;
        chain.ChainPolicy.CustomTrustStore.Add(ca);
        chain.ChainPolicy.RevocationMode = X509RevocationMode.NoCheck;
        return chain.Build(X509CertificateLoader.LoadCertificate(cert.GetRawCertData()));
    }

    public async Task<CommandResult> SendAsync(string line, CancellationToken cancellationToken = default)
    {
        var id = (ulong)Interlocked.Increment(ref _nextId);
        var answer = new TaskCompletionSource<CommandResult>(TaskCreationOptions.RunContinuationsAsynchronously);
        _pending[id] = answer;
        try
        {
            await WriteAsync(new ClientMessage { Command = new Command { Id = id, Line = line } }, cancellationToken).ConfigureAwait(false);
            return await answer.Task.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        finally
        {
            _pending.TryRemove(id, out _);
        }
    }

    void StartReading() => _ = Task.Run(async () =>
    {
        try
        {
            while (true)
            {
                var message = await ReadAsync(_closing.Token).ConfigureAwait(false);
                switch (message.BodyCase)
                {
                    case ServerMessage.BodyOneofCase.Snapshot:
                        SnapshotReceived?.Invoke(message.Snapshot);
                        break;
                    case ServerMessage.BodyOneofCase.CommandResult:
                        if (_pending.TryGetValue(message.CommandResult.Id, out var answer))
                            answer.TrySetResult(message.CommandResult);
                        break;
                }
            }
        }
        catch (Exception e) when (e is IOException or OperationCanceledException or ObjectDisposedException or InvalidDataException or InvalidProtocolBufferException or EndOfStreamException)
        {
            Close(_closing.IsCancellationRequested ? "Disconnected." : $"Connection lost: {e.Message}");
        }
    });

    async Task WriteAsync(ClientMessage message, CancellationToken cancellationToken)
    {
        var body = message.ToByteArray();
        var frame = new byte[4 + body.Length];
        BinaryPrimitives.WriteUInt32BigEndian(frame, (uint)body.Length);
        body.CopyTo(frame, 4);
        await _writeLock.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            await _tls.WriteAsync(frame, cancellationToken).ConfigureAwait(false);
            await _tls.FlushAsync(cancellationToken).ConfigureAwait(false);
        }
        finally
        {
            _writeLock.Release();
        }
    }

    async Task<ServerMessage> ReadAsync(CancellationToken cancellationToken)
    {
        var header = new byte[4];
        await _tls.ReadExactlyAsync(header, cancellationToken).ConfigureAwait(false);
        var length = BinaryPrimitives.ReadUInt32BigEndian(header);
        if (length > MaxFrameBytes)
            throw new InvalidDataException($"Frame too large: {length} bytes.");
        var body = new byte[length];
        await _tls.ReadExactlyAsync(body, cancellationToken).ConfigureAwait(false);
        return ServerMessage.Parser.ParseFrom(body);
    }

    void Close(string reason)
    {
        if (Interlocked.Exchange(ref _closed, 1) == 1) return;
        foreach (var answer in _pending.Values)
            answer.TrySetException(new IOException(reason));
        Closed?.Invoke(reason);
    }

    public async ValueTask DisposeAsync()
    {
        await _closing.CancelAsync().ConfigureAwait(false);
        Close("Disconnected.");
        await _tls.DisposeAsync().ConfigureAwait(false);
        _tcp.Dispose();
    }
}
