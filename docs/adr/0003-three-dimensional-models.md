# ADR 0003: Three-dimensional models

Status: accepted (2026-11-16)

## Context

An oblique impact has no axis of symmetry. Until now it was approximated in
plane strain, where the projectile is a slab rather than a body of
revolution.

## Decision

- **A separate three-dimensional model and solver.** `rap-solver::solid`
  holds `Model3`, `Particle3` and the `Solver3` trait; `rap-solver-cpu`
  implements them in `MpmSolver3`. The scheme is the two-dimensional one
  with a 3 x 3 x 3 stencil and a full deviatoric stress tensor. The
  two-dimensional types are unchanged rather than made generic over the
  dimension: the axisymmetric terms and the compact plane-strain stress do
  not generalise cleanly, and a generic rewrite would have put the
  validated solver at risk.
- **Half models.** An oblique impact is symmetric about the plane through
  the shot line and the plate normal, so only one half is simulated, with a
  sliding wall on that plane.
- **One layout for every kind of model.** `rap-app` computes a
  dimension-independent plan (sizes, positions, materials, bodies) and
  samples it into a two- or three-dimensional model. The armor face profile
  becomes a surface of revolution about the plate normal.
- **The front ends keep working on a two-dimensional picture.** For a solid
  model `Simulation::particles` returns the particles lying against the
  symmetry plane, which is the cross-section the view already draws. The
  impact summary is computed from all particles through a small `Tracer`
  type that both particle types convert to. Snapshots of a solid model are
  written in full.
- **Oblique impacts are solid by default.** Plane strain remains available
  as an explicit, cheap, indicative option.

## Consequences

- The three-dimensional solver reproduces the Hugoniot pressure to 0.1 %
  and agrees with the axisymmetric solver on a Taylor cylinder (final
  length ratio 0.8654 against 0.8661).
- It runs on the CPU only, and its particle-to-grid transfer is serial.
  A 51 000-particle oblique impact takes about a minute; realistic long
  rods at useful resolution need millions of particles and are out of
  reach until the solver is ported to the GPU. Requesting a solid model on
  the GPU solver is an error.
- The constitutive update exists twice, once per dimension. The two copies
  must be kept in step; the tests of each cover the same cases.
