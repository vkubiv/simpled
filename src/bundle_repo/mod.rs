pub mod gh_release;

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Where a released app bundle comes from: a file or directory on disk, or a
/// version downloaded from where `app-bundle create` uploaded it.
#[derive(clap::Args, Debug, Clone, Default)]
pub struct BundleArgs {
    /// An app bundle: a `.tar.gz` from `app-bundle create`, or a directory
    /// holding an appspec
    #[arg(long = "app-bundle", alias = "bundle")]
    pub bundle: Option<String>,

    /// The bundle version to download, with --download-bundle-from
    #[arg(long = "app-version", alias = "version")]
    pub version: Option<String>,

    /// Where to download the bundle from. Only `github-release` is supported
    #[arg(long)]
    pub download_bundle_from: Option<String>,

    /// `owner/repo` whose releases hold the bundle
    #[arg(long)]
    pub github_repo: Option<String>,

    /// Prefix of the release tag, e.g. `shopify-v` for `shopify-v1.2.3`
    #[arg(long)]
    pub github_tag_prefix: Option<String>,
}

impl BundleArgs {
    /// The bundle on disk, downloading it into `download_dir` first when asked
    /// to. `None` when no bundle was named at all.
    pub fn locate(&self, app_name: &str, download_dir: &Path) -> Result<Option<PathBuf>> {
        if let Some(source) = &self.download_bundle_from {
            if self.bundle.is_some() {
                bail!("--app-bundle and --download-bundle-from name two bundles; pass one");
            }
            if source != "github-release" {
                bail!(
                    "Unknown download source: {}. Only 'github-release' is supported.",
                    source
                );
            }
            let ver = self
                .version
                .as_ref()
                .context("--app-version is required when downloading from github-release")?;
            let repo = self
                .github_repo
                .as_ref()
                .context("--github-repo is required when downloading from github-release")?;
            let path = gh_release::download(repo, ver, app_name, self.github_tag_prefix.as_deref(), download_dir)?;
            return Ok(Some(path));
        }
        if self.version.is_some() {
            bail!("--app-version needs --download-bundle-from; to use a bundle on disk pass --app-bundle");
        }
        Ok(self.bundle.as_ref().map(PathBuf::from))
    }
}
