# ADR 0004: Armor from STL meshes

Status: accepted (2026-11-17)

## Context

Armor was an array of plates with an optional face profile. Real armor,
a cast turret for example, has a shape that no stack of plates describes.
The requirement is to read the whole armor solid from an STL file.

## Decision

- **The armor is one of two kinds.** `Scenario::armor` is a `Target`:
  `Plates` (the existing `Armor`) or `Solid` (`SolidArmor`: a mesh file,
  its units, one material and the placement of the shot). In a scenario
  file the kinds share one object, told apart by the `mesh` field, so
  existing files stay valid and a misspelt field is still reported by name.
- **The shot is placed in the coordinates of the file.** The user gives a
  point on the shot line and the direction of flight as they read them in
  the program the mesh came from. The model frame has `x` along the flight,
  its origin where the line enters the solid, and `y` as close to the
  file's `z` as the shot allows, so the view shows an upright slice.
- **The domain holds the file name, not the triangles.** `rap-io` reads
  STL (binary or text) into a `rap-geometry::TriangleMesh`; `rap-app`
  takes the mesh as an argument (`Simulation::with_mesh`, `load_mesh`).
  The front ends resolve a relative name against the scenario's folder.
- **Particles come from line crossings.** One line along `x` is cast
  through every lattice column and the lattice points between an odd and
  the next even crossing are kept. Each triangle is rasterised onto the
  columns it covers, so the cost follows the projected area of the mesh,
  not triangles times columns. A line through an edge or a vertex is
  treated as displaced by an infinitesimal amount, evaluated identically
  for both triangles sharing an edge, so the count stays even for any
  closed surface. An odd count means a hole and is an error; facet normals
  and winding are ignored.
- **A whole model, cut to the neighbourhood of the shot.** Nothing about an
  arbitrary shape is symmetric, so there is no half model. Only the solid
  within `armor_extent_radii` of the shot line is filled, and `max_depth`
  optionally stops the model short of the far wall of a hollow shape. The
  projectile starts just clear of the first armor in its path, which may
  lie ahead of the point where the shot line itself enters.

## Consequences

- A plate given as a mesh and struck at 45° is perforated like the same
  plate given as layers: the residual speeds of the whole and the half
  model differ by 2 % (`crates/rap-app/tests/mesh_armor.rs`).
- A mesh model costs about twice the half model of a plate of the same
  size, on a solver that is still CPU-only.
- The solid is one material. Layered or composite solids would need one
  mesh per material.
- The cut faces of the modelled piece are free surfaces, as the edges of a
  plate already are; the surrounding armor does not confine it.
- STL carries no units; a wrong `metres_per_unit` shows up as a shot that
  misses, a mesh smaller than a particle, or a grid that is too large.
