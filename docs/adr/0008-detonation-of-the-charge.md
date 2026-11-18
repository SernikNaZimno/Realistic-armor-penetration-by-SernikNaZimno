# ADR 0008: Detonation of the bursting charge

Status: accepted (2026-11-18)

## Context

The bursting charge of a shell was an inert solid. A shell that carries
one is meant to burst after it has entered the armor, and what its charge
does to a shell that broke up on the plate instead is part of the picture.

## Decision

- **A material may be an explosive.** It then names the energy its
  detonation releases and the Jones–Wilkins–Lee equation of state of its
  products (`Material::explosive`, `rap_materials::Jwl`). Until it is set
  off it is the solid its other parameters describe.
- **A charge may have a fuze.** `Filler::fuze_delay` is the time from the
  first contact with the armor to the detonation. A charge without a fuze
  stays inert; a fuze on a material that is not an explosive is an error.
- **The whole charge reacts in one step: a constant-volume explosion.**
  When the time comes, every point of the charge becomes a point of the
  products, gains the detonation energy at the volume it has, loses its
  deviatoric stress and is marked failed, so that it carries pressure but
  neither tension nor shear (`rap_solver::detonation`). The products are a
  material of their own, added to the model when it is assembled. The
  solvers need nothing else: one test per particle on the CPU, three more
  words per material and one branch in the shaders on the GPU.
- **No shock initiation.** The charge goes off only by its fuze, however
  hard it is struck.

## Consequences

- A slice of a long steel tube filled with TNT (metal to charge 2.11 by
  mass) reaches 1298 m/s by the time it has expanded to 1.6 times its
  radius; Gurney's formula for a cylinder gives 1509 m/s for the final
  speed, with the Gurney velocity of 2440 m/s commonly quoted for TNT. The
  tube is still accelerating when it leaves the grid of the test. Total
  energy is kept to one part in ten thousand.
- The detonation wave is left out. It would cross the charge of a 152 mm
  shell in about 17 µs, some tens of steps; the peak pressure at the wall
  of the cavity is that of a constant-volume explosion (8.4 GPa for TNT)
  instead of the 21 GPa behind a detonation front, so the first shock into
  the shell wall is too weak, while the impulse delivered is about right.
- The products are material points like any other. Once they have
  expanded to more than a few times their volume, neighbouring points no
  longer share grid nodes and the gas stops pushing; late expansion is
  not followed. Products count as failed projectile material.
- The JWL constants of the built-in TNT are quoted from memory of the open
  literature, and the fuze delay of the built-in 152 mm shell (1 ms) is a
  round value, not that of a particular fuze.
- Explosives cannot be edited in the material editor of the desktop
  application yet; a custom explosive has to be written in a material
  file.
