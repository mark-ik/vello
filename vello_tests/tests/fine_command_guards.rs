// Copyright 2026 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Execute the production fine-area shader against deliberately malformed PTCL.
//! These cases cannot be expressed through Scene: coarse normally emits valid
//! commands. Each dispatch is one tile, writes at most sixteen pixels, and has
//! a five-second device wait. No window or surface is created.

use std::{sync::mpsc, time::Duration};

use vello::wgpu::{self, util::DeviceExt};
use vello_encoding::ConfigUniform;

const END: u32 = 0;
const FILL: u32 = 1;
const SOLID: u32 = 3;
const COLOR: u32 = 5;
const END_CLIP: u32 = 11;
const JUMP: u32 = 12;
const RED: u32 = 0xff00_00ff;
const SIDE: u32 = 4;
const ROW_BYTES: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
const SENTINEL: [u8; 4] = [19, 37, 73, 255];

struct FineProbe {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
}

impl FineProbe {
    fn new() -> Self {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, None,
        ))
        .expect("an offscreen GPU adapter is required for this diagnostic");
        eprintln!("fine_command_guards adapter: {:?}", adapter.get_info());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("fine command guard probe"),
            ..Default::default()
        }))
        .expect("create diagnostic GPU device");
        // Use generated production WGSL, including shared imports and defines.
        // ShaderRuntimeChecks and workgroup initialization match the safe engine.
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("production fine_area"),
            source: wgpu::ShaderSource::Wgsl(vello_shaders::SHADERS.fine_area.wgsl.code.clone()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("production fine_area command guard probe"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions {
                zero_initialize_workgroup_memory: true,
                ..Default::default()
            },
            cache: None,
        });
        Self {
            device,
            queue,
            pipeline,
        }
    }

    fn pixels(&self, label: &str, commands: &[u32]) -> Vec<[u8; 4]> {
        let storage = |words: &[u32]| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytemuck::cast_slice(words),
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let config = ConfigUniform {
            width_in_tiles: 1,
            height_in_tiles: 1,
            target_width: SIDE,
            target_height: SIDE,
            base_color: 0xffff_0000, // Opaque blue, distinct from the unwritten sentinel.
            ptcl_size: u32::try_from(commands.len()).unwrap(),
            ..Default::default()
        };
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fine probe config"),
                contents: bytemuck::bytes_of(&config),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let segments = storage(&[0; 8]);
        let ptcl = storage(commands);
        let info = storage(&[0; 16]);
        let spill = storage(&[0; 16]);
        let texture = |width, height, usage| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage,
                view_formats: &[],
            })
        };
        let output = texture(
            SIDE,
            SIDE,
            wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
        );
        self.queue.write_texture(
            output.as_image_copy(),
            &SENTINEL.repeat((SIDE * SIDE) as usize),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIDE * 4),
                rows_per_image: Some(SIDE),
            },
            output.size(),
        );
        let gradients = texture(1, 1, wgpu::TextureUsages::TEXTURE_BINDING);
        let atlas = texture(1, 1, wgpu::TextureUsages::TEXTURE_BINDING);
        let output_view = output.create_view(&Default::default());
        let gradients_view = gradients.create_view(&Default::default());
        let atlas_view = atlas.create_view(&Default::default());
        let buffers = [&uniform, &segments, &ptcl, &info, &spill];
        let mut entries: Vec<_> = buffers
            .iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: u32::try_from(binding).unwrap(),
                resource: buffer.as_entire_binding(),
            })
            .collect();
        for (binding, view) in [(5, &output_view), (6, &gradients_view), (7, &atlas_view)] {
            entries.push(wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fine probe readback"),
            size: u64::from(ROW_BYTES * SIDE),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_texture_to_buffer(
            output.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW_BYTES),
                    rows_per_image: Some(SIDE),
                },
            },
            output.size(),
        );
        let submission = self.queue.submit([encoder.finish()]);
        let (send, receive) = mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(5)),
            })
            .unwrap_or_else(|error| panic!("{label}: bounded device wait failed: {error}"));
        receive
            .try_recv()
            .unwrap_or_else(|error| panic!("{label}: mapping callback missing after poll: {error}"))
            .unwrap_or_else(|error| panic!("{label}: readback mapping failed: {error}"));
        let mapped = readback
            .slice(..)
            .get_mapped_range()
            .unwrap_or_else(|error| panic!("{label}: mapped range unavailable: {error}"));
        let pixels = mapped
            .chunks_exact(ROW_BYTES as usize)
            .flat_map(|row| {
                row[..(SIDE * 4) as usize]
                    .chunks_exact(4)
                    .map(|pixel| pixel.try_into().unwrap())
            })
            .collect();
        drop(mapped);
        readback.unmap();
        pixels
    }
}

#[test]
#[ignore = "Deliberate malformed GPU command probe; run this exact test with --ignored after shader review"]
fn fine_area_rejects_malformed_commands_and_preserves_valid_forward_chains() {
    let probe = FineProbe::new();
    let malformed = [
        ("unknown opcode", vec![0, 0xfeed_1234]),
        ("self jump", vec![0, JUMP, 1]),
        ("backward jump", vec![0, SOLID, JUMP, 1]),
        ("jump to buffer end", vec![0, JUMP, 3]),
        ("jump far beyond buffer", vec![0, JUMP, u32::MAX]),
        (
            "unmatched end clip",
            vec![0, END_CLIP, 0, 1.0_f32.to_bits(), END],
        ),
        ("exhausted header", vec![0]),
        ("missing end command", vec![0, SOLID]),
        (
            "huge fill segment count",
            vec![0, FILL, 0xffff_fffe, 0, 0, END],
        ),
        (
            "fill start beyond segment buffer",
            vec![0, FILL, 2, 2, 0, END],
        ),
        (
            "fill span beyond segment buffer",
            vec![0, FILL, 4, 0, 0, END],
        ),
    ];
    for (name, commands) in malformed {
        assert_eq!(
            probe.pixels(name, &commands),
            vec![SENTINEL; 16],
            "{name} must terminate without writing output"
        );
    }
    // Every operand-bearing opcode is tested with each incomplete prefix.
    for (tag, size) in [
        (FILL, 4),
        (COLOR, 2),
        (6, 3),
        (7, 3),
        (8, 3),
        (9, 2),
        (END_CLIP, 3),
        (JUMP, 2),
        (13, 3),
    ] {
        for operands in 0..size - 1 {
            let mut commands = vec![0, tag];
            commands.extend(std::iter::repeat_n(0, operands));
            let name = format!("truncated opcode {tag}, {operands} operands");
            assert_eq!(
                probe.pixels(&name, &commands),
                vec![SENTINEL; 16],
                "{name} must refuse the incomplete command"
            );
        }
    }
    let direct = probe.pixels("valid solid red", &[0, SOLID, COLOR, RED, END]);
    assert_eq!(direct, vec![[255, 0, 0, 255]; 16]);
    // With no segments the backdrop supplies complete coverage. An empty
    // span at the end of the segment buffer is valid, just as at its start.
    for segment_start in [0, 1] {
        assert_eq!(
            probe.pixels(
                "valid zero-count backdrop fill",
                &[0, FILL, 0, segment_start, 1, COLOR, RED, END],
            ),
            direct,
            "a zero-count fill must retain its backdrop coverage"
        );
    }
    // Match coarse's 64-word initial allocation and 256-word increments,
    // including a second forward jump beyond the first dynamic chunk.
    let mut chain = vec![SOLID; 323];
    chain[0] = 0;
    chain[62] = JUMP;
    chain[63] = 64;
    chain[318] = JUMP;
    chain[319] = 320;
    chain[320] = COLOR;
    chain[321] = RED;
    chain[322] = END;
    assert_eq!(
        probe.pixels("valid two-allocation forward chain", &chain),
        direct,
        "valid allocated command chains must retain exact pixels"
    );
}
