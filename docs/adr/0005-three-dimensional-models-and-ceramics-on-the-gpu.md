# ADR 0005: Three-dimensional models and ceramics on the GPU

Status: accepted (2026-11-17)

## Context

The GPU solver of ADR 0002 handled two-dimensional models of metals. The
three-dimensional solver of ADR 0003 ran on the CPU with a serial
particle-to-grid transfer, and ceramics (JH-2) were refused on the GPU.
Three-dimensional models are the expensive ones, so that is where a GPU
matters most.

## Decision

- **Three shader files instead of one.** `common.wgsl` holds what does not
  depend on the number of dimensions: the material models, the clock and
  the two passes that compute and apply the time step. `flat.wgsl` and
  `solid.wgsl` hold the two- and three-dimensional transfers and declare
  the buffers. The source handed to the device is `common.wgsl` followed by
  one of the two; WGSL resolves module-scope names in any order, so the
  shared code can refer to `params` and `clock` declared after it.
- **One constitutive update for both shaders.** The scalar part of the
  update (yield, radial return factor, damage, heating, pressure, spall,
  wave speed) works on a small `Point` struct; each shader builds its own
  trial stress, calls the shared functions and scales its stress by the
  factor they return. The CPU solvers still have two copies of that logic.
- **A material carries the parameters of every model** and three selectors
  (equation of state, strength, damage). Unused parameters are zero and the
  shader branches on the selectors. Adding a model means adding fields and
  a branch, not a new shader.
- **One engine for both solvers.** `engine.rs` owns the device, the
  buffers, the pipelines and the stepping loop, and is told by a `Program`
  what to upload and how to dispatch. `GpuSolver` and `GpuSolver3` only
  encode a model and decode particles.
- **The three-dimensional transfer is the two-dimensional one with 27
  cells.** Bins are the cells of the stencil base; the prefix sum is split
  into rows along `x`, with one serial loop over the row totals. The
  symmetric stress is stored as six components so that a particle is 144
  bytes.
- **Shaders are validated in unit tests without a device,** with the
  parser and validator wgpu itself uses, so a machine without a GPU, such
  as a CI runner, still catches a shader that does not compile.

## Consequences

- The three-dimensional GPU solver takes the same number of steps as the
  CPU solver and agrees with it to four or more digits on a shock, on an
  oblique rod impact with contact, damage, a symmetry plane and outflow,
  and at the level of whole scenarios. Ceramics agree likewise in two
  dimensions (`crates/rap-solver-gpu/tests/solid_against_cpu.rs`,
  `crates/rap-app/tests`).
- No model is refused on the GPU any more for what it contains; only its
  size can rule it out.
- The size limits of ADR 0002 remain: the largest buffer the device
  allows, and 65 535 workgroups per dispatch, which is about 4 million
  particles or grid nodes. A three-dimensional grid reaches that sooner
  than the 8 million nodes the assembler allows.
- A three-dimensional node gathers from 27 cells and a particle is 144
  bytes, so memory traffic limits the speed even more than in two
  dimensions. On the integrated AMD Radeon the two three-dimensional
  examples run 1.12 and 1.21 times as fast as on the 16-thread CPU.
  The gather walks each row of bins as one run of particles and writes
  nothing for nodes no particle reaches, because the grid of a
  three-dimensional model is mostly empty.
