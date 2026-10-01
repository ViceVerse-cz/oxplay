// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use std::collections::HashMap;
use std::rc::Rc;

#[cfg(any(not(target_arch = "wasm32"), target_os = "emscripten"))]
use i_slint_core::graphics::BorrowedOpenGLTexture;
#[cfg(feature = "image-pixel-format-rgb565")]
use i_slint_core::graphics::Rgb8Pixel;
use i_slint_core::graphics::euclid;
use i_slint_core::graphics::{ImageCacheKey, IntSize, SharedImageBuffer, SharedPixelBuffer};
use i_slint_core::items::ImageTiling;
use i_slint_core::lengths::PhysicalPx;
use i_slint_core::{ImageInner, items::ImageRendering};

use super::itemrenderer::CanvasRc;

pub trait TextureImporter
where
    Self: femtovg::Renderer + Sized,
{
    #[cfg(any(not(target_arch = "wasm32"), target_os = "emscripten"))]
    fn convert_opengl_texture(opengl_texture: std::num::NonZero<u32>) -> Self::NativeTexture;

    #[cfg(feature = "unstable-wgpu-30")]
    fn convert_wgpu_30_texture(wgpu_texture: wgpu_30::Texture) -> Self::NativeTexture;
}

impl TextureImporter for femtovg::renderer::OpenGl {
    #[cfg(any(not(target_arch = "wasm32"), target_os = "emscripten"))]
    fn convert_opengl_texture(opengl_texture: std::num::NonZero<u32>) -> Self::NativeTexture {
        glow::NativeTexture(opengl_texture)
    }

    #[cfg(feature = "unstable-wgpu-30")]
    fn convert_wgpu_30_texture(_wgpu_texture: wgpu_30::Texture) -> Self::NativeTexture {
        unimplemented!()
    }
}

#[cfg(feature = "wgpu-30")]
impl TextureImporter for femtovg::renderer::WGPURenderer {
    #[cfg(any(not(target_arch = "wasm32"), target_os = "emscripten"))]
    fn convert_opengl_texture(_opengl_texture: std::num::NonZero<u32>) -> Self::NativeTexture {
        todo!()
    }

    #[cfg(feature = "unstable-wgpu-30")]
    fn convert_wgpu_30_texture(wgpu_texture: wgpu_30::Texture) -> Self::NativeTexture {
        wgpu_texture
    }
}
pub struct Texture<R: femtovg::Renderer + TextureImporter> {
    pub id: femtovg::ImageId,
    canvas: CanvasRc<R>,
    #[cfg(feature = "unstable-wgpu-30")]
    imported_wgpu_texture: Option<(wgpu_30::Texture, femtovg::ImageFlags)>,
}

impl<R: femtovg::Renderer + TextureImporter> Texture<R> {
    pub fn size(&self) -> Option<IntSize> {
        self.canvas
            .borrow()
            .image_info(self.id)
            .map(|info| [info.width() as u32, info.height() as u32].into())
            .ok()
    }

    pub fn as_render_target(&self) -> femtovg::RenderTarget {
        femtovg::RenderTarget::Image(self.id)
    }

    pub fn adopt(canvas: &CanvasRc<R>, image_id: femtovg::ImageId) -> Rc<Texture<R>> {
        Texture {
            id: image_id,
            canvas: canvas.clone(),
            #[cfg(feature = "unstable-wgpu-30")]
            imported_wgpu_texture: None,
        }
        .into()
    }

    pub fn new_empty_on_gpu(
        canvas: &CanvasRc<R>,
        width: u32,
        height: u32,
    ) -> Option<Rc<Texture<R>>> {
        if width == 0 || height == 0 {
            return None;
        }
        let image_id = canvas
            .borrow_mut()
            .create_image_empty(
                width as usize,
                height as usize,
                femtovg::PixelFormat::Rgba8,
                femtovg::ImageFlags::PREMULTIPLIED | femtovg::ImageFlags::FLIP_Y,
            )
            .unwrap();
        Some(Self::adopt(canvas, image_id))
    }

    pub(crate) fn filter(&self, filter: femtovg::ImageFilter) -> Rc<Self> {
        let size = self.size().unwrap();
        let filtered_image = Self::new_empty_on_gpu(&self.canvas, size.width, size.height).expect(
            "internal error: this can only fail if the filtered image was zero width or height",
        );

        self.canvas.borrow_mut().filter_image(filtered_image.id, filter, self.id);

        filtered_image
    }

    pub fn as_paint(&self) -> femtovg::Paint {
        self.as_paint_with_alpha(1.0)
    }

    pub fn as_paint_with_alpha(&self, alpha_tint: f32) -> femtovg::Paint {
        let size = self
            .size()
            .expect("internal error: CachedImage::as_paint() called on zero-sized texture");
        femtovg::Paint::image(
            self.id,
            0.,
            0.,
            size.width as f32,
            size.height as f32,
            0.,
            alpha_tint,
        )
    }

    pub fn id(&self) -> femtovg::ImageId {
        self.id
    }

    /// Reuse only this item's owning import of the exact same WGPU allocation.
    /// Pixel changes do not replace a texture; its dimensions and format cannot
    /// change in place. Keep source-property notifications so cached layers
    /// still repaint those changed pixels.
    pub(crate) fn reuse_imported_wgpu_texture(
        self: &Rc<Self>,
        image: &ImageInner,
        scaling: ImageRendering,
        tiling: (ImageTiling, ImageTiling),
    ) -> Option<Rc<Self>> {
        #[cfg(feature = "unstable-wgpu-30")]
        if let (
            Some((imported, flags)),
            ImageInner::WGPUTexture(i_slint_core::graphics::WGPUTexture::WGPU30Texture(texture)),
        ) = (&self.imported_wgpu_texture, image)
        {
            if imported == texture && *flags == base_image_flags(scaling, tiling) {
                return Some(self.clone());
            }
        }
        #[cfg(not(feature = "unstable-wgpu-30"))]
        let _ = (image, scaling, tiling);
        None
    }

    // Upload the image to the GPU. This function could take just a canvas as parameter,
    // but since an upload requires a current context, this is "enforced" by taking
    // a renderer instead (which implies a current context).
    pub fn new_from_image(
        image: &ImageInner,
        canvas: &CanvasRc<R>,
        target_size_for_scalable_source: Option<euclid::Size2D<u32, PhysicalPx>>,
        scaling: ImageRendering,
        tiling: (ImageTiling, ImageTiling),
    ) -> Option<Rc<Self>> {
        let image_flags = base_image_flags(scaling, tiling);

        let image_id = match image {
            #[cfg(all(target_arch = "wasm32", not(target_os = "emscripten")))]
            ImageInner::HTMLImage(html_image) => {
                if html_image.is_loaded() {
                    // Anecdotal evidence suggests that HTMLImageElement converts to a texture with
                    // pre-multiplied alpha. It's possible that this is not generally applicable, but it
                    // is the case for SVGs.
                    let image_flags = if html_image.is_svg() {
                        if let Some(target_size) = target_size_for_scalable_source {
                            let dom_element = &html_image.dom_element;
                            dom_element.set_width(target_size.width);
                            dom_element.set_height(target_size.height);
                        }
                        image_flags | femtovg::ImageFlags::PREMULTIPLIED
                    } else {
                        image_flags
                    };
                    canvas.borrow_mut().create_image(&html_image.dom_element, image_flags).unwrap()
                } else {
                    return None;
                }
            }
            #[cfg(any(not(target_arch = "wasm32"), target_os = "emscripten"))]
            ImageInner::BorrowedOpenGLTexture(BorrowedOpenGLTexture {
                texture_id,
                size,
                origin,
                ..
            }) => {
                let image_flags = match origin {
                    i_slint_core::graphics::BorrowedOpenGLTextureOrigin::TopLeft => image_flags,
                    i_slint_core::graphics::BorrowedOpenGLTextureOrigin::BottomLeft => {
                        image_flags | femtovg::ImageFlags::FLIP_Y
                    }
                    _ => unimplemented!(
                        "internal error: missing implementation for BorrowedOpenGLTextureOrigin"
                    ),
                };
                canvas
                    .borrow_mut()
                    .create_image_from_native_texture(
                        <R as TextureImporter>::convert_opengl_texture(*texture_id),
                        femtovg::ImageInfo::new(
                            image_flags,
                            size.width as _,
                            size.height as _,
                            femtovg::PixelFormat::Rgba8,
                        ),
                    )
                    .unwrap()
            }
            #[cfg(feature = "unstable-wgpu-30")]
            ImageInner::WGPUTexture(i_slint_core::graphics::WGPUTexture::WGPU30Texture(
                texture,
            )) => {
                let texture = texture.clone();
                let size = texture.size();

                canvas
                    .borrow_mut()
                    .create_image_from_native_texture(
                        <R as TextureImporter>::convert_wgpu_30_texture(texture),
                        femtovg::ImageInfo::new(
                            image_flags,
                            size.width as _,
                            size.height as _,
                            femtovg::PixelFormat::Rgba8,
                        ),
                    )
                    .unwrap()
            }
            _ => {
                let buffer = image.render_to_buffer(target_size_for_scalable_source)?;
                // femtovg has no 16-bit texture format; expand to RGB8 for the upload.
                #[cfg(feature = "image-pixel-format-rgb565")]
                let buffer = match buffer {
                    SharedImageBuffer::RGB565(b) => {
                        let mut rgb = SharedPixelBuffer::<Rgb8Pixel>::new(b.width(), b.height());
                        for (dst, src) in rgb.make_mut_slice().iter_mut().zip(b.as_slice()) {
                            *dst = (*src).into();
                        }
                        SharedImageBuffer::RGB8(rgb)
                    }
                    other => other,
                };
                let (image_source, flags) = image_buffer_to_image_source(&buffer);
                canvas.borrow_mut().create_image(image_source, image_flags | flags).unwrap()
            }
        };

        #[cfg(feature = "unstable-wgpu-30")]
        let imported_wgpu_texture = match image {
            ImageInner::WGPUTexture(i_slint_core::graphics::WGPUTexture::WGPU30Texture(
                texture,
            )) => Some((texture.clone(), image_flags)),
            _ => None,
        };
        Some(
            Self {
                id: image_id,
                canvas: canvas.clone(),
                #[cfg(feature = "unstable-wgpu-30")]
                imported_wgpu_texture,
            }
            .into(),
        )
    }
}

impl<R: femtovg::Renderer + TextureImporter> Drop for Texture<R> {
    fn drop(&mut self) {
        self.canvas.borrow_mut().delete_image(self.id);
    }
}

#[derive(PartialEq, Eq, Hash, Debug)]
pub struct TextureCacheKey {
    source_key: ImageCacheKey,
    target_size_for_scalable_source: Option<euclid::Size2D<u32, PhysicalPx>>,
    gpu_image_flags: ImageRendering,
    gpu_image_tiling: (ImageTiling, ImageTiling),
}

impl TextureCacheKey {
    pub fn new(
        resource: &ImageInner,
        target_size_for_scalable_source: Option<euclid::Size2D<u32, PhysicalPx>>,
        gpu_image_flags: ImageRendering,
        gpu_image_tiling: (ImageTiling, ImageTiling),
    ) -> Option<Self> {
        ImageCacheKey::new(resource).map(|source_key| Self {
            source_key,
            target_size_for_scalable_source,
            gpu_image_flags,
            gpu_image_tiling,
        })
    }
}

// Cache used to avoid repeatedly decoding images from disk. Entries with a count
// of 1 are drained after flushing the renderer commands to the screen.
pub struct TextureCache<R: femtovg::Renderer + TextureImporter>(
    HashMap<TextureCacheKey, Rc<Texture<R>>>,
);

impl<R: femtovg::Renderer + TextureImporter> Default for TextureCache<R> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<R: femtovg::Renderer + TextureImporter> TextureCache<R> {
    // Look up the given image cache key in the image cache and upgrade the weak reference to a strong one if found,
    // otherwise a new image is created/loaded from the given callback.
    pub(crate) fn lookup_image_in_cache_or_create(
        &mut self,
        cache_key: TextureCacheKey,
        image_create_fn: impl Fn() -> Option<Rc<Texture<R>>>,
    ) -> Option<Rc<Texture<R>>> {
        Some(match self.0.entry(cache_key) {
            std::collections::hash_map::Entry::Occupied(existing_entry) => {
                existing_entry.get().clone()
            }
            std::collections::hash_map::Entry::Vacant(vacant_entry) => {
                let new_image = image_create_fn()?;
                vacant_entry.insert(new_image.clone());
                new_image
            }
        })
    }

    pub(crate) fn drain(&mut self) {
        self.0.retain(|_, cached_image| {
            // * Retain images that are used by elements, so that they can be effectively
            // shared (one image element refers to foo.png, another element is created
            // and refers to the same -> share).
            // * Also retain images that are still loading (async HTML), where the size
            // is not known yet. Otherwise we end up in a loop where an image is not loaded
            // yet, we report (0, 0) to the layout, the image gets removed here, the closure
            // still triggers a load and marks the layout as dirt, which loads the
            // image again, etc.
            Rc::strong_count(cached_image) > 1 || cached_image.size().is_none()
        });
    }

    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }
}

fn image_buffer_to_image_source(
    buffer: &SharedImageBuffer,
) -> (femtovg::ImageSource<'_>, femtovg::ImageFlags) {
    fn image_source<Pixel: Clone>(buffer: &SharedPixelBuffer<Pixel>) -> imgref::ImgRef<'_, Pixel> {
        let pixels = buffer.as_slice();
        let read = buffer.width() as u64 * buffer.height() as u64;
        // `femtovg::Canvas::create_image` is unsound: it is safe to call, yet it hands the driver
        // the buffer's pointer along with the pixel count from `ImageSource::dimensions()`, so an
        // `ImgRef` that overstates its buffer reads out of bounds.
        assert!(pixels.len() as u64 >= read);
        imgref::ImgRef::new(&pixels[..read as usize], buffer.width() as _, buffer.height() as _)
    }

    match buffer {
        // Expanded to RGB8 before upload; see Texture::new_from_image.
        #[cfg(feature = "image-pixel-format-rgb565")]
        SharedImageBuffer::RGB565(..) => {
            unreachable!("RGB565 buffers are converted to RGB8 before the femtovg upload")
        }
        SharedImageBuffer::RGB8(buffer) => {
            (image_source(buffer).into(), femtovg::ImageFlags::empty())
        }
        SharedImageBuffer::RGBA8(buffer) => {
            (image_source(buffer).into(), femtovg::ImageFlags::empty())
        }
        SharedImageBuffer::RGBA8Premultiplied(buffer) => {
            (image_source(buffer).into(), femtovg::ImageFlags::PREMULTIPLIED)
        }
        #[cfg(feature = "image-pixel-format-gray8")]
        SharedImageBuffer::Gray8(buffer) => {
            (image_source(buffer).into(), femtovg::ImageFlags::empty())
        }
    }
}

pub fn base_image_flags(
    scaling: ImageRendering,
    tiling: (ImageTiling, ImageTiling),
) -> femtovg::ImageFlags {
    (match scaling {
        ImageRendering::Pixelated => femtovg::ImageFlags::NEAREST,
        ImageRendering::Smooth | _ => femtovg::ImageFlags::empty(),
    } | match tiling.0 {
        ImageTiling::Repeat | ImageTiling::Round => femtovg::ImageFlags::REPEAT_X,
        ImageTiling::None | _ => femtovg::ImageFlags::empty(),
    } | match tiling.1 {
        ImageTiling::Repeat | ImageTiling::Round => femtovg::ImageFlags::REPEAT_Y,
        ImageTiling::None | _ => femtovg::ImageFlags::empty(),
    })
}

#[cfg(all(test, feature = "unstable-wgpu-30", not(target_arch = "wasm32")))]
mod imported_wgpu_tests {
    use super::*;
    use std::cell::RefCell;
    use wgpu_30 as wgpu;

    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        struct ThreadWake(std::thread::Thread);
        impl std::task::Wake for ThreadWake {
            fn wake(self: std::sync::Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = std::task::Waker::from(std::sync::Arc::new(ThreadWake(std::thread::current())));
        let mut cx = std::task::Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }
            std::thread::park();
        }
    }

    fn texture(device: &wgpu::Device, size: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("import-reuse-regression"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    }

    fn write_pixels(queue: &wgpu::Queue, texture: &wgpu::Texture, rgba: [u8; 4]) {
        queue.write_texture(
            texture.as_image_copy(),
            &rgba.repeat((texture.width() * texture.height()) as usize),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(texture.width() * 4),
                rows_per_image: None,
            },
            texture.size(),
        );
    }

    fn render_pixels(
        canvas: &CanvasRc<femtovg::renderer::WGPURenderer>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: &Texture<femtovg::renderer::WGPURenderer>,
    ) -> Vec<u8> {
        let size = image.size().unwrap().width;
        let paint = image.as_paint().with_anti_alias(false);
        let output = texture(device, size);
        let commands = {
            let mut canvas = canvas.borrow_mut();
            canvas.set_size(size, size, 1.);
            canvas.reset();
            canvas.clear_rect(0, 0, size, size, femtovg::Color::black());
            let mut path = femtovg::Path::new();
            path.rect(0., 0., size as f32, size as f32);
            canvas.fill_path(&path, &paint);
            canvas.flush_to_output(&output)
        };
        queue.submit(commands);
        let row_stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("import-reuse-readback"),
            size: (row_stride * size) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            output.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_stride),
                    rows_per_image: None,
                },
            },
            output.size(),
        );
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let pixels = mapped
            .chunks_exact(row_stride as usize)
            .flat_map(|row| row[..size as usize * 4].iter().copied())
            .collect();
        drop(mapped);
        readback.unmap();
        pixels
    }

    #[test]
    #[ignore = "requires an uncontended hardware GPU; run explicitly after resource sampling"]
    fn same_wgpu_import_reads_fresh_pixels_and_rejects_new_handles_or_flags() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = block_on(instance.request_adapter(&Default::default())).unwrap();
        let info = adapter.get_info();
        assert_eq!(info.backend, wgpu::Backend::Metal);
        assert!(info.name.starts_with("Apple M1"), "expected Apple M1 Metal, got {info:?}");
        eprintln!("import reuse regression adapter: {} {:?}", info.name, info.backend);
        let (device, queue) = block_on(adapter.request_device(&Default::default())).unwrap();
        let canvas = Rc::new(RefCell::new(
            femtovg::Canvas::new(femtovg::renderer::WGPURenderer::new(
                device.clone(),
                queue.clone(),
            ))
            .unwrap(),
        ));
        let source_texture = texture(&device, 16);
        let source = ImageInner::WGPUTexture(i_slint_core::graphics::WGPUTexture::WGPU30Texture(
            source_texture.clone(),
        ));
        let import = |source: &ImageInner, rendering, tiling| {
            Texture::new_from_image(source, &canvas, None, rendering, tiling).unwrap()
        };
        let no_tiling = (ImageTiling::None, ImageTiling::None);
        let cached = import(&source, ImageRendering::Smooth, no_tiling);
        let image_id = cached.id();
        write_pixels(&queue, &source_texture, [255, 0, 0, 255]);
        assert_eq!(render_pixels(&canvas, &device, &queue, &cached), [255, 0, 0, 255].repeat(256));

        // Republish the same allocation after writing a different video frame.
        write_pixels(&queue, &source_texture, [0, 255, 0, 255]);
        let reused = cached
            .reuse_imported_wgpu_texture(&source, ImageRendering::Smooth, no_tiling)
            .unwrap_or_else(|| import(&source, ImageRendering::Smooth, no_tiling));
        assert_eq!(reused.id(), image_id);
        let fresh_pixels = render_pixels(&canvas, &device, &queue, &reused);
        assert_eq!(fresh_pixels, [0, 255, 0, 255].repeat(256));
        // The original fresh-import behavior is an explicit pixel differential control.
        let forced_fresh = import(&source, ImageRendering::Smooth, no_tiling);
        assert_ne!(forced_fresh.id(), image_id);
        assert_eq!(render_pixels(&canvas, &device, &queue, &forced_fresh), fresh_pixels);

        assert!(
            cached
                .reuse_imported_wgpu_texture(&source, ImageRendering::Pixelated, no_tiling)
                .is_none()
        );
        let nearest = import(&source, ImageRendering::Pixelated, no_tiling);
        assert_ne!(nearest.id(), image_id);
        for tiling in
            [(ImageTiling::Repeat, ImageTiling::None), (ImageTiling::None, ImageTiling::Repeat)]
        {
            assert!(
                cached
                    .reuse_imported_wgpu_texture(&source, ImageRendering::Smooth, tiling)
                    .is_none()
            );
            assert_ne!(import(&source, ImageRendering::Smooth, tiling).id(), image_id);
        }
        for size in [16, 8] {
            let replacement = texture(&device, size);
            write_pixels(&queue, &replacement, [0, 0, 255, 255]);
            let replacement = ImageInner::WGPUTexture(
                i_slint_core::graphics::WGPUTexture::WGPU30Texture(replacement),
            );
            assert!(
                cached
                    .reuse_imported_wgpu_texture(&replacement, ImageRendering::Smooth, no_tiling)
                    .is_none()
            );
            let changed = import(&replacement, ImageRendering::Smooth, no_tiling);
            assert_ne!(changed.id(), image_id);
            assert_eq!(changed.size().unwrap().width, size);
            assert_eq!(
                render_pixels(&canvas, &device, &queue, &changed),
                [0, 0, 255, 255].repeat((size * size) as usize)
            );
        }
        drop(source);
        drop(source_texture);
        drop(reused);
        assert_eq!(render_pixels(&canvas, &device, &queue, &cached), fresh_pixels);
        drop(cached);
        // No retained global ImageId entry survives the last owning item import.
        assert!(canvas.borrow().image_info(image_id).is_err());
    }
}
