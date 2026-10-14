ARG BASE_IMAGE
FROM mcr.microsoft.com/dotnet/aspnet:10.0 AS base
# curl for the container healthcheck (appspec.yaml); the aspnet image ships without it.
RUN apt-get update && apt-get install -y --no-install-recommends curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
EXPOSE 80


FROM $BASE_IMAGE AS publish
RUN dotnet publish "main-svc/main-svc.csproj" -c Release -o /app/publish /p:UseAppHost=false

FROM base AS final
WORKDIR /app
COPY ../emails emails
COPY --from=publish /app/publish .
ENV DOCKERIZED=true
ENV ASPNETCORE_HTTP_PORTS=80
ENTRYPOINT ["dotnet", "main-svc.dll"]
