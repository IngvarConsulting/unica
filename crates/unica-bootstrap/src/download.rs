use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::time::Duration;

use crate::error::{BootstrapError, Result};

pub trait Downloader: Send + Sync {
    fn download(&self, url: &str, destination: &Path) -> Result<()>;
}

pub struct HttpDownloader {
    agent: ureq::Agent,
}

impl Default for HttpDownloader {
    fn default() -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(30))
                .timeout_read(Duration::from_secs(60))
                .redirects(5)
                .build(),
        }
    }
}

impl Downloader for HttpDownloader {
    fn download(&self, url: &str, destination: &Path) -> Result<()> {
        if !url.starts_with("https://") {
            return Err(BootstrapError::new(format!(
                "runtime download URL must use HTTPS: {url}"
            )));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let response = self.agent.get(url).call().map_err(|error| {
            BootstrapError::new(format!("failed to download runtime asset {url}: {error}"))
        })?;
        if !response.get_url().starts_with("https://") {
            return Err(BootstrapError::new(format!(
                "runtime download redirected to a non-HTTPS URL: {}",
                response.get_url()
            )));
        }

        let mut reader = response.into_reader();
        let mut output = File::create(destination)?;
        io::copy(&mut reader, &mut output).map_err(|error| {
            BootstrapError::new(format!(
                "failed to write runtime asset {}: {error}",
                destination.display()
            ))
        })?;
        output.sync_all()?;
        Ok(())
    }
}
