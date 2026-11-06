// Client test check: "Generated protobuf client compiles and connects".
// Logs into the real server through client-core, buys a ship and prints the
// result. Engine-agnostic, so it applies to both the Avalonia and Godot tests.
using SpaceMmo.ClientCore;
using SpaceMmo.V1;

var addr = args.Length > 0 ? args[0] : "127.0.0.1:7777";
var caPath = args.Length > 1 ? args[1] : "dev-certs/ca.pem";
var account = args.Length > 2 ? args[2] : "csharp-check";
var split = addr.LastIndexOf(':');

await using var session = await GameConnection.ConnectAsync(
    new ConnectOptions(addr[..split], int.Parse(addr[(split + 1)..]), account, File.ReadAllText(caPath)));
var snapshots = new TaskCompletionSource<Snapshot>(TaskCreationOptions.RunContinuationsAsynchronously);
session.SnapshotReceived += s => snapshots.TrySetResult(s);
var first = await snapshots.Task;
Console.WriteLine($"logged in: {first.Character} in {first.System}, {first.Credits} cr, tutorial {first.TutorialStep}");

var result = await session.SendAsync("buy ship sparrow");
Console.WriteLine($"buy ship sparrow: ok={result.Ok} refusal={result.Refusal} \"{result.Message}\"");
var refused = await session.SendAsync("mine rock-1");
Console.WriteLine($"mine rock-1 while docked: ok={refused.Ok} refusal={refused.Refusal} \"{refused.Message}\"");
return result.Ok && !refused.Ok ? 0 : 1;
