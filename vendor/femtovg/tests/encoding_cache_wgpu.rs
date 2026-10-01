//! Pixel regressions for attachment clears and flush-local binding reuse.
//! A one-pixel prefix clear forces the original clear shader path without
//! changing the final image, providing a control for the optimized path.
#![cfg(feature = "wgpu")]

use femtovg::{renderer::WGPURenderer, Canvas, Color, FillRule, ImageFlags, Paint, Path, PixelFormat, RenderTarget};

const W: u32 = 64;
const H: u32 = 32;

fn headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    eprintln!("encoding regression adapter: {:?}", adapter.get_info());
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("femtovg encoding cache regression"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::default(),
    }))
    .ok()
}

fn target(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("encoding regression output"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn pixels(device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::Texture) -> Vec<u8> {
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("encoding regression pixels"),
        size: (W * H * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(W * 4),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mapped = slice.get_mapped_range().expect("readback mapped");
    let result = mapped.to_vec();
    drop(mapped);
    readback.unmap();
    result
}

fn clear(canvas: &mut Canvas<WGPURenderer>, control: bool, color: Color) {
    if control {
        canvas.clear_rect(0, 0, 1, 1, Color::white());
    }
    canvas.clear_rect(0, 0, W, H, color);
}

fn differential_scene(device: &wgpu::Device, queue: &wgpu::Queue, control: bool, color: Color) -> Vec<u8> {
    let target = target(device);
    let mut canvas = Canvas::new(WGPURenderer::new(device.clone(), queue.clone())).unwrap();
    canvas.set_size(W, H, 1.);
    canvas.global_composite_operation(femtovg::CompositeOperation::Xor);
    clear(&mut canvas, control, color);
    canvas.global_composite_operation(femtovg::CompositeOperation::SourceOver);
    let image = canvas
        .create_image_empty(W as usize, H as usize, PixelFormat::Rgba8, ImageFlags::PREMULTIPLIED)
        .unwrap();
    canvas.set_render_target(RenderTarget::Image(image));
    clear(&mut canvas, control, color);

    // Exercise concave/stencil rendering on the cleared image attachment.
    let mut bowtie = Path::new();
    bowtie.move_to(4., 4.);
    bowtie.line_to(28., 28.);
    bowtie.line_to(4., 28.);
    bowtie.line_to(28., 4.);
    bowtie.close();
    canvas.fill_path(
        &bowtie,
        &Paint::color(Color::rgba(0, 255, 0, 128)).with_fill_rule(FillRule::EvenOdd),
    );
    canvas.set_render_target(RenderTarget::Screen);
    // This full clear is leading after a switch back to an already-used
    // attachment; folding it must preserve command order and stencil state.
    clear(&mut canvas, control, Color::rgb(20, 40, 60));
    canvas.rounded_scissor(0., 0., W as f32, H as f32, 8.);
    let mut rect = Path::new();
    rect.rect(0., 0., W as f32, H as f32);
    canvas.fill_path(
        &rect,
        &Paint::image(image, 0., 0., W as f32, H as f32, 0., 1.).with_anti_alias(false),
    );
    canvas.reset_scissor();
    // A later partial clear must retain the draw path and leave its neighbors.
    canvas.clear_rect(48, 8, 8, 8, Color::rgba(255, 0, 0, 128));
    canvas.fill_path(
        &bowtie,
        &Paint::color(Color::rgba(0, 0, 255, 64)).with_fill_rule(FillRule::EvenOdd),
    );
    queue.submit(canvas.flush_to_output(&target));
    pixels(device, queue, &target)
}

#[test]
fn attachment_clear_matches_shader_clear_with_blending_stencil_and_rounded_images() {
    let Some((device, queue)) = headless_device() else {
        eprintln!("skipping: no wgpu adapter available");
        return;
    };
    for color in [
        Color::white(),
        Color::rgba(210, 120, 60, 128),
        Color::rgba(220, 80, 40, 0),
    ] {
        let optimized = differential_scene(&device, &queue, false, color);
        let original = differential_scene(&device, &queue, true, color);
        assert_eq!(
            optimized, original,
            "attachment clear changed blended/stencil/clipped pixels"
        );
        assert_eq!(
            &optimized[..4],
            &[20, 40, 60, 255],
            "rounded corner must preserve its background"
        );
        let partial = ((10 * W + 50) * 4) as usize;
        assert_eq!(
            &optimized[partial..partial + 4],
            &[128, 0, 0, 128],
            "partial clear keeps replacement semantics"
        );
    }
}

fn draw_image(canvas: &mut Canvas<WGPURenderer>, image: femtovg::ImageId) {
    let mut rect = Path::new();
    rect.rect(8., 8., 16., 16.);
    let image_paint = Paint::image(image, 8., 8., 16., 16., 0., 1.).with_anti_alias(false);
    canvas.fill_path(&rect, &image_paint);
    let mut separator = Path::new();
    separator.rect(30., 8., 2., 16.);
    canvas.fill_path(&separator, &Paint::color(Color::black()).with_anti_alias(false));
    canvas.fill_path(&rect, &image_paint);
}

#[test]
fn image_reallocation_and_sampler_change_between_flushes_use_new_bindings() {
    let Some((device, queue)) = headless_device() else {
        eprintln!("skipping: no wgpu adapter available");
        return;
    };
    let target = target(&device);
    let mut canvas = Canvas::new(WGPURenderer::new(device.clone(), queue.clone())).unwrap();
    canvas.set_size(W, H, 1.);
    let image = canvas
        .create_image_empty(16, 16, PixelFormat::Rgba8, ImageFlags::empty())
        .unwrap();
    canvas.set_render_target(RenderTarget::Image(image));
    canvas.clear_rect(0, 0, 16, 16, Color::rgb(255, 0, 0));
    canvas.set_render_target(RenderTarget::Screen);
    canvas.clear_rect(0, 0, W, H, Color::white());
    draw_image(&mut canvas, image);
    queue.submit(canvas.flush_to_output(&target));
    let first = pixels(&device, &queue, &target);
    let center = ((16 * W + 16) * 4) as usize;
    assert_eq!(&first[center..center + 4], &[255, 0, 0, 255]);

    // Same ImageId, different underlying texture size and sampler flags.
    canvas
        .realloc_image(image, 8, 8, PixelFormat::Rgba8, ImageFlags::NEAREST)
        .unwrap();
    canvas.set_render_target(RenderTarget::Image(image));
    canvas.clear_rect(0, 0, 8, 8, Color::rgb(0, 255, 0));
    canvas.set_render_target(RenderTarget::Screen);
    canvas.clear_rect(0, 0, W, H, Color::white());
    draw_image(&mut canvas, image);
    queue.submit(canvas.flush_to_output(&target));
    let second = pixels(&device, &queue, &target);
    assert_eq!(
        &second[center..center + 4],
        &[0, 255, 0, 255],
        "old texture bindings leaked across flushes"
    );
    assert_eq!(&second[..4], &[255; 4]);
}
