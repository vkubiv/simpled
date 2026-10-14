ARG BASE_IMAGE
FROM mcr.microsoft.com/dotnet/aspnet:10.0 AS base
WORKDIR /app


FROM $BASE_IMAGE AS publish
RUN dotnet publish "db-setup/db-setup.csproj" -c Release -o /app/publish /p:UseAppHost=false

FROM base AS final
WORKDIR /app

COPY --from=publish /app/publish .
ENV DOCKERIZED=true
ENTRYPOINT ["dotnet", "db-setup.dll"]
