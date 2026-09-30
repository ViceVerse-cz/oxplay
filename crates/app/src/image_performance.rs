// SPDX-License-Identifier: GPL-3.0-or-later
//! Opt-in, offline release microbenchmarks. Never part of application startup.
use super::{Counters, decode_sized, store};
use image::{DynamicImage, ImageFormat, RgbaImage};
use serein_core::VideoId;
use std::{
    hint::black_box,
    io::Cursor,
    sync::{Arc, Mutex},
    time::Instant,
};

fn median_us(mut operation: impl FnMut(), iterations: usize) -> f64 {
    let mut samples = Vec::new();
    for _ in 0..21 {
        let start = Instant::now();
        for _ in 0..iterations {
            operation();
        }
        samples.push(start.elapsed().as_secs_f64() * 1_000_000. / iterations as f64);
    }
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn pixels(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        let noise = x
            .wrapping_mul(1_664_525)
            .wrapping_add(y.wrapping_mul(1_013_904_223));
        image::Rgba([noise as u8, (noise >> 8) as u8, (noise >> 16) as u8, 255])
    })
}

#[test]
#[ignore = "offline microbenchmark; run in release with --nocapture and one test thread"]
fn benchmark_disabled_artwork_cache() {
    let pixels = pixels(320, 180);
    let id = VideoId::new("abcdefghijk").unwrap();
    let cache = Arc::new(Mutex::new(None));
    let counters = Counters::default();
    let before = median_us(
        || {
            // The old disabled-cache path encoded first, then discovered no writer.
            let mut png = Cursor::new(Vec::new());
            black_box(&pixels)
                .write_to(&mut png, ImageFormat::Png)
                .unwrap();
            black_box(png);
        },
        20,
    );
    let after = median_us(|| store(&cache, &id, black_box(&pixels), &counters), 2000);
    println!(
        "{{\"benchmark\":\"disabled_artwork_cache\",\"unit\":\"us/image\",\"before\":{before},\"after\":{after}}}"
    );
}

#[test]
#[ignore = "offline microbenchmark; run in release with --nocapture and one test thread"]
fn benchmark_portrait_decode() {
    for edge in [48, 180, 1024] {
        let source = pixels(edge, edge);
        let mut png = Cursor::new(Vec::new());
        source.write_to(&mut png, ImageFormat::Png).unwrap();
        let before = median_us(
            || {
                let card = image::load_from_memory(black_box(png.get_ref()))
                    .unwrap()
                    .thumbnail(320, 180)
                    .into_rgba8();
                black_box(
                    DynamicImage::ImageRgba8(card)
                        .thumbnail(88, 88)
                        .into_rgba8(),
                );
            },
            20,
        );
        let after = median_us(
            || {
                black_box(decode_sized(black_box(png.get_ref()), 88, 88).unwrap());
            },
            20,
        );
        println!(
            "{{\"benchmark\":\"portrait_decode\",\"edge\":{edge},\"unit\":\"us/image\",\"before\":{before},\"after\":{after}}}"
        );
    }
}
