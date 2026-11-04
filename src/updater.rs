use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::env;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use tar::Archive;

const GITHUB_REPO: &str = "vkubiv/simpled";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Release asset listing the SHA-256 of every other asset, written by the
/// release workflow with `sha256sum`.
const CHECKSUMS_ASSET: &str = "sha256sums.txt";

#[derive(Deserialize)]
struct Asset {
    name: String,
    url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

fn platform_asset_name() -> Result<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let os_str = match os {
        "linux" => "linux",
        "macos" => "macos",
        "windows" => "windows",
        other => bail!("Unsupported OS: {}", other),
    };

    let arch_str = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => bail!("Unsupported architecture: {}", other),
    };

    Ok(format!("simpled_{}_{}.tar.gz", os_str, arch_str))
}

fn parse_version(tag: &str) -> Result<Version> {
    let stripped = tag.strip_prefix('v').unwrap_or(tag);
    Version::parse(stripped).with_context(|| format!("Invalid version tag: {}", tag))
}

fn fetch_latest_release() -> Result<Release> {
    fetch_release(&format!("https://api.github.com/repos/{}/releases/latest", GITHUB_REPO))
}

fn fetch_release(url: &str) -> Result<Release> {
    let mut builder = reqwest::blocking::Client::new()
        .get(url)
        .header("User-Agent", "simpled");

    if let Ok(token) = env::var("GITHUB_TOKEN") {
        builder = builder.header("Authorization", format!("Bearer {}", token));
    }

    let response = builder.send().context("Failed to reach GitHub API")?;

    if response.status().as_u16() == 404 {
        bail!("No releases found for {}", GITHUB_REPO);
    }
    if !response.status().is_success() {
        bail!("GitHub API returned status {}", response.status());
    }

    response.json::<Release>().context("Failed to parse release info")
}

/// Path of the binary to replace. On unix the path is canonicalized so an
/// update rewrites the real file rather than clobbering a symlink pointing at
/// it; on Windows canonicalize only adds a `\\?\` prefix, so it is skipped.
fn resolve_current_exe() -> Result<PathBuf> {
    let exe = env::current_exe().context("Failed to determine current executable path")?;

    #[cfg(unix)]
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);

    Ok(exe)
}

/// Installs commonly land in a directory the current user cannot write to
/// (/usr/local/bin on macOS, /usr/bin on Linux), so check up front rather than
/// failing after the download.
fn ensure_writable(exe: &Path) -> Result<()> {
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let probe = dir.join(".simpled_update_probe");

    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            let hint = if cfg!(windows) {
                "run simpled update from a terminal opened as Administrator"
            } else {
                "run `sudo simpled update`"
            };
            bail!("No write access to {} - {}", dir.display(), hint)
        }
        Err(e) => Err(e).with_context(|| format!("Cannot write to {}", dir.display())),
    }
}

pub fn check_and_update(check_only: bool) -> Result<()> {
    let current = Version::parse(CURRENT_VERSION).expect("invalid package version");

    println!("Current version: {}", current);
    println!("Checking for updates...");

    let release = fetch_latest_release()?;
    let latest = parse_version(&release.tag_name)?;

    if latest <= current {
        println!("Already up to date ({})", current);
        return Ok(());
    }

    println!("New version available: {} -> {}", current, latest);

    if check_only {
        println!("Run `simpled update` to install the update.");
        return Ok(());
    }

    let asset_name = platform_asset_name()?;
    let current_exe = resolve_current_exe()?;
    ensure_writable(&current_exe)?;

    let binary_name = if cfg!(windows) { "simpled.exe" } else { "simpled" };
    let binary = verified_binary(&release, &asset_name, binary_name)?;

    let tmp_path = current_exe.with_extension("update_tmp");
    write_executable(&tmp_path, &binary).context("Failed to write the update")?;

    if let Err(e) = replace_exe(&current_exe, &tmp_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    println!("Updated to {}. Restart simpled to use the new version.", latest);
    Ok(())
}

/// The simpled binary inside one asset of `release`, downloaded and checked
/// against the release's `sha256sums.txt`. Without that digest list nothing
/// vouches for the download, so it is refused rather than used unverified.
fn verified_binary(release: &Release, asset_name: &str, binary_name: &str) -> Result<Vec<u8>> {
    let asset = release.assets.iter().find(|a| a.name == asset_name).with_context(|| {
        format!(
            "Release {} has no binary for this platform (expected asset '{}')",
            release.tag_name, asset_name
        )
    })?;
    let checksums_asset = release
        .assets
        .iter()
        .find(|a| a.name == CHECKSUMS_ASSET)
        .with_context(|| {
            format!(
                "Release {} carries no {}, so the downloaded binary cannot be verified; refusing to use it",
                release.tag_name, CHECKSUMS_ASSET
            )
        })?;

    let checksums = String::from_utf8(download_asset(&checksums_asset.url)?)
        .with_context(|| format!("{} is not valid UTF-8", CHECKSUMS_ASSET))?;
    let expected = expected_sha256(&checksums, asset_name).with_context(|| {
        format!(
            "{} of release {} has no entry for {}",
            CHECKSUMS_ASSET, release.tag_name, asset_name
        )
    })?;

    eprintln!("Downloading {} from release {}...", asset_name, release.tag_name);
    let archive_bytes = download_asset(&asset.url)?;
    let actual = sha256_hex(&archive_bytes);
    if actual != expected {
        bail!(
            "Checksum mismatch for {}: expected {} but downloaded {}. The download is corrupt or has been tampered with; it was not used.",
            asset_name,
            expected,
            actual
        );
    }
    eprintln!("Checksum verified.");

    let mut archive = Archive::new(GzDecoder::new(Cursor::new(archive_bytes)));
    for entry in archive.entries().context("Failed to read archive entries")? {
        let mut entry = entry.context("Failed to read archive entry")?;
        let path = entry.path().context("Failed to read entry path")?;
        if path.file_name().and_then(|n| n.to_str()) == Some(binary_name) {
            let mut binary = Vec::new();
            std::io::copy(&mut entry, &mut binary).context("Failed to extract binary from archive")?;
            return Ok(binary);
        }
    }
    bail!("Binary '{}' not found inside {}", binary_name, asset_name)
}

/// The Linux binary of this exact simpled version for `arch` (`amd64`, `arm64`),
/// from its GitHub release, verified. What `prepare-deployment` ships to a Swarm
/// deploy target when it cannot copy itself.
pub fn release_linux_binary(arch: &str) -> Result<Vec<u8>> {
    let tag = format!("v{}", CURRENT_VERSION);
    let release = fetch_release(&format!(
        "https://api.github.com/repos/{}/releases/tags/{}",
        GITHUB_REPO, tag
    ))
    .with_context(|| format!("Cannot look up release {} of {}", tag, GITHUB_REPO))?;
    verified_binary(&release, &format!("simpled_linux_{}.tar.gz", arch), "simpled")
}

/// Writes `bytes` to `path`, executable on unix.
pub fn write_executable(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = std::fs::File::create(path).with_context(|| format!("Failed to create {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o755))?;
    }
    file.write_all(bytes)?;
    file.flush()?;
    Ok(())
}

/// Fetch one release asset in full. Assets are small (a compressed binary), so
/// holding it in memory lets the digest be checked before anything is extracted.
fn download_asset(url: &str) -> Result<Vec<u8>> {
    let mut builder = reqwest::blocking::Client::new()
        .get(url)
        .header("User-Agent", "simpled")
        .header("Accept", "application/octet-stream");

    if let Ok(token) = env::var("GITHUB_TOKEN") {
        builder = builder.header("Authorization", format!("Bearer {}", token));
    }

    let response = builder.send().with_context(|| format!("Failed to download {}", url))?;
    if !response.status().is_success() {
        bail!("Download of {} failed with status {}", url, response.status());
    }
    Ok(response.bytes().context("Failed to read download")?.to_vec())
}

/// The digest `sha256sum` recorded for `asset_name`, from its `<hex>  <name>`
/// lines. `<hex> *<name>` (binary mode) is accepted too.
fn expected_sha256(checksums: &str, asset_name: &str) -> Option<String> {
    checksums.lines().find_map(|line| {
        let (hex, name) = line.trim().split_once(char::is_whitespace)?;
        let name = name.trim_start().trim_start_matches('*');
        (name == asset_name).then(|| hex.to_ascii_lowercase())
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(windows)]
fn replace_exe(current: &std::path::Path, new: &std::path::Path) -> Result<()> {
    // Windows won't let you overwrite a running exe, but allows renaming it.
    let bak = current.with_extension("exe.bak");
    // Remove stale backup if present
    let _ = std::fs::remove_file(&bak);
    std::fs::rename(current, &bak).context("Failed to rename current executable to backup")?;
    std::fs::rename(new, current).context("Failed to move new executable into place")?;
    Ok(())
}

#[cfg(not(windows))]
fn replace_exe(current: &std::path::Path, new: &std::path::Path) -> Result<()> {
    std::fs::rename(new, current).context("Failed to replace executable")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUMS: &str = "\
0123abcd  simpled_linux_amd64.tar.gz
ABCD0123 *simpled_windows_amd64.tar.gz
";

    #[test]
    fn the_digest_of_an_asset_is_looked_up_by_name() {
        assert_eq!(
            expected_sha256(SUMS, "simpled_linux_amd64.tar.gz").as_deref(),
            Some("0123abcd")
        );
        // Binary-mode marker and upper-case hex are normalized.
        assert_eq!(
            expected_sha256(SUMS, "simpled_windows_amd64.tar.gz").as_deref(),
            Some("abcd0123")
        );
        assert_eq!(expected_sha256(SUMS, "simpled_macos_arm64.tar.gz"), None);
    }

    #[test]
    fn sha256_matches_the_known_digest_of_abc() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
