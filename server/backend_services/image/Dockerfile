FROM mcr.microsoft.com/dotnet/sdk:10.0 AS build
WORKDIR /src
COPY ["config/", "config/"]
COPY ["db/", "db/"]
COPY ["core/", "core/"]
COPY ["primary-db/", "primary-db/"]
COPY ["db-setup/", "db-setup/"]
COPY ["main-svc/", "main-svc/"]
COPY ["utest/", "utest/"]
COPY ["backend.sln", "backend.sln"]
COPY [".editorconfig", ".editorconfig"]
COPY ["global.json", "global.json"]
COPY ["Directory.Build.props", "Directory.Build.props"]
COPY ["BannedSymbols.txt", "BannedSymbols.txt"]

RUN dotnet restore "backend.sln"

RUN dotnet build "backend.sln" -c Release -o /app/build
