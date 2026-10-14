using Microsoft.AspNetCore.Http;

namespace core.Auth;

/// <summary>The session cookie of one service. <c>Secure</c> is off only in development, where the stack is plain http.</summary>
public record SessionCookie(string Name, SameSiteMode SameSite, bool Secure)
{
    public string? Read(HttpRequest request) =>
        request.Cookies.TryGetValue(Name, out var token) && !string.IsNullOrEmpty(token) ? token : null;

    public void Write(HttpResponse response, string token) => response.Cookies.Append(Name, token, Options());

    public void Delete(HttpResponse response) => response.Cookies.Delete(Name, Options());

    private CookieOptions Options() => new()
    {
        HttpOnly = true,
        Secure = Secure,
        SameSite = SameSite,
        Path = "/",
        IsEssential = true,
    };
}
