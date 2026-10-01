// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit renderer selection and observed GPU identity for the pinned Slint API.
//! Native selection stays on one graphics API; OpenGL is an explicit comparison.
use slint::{
    GraphicsAPI, PlatformError,
    wgpu_30::{WGPUConfiguration, WGPUSettings, wgpu},
    winit_030::winit::window::WindowAttributes,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphicsBackend {
    #[default]
    Native,
    OpenGl,
}

impl GraphicsBackend {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "native" => Ok(Self::Native),
            "opengl" => Ok(Self::OpenGl),
            _ => Err("Graphics backend must be native or opengl"),
        }
    }

    /// Select before constructing any Slint component. Adapter discovery can
    /// choose another GPU on the requested API, but cannot fall back to OpenGL.
    pub fn select(
        self,
        attributes_hook: impl Fn(WindowAttributes) -> WindowAttributes + 'static,
    ) -> Result<(), PlatformError> {
        let selector = slint::BackendSelector::new().backend_name("winit".into());
        let selector = match self {
            Self::Native => selector
                .renderer_name("femtovg-wgpu".into())
                .require_wgpu_30(native_configuration()?),
            Self::OpenGl => selector.renderer_name("femtovg".into()).require_opengl(),
        };
        selector
            .with_winit_window_attributes_hook(attributes_hook)
            .select()
    }

    /// Read the observed API during RenderingSetup. The selector alone is not
    /// runtime proof, especially when GPU adapter environment overrides exist.
    pub fn identity(self, api: &GraphicsAPI<'_>) -> Result<String, &'static str> {
        match (self, api) {
            (Self::Native, GraphicsAPI::WGPU30 { device, .. }) => {
                let info = device.adapter_info();
                let expected = native_backend_for(std::env::consts::OS)
                    .ok_or("Native graphics is unsupported on this operating system")?;
                validate_adapter(expected, info.backend, info.device_type)?;
                Ok(format!(
                    "FemtoVG WGPU30 | {:?} | {} | {:?} | vendor={:#x} device={:#x} | driver={} | {}",
                    info.backend,
                    info.name,
                    info.device_type,
                    info.vendor,
                    info.device,
                    info.driver,
                    info.driver_info,
                ))
            }
            (Self::OpenGl, GraphicsAPI::NativeOpenGL { .. }) => {
                Ok("FemtoVG | OpenGL (explicit comparison)".into())
            }
            _ => Err("Observed graphics API does not match the selected backend"),
        }
    }
}

fn native_configuration() -> Result<WGPUConfiguration, PlatformError> {
    let settings = native_settings().map_err(PlatformError::from)?;
    #[cfg(all(target_os = "linux", feature = "native-rendering"))]
    {
        pollster::block_on(oxplay_media::native_gpu_configuration(settings))
            .map_err(|error| PlatformError::Other(error.to_string()))
    }
    #[cfg(not(all(target_os = "linux", feature = "native-rendering")))]
    Ok(WGPUConfiguration::Automatic(settings))
}

fn native_backend_for(os: &str) -> Option<wgpu::Backend> {
    match os {
        "macos" => Some(wgpu::Backend::Metal),
        "windows" => Some(wgpu::Backend::Dx12),
        "linux" => Some(wgpu::Backend::Vulkan),
        _ => None,
    }
}

fn native_settings() -> Result<WGPUSettings, &'static str> {
    let backend = native_backend_for(std::env::consts::OS)
        .ok_or("Native graphics is unsupported on this operating system")?;
    let mut settings = WGPUSettings::default();
    // Override WGPU_BACKEND and WGPU_POWER_PREF defaults deliberately: native
    // mode has a reproducible API and prefers an integrated GPU when available.
    settings.backends = backend.into();
    settings.power_preference = wgpu::PowerPreference::LowPower;
    // Native video ambience reduces its colour grid on the GPU. The UI-only
    // WebGL limits disable compute/storage buffers, even on capable adapters.
    settings.device_required_limits = wgpu::Limits::downlevel_defaults();
    settings.device_required_limits.max_texture_dimension_2d = 4096;
    settings.device_label = Some("OxPlay native UI".into());
    Ok(settings)
}

fn validate_adapter(
    expected: wgpu::Backend,
    observed: wgpu::Backend,
    device_type: wgpu::DeviceType,
) -> Result<(), &'static str> {
    if expected != observed {
        return Err("Observed GPU backend does not match the platform native API");
    }
    if device_type == wgpu::DeviceType::Cpu {
        return Err("Native graphics requires a hardware GPU adapter");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_policy_uses_only_the_native_api() {
        assert_eq!(native_backend_for("macos"), Some(wgpu::Backend::Metal));
        assert_eq!(native_backend_for("windows"), Some(wgpu::Backend::Dx12));
        assert_eq!(native_backend_for("linux"), Some(wgpu::Backend::Vulkan));
        assert_eq!(native_backend_for("unknown"), None);
        let settings = native_settings().unwrap();
        let expected = native_backend_for(std::env::consts::OS).unwrap();
        assert_eq!(settings.backends, wgpu::Backends::from(expected));
        assert_eq!(settings.power_preference, wgpu::PowerPreference::LowPower);
        assert!(!settings.backends.intersects(wgpu::Backends::GL));
    }

    #[test]
    fn observed_adapter_cannot_silently_change_api_or_use_software() {
        for expected in [
            wgpu::Backend::Metal,
            wgpu::Backend::Dx12,
            wgpu::Backend::Vulkan,
        ] {
            for observed in [
                wgpu::Backend::Metal,
                wgpu::Backend::Dx12,
                wgpu::Backend::Vulkan,
                wgpu::Backend::Gl,
                wgpu::Backend::Noop,
            ] {
                assert_eq!(
                    validate_adapter(expected, observed, wgpu::DeviceType::IntegratedGpu).is_ok(),
                    expected == observed,
                );
            }
            assert!(validate_adapter(expected, expected, wgpu::DeviceType::Cpu).is_err());
            assert!(validate_adapter(expected, expected, wgpu::DeviceType::DiscreteGpu).is_ok());
        }
    }

    #[test]
    fn opengl_requires_an_explicit_choice() {
        assert_eq!(GraphicsBackend::default(), GraphicsBackend::Native);
        assert_eq!(
            GraphicsBackend::parse("native"),
            Ok(GraphicsBackend::Native)
        );
        assert_eq!(
            GraphicsBackend::parse("opengl"),
            Ok(GraphicsBackend::OpenGl)
        );
        assert!(GraphicsBackend::parse("auto").is_err());
    }
}
