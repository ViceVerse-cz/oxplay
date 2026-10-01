// SPDX-License-Identifier: GPL-3.0-or-later
//! Native GPU ambient summary. Only the final 16×9 RGBA8 cells leave the GPU.
//! A single worker advances each asynchronous map with a bounded wait for its
//! submission; the UI never waits and paused first frames need no polling timer.
use crate::{
    AMBIENT_CELLS, AmbientSample, AmbientStats, MediaError, Result, ambient::SAMPLE_INTERVAL,
};
use slint::wgpu_30::wgpu;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const BYTES: u64 = (AMBIENT_CELLS * 4) as u64;
const GPU_WAIT: Duration = Duration::from_millis(500);
const CALLBACK_WAIT: Duration = Duration::from_millis(100);

// Sample the same 128×72 grid as the GL reducer, averaging 8×8 samples per
// output cell. Bilinear taps clamp to the content rectangle to exclude bars.
// RGBA8Unorm keeps encoded SDR values: an sRGB view would decode the samples.
const SHADER: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> rect: vec4<u32>;
@group(0) @binding(2) var<storage, read_write> cells: array<u32, 144>;

fn bilinear(pixel: vec2<f32>) -> vec4<f32> {
    let lower = vec2<i32>(rect.xy);
    let upper = vec2<i32>(rect.zw) - vec2<i32>(1);
    let p = clamp(pixel, vec2<f32>(lower), vec2<f32>(upper));
    let lo = vec2<i32>(floor(p));
    let hi = min(lo + vec2<i32>(1), upper);
    let weight = fract(p);
    let a = textureLoad(source, lo, 0);
    let b = textureLoad(source, vec2<i32>(hi.x, lo.y), 0);
    let c = textureLoad(source, vec2<i32>(lo.x, hi.y), 0);
    let d = textureLoad(source, hi, 0);
    return mix(mix(a, b, weight.x), mix(c, d, weight.x), weight.y);
}

@compute @workgroup_size(8, 8, 1)
fn reduce(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= 16u || id.y >= 9u { return; }
    let extent = vec2<f32>(rect.zw - rect.xy);
    var total = vec4<f32>(0.0);
    for (var sy = 0u; sy < 8u; sy += 1u) {
        for (var sx = 0u; sx < 8u; sx += 1u) {
            let grid = vec2<f32>(id.xy * 8u + vec2<u32>(sx, sy)) + vec2<f32>(0.5);
            let pixel = vec2<f32>(rect.xy) + grid * extent / vec2<f32>(128.0, 72.0)
                - vec2<f32>(0.5);
            total += bilinear(pixel);
        }
    }
    cells[id.y * 16u + id.x] = pack4x8unorm(total / 64.0);
}
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReadbackToken {
    generation: u64,
    load: u64,
}

struct ReadbackJob {
    token: ReadbackToken,
    buffer: wgpu::Buffer,
    submission: wgpu::SubmissionIndex,
}

struct Completion {
    token: ReadbackToken,
    result: std::result::Result<Option<[[u8; 4]; AMBIENT_CELLS]>, &'static str>,
}

/// The logical cancellation state is independent of GPU ownership. Cancelled
/// work still occupies the one buffer until the worker unmaps and retires it.
#[derive(Default)]
struct SamplingState {
    enabled: bool,
    generation: u64,
    load: Option<u64>,
    last: Option<Instant>,
    rearmed: bool,
    pending: Option<ReadbackToken>,
}

impl SamplingState {
    fn invalidate(&mut self, shared: &AtomicU64) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("ambient generation overflow");
        shared.store(
            if self.enabled { self.generation } else { 0 },
            Ordering::Release,
        );
    }

    fn observe_load(&mut self, load: u64, shared: &AtomicU64) -> bool {
        if self.load == Some(load) {
            return false;
        }
        self.load = Some(load);
        self.last = None;
        self.rearmed = true;
        self.invalidate(shared);
        true
    }

    fn due(&self, now: Instant) -> bool {
        self.enabled
            && self.load.is_some()
            && self.pending.is_none()
            && (self.rearmed
                || self
                    .last
                    .is_none_or(|at| now.saturating_duration_since(at) >= SAMPLE_INTERVAL))
    }

    fn current(&self, token: ReadbackToken) -> bool {
        self.enabled && self.generation == token.generation && self.load == Some(token.load)
    }
}

pub(crate) struct WgpuAmbientSampler {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    rect_buffer: wgpu::Buffer,
    cells_buffer: wgpu::Buffer,
    readback: wgpu::Buffer,
    state: SamplingState,
    generation: Arc<AtomicU64>,
    completion: Arc<Mutex<Option<Completion>>>,
    jobs: Option<SyncSender<ReadbackJob>>,
    worker: Option<JoinHandle<()>>,
    ready: Option<AmbientSample>,
    stats: AmbientStats,
    measure: bool,
}

impl WgpuAmbientSampler {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        wake: Arc<dyn Fn() + Send + Sync>,
        measure: bool,
    ) -> Result<Self> {
        let limits = device.limits();
        if limits.max_compute_invocations_per_workgroup < 64
            || limits.max_compute_workgroup_size_x < 8
            || limits.max_compute_workgroup_size_y < 8
            || limits.max_compute_workgroup_size_z < 1
            || limits.max_compute_workgroups_per_dimension < 2
            || limits.max_storage_buffers_per_shader_stage < 1
            || limits.max_storage_buffer_binding_size < BYTES
        {
            return Err(MediaError(
                "Native ambient sampling requires compute device limits".into(),
            ));
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("OxPlay ambient reduction"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("OxPlay ambient reduction"),
            layout: None,
            module: &shader,
            entry_point: Some("reduce"),
            compilation_options: Default::default(),
            cache: None,
        });
        let rect_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OxPlay ambient content rectangle"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cells_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OxPlay ambient 16x9 cells"),
            size: BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("OxPlay ambient 576-byte readback"),
            size: BYTES,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let generation = Arc::new(AtomicU64::new(0));
        let completion = Arc::new(Mutex::new(None));
        let (jobs, receiver) = mpsc::sync_channel(1);
        let worker_device = device.clone();
        let worker_generation = generation.clone();
        let worker_completion = completion.clone();
        let worker = thread::Builder::new()
            .name("oxplay-ambient-readback".into())
            .spawn(move || {
                readback_worker(
                    worker_device,
                    receiver,
                    worker_generation,
                    worker_completion,
                    wake,
                )
            })
            .map_err(|_| MediaError("Cannot start native ambient readback worker".into()))?;
        Ok(Self {
            device: device.clone(),
            queue: queue.clone(),
            pipeline,
            rect_buffer,
            cells_buffer,
            readback,
            state: SamplingState::default(),
            generation,
            completion,
            jobs: Some(jobs),
            worker: Some(worker),
            ready: None,
            stats: AmbientStats::default(),
            measure,
        })
    }

    pub(crate) fn stats(&self) -> AmbientStats {
        self.stats
    }

    pub(crate) fn take(&mut self) -> Option<AmbientSample> {
        self.ready.take()
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        if self.state.enabled == enabled {
            return;
        }
        self.state.enabled = enabled;
        self.state.invalidate(&self.generation);
        self.state.rearmed = enabled;
        self.ready = None;
    }

    /// Observe replacements even while the previous map is pending. That map
    /// must retire before another submission can use the staging buffer.
    pub(crate) fn due(&mut self, now: Instant, load: u64) -> bool {
        self.observe_load(load);
        !self.stats.failed && self.jobs.is_some() && self.state.due(now)
    }

    fn observe_load(&mut self, load: u64) {
        if self.state.observe_load(load, &self.generation) {
            self.ready = None;
        }
    }

    /// Call only after the published frame's producer commands have been
    /// submitted on this same queue (or after a native cross-queue fence).
    /// A new enabled load samples immediately; unchanged loads run at <=4 Hz.
    pub(crate) fn sample(
        &mut self,
        texture: &wgpu::Texture,
        rect: [i32; 4],
        load: u64,
        now: Instant,
    ) -> Result<()> {
        if !self.due(now, load) {
            return Ok(());
        }
        let started = self.measure.then(Instant::now);
        if texture.format() != wgpu::TextureFormat::Rgba8Unorm
            || texture.dimension() != wgpu::TextureDimension::D2
            || texture.depth_or_array_layers() != 1
            || texture.sample_count() != 1
            || !texture
                .usage()
                .contains(wgpu::TextureUsages::TEXTURE_BINDING)
        {
            self.stats.failed = true;
            return Err(MediaError(
                "Native ambient source must be a sampleable RGBA8Unorm 2D texture".into(),
            ));
        }
        let rect = match checked_rect(texture.width(), texture.height(), rect) {
            Ok(rect) => rect,
            Err(error) => {
                self.stats.failed = true;
                return Err(error);
            }
        };
        let mut bytes = [0u8; 16];
        for (field, value) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(rect) {
            field.copy_from_slice(&value.to_ne_bytes());
        }
        self.queue.write_buffer(&self.rect_buffer, 0, &bytes);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("OxPlay ambient source"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.rect_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.cells_buffer.as_entire_binding(),
                },
            ],
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("OxPlay ambient summary"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("OxPlay ambient 8x8 averages"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(2, 2, 1);
        }
        encoder.copy_buffer_to_buffer(&self.cells_buffer, 0, &self.readback, 0, BYTES);
        let submission = self.queue.submit([encoder.finish()]);
        let token = ReadbackToken {
            generation: self.state.generation,
            load,
        };
        self.state.pending = Some(token);
        self.state.last = Some(now);
        self.state.rearmed = false;
        self.stats.issued += 1;
        // There is only one in-flight readback; a full channel means ownership
        // accounting failed. Never block the UI trying to dispatch GPU work.
        let job = ReadbackJob {
            token,
            buffer: self.readback.clone(),
            submission,
        };
        let dispatched = self
            .jobs
            .as_ref()
            .is_some_and(|sender| sender.try_send(job).is_ok());
        if let Some(start) = started {
            self.stats.cpu_us += start.elapsed().as_micros() as u64;
        }
        if !dispatched {
            self.state.pending = None;
            self.stats.discarded += 1;
            self.stats.failed = true;
            return Err(MediaError(
                "Native ambient readback worker is unavailable".into(),
            ));
        }
        Ok(())
    }

    /// Collect only the 576-byte worker result. The active load is checked
    /// before delivery, including paused loads with no later media frame.
    pub(crate) fn collect(&mut self, active_load: u64) -> Result<()> {
        self.observe_load(active_load);
        // A completion always owns the single pending job. Disabled/idle
        // frames therefore need no cross-thread mutex or timing work.
        if self.state.pending.is_none() {
            return Ok(());
        }
        let started = self.measure.then(Instant::now);
        let completion = self
            .completion
            .lock()
            .map_err(|_| MediaError("Native ambient completion is unavailable".into()))?
            .take();
        let result = if let Some(completion) = completion {
            debug_assert_eq!(self.state.pending, Some(completion.token));
            self.state.pending = None;
            if !self.state.current(completion.token) {
                self.stats.discarded += 1;
                Ok(())
            } else {
                match completion.result {
                    Ok(Some(pixels)) => {
                        self.ready = Some(AmbientSample {
                            load_request_id: completion.token.load,
                            pixels,
                        });
                        self.stats.delivered += 1;
                        Ok(())
                    }
                    Ok(None) => {
                        self.stats.discarded += 1;
                        Ok(())
                    }
                    Err(message) => {
                        self.stats.discarded += 1;
                        self.stats.failed = true;
                        Err(MediaError(message.into()))
                    }
                }
            }
        } else {
            Ok(())
        };
        if let Some(start) = started {
            self.stats.cpu_us += start.elapsed().as_micros() as u64;
        }
        result
    }

    /// Shutdown may wait only for the one worker's finite submission wait.
    /// It retains its device and buffer handles until the map has been unmapped.
    pub(crate) fn delete(&mut self) {
        self.state.enabled = false;
        self.state.invalidate(&self.generation);
        self.ready = None;
        self.jobs.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if self.state.pending.take().is_some() {
            self.stats.discarded += 1;
        }
        if let Ok(mut completion) = self.completion.lock() {
            completion.take();
        }
    }
}

impl Drop for WgpuAmbientSampler {
    fn drop(&mut self) {
        self.delete();
    }
}

fn checked_rect(width: u32, height: u32, rect: [i32; 4]) -> Result<[u32; 4]> {
    let [x0, y0, x1, y1] = rect;
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || x0 < 0
        || y0 < 0
        || x1 <= x0
        || y1 <= y0
        || x1 as u32 > width
        || y1 as u32 > height
    {
        return Err(MediaError(
            "Native ambient content rectangle is outside its target".into(),
        ));
    }
    Ok([x0 as u32, y0 as u32, x1 as u32, y1 as u32])
}

fn readback_worker(
    device: wgpu::Device,
    jobs: Receiver<ReadbackJob>,
    generation: Arc<AtomicU64>,
    completion: Arc<Mutex<Option<Completion>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    while let Ok(job) = jobs.recv() {
        let (mapped_sender, mapped_receiver) = mpsc::sync_channel(1);
        job.buffer
            .slice(..BYTES)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = mapped_sender.try_send(result);
            });
        let polled = device.poll(wgpu::PollType::Wait {
            submission_index: Some(job.submission),
            timeout: Some(GPU_WAIT),
        });
        let result = if polled.is_err() {
            Err("Native ambient GPU submission did not complete within its deadline")
        } else {
            match mapped_receiver.recv_timeout(CALLBACK_WAIT) {
                Ok(Ok(())) if generation.load(Ordering::Acquire) == job.token.generation => {
                    match job.buffer.slice(..BYTES).get_mapped_range() {
                        Ok(mapped) => {
                            let mut pixels = [[0u8; 4]; AMBIENT_CELLS];
                            for (pixel, bytes) in pixels.iter_mut().zip(mapped.as_chunks::<4>().0) {
                                pixel.copy_from_slice(bytes);
                            }
                            drop(mapped);
                            Ok(Some(pixels))
                        }
                        Err(_) => Err("Native ambient GPU mapping could not be read"),
                    }
                }
                Ok(Ok(())) => Ok(None),
                _ => Err("Native ambient GPU buffer could not be mapped"),
            }
        };
        // Includes the timeout/error path: cancel any mapping request before
        // the UI can issue the next copy to the same buffer.
        job.buffer.unmap();
        let Ok(mut slot) = completion.lock() else {
            break;
        };
        *slot = Some(Completion {
            token: job.token,
            result,
        });
        drop(slot);
        // A cancelled load still retires ownership. Wake a currently enabled
        // replacement to sample its paused frame, but never wake disabled or
        // torn-down sampling. The UI verifies the completion token separately.
        if generation.load(Ordering::Acquire) != 0 {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| wake()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AMBIENT_COLUMNS, AMBIENT_ROWS};

    #[test]
    fn shader_validates_without_gpu_or_extra_features() {
        let module = wgpu::naga::front::wgsl::parse_str(SHADER).expect("valid WGSL");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .expect("portable native ambient shader");
        assert_eq!((AMBIENT_COLUMNS, AMBIENT_ROWS, BYTES), (16, 9, 576));
    }

    #[test]
    fn cancellation_keeps_buffer_owned_until_old_work_retires() {
        let shared = AtomicU64::new(0);
        let mut state = SamplingState {
            enabled: true,
            ..Default::default()
        };
        let now = Instant::now();
        state.observe_load(10, &shared);
        assert!(state.due(now));
        let old = ReadbackToken {
            generation: state.generation,
            load: 10,
        };
        state.pending = Some(old);
        state.last = Some(now);
        state.rearmed = false;
        state.observe_load(11, &shared);
        assert!(!state.current(old));
        assert!(!state.due(now + Duration::from_secs(1)));
        assert_eq!(state.pending, Some(old));
        state.pending = None;
        assert!(state.due(now));
        let current = ReadbackToken {
            generation: state.generation,
            load: 11,
        };
        assert!(state.current(current));
        state.enabled = false;
        state.invalidate(&shared);
        assert!(!state.current(current));
        assert_eq!(shared.load(Ordering::Acquire), 0);
        assert!(!state.due(now + Duration::from_secs(1)));
    }

    #[test]
    fn unchanged_load_obeys_interval_and_single_readback_limit() {
        let shared = AtomicU64::new(0);
        let mut state = SamplingState {
            enabled: true,
            ..Default::default()
        };
        let now = Instant::now();
        state.observe_load(1, &shared);
        state.last = Some(now);
        state.rearmed = false;
        assert!(!state.due(now + SAMPLE_INTERVAL - Duration::from_nanos(1)));
        assert!(state.due(now + SAMPLE_INTERVAL));
        state.pending = Some(ReadbackToken {
            generation: state.generation,
            load: 1,
        });
        assert!(!state.due(now + Duration::from_secs(60)));
    }

    #[test]
    fn rectangles_exclude_bars_and_reject_invalid_resource_bounds() {
        assert_eq!(
            checked_rect(1600, 900, [200, 0, 1400, 900]).unwrap(),
            [200, 0, 1400, 900]
        );
        assert_eq!(checked_rect(1, 1, [0, 0, 1, 1]).unwrap(), [0, 0, 1, 1]);
        for rect in [
            [-1, 0, 1600, 900],
            [0, -1, 1600, 900],
            [0, 0, 0, 900],
            [0, 0, 1601, 900],
            [0, 0, 1600, 901],
            [200, 0, 100, 900],
        ] {
            assert!(checked_rect(1600, 900, rect).is_err());
        }
        assert!(checked_rect(0, 900, [0, 0, 1, 1]).is_err());
        assert!(checked_rect(4097, 900, [0, 0, 1, 1]).is_err());
    }
}
