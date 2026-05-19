use anyhow::{bail, Context, Result};
use semver::Version;
use serde::Deserialize;
use std::env;
use std::io::Write;

const GITHUB_REPO: &str = "vkubiv/simpled";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

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

    let name = if os == "windows" {
        format!("simpled-{}-{}.exe", os_str, arch_str)
    } else {
        format!("simpled-{}-{}", os_str, arch_str)
    };

    Ok(name)
}

fn parse_version(tag: &str) -> Result<Version> {
    let stripped = tag.strip_prefix('v').unwrap_or(tag);
    Version::parse(stripped).with_context(|| format!("Invalid version tag: {}", tag))
}

fn fetch_latest_release() -> Result<Release> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        GITHUB_REPO
    );

    let mut builder = reqwest::blocking::Client::new()
        .get(&url)
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
    let asset = release
        .assets
        .into_iter()
        .find(|a| a.name == asset_name)
        .with_context(|| {
            format!(
                "No binary found for this platform in release {} (expected asset '{}')",
                release.tag_name, asset_name
            )
        })?;

    println!("Downloading {}...", asset_name);

    let mut builder = reqwest::blocking::Client::new()
        .get(&asset.url)
        .header("User-Agent", "simpled")
        .header("Accept", "application/octet-stream");

    if let Ok(token) = env::var("GITHUB_TOKEN") {
        builder = builder.header("Authorization", format!("Bearer {}", token));
    }

    let mut response = builder.send().context("Failed to download update")?;

    if !response.status().is_success() {
        bail!("Download failed with status {}", response.status());
    }

    let current_exe = env::current_exe().context("Failed to determine current executable path")?;

    // Write to a temp file next to the current exe
    let tmp_path = current_exe.with_extension("update_tmp");
    {
        let mut tmp_file =
            std::fs::File::create(&tmp_path).context("Failed to create temp file for update")?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tmp_file
                .set_permissions(std::fs::Permissions::from_mode(0o755))
                .context("Failed to set permissions on temp file")?;
        }

        response
            .copy_to(&mut tmp_file)
            .context("Failed to write update to disk")?;
        tmp_file.flush().context("Failed to flush update file")?;
    }

    replace_exe(&current_exe, &tmp_path)?;

    println!("Updated to {}. Restart simpled to use the new version.", latest);
    Ok(())
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
