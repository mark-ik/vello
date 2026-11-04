// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use std::{
    future::Future,
    task::{Context, Poll},
    time::{Duration, Instant},
};

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut cx = Context::from_waker(std::task::Waker::noop());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        assert!(Instant::now() < deadline, "GPU device setup timed out");
        std::thread::yield_now();
    }
}

#[test]
#[ignore = "actual GPU; execute serialized native gate"]
fn first_frame_deep_blend_qualifies_without_allocation_shortfall() {
    use peniko::kurbo::{Affine, Rect};
    use peniko::{Color, Fill, Mix};
    let instance = wgpu::Instance::default();
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .expect("native adapter");
    eprintln!("adapter={:?}", adapter.get_info());
    let (device, queue) =
        block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("device");
    eprintln!(
        "storage_binding_bytes={} max_buffer_bytes={}",
        device.limits().max_storage_buffer_binding_size,
        device.limits().max_buffer_size
    );
    let mut renderer = Renderer::new(
        &device,
        RendererOptions {
            use_cpu: false,
            antialiasing_support: AaSupport::area_only(),
            num_init_threads: NonZeroUsize::new(1),
            pipeline_cache: None,
        },
    )
    .unwrap();
    let (width, height) = (1024, 600);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("deep blend regression"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut scene = Scene::new();
    let rect = Rect::new(0., 0., f64::from(width), f64::from(height));
    for _ in 0..6 {
        scene.push_layer(Fill::NonZero, Mix::Normal, 0.8, Affine::IDENTITY, &rect);
    }
    scene.fill(Fill::NonZero, Affine::IDENTITY, Color::WHITE, None, &rect);
    for _ in 0..6 {
        scene.pop_layer();
    }
    renderer
        .render_to_texture(
            &device,
            &queue,
            &scene,
            &texture.create_view(&Default::default()),
            &RenderParams {
                base_color: Color::BLACK,
                width,
                height,
                antialiasing_method: AaConfig::Area,
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while renderer.pending_curve_quality().0 != 0 {
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_millis(10)),
        });
        renderer
            .poll_curve_quality(&device)
            .expect("first frame qualification");
        assert!(Instant::now() < deadline, "quality readback timed out");
    }
    eprintln!("blend_high_water_entries={}", renderer.high_water.blend);
    assert!(
        renderer.high_water.blend > (1 << 20),
        "fixture did not exercise the old default spill allocation"
    );
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(width) * u64::from(height) * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_millis(10)),
        });
        match recv.try_recv() {
            Ok(result) => {
                result.unwrap();
                break;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(error) => panic!("map callback: {error}"),
        }
        assert!(Instant::now() < deadline, "pixel readback timed out");
    }
    let bytes = buffer.slice(..).get_mapped_range().unwrap();
    let pixel = (height as usize / 2 * width as usize + width as usize / 2) * 4;
    assert!(bytes[pixel] > 0, "first-frame center pixel remained black");
}
