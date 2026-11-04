//! The simpled binary a Swarm deployment ships next to its `deploy.sh`, which runs
//! the deploy steps (`simpled step ...`) on the target. It is always the version
//! that prepared the deployment, so the steps and the plan they read agree.

use crate::spec::TargetPlatform;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Environment variable naming a binary to ship, for when `--target-binary`
/// cannot be passed.
pub const OVERRIDE_VAR: &str = "SIMPLED_TARGET_BINARY";

/// Writes the binary to `dest`, taken from, in order:
///
/// 1. `explicit` (`--target-binary`), or the file `SIMPLED_TARGET_BINARY` names;
/// 2. this very binary, when it runs on Linux on the target's CPU, which is the
///    usual CI runner;
/// 3. this version's release on GitHub, verified against its checksums.
pub fn provide(dest: &Path, platform: TargetPlatform, explicit: Option<&Path>) -> Result<()> {
    let explicit: Option<PathBuf> = explicit
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os(OVERRIDE_VAR).map(PathBuf::from));

    let bytes = if let Some(path) = explicit {
        std::fs::read(&path).with_context(|| format!("Cannot read the target binary {}", path.display()))?
    } else if cfg!(target_os = "linux") && std::env::consts::ARCH == platform.rust_arch() {
        let exe = std::env::current_exe().context("Cannot locate the running simpled binary")?;
        std::fs::read(&exe).with_context(|| format!("Cannot read {}", exe.display()))?
    } else {
        crate::updater::release_linux_binary(platform.arch()).with_context(|| {
            format!(
                "prepare-deployment runs on {}-{} and cannot ship itself to a {} deploy target, so it \
                 downloads the {} binary of simpled {} from its release. For an unreleased build or an \
                 offline machine, pass --target-binary <a simpled {} built for {}>",
                std::env::consts::OS,
                std::env::consts::ARCH,
                platform,
                platform,
                env!("CARGO_PKG_VERSION"),
                env!("CARGO_PKG_VERSION"),
                platform
            )
        })?
    };
    crate::updater::write_executable(dest, &bytes)
}
