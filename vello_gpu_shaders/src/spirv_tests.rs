// Copyright 2026 the Vello Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::{env, fs, path::PathBuf, process::Command};

// WGSL validation alone cannot detect backend layout mistakes. Use the same
// SPIR-V version as wgpu on modern Vulkan devices, including separate stages.
#[test]
#[ignore = "requires spirv-val and VELLO_SHADER_VALIDATION_DIR; run explicitly"]
fn shipped_shaders_validate_as_spirv_1_6() {
    let output = PathBuf::from(
        env::var_os("VELLO_SHADER_VALIDATION_DIR")
            .expect("set VELLO_SHADER_VALIDATION_DIR to the retained shader output directory"),
    );
    fs::create_dir_all(&output).unwrap();
    assert!(
        !crate::wgsl::ALL.is_empty(),
        "expected shipped shader sources"
    );
    let mut count = 0;
    for &(name, source) in crate::wgsl::ALL {
        let module = naga::front::wgsl::parse_str(source).expect("linked WGSL parses");
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .expect("linked WGSL validates");
        fs::write(output.join(format!("{name}.wgsl")), source).unwrap();
        let pipelines = std::iter::once(None).chain(module.entry_points.iter().map(|ep| {
            Some(naga::back::spv::PipelineOptions {
                shader_stage: ep.stage,
                entry_point: ep.name.clone(),
            })
        }));
        for pipeline in pipelines {
            let suffix = pipeline
                .as_ref()
                .map_or("module", |p| p.entry_point.as_str());
            let path = output.join(format!("{name}-{suffix}.spv"));
            let options = naga::back::spv::Options {
                lang_version: (1, 6),
                ..Default::default()
            };
            let words = naga::back::spv::write_vec(&module, &info, &options, pipeline.as_ref())
                .expect("SPIR-V generation succeeds");
            let bytes: Vec<u8> = words.into_iter().flat_map(u32::to_le_bytes).collect();
            fs::write(&path, bytes).unwrap();
            let result = Command::new("spirv-val")
                .arg("--relax-block-layout")
                .args(["--target-env", "vulkan1.3"])
                .arg(&path)
                .output()
                .expect("spirv-val must be installed and on PATH");
            assert!(
                result.status.success(),
                "{name}/{suffix}: {}{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr),
            );
            count += 1;
        }
    }
    eprintln!(
        "validated {count} SPIR-V modules from {} shipped shaders",
        crate::wgsl::ALL.len()
    );
}
