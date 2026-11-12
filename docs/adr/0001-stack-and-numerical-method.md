# ADR 0001: Stack and numerical method

Status: accepted (2026-11-12)

## Context

The simulator must deform both armor and penetrator, handle several materials,
contact and fragmentation, and run on an AMD or Intel GPU under Windows.

## Decision

- Numerical method: explicit Material Point Method (MLS-MPM).
- Language: Rust, one Cargo workspace, one crate per responsibility.
- GPU: `wgpu` compute shaders (Vulkan / DX12); no CUDA.
- UI: `egui` / `eframe`.
- Two solvers behind one trait: a reference `f64` CPU solver, on which the
  physics is developed and validated, and an `f32` GPU solver tested against it.

## Consequences

- WGSL has no 64-bit floats, so the GPU solver works in mm–g–µs units.
- WGSL has no atomic float addition, so particle-to-grid transfer must be
  written as a per-cell gather over sorted particles. This is the main
  technical risk and is deferred to M6.
- External dependencies are added only in the milestone that first uses them.
