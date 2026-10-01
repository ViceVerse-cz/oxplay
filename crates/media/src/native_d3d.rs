// SPDX-License-Identifier: GPL-3.0-or-later
//! D3D11VA + native mpv D3D11 output shared directly with the DX12 UI.
use crate::{MediaError, Result, Wake, checked, ffi};
use std::{
    ffi::c_void,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, GENERIC_ALL, HANDLE, HMODULE, WAIT_OBJECT_0},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1},
            Direct3D11::*,
            Direct3D12::*,
            Dxgi::Common::*,
            Dxgi::*,
        },
        System::Threading::{CreateEventW, WaitForSingleObject},
    },
    core::{Interface, PCWSTR},
};

fn native<T>(value: windows::core::Result<T>, operation: &str) -> Result<T> {
    value.map_err(|e| MediaError(format!("{operation}: {e}")))
}
struct NtHandle(HANDLE);
impl Drop for NtHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
#[repr(C)]
struct Init {
    device: *mut c_void,
}
#[repr(C)]
struct Framebuffer {
    texture: *mut c_void,
    width: i32,
    height: i32,
    wait_fence: *mut c_void,
    wait_value: u64,
    signal_fence: *mut c_void,
    signal_value: u64,
    result: *mut i32,
}
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(std::mem::size_of::<Init>() == 8);
    assert!(std::mem::size_of::<Framebuffer>() == 56);
    assert!(std::mem::offset_of!(Framebuffer, wait_fence) == 16);
    assert!(std::mem::offset_of!(Framebuffer, signal_value) == 40);
    assert!(std::mem::offset_of!(Framebuffer, result) == 48);
};
#[derive(Default)]
struct CapacityWait {
    pending: AtomicBool,
    failed: AtomicBool,
}
struct Fences {
    producer11: ID3D11Fence,
    producer12: ID3D12Fence,
    consumer11: ID3D11Fence,
    consumer12: ID3D12Fence,
}
struct TargetLease {
    native: ID3D11Texture2D,
    // Both interfaces and their shared fence leases survive the UI's use.
    _resource: ID3D12Resource,
    _fences: Arc<Fences>,
}
pub(super) struct Target {
    pub(super) texture: wgpu::Texture,
    lease: Arc<TargetLease>,
}
pub(super) struct Output {
    device: wgpu::Device,
    queue: wgpu::Queue,
    d3d11: ID3D11Device5,
    immediate: ID3D11DeviceContext4,
    d3d12: ID3D12Device,
    fences: Arc<Fences>,
    producer_value: u64,
    consumer_value: u64,
    failed: bool,
    targets: Vec<Weak<TargetLease>>,
    wake: Option<Arc<Wake>>,
    capacity: Arc<CapacityWait>,
}
impl Output {
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self> {
        if device.adapter_info().backend != wgpu::Backend::Dx12 {
            return Err(MediaError(
                "Native Windows video requires the DX12 UI backend".into(),
            ));
        }
        let d3d12 = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }
            .ok_or_else(|| MediaError("Cannot borrow the UI's DX12 device".into()))?
            .raw_device()
            .clone();
        unsafe {
            let factory: IDXGIFactory4 = native(
                CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)),
                "Create DXGI factory",
            )?;
            let adapter: IDXGIAdapter = native(
                factory.EnumAdapterByLuid(d3d12.GetAdapterLuid()),
                "Match media adapter to DX12 LUID",
            )?;
            let mut d3d11: Option<ID3D11Device> = None;
            let mut immediate: Option<ID3D11DeviceContext> = None;
            native(
                D3D11CreateDevice(
                    &adapter,
                    D3D_DRIVER_TYPE_UNKNOWN,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                    Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                    D3D11_SDK_VERSION,
                    Some(&mut d3d11),
                    None,
                    Some(&mut immediate),
                ),
                "Create same-adapter D3D11 media device",
            )?;
            let d3d11: ID3D11Device5 = native(
                d3d11
                    .ok_or_else(|| MediaError("D3D11 returned no device".into()))?
                    .cast(),
                "Require D3D11 shared fences",
            )?;
            let immediate: ID3D11DeviceContext4 = native(
                immediate
                    .ok_or_else(|| MediaError("D3D11 returned no context".into()))?
                    .cast(),
                "Require D3D11 GPU fence waits",
            )?;
            let multithread: ID3D11Multithread =
                native(immediate.cast(), "Enable decoder/render context protection")?;
            let _ = multithread.SetMultithreadProtected(true);

            let mut producer11: Option<ID3D11Fence> = None;
            native(
                d3d11.CreateFence(0, D3D11_FENCE_FLAG_SHARED, &mut producer11),
                "Create D3D11 producer fence",
            )?;
            let producer11 =
                producer11.ok_or_else(|| MediaError("D3D11 returned no fence".into()))?;
            let producer_handle = NtHandle(native(
                producer11.CreateSharedHandle(None, GENERIC_ALL.0, PCWSTR::null()),
                "Share producer fence",
            )?);
            let mut producer12: Option<ID3D12Fence> = None;
            native(
                d3d12.OpenSharedHandle(producer_handle.0, &mut producer12),
                "Open producer fence on DX12",
            )?;
            let producer12 =
                producer12.ok_or_else(|| MediaError("DX12 returned no producer fence".into()))?;

            let consumer12: ID3D12Fence = native(
                d3d12.CreateFence(0, D3D12_FENCE_FLAG_SHARED),
                "Create DX12 consumer fence",
            )?;
            let consumer_handle = NtHandle(native(
                d3d12.CreateSharedHandle(&consumer12, None, GENERIC_ALL.0, PCWSTR::null()),
                "Share consumer fence",
            )?);
            let mut consumer11: Option<ID3D11Fence> = None;
            native(
                d3d11.OpenSharedFence(consumer_handle.0, &mut consumer11),
                "Open consumer fence on D3D11",
            )?;
            let consumer11 =
                consumer11.ok_or_else(|| MediaError("D3D11 returned no consumer fence".into()))?;
            Ok(Self {
                device: device.clone(),
                queue: queue.clone(),
                d3d11,
                immediate,
                d3d12,
                fences: Arc::new(Fences {
                    producer11,
                    producer12,
                    consumer11,
                    consumer12,
                }),
                producer_value: 0,
                consumer_value: 0,
                failed: false,
                targets: Vec::new(),
                wake: None,
                capacity: Arc::default(),
            })
        }
    }
    pub(super) fn new_target(&mut self, width: u32, height: u32) -> Result<Target> {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let descriptor = wgpu::TextureDescriptor {
            label: Some("Native shared D3D11/DX12 video target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };
        unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE).0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: (D3D11_RESOURCE_MISC_SHARED_NTHANDLE | D3D11_RESOURCE_MISC_SHARED).0
                    as u32,
            };
            let mut texture11: Option<ID3D11Texture2D> = None;
            native(
                self.d3d11
                    .CreateTexture2D(&desc, None, Some(&mut texture11)),
                "Allocate persistent D3D11 shared texture",
            )?;
            let texture11 =
                texture11.ok_or_else(|| MediaError("D3D11 returned no shared texture".into()))?;
            let shared: IDXGIResource1 = native(texture11.cast(), "Get texture sharing interface")?;
            let handle = NtHandle(native(
                shared.CreateSharedHandle(
                    None,
                    (DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE).0,
                    PCWSTR::null(),
                ),
                "Share native media texture",
            )?);
            let mut resource: Option<ID3D12Resource> = None;
            native(
                self.d3d12.OpenSharedHandle(handle.0, &mut resource),
                "Import media texture into DX12",
            )?;
            let resource =
                resource.ok_or_else(|| MediaError("DX12 returned no shared texture".into()))?;
            let hal = wgpu::hal::dx12::Device::texture_from_raw(
                resource.clone(),
                descriptor.format,
                descriptor.dimension,
                size,
                1,
                1,
            );
            // A newly opened shared resource begins in COMMON (PRESENT in WGPU).
            let texture = self.device.create_texture_from_hal::<wgpu::hal::api::Dx12>(
                hal,
                &descriptor,
                wgpu::TextureUses::PRESENT,
            );
            let lease = Arc::new(TargetLease {
                native: texture11,
                _resource: resource,
                _fences: self.fences.clone(),
            });
            self.targets.retain(|target| target.strong_count() > 0);
            self.targets.push(Arc::downgrade(&lease));
            Ok(Target { texture, lease })
        }
    }
    pub(super) unsafe fn create_context(
        &mut self,
        player: *mut ffi::Handle,
        wake: &Arc<Wake>,
        context: &mut *mut ffi::RenderContext,
    ) -> Result<()> {
        self.wake = Some(wake.clone());
        let mut init = Init {
            device: self.d3d11.as_raw(),
        };
        let mut params = [
            ffi::RenderParam {
                kind: 1,
                data: c"d3d11".as_ptr().cast_mut().cast(),
            },
            ffi::RenderParam::new(25, &mut init),
            ffi::RenderParam::end(),
        ];
        unsafe {
            checked(
                ffi::mpv_render_context_create(context, player, params.as_mut_ptr()),
                "Create native D3D11 media renderer",
            )
        }
    }
    pub(super) fn busy(&self, _context: *mut ffi::RenderContext) -> Result<bool> {
        let completed = unsafe { self.fences.producer12.GetCompletedValue() };
        if self.failed || self.capacity.failed.load(Ordering::Acquire) || completed == u64::MAX {
            return Err(MediaError(
                "Native D3D11 media device lost or submission failed".into(),
            ));
        }
        let busy = self.producer_value.saturating_sub(completed) >= 2;
        if busy && !self.capacity.pending.swap(true, Ordering::AcqRel) {
            let fence = self.fences.producer12.clone();
            let value = self.producer_value - 1;
            let capacity = self.capacity.clone();
            let wake = self
                .wake
                .clone()
                .ok_or_else(|| MediaError("Native media wake is not attached".into()))?;
            // One finite event waiter only when all producer slots are busy.
            // It never blocks the UI or polls once per decoded frame.
            let spawned = std::thread::Builder::new()
                .name("native-video-capacity".into())
                .spawn(move || {
                    let ready = (|| -> windows::core::Result<bool> {
                        let event =
                            NtHandle(unsafe { CreateEventW(None, false, false, PCWSTR::null()) }?);
                        unsafe {
                            fence.SetEventOnCompletion(value, event.0)?;
                        }
                        Ok(unsafe { WaitForSingleObject(event.0, 5_000) } == WAIT_OBJECT_0)
                    })();
                    if !matches!(ready, Ok(true)) {
                        capacity.failed.store(true, Ordering::Release);
                    }
                    capacity.pending.store(false, Ordering::Release);
                    wake.due.store(true, Ordering::Release);
                    wake.notify();
                });
            if spawned.is_err() {
                self.capacity.pending.store(false, Ordering::Release);
                self.capacity.failed.store(true, Ordering::Release);
                return Err(MediaError(
                    "Cannot schedule native video capacity wake".into(),
                ));
            }
        }
        Ok(busy)
    }
    pub(super) fn render(
        &mut self,
        context: *mut ffi::RenderContext,
        target: &Target,
        width: u32,
        height: u32,
    ) -> Result<()> {
        self.consumer_value = self
            .consumer_value
            .checked_add(1)
            .ok_or_else(|| MediaError("Consumer fence exhausted".into()))?;
        self.producer_value = self
            .producer_value
            .checked_add(1)
            .ok_or_else(|| MediaError("Producer fence exhausted".into()))?;
        let mut before = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("DX12 release video texture to D3D11"),
            });
        before.transition_resources(
            std::iter::empty(),
            [wgpu::TextureTransition {
                texture: &target.texture,
                selector: None,
                state: wgpu::TextureUses::PRESENT,
            }]
            .into_iter(),
        );
        // Signals only after all prior Slint reads and the COMMON transition.
        {
            let queue = unsafe { self.queue.as_hal::<wgpu::hal::api::Dx12>() }
                .ok_or_else(|| MediaError("Cannot borrow UI DX12 queue".into()))?;
            queue.add_signal_fence(self.fences.consumer12.clone(), self.consumer_value);
        }
        self.queue.submit([before.finish()]);
        let mut result = -1i32;
        let mut framebuffer = Framebuffer {
            texture: target.lease.native.as_raw(),
            width: width as i32,
            height: height as i32,
            wait_fence: self.fences.consumer11.as_raw(),
            wait_value: self.consumer_value,
            signal_fence: self.fences.producer11.as_raw(),
            signal_value: self.producer_value,
            result: &mut result,
        };
        let mut flip = 0i32;
        let mut block = 0i32;
        let mut params = [
            ffi::RenderParam::new(26, &mut framebuffer),
            ffi::RenderParam::new(4, &mut flip),
            ffi::RenderParam::new(12, &mut block),
            ffi::RenderParam::end(),
        ];
        let code = unsafe { ffi::mpv_render_context_render(context, params.as_mut_ptr()) };
        if code < 0 || result < 0 {
            self.failed = true;
            return checked(
                if code < 0 { code } else { result },
                "Render native shared D3D11 frame",
            );
        }
        // D3D11 shared resources are returned to COMMON after its submission;
        // the tracker remains COMMON and can issue the matching read barrier.
        let mut after = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("DX12 acquire produced native video"),
            });
        after.transition_resources(
            std::iter::empty(),
            [wgpu::TextureTransition {
                texture: &target.texture,
                selector: None,
                state: wgpu::TextureUses::RESOURCE,
            }]
            .into_iter(),
        );
        {
            let queue = unsafe { self.queue.as_hal::<wgpu::hal::api::Dx12>() }
                .ok_or_else(|| MediaError("Cannot borrow UI DX12 queue".into()))?;
            queue.add_wait_fence(self.fences.producer12.clone(), self.producer_value);
        }
        self.queue.submit([after.finish()]);
        Ok(())
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        // Shutdown alone may wait; the per-frame path uses GPU waits only.
        // Bound the host wait and retain native leases if the device is hung.
        let drained = (|| -> Result<bool> {
            let value = self
                .consumer_value
                .checked_add(1)
                .ok_or_else(|| MediaError("Shutdown fence exhausted".into()))?;
            let queue = unsafe { self.queue.as_hal::<wgpu::hal::api::Dx12>() }
                .ok_or_else(|| MediaError("Missing DX12 shutdown queue".into()))?;
            queue.add_signal_fence(self.fences.consumer12.clone(), value);
            drop(queue);
            self.queue.submit(std::iter::empty());
            let event = NtHandle(native(
                unsafe { CreateEventW(None, false, false, PCWSTR::null()) },
                "Create shutdown fence event",
            )?);
            native(
                unsafe { self.fences.consumer12.SetEventOnCompletion(value, event.0) },
                "Wait for shared GPU leases",
            )?;
            Ok(unsafe { WaitForSingleObject(event.0, 5_000) } == WAIT_OBJECT_0)
        })();
        if self.failed || !matches!(drained, Ok(true)) {
            let targets: Vec<_> = self.targets.iter().filter_map(Weak::upgrade).collect();
            std::mem::forget((
                self.device.clone(),
                self.queue.clone(),
                self.d3d11.clone(),
                self.immediate.clone(),
                self.d3d12.clone(),
                self.fences.clone(),
                targets,
            ));
        }
    }
}
