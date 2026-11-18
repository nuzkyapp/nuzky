# Patched dependencies

`wgpu-hal/` is wgpu-hal 30.0.1 exactly as published on crates.io (the `.crate` with SHA-256
`b6b7fb58561a792bc237628ba0792e332de418fefe145f13b5ed8201e6d52f58`, the checksum Cargo.lock had for it), plus one
change: wgpu's own fix [gfx-rs/wgpu#10124](https://github.com/gfx-rs/wgpu/pull/10124) in
`src/vulkan/descriptor.rs`. The root `Cargo.toml` puts it in place of the crates.io one through `[patch.crates-io]`.

wgpu-core creates an empty bind group layout for every device, and wgpu-hal 30 gives it a Vulkan descriptor pool with
no pool sizes, which Vulkan allows only since 1.3.215. Intel's older Vulkan driver for Core graphics (`igvk64.dll`
31.0.101.2112 on a 7th generation Core i5 in the crash dumps) crashes on it, so Nuzky crashed a few seconds after
every start on those PCs, before the preview showed anything ([gfx-rs/wgpu#9845](https://github.com/gfx-rs/wgpu/issues/9845)).

Delete this folder and the `[patch.crates-io]` entry when Nuzky moves to a wgpu release that contains the fix. Until
then, a wgpu upgrade means taking that release's wgpu-hal from crates.io again and applying the fix to it.
