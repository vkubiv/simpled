namespace core.Auth;

public enum AuthType
{
    /// <summary>Bearer token in the Authorization header; the mobile app.</summary>
    TokenSession,

    /// <summary>HttpOnly cookie; the web builds and the admin console.</summary>
    CookieSession,
}

public class AuthClaims
{
    public required int AccountId { get; init; }

    public required long SessionId { get; init; }

    public required AuthType AuthType { get; init; }
}
