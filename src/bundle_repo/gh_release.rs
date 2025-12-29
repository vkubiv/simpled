use anyhow::{Context, Result, bail};
use std::env;
use std::fs::File;
use std::path::Path;
use serde::Deserialize;
use std::io::Read;

#[derive(Deserialize)]
struct Release {
    #[allow(dead_code)]
    id: u64,
    upload_url: String,
}

pub fn download(repo: &str, ver: &str, app_name: &str, tag_prefix: Option<&str>) -> Result<String> {
    let filename = format!("{}.{}.tar.gz", app_name, ver);
    let tag = format!("{}{}", tag_prefix.unwrap_or(""), ver);
    
    // Check GITHUB_TOKEN
    let token = env::var("GITHUB_TOKEN").context("GITHUB_TOKEN is not set. It is required for downloading releases.")?;

    let url = format!("https://github.com/{}/releases/download/{}/{}", repo, tag, filename);
    println!("Downloading bundle from {}", url);

    let client = reqwest::blocking::Client::new();
    let mut response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "simpled")
        .send()
        .context("Failed to send request")?;

    if !response.status().is_success() {
        bail!("Failed to download bundle from {}: status code {}", url, response.status());
    }

    let mut dest = File::create(Path::new(&filename)).context("Failed to create file")?;
    response.copy_to(&mut dest).context("Failed to write content to file")?;

    Ok(filename)
}

pub fn upload(repo: &str, ver: &str, filename: &str, tag_prefix: Option<&str>) -> Result<()> {
    let token = env::var("GITHUB_TOKEN").context("GITHUB_TOKEN is not set")?;
    let client = reqwest::blocking::Client::new();
    let tag = format!("{}{}", tag_prefix.unwrap_or(""), ver);

    // 1. Get release by tag
    let url = format!("https://api.github.com/repos/{}/releases/tags/{}", repo, tag);
    let mut response = client.get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "simpled")
        .send()
        .context("Failed to get release")?;

    let release: Release = if response.status().is_success() {
        bail!("Release {} already exists. Increase the app version number.", tag);
    } else if response.status().as_u16() == 404 {
        // Create release
        println!("Release {} not found, creating...", tag);
        let create_url = format!("https://api.github.com/repos/{}/releases", repo);
        let body = format!(r#"{{ "tag_name": "{}", "name": "{}", "body": "Release {}" }}"#, tag, tag, tag);
        
        let response = client.post(&create_url)
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", "simpled")
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .context("Failed to create release")?;
            
        if !response.status().is_success() {
             bail!("Failed to create release: {}", response.status());
        }
        let text = response.text()?;
        serde_yaml::from_str(&text).context("Failed to parse created release info")?
    } else {
        bail!("Failed to get release info: {}", response.status());
    };

    // 2. Upload asset
    // upload_url looks like "https://uploads.github.com/repos/octocat/Hello-World/releases/1/assets{?name,label}"
    let upload_url_template = release.upload_url;
    let upload_url = upload_url_template.split('{').next().unwrap_or(&upload_url_template);
    let target_url = format!("{}?name={}", upload_url, filename);
    
    println!("Uploading {} to {}", filename, target_url);

    let mut file = File::open(filename).context("Failed to open file for upload")?;
    let mut content = Vec::new();
    file.read_to_end(&mut content)?;

    let response = client.post(&target_url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "simpled")
        .header("Content-Type", "application/gzip")
        .body(content)
        .send()
        .context("Failed to upload asset")?;

    if !response.status().is_success() {
        bail!("Failed to upload asset: {}", response.text().unwrap_or_default());
    }

    println!("Upload successful");

    Ok(())
}
