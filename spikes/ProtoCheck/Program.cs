// Client test check: "Generated protobuf client compiles and connects".
// Connects to the real server over TLS, logs in, buys a ship and prints the
// snapshot. Engine-agnostic, so the Godot test can reuse it as is.
using System.Buffers.Binary;
using System.Net.Security;
using System.Net.Sockets;
using System.Security.Cryptography.X509Certificates;
using Google.Protobuf;
using SpaceMmo.V1;

var addr = args.Length > 0 ? args[0] : "127.0.0.1:7777";
var caPath = args.Length > 1 ? args[1] : "dev-certs/ca.pem";
var account = args.Length > 2 ? args[2] : "csharp-check";

var ca = X509Certificate2.CreateFromPem(File.ReadAllText(caPath));
var (host, port) = (addr[..addr.LastIndexOf(':')], int.Parse(addr[(addr.LastIndexOf(':') + 1)..]));

using var tcp = new TcpClient { NoDelay = true };
await tcp.ConnectAsync(host, port);
await using var tls = new SslStream(tcp.GetStream(), false, (_, cert, _, errors) =>
{
    // Trust only the server's own CA, and require the name to match.
    if (cert is null || errors.HasFlag(SslPolicyErrors.RemoteCertificateNameMismatch)) return false;
    using var chain = new X509Chain();
    chain.ChainPolicy.TrustMode = X509ChainTrustMode.CustomRootTrust;
    chain.ChainPolicy.CustomTrustStore.Add(ca);
    chain.ChainPolicy.RevocationMode = X509RevocationMode.NoCheck;
    if (chain.Build(X509CertificateLoader.LoadCertificate(cert.GetRawCertData()))) return true;
    Console.Error.WriteLine("certificate rejected: " +
        string.Join("; ", chain.ChainStatus.Select(s => $"{s.Status} {s.StatusInformation.Trim()}")));
    return false;
});
await tls.AuthenticateAsClientAsync("localhost");
Console.WriteLine($"TLS: {tls.SslProtocol}, {tls.NegotiatedCipherSuite}");

await Send(new ClientMessage { Login = new Login { DevAccount = account } });
var login = await Receive();
Console.WriteLine($"login: ok={login.LoginResult.Ok} {login.LoginResult.Message}");
if (!login.LoginResult.Ok) return 1;

var first = await NextSnapshot();
Console.WriteLine($"snapshot: {first.Character} in {first.System}, {first.Credits} cr, tutorial {first.TutorialStep}");

await Send(new ClientMessage { Command = new Command { Id = 1, Line = "buy ship sparrow" } });
while (true)
{
    var message = await Receive();
    if (message.BodyCase != ServerMessage.BodyOneofCase.CommandResult) continue;
    var result = message.CommandResult;
    Console.WriteLine($"buy ship sparrow: ok={result.Ok} refusal={result.Refusal} \"{result.Message}\"");
    break;
}
var after = await NextSnapshot();
Console.WriteLine($"snapshot: {after.Credits} cr, ship {after.Ship?.TypeId} {after.Ship?.State}, " +
                  $"shipyard offers: {string.Join(", ", after.Station.Shipyard.Select(s => s.Command))}");
return 0;

async Task Send(ClientMessage message)
{
    var body = message.ToByteArray();
    var header = new byte[4];
    BinaryPrimitives.WriteUInt32BigEndian(header, (uint)body.Length);
    await tls.WriteAsync(header);
    await tls.WriteAsync(body);
    await tls.FlushAsync();
}

async Task<ServerMessage> Receive()
{
    var header = new byte[4];
    await tls.ReadExactlyAsync(header);
    var length = BinaryPrimitives.ReadUInt32BigEndian(header);
    if (length > 256 * 1024) throw new InvalidDataException($"frame too large: {length}");
    var body = new byte[length];
    await tls.ReadExactlyAsync(body);
    return ServerMessage.Parser.ParseFrom(body);
}

async Task<Snapshot> NextSnapshot()
{
    while (true)
    {
        var message = await Receive();
        if (message.BodyCase == ServerMessage.BodyOneofCase.Snapshot) return message.Snapshot;
    }
}
