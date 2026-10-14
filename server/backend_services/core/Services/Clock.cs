namespace core.Services;

/// <summary>
/// The only source of "now". Always UTC: Npgsql maps DateTime to `timestamp with time zone` and refuses
/// Kind=Local, and a ledger must never depend on the server's zone. Tests override it.
/// </summary>
public class Clock
{
    private DateTime? _overrideNow;

    public void OverrideNow(DateTime now)
    {
        _overrideNow = now.Kind == DateTimeKind.Utc ? now : DateTime.SpecifyKind(now, DateTimeKind.Utc);
    }

    public DateTime Now => _overrideNow ?? DateTime.UtcNow;

    public DateOnly Today => DateOnly.FromDateTime(Now);
}
