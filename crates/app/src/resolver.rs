// SPDX-License-Identifier: GPL-3.0-or-later
//! One lazily constructed guest/account extractor, including its concurrency and
//! rate-limit state. Construction and cloning perform no filesystem or network I/O.
use oxplay_core::ProviderError;
use oxplay_youtube::{YtDlp, innertube::GuestTransport};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

#[derive(Clone)]
pub struct SharedResolver {
    helper: Arc<PathBuf>,
    deno: Arc<PathBuf>,
    initialized: Arc<OnceLock<Result<Arc<YtDlp>, ProviderError>>>,
    /// One anonymous InnerTube transport shared by guest workers, so its 429
    /// cooldown applies to every native watch-page/comment read.
    native: Arc<OnceLock<Result<Arc<GuestTransport>, ProviderError>>>,
}
impl SharedResolver {
    pub fn new(helper: PathBuf, deno: PathBuf) -> Self {
        Self {
            helper: Arc::new(helper),
            deno: Arc::new(deno),
            initialized: Arc::new(OnceLock::new()),
            native: Arc::new(OnceLock::new()),
        }
    }

    /// Worker-only, like [`Self::get_on_worker`]: builds the HTTP client and its
    /// runtime lazily. Never call from a thread that is driving a Tokio runtime.
    pub fn native_on_worker(&self) -> Result<Arc<GuestTransport>, ProviderError> {
        self.native
            .get_or_init(|| GuestTransport::new().map(Arc::new))
            .clone()
    }

    /// Call only from an owned worker: helper validation and any future provider
    /// initialization must never migrate onto the UI thread.
    pub fn get_on_worker(&self) -> Result<Arc<YtDlp>, ProviderError> {
        self.initialized
            .get_or_init(|| {
                YtDlp::new(self.helper.as_path())
                    .and_then(|provider| provider.with_deno(self.deno.as_path()))
                    .map(Arc::new)
            })
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_is_lazy_and_clones_share_initialization_failure() {
        let resolver = SharedResolver::new("relative-helper".into(), "relative-deno".into());
        let clone = resolver.clone();
        assert!(resolver.initialized.get().is_none());
        assert!(clone.initialized.get().is_none());
        assert!(matches!(
            std::thread::spawn(move || clone.get_on_worker())
                .join()
                .unwrap(),
            Err(ProviderError::InvalidInput)
        ));
        assert!(matches!(
            resolver.initialized.get(),
            Some(Err(ProviderError::InvalidInput))
        ));
    }

    #[test]
    fn workers_share_one_provider_without_executing_helpers() {
        let root = std::env::temp_dir();
        let resolver =
            SharedResolver::new(root.join("synthetic-yt-dlp"), root.join("synthetic-deno"));
        let other = resolver.clone();
        let first = std::thread::spawn(move || resolver.get_on_worker().unwrap());
        let second = std::thread::spawn(move || other.get_on_worker().unwrap());
        assert!(Arc::ptr_eq(&first.join().unwrap(), &second.join().unwrap()));
    }
}
