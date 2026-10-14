namespace core.Auth;

/// <summary>A session ends after <see cref="IdleTimeout"/> without requests or at <see cref="AbsoluteLifetime"/>.</summary>
public record SessionPolicy(TimeSpan IdleTimeout, TimeSpan AbsoluteLifetime);
