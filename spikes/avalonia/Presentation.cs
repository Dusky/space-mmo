using SpaceMmo.V1;

namespace SpaceMmo.Spike;

/// <summary>Words the client shows for server values. Display only, no rules.</summary>
public static class Presentation
{
    public static string Kind(ContactKind kind) => kind switch
    {
        ContactKind.Station => "Station",
        ContactKind.Gate => "Gate",
        ContactKind.Asteroid => "Asteroid",
        ContactKind.RichVein => "Anomaly",
        ContactKind.PlayerShip => "Pilot",
        ContactKind.NpcShip => "NPC",
        _ => "Unknown",
    };

    public static string State(ShipState state) => state switch
    {
        ShipState.None => "No ship",
        _ => state.ToString(),
    };

    public static string Distance(double km) => km < 10 ? $"{km:0.0} km" : $"{km:0} km";

    public static string Title(string id) => id.Length == 0 ? id : char.ToUpperInvariant(id[0]) + id[1..];

    /// <summary>
    /// Shown as a banner. Whether the tutorial ends up as an overlay or as Comms
    /// orders is an open design question; the server only reports the step.
    /// </summary>
    public static string? TutorialHint(TutorialStep step) => step switch
    {
        TutorialStep.BuyShip => "Open the Shipyard (F1) and buy the Sparrow, or type: buy ship sparrow",
        TutorialStep.Undock => "You have a ship. Undock (F4) to fly it.",
        TutorialStep.WarpToBelt => "Warp to the Training Belt: warp belt-1",
        TutorialStep.Approach => "Your laser reaches 10 km. Pick an asteroid and approach it.",
        TutorialStep.Mine => "Mine the asteroid. Watch for a rich vein: it pays triple but fades fast.",
        TutorialStep.WarpToStation => "Hold full. Warp back to the station: warp station",
        TutorialStep.Dock => "Dock at the station (within 2.5 km).",
        TutorialStep.Sell => "Open the Market (F2) and sell your ore.",
        TutorialStep.Exit => "Training complete. Undock, warp to the Exit Gate and jump: jump exit",
        _ => null,
    };
}
