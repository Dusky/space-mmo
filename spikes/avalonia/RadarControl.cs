using System.Diagnostics;
using System.Globalization;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;

namespace SpaceMmo.Spike;

public enum GlowMode { Off, Fake, Blur }

/// <summary>
/// The radar from the mockup, drawn by hand: range rings, a rotating sweep and
/// contacts. Everything is vector, so it stays sharp at any size and DPI.
/// </summary>
public sealed class RadarControl : Control
{
    static readonly Stopwatch Clock = Stopwatch.StartNew();
    static readonly Typeface Mono = new("avares://SpaceMmoSpike/Assets/Fonts#IBM Plex Mono");

    public FakeWorld? World { get; set; }
    public string? SelectedId { get; set; }
    public double RangeKm { get; set; } = 100;
    public int FramesDrawn { get; private set; }

    GlowMode _glow;
    public GlowMode Glow
    {
        get => _glow;
        set
        {
            _glow = value;
            // The real glow: a GPU blur of everything this control draws.
            Effect = value == GlowMode.Blur
                ? new DropShadowEffect { BlurRadius = 10, OffsetX = 0, OffsetY = 0, Color = Color("accent"), Opacity = 0.9 }
                : null;
            InvalidateVisual();
        }
    }

    public RadarControl()
    {
        ClipToBounds = true;
        Themes.Changed += () => { Glow = _glow; InvalidateVisual(); };
    }

    static Color Color(string token) => Themes.Colors.TryGetValue(token, out var c) ? c : Colors.Magenta;

    static IBrush Brush(string token, double opacity = 1) =>
        new SolidColorBrush(Color(token), opacity);

    public override void Render(DrawingContext ctx)
    {
        FramesDrawn++;
        var bounds = Bounds;
        ctx.FillRectangle(Brush("background"), new Rect(bounds.Size));
        var center = new Point(bounds.Width / 2, bounds.Height / 2);
        var radius = Math.Max(10, Math.Min(bounds.Width, bounds.Height) / 2 - 28);
        var scale = radius / RangeKm;

        // Fake glow: draw each line twice, wide and faint underneath.
        void Line(Point a, Point b, IBrush brush, double width)
        {
            if (_glow == GlowMode.Fake)
                ctx.DrawLine(new Pen(brush, width * 5) { Brush = WithOpacity(brush, 0.12) }, a, b);
            ctx.DrawLine(new Pen(brush, width), a, b);
        }
        void Circle(Point c, double r, IBrush? fill, IBrush stroke, double width)
        {
            if (_glow == GlowMode.Fake)
                ctx.DrawEllipse(null, new Pen(WithOpacity(stroke, 0.12), width * 5), c, r, r);
            ctx.DrawEllipse(fill, new Pen(stroke, width), c, r, r);
        }

        for (var ring = 1; ring <= 4; ring++)
        {
            var r = radius * ring / 4;
            Circle(center, r, null, Brush("accent", ring == 4 ? 0.45 : 0.22), 1);
            Text(ctx, $"{RangeKm * ring / 4:0}", new Point(center.X + 6, center.Y - r - 16), Brush("textFaint"), 10);
        }
        Line(new Point(center.X - radius, center.Y), new Point(center.X + radius, center.Y), Brush("accent", 0.15), 1);
        Line(new Point(center.X, center.Y - radius), new Point(center.X, center.Y + radius), Brush("accent", 0.15), 1);
        Text(ctx, "N", new Point(center.X - 4, center.Y - radius - 22), Brush("textDim"), 11);

        // Sweep: one turn every 4 seconds, with a faint trailing wedge.
        var angle = Clock.Elapsed.TotalSeconds % 4 / 4 * Math.Tau;
        var tip = new Point(center.X + Math.Cos(angle) * radius, center.Y + Math.Sin(angle) * radius);
        var trail = angle - 0.6;
        var wedge = new StreamGeometry();
        using (var g = wedge.Open())
        {
            g.BeginFigure(center, true);
            g.LineTo(new Point(center.X + Math.Cos(trail) * radius, center.Y + Math.Sin(trail) * radius));
            g.ArcTo(tip, new Size(radius, radius), 0, false, SweepDirection.Clockwise);
            g.EndFigure(true);
        }
        ctx.DrawGeometry(Brush("accent", 0.07), null, wedge);
        Line(center, tip, Brush("accent", 0.7), 1.5);

        if (World is not null && !World.Docked)
        {
            foreach (var c in World.Contacts)
            {
                if (c.Distance > RangeKm) continue;
                var p = new Point(center.X + c.X * scale, center.Y - c.Y * scale);
                DrawContact(ctx, c, p);
            }
        }

        // Your ship, at the centre.
        var ship = new StreamGeometry();
        using (var g = ship.Open())
        {
            g.BeginFigure(new Point(center.X, center.Y - 11), true);
            g.LineTo(new Point(center.X + 7, center.Y + 7));
            g.LineTo(new Point(center.X, center.Y + 3));
            g.LineTo(new Point(center.X - 7, center.Y + 7));
            g.EndFigure(true);
        }
        ctx.DrawGeometry(Brush("text"), null, ship);
    }

    void DrawContact(DrawingContext ctx, Contact c, Point p)
    {
        var selected = c.Id == SelectedId;
        var label = true;
        switch (c.Kind)
        {
            case ContactKind.Station:
                ctx.DrawRectangle(Brush("accent", 0.5), new Pen(Brush("accent"), 1.5), new Rect(p.X - 8, p.Y - 8, 16, 16));
                break;
            case ContactKind.Gate:
                var diamond = new StreamGeometry();
                using (var g = diamond.Open())
                {
                    g.BeginFigure(new Point(p.X, p.Y - 9), false);
                    g.LineTo(new Point(p.X + 9, p.Y));
                    g.LineTo(new Point(p.X, p.Y + 9));
                    g.LineTo(new Point(p.X - 9, p.Y));
                    g.EndFigure(true);
                }
                ctx.DrawGeometry(null, new Pen(Brush("text"), 1.5), diamond);
                break;
            case ContactKind.Asteroid:
                ctx.DrawEllipse(Brush("textDim"), null, p, 2.5, 2.5);
                label = false;
                break;
            case ContactKind.Anomaly:
                ctx.DrawEllipse(null, new Pen(Brush("warning"), 1.5), p, 8, 8);
                ctx.DrawEllipse(Brush("warning"), null, p, 2.5, 2.5);
                break;
            case ContactKind.PlayerShip or ContactKind.NpcShip:
                var tri = new StreamGeometry();
                using (var g = tri.Open())
                {
                    g.BeginFigure(new Point(p.X, p.Y - 7), true);
                    g.LineTo(new Point(p.X + 7, p.Y + 6));
                    g.LineTo(new Point(p.X - 7, p.Y + 6));
                    g.EndFigure(true);
                }
                var brush = Brush(c.Kind == ContactKind.PlayerShip ? "friendly" : "accent");
                ctx.DrawGeometry(brush, null, tri);
                break;
        }
        if (selected)
            ctx.DrawEllipse(null, new Pen(Brush("warning"), 1.5) { DashStyle = DashStyle.Dash }, p, 15, 15);
        if (label || selected)
            Text(ctx, c.Name, new Point(p.X + 12, p.Y - 8), Brush(selected ? "warning" : "textDim"), 11);
    }

    static void Text(DrawingContext ctx, string text, Point at, IBrush brush, double size)
    {
        var formatted = new FormattedText(text, CultureInfo.InvariantCulture, FlowDirection.LeftToRight, Mono, size, brush);
        ctx.DrawText(formatted, at);
    }

    static IBrush WithOpacity(IBrush brush, double opacity) =>
        brush is ISolidColorBrush s ? new SolidColorBrush(s.Color, s.Opacity * opacity) : brush;
}
