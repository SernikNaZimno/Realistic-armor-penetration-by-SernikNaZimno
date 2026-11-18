# ADR 0007: Brittle fracture

Status: accepted (2026-11-18)

## Context

Material failed in two ways: by accumulated plastic strain (Johnson–Cook
or Johnson–Holmquist damage) and by spallation, when the tensile pressure
passed a limit. A hardened shell does neither when it is bent: it has
hardly deformed and its pressure is modest, yet it cracks. With only those
two ways to fail, a shell of hardened steel striking a plate at a steep
angle curled like a hook instead of breaking.

## Decision

- **A material may have a tensile strength.** It is compared with the
  largest principal stress: the largest principal value of the deviatoric
  stress minus the pressure. In two dimensions that value is in closed form
  from the in-plane components and the hoop component; in three, from the
  second and third invariants of the deviator.
- **Above the limit, damage grows at the rate of a running crack.** Breaking
  a point the instant its stress passes the limit lets one stress wave
  shatter everything it runs through, since no point has time to be
  unloaded by its neighbours giving way; a first version did exactly that
  and turned the whole shell to debris. A crack cannot run faster than a
  fraction of the wave speed, so the damage of an overstressed point grows
  by `0.2 · c · Δt / Δx` times the relative overstress per step
  (`rap_solver::fracture`): the time a crack at a fifth of the wave speed
  takes to cross a cell. A brief overstress weakens a point without
  breaking it.
- **The same rule in all four solvers.** On the GPU it lives in the shared
  closing step of the constitutive update, which is given the largest
  principal deviatoric stress by each shader.
- Only the hardened shell steel of the library has a tensile strength
  (2.2 GPa, an estimate). Armor steel, tungsten alloy and the ceramic keep
  their existing ways of failing.

## Consequences

- The comparison of the 152 mm shell with quoted penetration figures is
  unchanged: head-on, the shell is in compression and does not crack.
- The fraction 0.2 and the linear dependence on the overstress are
  assumptions, not measurements.
- A crack is one cell wide whatever the cell size. The time to break scales
  with the cell, but the energy a crack takes does not, so crack patterns
  and the amount of debris depend on the grid.
- The GPU and CPU solvers agree less closely where material cracks: where
  a crack starts depends on the last digits of the stress.
