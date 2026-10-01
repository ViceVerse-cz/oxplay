// SPDX-License-Identifier: GPL-3.0-or-later
//! One presentation contract for native graphics and the explicit GL comparison.
use crate::{GlPresenter, MediaError, Player, RenderStats, Result};

// This persistent UI-thread owner moves only at setup/teardown. Keeping both
// variants inline avoids another allocation and pointer hop on every draw.
#[allow(clippy::large_enum_variant)]
pub enum VideoPresenter {
    OpenGl(GlPresenter),
    #[cfg(feature = "native-rendering")]
    Native(crate::native_presenter::NativePresenter),
}
impl VideoPresenter {
    /// # Safety
    /// The GL comparison requires the notifier's owning context to be current.
    /// Native mode verifies that device, queue and textures share one backend.
    pub unsafe fn new(
        player: &Player,
        api: &slint::GraphicsAPI<'_>,
        display: Option<u32>,
    ) -> Result<Self> {
        match api {
            slint::GraphicsAPI::NativeOpenGL { get_proc_address } => unsafe {
                GlPresenter::new(player, get_proc_address).map(Self::OpenGl)
            },
            #[cfg(feature = "native-rendering")]
            slint::GraphicsAPI::WGPU30 { device, queue, .. } => {
                crate::native_presenter::NativePresenter::new(player, device, queue, display)
                    .map(Self::Native)
            }
            _ => {
                let _ = display;
                Err(MediaError("Build with native-rendering and the pinned native media libraries for this GPU backend".into()))
            }
        }
    }
    pub fn graphics_info(&self) -> &str {
        match self {
            Self::OpenGl(p) => p.graphics_info(),
            #[cfg(feature = "native-rendering")]
            Self::Native(p) => p.graphics_info(),
        }
    }
    pub fn stats(&self) -> RenderStats {
        match self {
            Self::OpenGl(p) => p.stats(),
            #[cfg(feature = "native-rendering")]
            Self::Native(p) => p.stats(),
        }
    }
    pub fn set_ambient_sampling(&mut self, enabled: bool) {
        match self {
            Self::OpenGl(p) => p.set_ambient_sampling(enabled),
            #[cfg(feature = "native-rendering")]
            Self::Native(p) => p.set_ambient_sampling(enabled),
        }
    }
    pub fn observed_load_request(&self) -> u64 {
        match self {
            Self::OpenGl(p) => p.observed_load_request(),
            #[cfg(feature = "native-rendering")]
            Self::Native(p) => p.observed_load_request(),
        }
    }
    pub fn take_ambient_sample(&mut self) -> Option<crate::AmbientSample> {
        match self {
            Self::OpenGl(p) => p.take_ambient_sample(),
            #[cfg(feature = "native-rendering")]
            Self::Native(p) => p.take_ambient_sample(),
        }
    }
    pub fn set_display(&self, display: Option<u32>) -> Result<()> {
        match self {
            Self::OpenGl(_) => {
                let _ = display;
                Ok(())
            }
            #[cfg(feature = "native-rendering")]
            Self::Native(p) => p.set_display(display),
        }
    }
    /// # Safety
    /// Called only on the owning Slint window's BeforeRendering boundary.
    pub unsafe fn render(
        &mut self,
        width: u32,
        height: u32,
        publish: bool,
    ) -> Result<Option<slint::Image>> {
        match self {
            Self::OpenGl(p) => unsafe { p.render(width, height, publish) },
            #[cfg(feature = "native-rendering")]
            Self::Native(p) => p.render(width, height, publish),
        }
    }
    /// # Safety
    /// Called only on the owning window's AfterRendering boundary.
    pub unsafe fn after_render(&mut self) {
        match self {
            Self::OpenGl(p) => unsafe { p.after_render() },
            #[cfg(feature = "native-rendering")]
            Self::Native(p) => p.after_render(),
        }
    }
}
