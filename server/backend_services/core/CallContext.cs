using core.Services;
using Microsoft.Extensions.Logging;

namespace core;

public class CallContext
{
    public required CancellationToken CancellationToken { get; init; }
    public required ILogger Logger { get; init; }
    public required Clock Clock { get; init; }
}