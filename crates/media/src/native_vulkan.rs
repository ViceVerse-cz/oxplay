// SPDX-License-Identifier: GPL-3.0-or-later
//! Same-device Vulkan output. Timeline semaphores transfer ownership between
//! WGPU and libplacebo without waiting for video frames on the UI thread.
use crate::{MediaError, Result, Wake, checked, ffi};
use ash::{vk, vk::Handle};
use slint::wgpu_30::{WGPUConfiguration, WGPUSettings};
use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::{CString, c_char, c_void},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle, ThreadId},
    time::Duration,
};

const MAX_HOST_LEASES: usize = 32;
const RECOVERY_TIMEOUT: Duration = Duration::from_millis(500);
type FeatureSet = wgpu::hal::vulkan::PhysicalDeviceFeatures;

struct EnabledFeatures {
    hal: Box<FeatureSet>,
    host_reset: Box<vk::PhysicalDeviceHostQueryResetFeatures<'static>>,
}

/// Owns the logical device and its exact enabled feature chain. The HAL drop
/// callback retains this owner; the registry contains weak references only.
struct DeviceOwner {
    raw: ash::Device,
    _instance: wgpu::Instance,
    enabled: Mutex<EnabledFeatures>,
    family: u32,
    drm_node: Option<CString>,
}

static DEVICES: OnceLock<Mutex<HashMap<u64, Weak<DeviceOwner>>>> = OnceLock::new();
fn devices() -> &'static Mutex<HashMap<u64, Weak<DeviceOwner>>> {
    DEVICES.get_or_init(Default::default)
}

impl Drop for DeviceOwner {
    fn drop(&mut self) {
        if let Ok(mut registry) = devices().lock() {
            registry.remove(&self.raw.handle().as_raw());
        }
        // HAL owns synchronization through its device lifecycle; this callback
        // replaces only raw-device destruction, after HAL resources are gone.
        unsafe { self.raw.destroy_device(None) };
    }
}

/// Slint Automatic configuration cannot enable hostQueryReset at this pinned
/// WGPU revision. Build a device with HAL's requested features plus that Vulkan
/// feature, then give Slint the same instance/adapter/device/queue manually.
pub(super) async fn configuration(mut settings: WGPUSettings) -> Result<WGPUConfiguration> {
    settings.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: settings.backends,
        flags: settings.instance_flags,
        backend_options: settings.backend_options.clone(),
        memory_budget_thresholds: settings.instance_memory_budget_thresholds,
        display: None,
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: settings.power_preference,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        })
        .await
        .map_err(|_| MediaError("No native Vulkan GPU adapter is available".into()))?;
    let info = adapter.get_info();
    if info.backend != wgpu::Backend::Vulkan || info.device_type == wgpu::DeviceType::Cpu {
        return Err(MediaError(
            "Native Linux media requires a hardware Vulkan adapter".into(),
        ));
    }
    let mut requested = settings.device_required_features;
    let dma_buf = wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF;
    if adapter.features().contains(dma_buf) {
        requested |= dma_buf;
    }
    if !adapter.features().contains(requested) {
        return Err(MediaError(
            "Vulkan GPU lacks requested native rendering features".into(),
        ));
    }
    let limits = settings
        .device_required_limits
        .clone()
        .using_resolution(adapter.limits());
    let descriptor = wgpu::DeviceDescriptor {
        label: settings.device_label.as_deref(),
        required_features: requested,
        required_limits: limits.clone(),
        experimental_features: settings.device_experimental_features,
        memory_hints: settings.device_memory_hints.clone(),
        trace: wgpu::Trace::default(),
    };
    let (owner, open) = {
        let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or_else(|| MediaError("Cannot borrow the selected Vulkan adapter".into()))?;
        let raw_instance = hal.shared_instance().raw_instance();
        let physical = hal.raw_physical_device();
        let properties = unsafe { raw_instance.get_physical_device_properties(physical) };
        if properties.api_version < vk::API_VERSION_1_2
            || hal.shared_instance().instance_api_version() < vk::API_VERSION_1_2
        {
            return Err(MediaError(
                "Native Linux media requires Vulkan 1.2 or later".into(),
            ));
        }
        let mut timeline = vk::PhysicalDeviceTimelineSemaphoreFeatures::default();
        let mut host_reset = vk::PhysicalDeviceHostQueryResetFeatures::default();
        let mut supported = vk::PhysicalDeviceFeatures2::default()
            .push_next(&mut timeline)
            .push_next(&mut host_reset);
        unsafe { raw_instance.get_physical_device_features2(physical, &mut supported) };
        if timeline.timeline_semaphore == 0 || host_reset.host_query_reset == 0 {
            return Err(MediaError(
                "Vulkan GPU lacks required native media synchronization features".into(),
            ));
        }
        let families =
            unsafe { raw_instance.get_physical_device_queue_family_properties(physical) };
        let family = families
            .iter()
            .position(|f| {
                f.queue_count > 0
                    && f.queue_flags
                        .contains(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)
            })
            .ok_or_else(|| {
                MediaError("Vulkan GPU has no shared graphics and compute queue".into())
            })? as u32;
        let extensions = hal.required_device_extensions(requested);
        let names: Vec<_> = extensions.iter().map(|name| name.as_ptr()).collect();
        let queue_info = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&[1.0])];
        let mut enabled = EnabledFeatures {
            hal: Box::new(hal.physical_device_features(&extensions, requested)),
            host_reset: Box::new(
                vk::PhysicalDeviceHostQueryResetFeatures::default().host_query_reset(true),
            ),
        };
        let raw = {
            let EnabledFeatures {
                hal: features,
                host_reset,
            } = &mut enabled;
            let create = features
                .add_to_device_create(
                    vk::DeviceCreateInfo::default()
                        .queue_create_infos(&queue_info)
                        .enabled_extension_names(&names),
                )
                .push_next(&mut **host_reset);
            unsafe { raw_instance.create_device(physical, &create, None) }
                .map_err(|_| MediaError("Cannot create a Vulkan device for native media".into()))?
        };
        let drm_node = if requested.contains(dma_buf) {
            matching_drm_node(raw_instance, physical)
        } else {
            None
        };
        let owner = Arc::new(DeviceOwner {
            raw: raw.clone(),
            _instance: instance.clone(),
            enabled: Mutex::new(enabled),
            family,
            drm_node,
        });
        let retained = owner.clone();
        let open = unsafe {
            hal.device_from_raw(
                raw,
                Some(Box::new(move || drop(retained))),
                &extensions,
                requested,
                &limits,
                &settings.device_memory_hints,
                family,
                0,
            )
        }
        .map_err(|_| MediaError("Cannot import the native Vulkan device into WGPU".into()))?;
        (owner, open)
    };
    let (device, queue) =
        unsafe { adapter.create_device_from_hal::<wgpu::hal::api::Vulkan>(open, &descriptor) }
            .map_err(|_| MediaError("Cannot create WGPU handles for native Vulkan media".into()))?;
    devices()
        .lock()
        .map_err(|_| MediaError("Native Vulkan device registry is unavailable".into()))?
        .insert(owner.raw.handle().as_raw(), Arc::downgrade(&owner));
    Ok(WGPUConfiguration::Manual {
        instance,
        adapter,
        device,
        queue,
    })
}

fn matching_drm_node(instance: &ash::Instance, physical: vk::PhysicalDevice) -> Option<CString> {
    let extensions = unsafe { instance.enumerate_device_extension_properties(physical) }.ok()?;
    if !extensions
        .iter()
        .any(|ext| ext.extension_name_as_c_str().ok() == Some(ash::ext::physical_device_drm::NAME))
    {
        return None;
    }
    let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
    let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut drm);
    unsafe { instance.get_physical_device_properties2(physical, &mut properties) };
    if drm.has_render == 0 || drm.render_major < 0 || drm.render_minor < 0 {
        return None;
    }
    let uevent = std::fs::read_to_string(format!(
        "/sys/dev/char/{}:{}/uevent",
        drm.render_major, drm.render_minor
    ))
    .ok()?;
    let name = uevent
        .lines()
        .find_map(|line| line.strip_prefix("DEVNAME="))?;
    let suffix = name.strip_prefix("dri/renderD")?;
    if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // The C backend independently stat()s this node and checks its major/minor
    // against this same physical GPU before enabling direct VAAPI mapping.
    CString::new(format!("/dev/{name}")).ok()
}

#[repr(C)]
struct Init {
    instance: vk::Instance,
    get_instance_proc_addr: vk::PFN_vkGetInstanceProcAddr,
    physical: vk::PhysicalDevice,
    device: vk::Device,
    graphics_family: u32,
    graphics_index: u32,
    graphics_count: u32,
    compute_family: u32,
    compute_index: u32,
    compute_count: u32,
    transfer_family: u32,
    transfer_index: u32,
    transfer_count: u32,
    extensions: *const *const c_char,
    num_extensions: i32,
    features: *const vk::PhysicalDeviceFeatures2<'static>,
    queue_context: *mut c_void,
    lock_queue: unsafe extern "C" fn(*mut c_void, u32, u32),
    unlock_queue: unsafe extern "C" fn(*mut c_void, u32, u32),
    drm_render_node: *const c_char,
}

#[repr(C)]
struct Framebuffer {
    image: vk::Image,
    format: vk::Format,
    usage: vk::ImageUsageFlags,
    layout: vk::ImageLayout,
    width: i32,
    height: i32,
    target_layout: vk::ImageLayout,
    wait_semaphore: vk::Semaphore,
    wait_value: u64,
    signal_semaphore: vk::Semaphore,
    signal_value: u64,
    result: *mut i32,
}

// Match OXPLAY_NATIVE_RENDER_ABI 1's 64-bit C layout, including padding after
// the queue-family triples and extension count. Vulkan handles are opaque;
// accidentally flattening the wrong prefix would corrupt device ownership.
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(std::mem::size_of::<Init>() == 128);
    assert!(std::mem::offset_of!(Init, graphics_family) == 32);
    assert!(std::mem::offset_of!(Init, extensions) == 72);
    assert!(std::mem::offset_of!(Init, features) == 88);
    assert!(std::mem::offset_of!(Init, drm_render_node) == 120);
    assert!(std::mem::size_of::<Framebuffer>() == 72);
    assert!(std::mem::offset_of!(Framebuffer, wait_semaphore) == 32);
    assert!(std::mem::offset_of!(Framebuffer, signal_value) == 56);
    assert!(std::mem::offset_of!(Framebuffer, result) == 64);
};

/// All Slint/WGPU submits and native libplacebo submits run on this same UI
/// thread. The pinned libplacebo Vulkan backend creates no submission thread;
/// decoder VAAPI work uses its own VAAPI queues, never this VkQueue.
struct QueueOwner {
    thread: ThreadId,
    family: u32,
    locked: AtomicBool,
}
unsafe extern "C" fn lock_queue(context: *mut c_void, family: u32, index: u32) {
    let owner = unsafe { &*context.cast::<QueueOwner>() };
    if thread::current().id() != owner.thread
        || family != owner.family
        || index != 0
        || owner
            .locked
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        // Violating VkQueue external synchronization is not recoverable. Never
        // unwind into C or pretend a foreign-thread submit was synchronized.
        std::process::abort();
    }
}
unsafe extern "C" fn unlock_queue(context: *mut c_void, family: u32, index: u32) {
    let owner = unsafe { &*context.cast::<QueueOwner>() };
    if thread::current().id() != owner.thread
        || family != owner.family
        || index != 0
        || !owner.locked.swap(false, Ordering::AcqRel)
    {
        std::process::abort();
    }
}

struct Timelines {
    owner: Arc<DeviceOwner>,
    ready: vk::Semaphore,
    done: vk::Semaphore,
}
impl Drop for Timelines {
    fn drop(&mut self) {
        unsafe {
            self.owner.raw.destroy_semaphore(self.ready, None);
            self.owner.raw.destroy_semaphore(self.done, None);
        }
    }
}

pub(super) struct Target {
    pub(super) texture: wgpu::Texture,
}
struct Lease {
    finished: Arc<AtomicBool>,
    submission: wgpu::SubmissionIndex,
}
struct Recovery {
    submission: wgpu::SubmissionIndex,
    wake: Arc<Wake>,
}

pub(super) struct Output {
    device: wgpu::Device,
    queue: wgpu::Queue,
    owner: Arc<DeviceOwner>,
    queue_owner: Box<QueueOwner>,
    timelines: Arc<Timelines>,
    value: u64,
    leases: RefCell<Vec<Lease>>,
    failed_targets: Vec<wgpu::Texture>,
    submission_failed: bool,
    wake: Option<Arc<Wake>>,
    recovery_jobs: Option<SyncSender<Recovery>>,
    recovery_worker: Option<JoinHandle<()>>,
    recovery_pending: Arc<AtomicBool>,
    recovery_failed: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
}

impl Output {
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self> {
        if device.adapter_info().backend != wgpu::Backend::Vulkan {
            return Err(MediaError(
                "Native Linux media requires the Vulkan GPU backend".into(),
            ));
        }
        let raw = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or_else(|| MediaError("Cannot borrow the renderer's Vulkan device".into()))?;
        let owner = devices()
            .lock()
            .map_err(|_| MediaError("Native Vulkan device registry is unavailable".into()))?
            .get(&raw.raw_device().handle().as_raw())
            .and_then(Weak::upgrade)
            .ok_or_else(|| {
                MediaError(
                    "Native Vulkan media requires the manually configured shared GPU device".into(),
                )
            })?;
        let raw_queue = unsafe { queue.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or_else(|| MediaError("Cannot borrow the renderer's Vulkan queue".into()))?;
        if raw.queue_family_index() != owner.family
            || raw.queue_index() != 0
            || raw_queue.as_raw() != raw.raw_queue()
            || raw_queue.raw_device().handle() != owner.raw.handle()
        {
            return Err(MediaError(
                "Native Vulkan media must share the renderer's graphics queue".into(),
            ));
        }
        let mut timeline_info = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(0);
        let semaphore_info = vk::SemaphoreCreateInfo::default().push_next(&mut timeline_info);
        let ready = unsafe { owner.raw.create_semaphore(&semaphore_info, None) }
            .map_err(|_| MediaError("Cannot create Vulkan media producer semaphore".into()))?;
        let done = match unsafe { owner.raw.create_semaphore(&semaphore_info, None) } {
            Ok(done) => done,
            Err(_) => {
                unsafe { owner.raw.destroy_semaphore(ready, None) };
                return Err(MediaError(
                    "Cannot create Vulkan media consumer semaphore".into(),
                ));
            }
        };
        let timelines = Arc::new(Timelines {
            owner: owner.clone(),
            ready,
            done,
        });
        let (sender, receiver) = mpsc::sync_channel(1);
        let recovery_pending = Arc::new(AtomicBool::new(false));
        let recovery_failed = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        let worker_device = device.clone();
        let worker_pending = recovery_pending.clone();
        let worker_failed = recovery_failed.clone();
        let worker_alive = alive.clone();
        let recovery_worker = thread::Builder::new()
            .name("oxplay-vulkan-capacity".into())
            .spawn(move || {
                recovery_worker(
                    worker_device,
                    receiver,
                    worker_pending,
                    worker_failed,
                    worker_alive,
                )
            })
            .map_err(|_| MediaError("Cannot start Vulkan submission capacity worker".into()))?;
        Ok(Self {
            device: device.clone(),
            queue: queue.clone(),
            queue_owner: Box::new(QueueOwner {
                thread: thread::current().id(),
                family: owner.family,
                locked: AtomicBool::new(false),
            }),
            owner,
            timelines,
            value: 0,
            leases: RefCell::new(Vec::with_capacity(MAX_HOST_LEASES)),
            failed_targets: Vec::new(),
            submission_failed: false,
            wake: None,
            recovery_jobs: Some(sender),
            recovery_worker: Some(recovery_worker),
            recovery_pending,
            recovery_failed,
            alive,
        })
    }

    pub(super) unsafe fn create_context(
        &mut self,
        player: *mut ffi::Handle,
        wake: &Arc<Wake>,
        context: &mut *mut ffi::RenderContext,
    ) -> Result<()> {
        let raw = unsafe { self.device.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or_else(|| MediaError("Cannot borrow native Vulkan context handles".into()))?;
        let shared = raw.shared_instance();
        let extensions: Vec<_> = raw
            .enabled_device_extensions()
            .iter()
            .map(|name| name.as_ptr())
            .collect();
        let mut enabled = self
            .owner
            .enabled
            .lock()
            .map_err(|_| MediaError("Native Vulkan feature metadata is unavailable".into()))?;
        let EnabledFeatures { hal, host_reset } = &mut *enabled;
        let core = hal.get_core();
        let info = hal
            .add_to_device_create(vk::DeviceCreateInfo::default())
            .push_next(&mut **host_reset);
        let mut features = vk::PhysicalDeviceFeatures2::default().features(core);
        features.p_next = info.p_next.cast_mut();
        let mut init = Init {
            instance: shared.raw_instance().handle(),
            get_instance_proc_addr: shared.entry().static_fn().get_instance_proc_addr,
            physical: raw.raw_physical_device(),
            device: raw.raw_device().handle(),
            graphics_family: raw.queue_family_index(),
            graphics_index: 0,
            graphics_count: 1,
            compute_family: 0,
            compute_index: 0,
            compute_count: 0,
            transfer_family: 0,
            transfer_index: 0,
            transfer_count: 0,
            extensions: extensions.as_ptr(),
            num_extensions: extensions.len() as i32,
            features: &features,
            queue_context: std::ptr::from_mut(&mut *self.queue_owner).cast(),
            lock_queue,
            unlock_queue,
            drm_render_node: self
                .owner
                .drm_node
                .as_ref()
                .map_or(std::ptr::null(), |node| node.as_ptr()),
        };
        let mut params = [
            ffi::RenderParam {
                kind: 1,
                data: c"vulkan".as_ptr().cast_mut().cast(),
            },
            ffi::RenderParam::new(21, &mut init),
            ffi::RenderParam::end(),
        ];
        unsafe {
            checked(
                ffi::mpv_render_context_create(context, player, params.as_mut_ptr()),
                "Create native Vulkan media renderer",
            )?;
        }
        self.wake = Some(wake.clone());
        Ok(())
    }

    pub(super) fn new_target(&mut self, width: u32, height: u32) -> Result<Target> {
        if width == 0 || height == 0 || width > 4096 || height > 4096 {
            return Err(MediaError(
                "Native Vulkan target dimensions are invalid".into(),
            ));
        }
        Ok(Target {
            texture: self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("OxPlay native Vulkan video target"),
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
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }),
        })
    }

    pub(super) fn busy(&self, context: *mut ffi::RenderContext) -> Result<bool> {
        if self.submission_failed {
            return Err(MediaError(
                "Native Vulkan renderer rejected its previous submission".into(),
            ));
        }
        self.device.poll(wgpu::PollType::Poll).map_err(|_| {
            MediaError("Native Vulkan device could not collect completed submissions".into())
        })?;
        self.leases
            .borrow_mut()
            .retain(|lease| !lease.finished.load(Ordering::Acquire));
        if self.recovery_failed.load(Ordering::Acquire) {
            return Err(MediaError(
                "Native Vulkan GPU did not recover submission capacity".into(),
            ));
        }
        let mut busy = 0i32;
        unsafe {
            checked(
                ffi::mpv_render_context_get_info(context, ffi::RenderParam::new(27, &mut busy)),
                "Check native Vulkan submission capacity",
            )?;
        }
        let capped = self.leases.borrow().len() >= MAX_HOST_LEASES;
        if capped || busy != 0 {
            self.request_capacity_wake()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn request_capacity_wake(&self) -> Result<()> {
        let submission = self
            .leases
            .borrow()
            .first()
            .map(|lease| lease.submission.clone())
            .ok_or_else(|| {
                MediaError("Native Vulkan renderer is busy without a recoverable submission".into())
            })?;
        if self.recovery_pending.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let job = Recovery {
            submission,
            wake: self
                .wake
                .as_ref()
                .ok_or_else(|| MediaError("Native Vulkan completion wake is unavailable".into()))?
                .clone(),
        };
        if self
            .recovery_jobs
            .as_ref()
            .is_none_or(|sender| sender.try_send(job).is_err())
        {
            self.recovery_pending.store(false, Ordering::Release);
            return Err(MediaError(
                "Native Vulkan submission recovery worker is unavailable".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn render(
        &mut self,
        context: *mut ffi::RenderContext,
        target: &Target,
        width: u32,
        height: u32,
    ) -> Result<()> {
        if self.submission_failed {
            return Err(MediaError(
                "Native Vulkan renderer rejected its previous submission".into(),
            ));
        }
        let texture = &target.texture;
        if texture.width() != width || texture.height() != height {
            return Err(MediaError(
                "Native Vulkan output dimensions do not match its target".into(),
            ));
        }
        let value = self
            .value
            .checked_add(1)
            .ok_or_else(|| MediaError("Native Vulkan timeline exhausted".into()))?;
        self.value = value;
        let mut before = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Native Vulkan producer boundary"),
            });
        // Both APIs agree on sampled layout before and after native rendering.
        // PL performs its internal attachment transitions and returns sampled.
        before.transition_resources(
            std::iter::empty(),
            [wgpu::TextureTransition {
                texture,
                selector: None,
                state: wgpu::TextureUses::RESOURCE,
            }]
            .into_iter(),
        );
        {
            let queue = unsafe { self.queue.as_hal::<wgpu::hal::api::Vulkan>() }
                .ok_or_else(|| MediaError("Native Vulkan producer queue is unavailable".into()))?;
            queue.add_signal_semaphore(self.timelines.ready, Some(value));
        }
        self.queue.submit([before.finish()]);
        let retained_ready = self.timelines.clone();
        let retained_texture = texture.clone();
        self.queue
            .on_submitted_work_done(move || drop((retained_ready, retained_texture)));
        let mut enqueue_result = -1i32;
        let rendered = {
            let raw = unsafe { texture.as_hal::<wgpu::hal::api::Vulkan>() }
                .ok_or_else(|| MediaError("Native media target is not a Vulkan image".into()))?;
            let mut framebuffer = Framebuffer {
                image: unsafe { raw.raw_handle() },
                format: vk::Format::R8G8B8A8_UNORM,
                usage: vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::TRANSFER_DST
                    | vk::ImageUsageFlags::SAMPLED,
                layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                width: width as i32,
                height: height as i32,
                target_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                wait_semaphore: self.timelines.ready,
                wait_value: value,
                signal_semaphore: self.timelines.done,
                signal_value: value,
                result: &mut enqueue_result,
            };
            let mut flip = 0i32;
            let mut block = 0i32;
            let mut params = [
                ffi::RenderParam::new(22, &mut framebuffer),
                ffi::RenderParam::new(4, &mut flip),
                ffi::RenderParam::new(12, &mut block),
                ffi::RenderParam::end(),
            ];
            unsafe { ffi::mpv_render_context_render(context, params.as_mut_ptr()) }
        };
        if rendered < 0 || enqueue_result < 0 {
            // Do not enqueue a wait on a signal the producer failed to submit.
            // Keep failed targets until mpv context teardown drains native work.
            self.failed_targets.push(texture.clone());
            self.submission_failed = true;
            checked(rendered, "Render native Vulkan video")?;
            checked(enqueue_result, "Submit native Vulkan video")?;
        }
        let mut after = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Native Vulkan consumer boundary"),
            });
        after.transition_resources(
            std::iter::empty(),
            [wgpu::TextureTransition {
                texture,
                selector: None,
                state: wgpu::TextureUses::RESOURCE,
            }]
            .into_iter(),
        );
        {
            let queue = unsafe { self.queue.as_hal::<wgpu::hal::api::Vulkan>() }
                .ok_or_else(|| MediaError("Native Vulkan consumer queue is unavailable".into()))?;
            queue.add_wait_semaphore(
                self.timelines.done,
                Some(value),
                vk::PipelineStageFlags::TOP_OF_PIPE,
            );
        }
        let submission = self.queue.submit([after.finish()]);
        let finished = Arc::new(AtomicBool::new(false));
        let callback_finished = finished.clone();
        let retained = (texture.clone(), self.timelines.clone());
        self.queue.on_submitted_work_done(move || {
            drop(retained);
            callback_finished.store(true, Ordering::Release);
        });
        self.leases.borrow_mut().push(Lease {
            finished,
            submission,
        });
        Ok(())
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        self.recovery_jobs.take();
        if let Some(worker) = self.recovery_worker.take() {
            let _ = worker.join();
        }
        // The common presenter frees the mpv context before Output is dropped.
        // Native GPU work is drained then, and pending WGPU submissions retain
        // their texture/timeline leases through the completion callbacks above.
    }
}

fn recovery_worker(
    device: wgpu::Device,
    jobs: Receiver<Recovery>,
    pending: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
) {
    while let Ok(job) = jobs.recv() {
        // This is exceptional bounded capacity recovery, not a per-frame wait.
        if device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(job.submission),
                timeout: Some(RECOVERY_TIMEOUT),
            })
            .is_err()
        {
            failed.store(true, Ordering::Release);
        }
        pending.store(false, Ordering::Release);
        if alive.load(Ordering::Acquire) {
            job.wake.due.store(true, Ordering::Release);
            job.wake.notify();
        }
    }
}
