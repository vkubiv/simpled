using System.Diagnostics.CodeAnalysis;
using config;
using core.Auth;
using core.Libs.EntityAccess;
using core.Services;
using core.Services.Mailer;
using db.Entities;
using main_svc;
using main_svc.Auth;
using Microsoft.AspNetCore.Authorization;
using Microsoft.EntityFrameworkCore;
using primary_db;

var config = ConfigurationLoader.LoadDefault();

var builder = WebApplication.CreateBuilder(args);

// Database context. Registered as the base DbContext so `core` never references primary-db.
builder.Services.AddDbContext<DbContext, PrimaryDbContext>(options =>
{
    options.UseNpgsql(config.DefaultConnectionString);
});

// Entity accessors: open generics, so a new entity never needs a registration line.
builder.Services.AddScoped(typeof(Read<>));
builder.Services.AddScoped(typeof(Mutate<>));
builder.Services.AddScoped<UnitOfWork>();

builder.Services.AddSingleton(config);
builder.Services.AddScoped<Clock>();

// Mail.
builder.Services.AddSingleton(new UserMailer(
    SetupMailer.CreateMailer(config, config.IsDockerized ? "emails" : "../emails"),
    new UserMailConfig { PostMasterEmail = config.PostMasterEmail, WebAppUrl = config.WebAppUrl }));

// GitHub: organizations' tokens, read when repositories are linked and synced.
builder.Services.AddGitHub(config);

// Claims and call context.
builder.Services.AddHttpContextAccessor();
builder.Services.AddTransient<MainClaims>(x =>
{
    var context = x.GetService<IHttpContextAccessor>();
    return context!.HttpContext!.User.ToAuthClaims();
});

builder.Services.AddScoped<MainCallContext>(serviceProvider =>
{
    var loggerFactory = serviceProvider.GetRequiredService<ILoggerFactory>();
    var httpContextAccessor = serviceProvider.GetRequiredService<IHttpContextAccessor>();

    var httpContext = httpContextAccessor.HttpContext;
    var token = httpContext?.RequestAborted ?? CancellationToken.None;
    var endpoint = httpContext?.GetEndpoint();

    return new MainCallContext
    {
        Logger = loggerFactory.CreateLogger(endpoint?.DisplayName ?? "Unknown"),
        CancellationToken = token,
        Clock = serviceProvider.GetRequiredService<Clock>(),
        UnitOfWork = serviceProvider.GetRequiredService<UnitOfWork>(),
    };
});

// Auth: opaque session tokens in auth_sessions; HttpOnly cookie for the web app, bearer header for the CLI.
builder.Services.AddSingleton(new SessionCookie("session", SameSiteMode.Lax, Secure: !config.IsDevelopmentEnvironment));
builder.Services.AddAuthentication(SessionAuthenticationHandler.SchemeName)
    .AddScheme<SessionAuthenticationOptions, MainAuthenticationHandler>(
        SessionAuthenticationHandler.SchemeName,
        options =>
        {
            options.Audience = AuthSessionEntity.AudienceMain;
            options.AcceptBearer = true;
        });

builder.Services.AddAuthRateLimiting(config.AuthRateLimitPerMinute);

builder.Services.AddAuthorization(options =>
{
    options.DefaultPolicy = new AuthorizationPolicyBuilder()
        .AddAuthenticationSchemes(SessionAuthenticationHandler.SchemeName)
        .RequireAuthenticatedUser()
        .Build();
    options.FallbackPolicy = options.DefaultPolicy;
});

builder.Services.AddCors(options =>
{
    options.AddPolicy("CorsPolicy",
        policy => policy.WithOrigins(config.CorsOrigins.ToArray())
            .AllowAnyMethod()
            .AllowAnyHeader()
            .AllowCredentials());
});

// MVC + OpenAPI.
builder.Services.AddControllers();
builder.Services.AddEndpointsApiExplorer();
builder.Services.AddSwaggerGen();

if (!string.IsNullOrEmpty(config.SentryDsn))
{
    builder.WebHost.UseSentry(o => { o.Dsn = config.SentryDsn; });
}

var app = builder.Build();

app.UseForwardedHeaders();

app.UseCors("CorsPolicy");

app.UseSwagger();
app.UseSwaggerUI();

app.UseRateLimiter();

app.UseAuthentication();
app.UseAuthorization();

app.MapControllers();
app.MapGet("/health", () => Results.Ok("ok")).AllowAnonymous();

app.Run();

[ExcludeFromCodeCoverage]
public partial class Program
{
}
