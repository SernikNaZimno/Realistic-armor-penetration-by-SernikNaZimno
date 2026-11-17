# ADR 0002: GPU solver

Status: accepted (2026-11-16); extended by [ADR 0005](0005-three-dimensional-models-and-ceramics-on-the-gpu.md), which splits the shader and adds three-dimensional models and ceramics

## Context

ADR 0001 planned a second solver in wgpu compute shaders, tested against
the CPU reference, and named the particle-to-grid transfer as the main risk:
WGSL cannot add floats atomically.

## Decision

- The GPU solver is a port of the two-dimensional CPU scheme (plane strain
  and axisymmetric, contact, damage, spall, outflow walls) to one WGSL
  module, `crates/rap-solver-gpu/src/mpm.wgsl`. Three-dimensional models are
  a separate, later milestone.
- **Particle-to-grid transfer is a gather.** Each step sorts particles by
  the cell of their interpolation stencil with a counting sort (count with
  integer atomics, prefix sum, fill), and every grid node sums the
  contributions of the particles in the nine cells that can reach it. Float
  sums are therefore deterministic for a given particle order within a
  cell.
- **The prefix sum is split by grid row.** A single invocation looping over
  all cells took half the step time; one invocation per row followed by a
  loop over the row totals removed that cost.
- **The state stays on the GPU.** The time step is computed there from an
  atomic maximum over particles, steps are submitted in batches of 64 and
  only the 32-byte clock is read back between batches. Particles are read
  back when a run or step returns.
- **One compute pass per stage,** so that wgpu orders the storage-buffer
  accesses of consecutive stages.
- Arithmetic is `f32` in SI units. The clock uses compensated summation.
- wgpu is pinned to major version 24, the version already resolved for the
  user interface.

## Consequences

- On the same model the GPU and CPU solvers take the same number of steps
  and agree to five or six digits in the validation cases.
- A missing GPU is an error (`SolverError::NoGpu`), never a silent fallback
  to the CPU. Tests that need a GPU skip themselves with a note when there
  is none, which is the case on hosted CI runners.
- On an integrated AMD Radeon the GPU solver is 1.3 to 1.7 times faster
  than the CPU solver on a 16-thread processor. The gather reads every
  particle nine times per step, so memory traffic, not arithmetic, limits
  it; a leaner particle-to-grid payload is the obvious next optimisation.
- Models are limited by the device's storage-buffer size and by 65 535
  workgroups per dispatch (about 4 million particles or nodes).
