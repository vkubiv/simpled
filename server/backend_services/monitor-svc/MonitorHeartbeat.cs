namespace monitor_svc;

/// <summary>When the last monitor round ended, so /health fails once the loop has stalled.</summary>
public sealed class MonitorHeartbeat
{
    private long _lastRoundTicks;

    public void Beat(DateTime now) => Interlocked.Exchange(ref _lastRoundTicks, now.Ticks);

    public bool IsAlive(DateTime now, TimeSpan tolerance)
    {
        var last = Interlocked.Read(ref _lastRoundTicks);
        return last != 0 && now - new DateTime(last, DateTimeKind.Utc) <= tolerance;
    }
}
