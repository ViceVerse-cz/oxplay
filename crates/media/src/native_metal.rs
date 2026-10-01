// SPDX-License-Identifier: GPL-3.0-or-later
//! Same-device, same-command-queue native Metal media integration.
use crate::{MediaError, Result, Wake, checked, ffi};
use std::{ffi::c_void, sync::Arc};

#[repr(C)]
struct Init {
    layer: *mut c_void,
    device: *mut c_void,
    queue: *mut c_void,
    wake: unsafe extern "C" fn(*mut c_void),
    wake_context: *mut c_void,
}
#[repr(C)]
struct Framebuffer {
    texture: *mut c_void,
    width: i32,
    height: i32,
}
pub(super) struct Output {
    device: wgpu::Device,
    queue: wgpu::Queue,
}
pub(super) struct Target {
    pub(super) texture: wgpu::Texture,
}
impl Output {
    pub(super) fn new_target(&mut self, width: u32, height: u32) -> Result<Target> {
        Ok(Target {
            texture: self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Oxplay native Metal video target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            }),
        })
    }
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self> {
        if device.adapter_info().backend != wgpu::Backend::Metal {
            return Err(MediaError(
                "Native macOS media requires the Metal GPU backend".into(),
            ));
        }
        Ok(Self {
            device: device.clone(),
            queue: queue.clone(),
        })
    }
    pub(super) unsafe fn create_context(
        &mut self,
        player: *mut ffi::Handle,
        wake: &Arc<Wake>,
        context: &mut *mut ffi::RenderContext,
    ) -> Result<()> {
        let device = unsafe { self.device.as_hal::<wgpu::hal::api::Metal>() }
            .ok_or_else(|| MediaError("Cannot borrow this renderer's Metal device".into()))?;
        let queue = unsafe { self.queue.as_hal::<wgpu::hal::api::Metal>() }
            .ok_or_else(|| MediaError("Cannot borrow this renderer's Metal queue".into()))?;
        let mut init = Init {
            layer: std::ptr::null_mut(),
            device: std::ptr::from_ref(&**device.raw_device()).cast_mut().cast(),
            queue: std::ptr::from_ref(queue.as_raw()).cast_mut().cast(),
            wake: super::native_slot_wake,
            wake_context: Arc::as_ptr(wake).cast_mut().cast(),
        };
        let mut params = [
            ffi::RenderParam {
                kind: 1,
                data: c"metal".as_ptr().cast_mut().cast(),
            },
            ffi::RenderParam::new(23, &mut init),
            ffi::RenderParam::end(),
        ];
        unsafe {
            checked(
                ffi::mpv_render_context_create(context, player, params.as_mut_ptr()),
                "Create native Metal media renderer",
            )
        }
    }
    pub(super) fn busy(&self, context: *mut ffi::RenderContext) -> Result<bool> {
        let mut busy = 0i32;
        unsafe {
            checked(
                ffi::mpv_render_context_get_info(context, ffi::RenderParam::new(27, &mut busy)),
                "Check native Metal submission capacity",
            )?;
        }
        Ok(busy != 0)
    }
    pub(super) fn render(
        &mut self,
        context: *mut ffi::RenderContext,
        target: &Target,
        width: u32,
        height: u32,
    ) -> Result<()> {
        let texture = &target.texture;
        // Metal has no image layouts, and WGPU's Metal texture transitions
        // emit no GPU barriers. The shared MTLCommandQueue strictly orders
        // prior Slint reads, the native write committed before render returns,
        // and subsequent Slint reads. Keep WGPU tracking its actual uses;
        // empty boundary submissions add no synchronization. Target::new's
        // initial clear still marks this allocation initialized for WGPU.
        {
            let raw = unsafe { texture.as_hal::<wgpu::hal::api::Metal>() }
                .ok_or_else(|| MediaError("Native video target is not a Metal texture".into()))?;
            let mut framebuffer = Framebuffer {
                texture: std::ptr::from_ref(raw.raw_handle()).cast_mut().cast(),
                width: width as i32,
                height: height as i32,
            };
            let mut flip = 0i32;
            let mut block = 0i32;
            let mut params = [
                ffi::RenderParam::new(24, &mut framebuffer),
                ffi::RenderParam::new(4, &mut flip),
                ffi::RenderParam::new(12, &mut block),
                ffi::RenderParam::end(),
            ];
            unsafe {
                checked(
                    ffi::mpv_render_context_render(context, params.as_mut_ptr()),
                    "Render native Metal video",
                )?;
            }
        }
        Ok(())
    }
}
