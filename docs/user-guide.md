# User guide

*Realistic armor penetration by SernikNaZimno* simulates a projectile
striking armor, with both deforming. This guide covers building the
program, the desktop application, the command line, the scenario file and
how to read the results. For how the solver works and how it was checked,
see the [README](../README.md) and the decision records in [adr](adr).

## Contents

1. [Building](#building)
2. [Desktop application](#desktop-application)
3. [Command line](#command-line)
4. [Scenario file](#scenario-file)
5. [Choosing the settings](#choosing-the-settings)
6. [Reading the results](#reading-the-results)
7. [Limits of the model](#limits-of-the-model)
8. [Troubleshooting](#troubleshooting)

## Building

You need the stable Rust toolchain (`rustup` installs it). From the
repository folder:

```bash
cargo build --release
```

The first build downloads and compiles the dependencies and takes several
minutes. It produces two programs in `target/release`: `rap-ui`, the
desktop application, and `rap`, the command line. Always use `--release`;
a debug build runs the simulations many times slower.

## Desktop application

```bash
cargo run --release -p rap-ui
```

The window has a toolbar, the editors on the left, the particle view in
the middle and the results on the right. It opens with a long-rod example.

### Editing a scenario

- **Projectile.** Choose APBC (a full-caliber shot with an optional
  ballistic cap) or APFSDS (a long rod). Set the diameter, the length of
  the cylindrical part, the nose shape and the material. The nose length
  is added to the body length. Switching the type keeps the dimensions.
- **Armor.** Choose between plates and a solid from an STL file. For
  plates, set the obliquity: 0° is a perpendicular hit. Add layers
  from the struck face inwards, each with a material, a thickness and,
  from the second layer on, the gap in front of it. The arrows reorder
  layers.
- **Face shape.** A table of points, each a distance from the shot line
  and a displacement of the armor face along its normal at that distance.
  Positive displacement moves the face away from the projectile. The face
  is straight between points and flat beyond the last one, and every
  layer follows it. With no points the armor is flat.
- **Solid from an STL file.** Pick the file and the units it is written
  in, the material, a point the shot passes through and the direction of
  flight; see [Armor from an STL file](#armor-from-an-stl-file). Save the
  scenario next to the mesh, or give the full path of the mesh.
- **Materials.** Built-in materials cannot be changed. *Copy to edit*
  makes a copy that belongs to the scenario and appears in the material
  lists under a new identifier.
- **Simulation.** The numerical settings; see
  [Choosing the settings](#choosing-the-settings).

The view redraws the model after every change. If the scenario is invalid,
the reason is shown in red above the editors and *Run* is disabled.

### Running

*Run* starts the simulation in the background; *Pause* and *Resume*
suspend and continue it; *Reset* discards it and returns to editing. The
editors are locked while a run exists.

In the view, drag to pan, scroll to zoom and double-click to reset.
*Colour by* selects what the colours show. An axisymmetric model is drawn
mirrored about its axis. For a three-dimensional model the view shows the
slice in the symmetry plane; for armor from an STL file, the slice through
the shot line that is as close to upright in the file as the shot allows.

*New* loads an example, *Open…* and *Save as…* read and write scenario
files, the same ones the command line uses.

## Command line

```bash
cargo run --release -p rap-cli -- preset apfsds -o shot.json
```

writes an example scenario (`apbc` or `apfsds`) to edit.

```bash
cargo run --release -p rap-cli -- run shot.json
```

simulates it. While it runs it prints the time, the depth reached, the
residual speed and the failed fraction of the armor; at the end it prints
whether the armor was perforated. Options of `run`:

| Option | Effect |
|---|---|
| `-o`, `--output-dir <DIR>` | where to write the results; default `<scenario name>-results` next to the scenario |
| `--no-snapshots` | write only the summary |
| `--gpu` | use the GPU solver, whatever the scenario says |

```bash
cargo run --release -p rap-cli -- materials
```

lists the built-in materials.

## Scenario file

A scenario is a JSON file. Lengths are in metres, speeds in metres per
second, times in seconds, angles in degrees, stresses in pascals. Three
complete examples are in [examples](examples):

- [ceramic-spaced-armor.json](examples/ceramic-spaced-armor.json): a rod
  against a ceramic-faced plate with a spaced backing plate;
- [capped-shot-domed-plate.json](examples/capped-shot-domed-plate.json): a
  capped shot of a custom steel against a plate with a shaped face;
- [oblique-plate.json](examples/oblique-plate.json): a rod against a plate
  at 60°, in three dimensions;
- [cast-dome.json](examples/cast-dome.json): a rod against the wall of a
  hollow dome read from [cast-dome.stl](examples/cast-dome.stl).

Unknown fields are rejected, so a misspelt name is reported instead of
ignored.

### Top level

| Field | Meaning |
|---|---|
| `format_version` | must be `1` |
| `scenario` | the projectile, the armor and the impact speed |
| `settings` | optional; numerical settings |
| `materials` | optional; materials defined by this file |

### `scenario.projectile`

`"type": "apfsds"` — a long rod:

| Field | Meaning |
|---|---|
| `rod_diameter` | diameter |
| `rod_length` | length of the cylindrical part, behind the nose |
| `nose` | nose shape, see below |
| `material` | material identifier |

`"type": "apbc"` — a full-caliber shot:

| Field | Meaning |
|---|---|
| `caliber` | body diameter |
| `body_length` | length of the cylindrical part, behind the nose |
| `nose` | shape of the penetrating nose under the cap |
| `cap` | `null`, or `length` (ahead of the shoulder of the nose, at least half the caliber), `wall_thickness` and `material` of the ballistic cap |
| `material` | material identifier |

`nose` is one of `{ "shape": "flat" }`, `{ "shape": "hemisphere" }`,
`{ "shape": "cone", "half_angle_deg": … }` and
`{ "shape": "ogive", "caliber_radius_head": … }`, where the radius head is
the ogive radius divided by the body diameter and is at least 0.5.

### `scenario.armor`

| Field | Meaning |
|---|---|
| `layers` | plates from the struck face inwards, each with `material`, `thickness` and optionally `gap_before`, the empty space in front of it |
| `obliquity_deg` | angle between the shot line and the plate normal, from 0 up to but not including 90 |
| `face_profile` | optional; points with `lateral`, the distance from the shot line, and `offset`, the displacement of the face along its normal there; `lateral` must increase from point to point |

Instead of these three fields the armor can be a solid of any shape:
`"armor": { "mesh": { … } }`.

### Armor from an STL file

| Field of `mesh` | Meaning |
|---|---|
| `file` | the STL file, binary or text; a relative path starts at the folder of the scenario file |
| `metres_per_unit` | length of one unit of the file in metres; default `0.001`, a file in millimetres |
| `material` | material identifier of the whole solid |
| `aim_point` | a point on the shot line, in the coordinates and units of the file |
| `direction` | direction in which the projectile flies, in the coordinates of the file; its length does not matter |
| `max_depth` | optional; how far behind the point of impact the solid is modelled, along the shot line, in metres |

The projectile flies along the line through `aim_point` and strikes where
that line first enters the solid, so the aim point may be anywhere on the
line, also inside the solid or behind it. The mesh must be one or more closed
surfaces without holes. Only the part of the solid within
`armor_extent_radii` of the shot line is modelled. A hollow shape is
modelled through to its far wall unless `max_depth` stops short of it; the
reported depth and perforation refer to the last surface the shot line
leaves within the modelled part.

Such armor always gives a three-dimensional model of the whole
neighbourhood of the shot line, which costs about twice as much as the
half model of an oblique plate of the same size.

`scenario.impact_speed` is the speed of the projectile at impact.

### `settings`

Every field is optional.

| Field | Default | Meaning |
|---|---|---|
| `cells_per_radius` | 4 | grid cells across the projectile radius |
| `particles_per_cell_edge` | 2 | particles along each edge of a cell |
| `armor_extent_radii` | 10 | half-width of the armor, in projectile radii |
| `friction` | 0 | Coulomb friction between projectile and armor |
| `end_time` | automatic | simulated time; by default twice the time the projectile needs to travel its own length plus the armor along the shot line |
| `snapshots` | 20 | number of snapshots written by the command line |
| `backend` | `"cpu"` | `"cpu"` or `"gpu"` |
| `oblique_model` | `"solid"` | `"solid"` or `"plane_strain"`; used only when the obliquity is not zero |

### `materials`

Each entry is a complete material: `id`, `name`, an equation of state
`eos`, a `strength` model, a `damage` model, `spall_strength`,
`specific_heat` and `taylor_quinney`. A material whose `id` matches a
built-in one replaces it. The easiest way to write one is to copy a
built-in material in the desktop application, adjust it and save the
scenario; the second example file contains the result for a steel.

Metals use `"model": "mie_gruneisen"` for `eos` and
`"model": "johnson_cook"` for `strength` and `damage`. Ceramics use
`"polynomial"` and `"johnson_holmquist"`.

## Choosing the settings

**Resolution.** `cells_per_radius` sets the cell size and dominates both
accuracy and cost. In two dimensions the cost grows roughly with its
cube, in three with its fourth power. The default of 4 is for a quick
look: in the built-in long-rod example the depth reached after 60 µs is
39.8 mm at 4 cells per radius, 49.8 mm at 10 and 53.0 mm at 16. Use 10 or
more for numbers you intend to rely on, and compare two resolutions
before trusting either.

**Kind of model.** A perpendicular impact is always axisymmetric, which is
exact and cheap. An oblique impact is three-dimensional by default, which
is faithful and expensive: the example in this guide, at 3 cells per
radius, has 51 000 particles and takes about a minute, and a long rod at
useful resolution needs millions. `"plane_strain"` is a fast
approximation in which the projectile is a slab. In the one case compared,
the example above, it let the rod through faster (1269 m/s against
1168 m/s); it is only good for a qualitative look.

**Solver.** The GPU solver is somewhat faster on large two-dimensional
models and gives the same results to five or six digits. It does not run
three-dimensional models or ceramics, and there is no fallback: asking
for it where it cannot run is an error.

**Armor width.** `armor_extent_radii` should be large enough that the
edge of the plate does not matter. Ten radii is adequate for thin plates;
thick plates penetrated deeply need more, and the cost grows with it.

**Time.** The automatic end time is generous for perforations and may be
short for a projectile that is slowly brought to rest. If the depth is
still rising in the last rows of the report, set `end_time` higher.

## Reading the results

The summary reports, along the shot line:

| Quantity | Meaning |
|---|---|
| depth | how far the projectile has advanced past the armor face, at most the length of the shot line inside the armor |
| perforated | whether at least 1 % of the projectile mass is behind the armor and still moving at more than 5 % of the impact speed |
| residual speed | mean speed along the shot line of the projectile material behind the armor |
| residual mass | fraction of the projectile mass behind the armor, eroded debris included |
| failed fractions | fractions of the projectile and of the armor mass that have failed |

The command line writes `summary.json` with these values at every
snapshot and at the end, and one `particles_NNNN.vtk` file per snapshot.
Open the `.vtk` files in ParaView as a series; each particle carries its
body, material, damage, plastic strain, pressure, equivalent stress,
temperature, density and velocity. Colouring by `damage` or
`plastic_strain` shows the crater; a threshold on `body` isolates the
projectile.

## Limits of the model

Treat the output as a physically grounded estimate, not a measurement.

- The material parameters are literature values that have not been
  checked against primary sources or calibrated.
- A material point fails all at once, so fracture patterns depend on the
  grid and perforating rods are reported as mostly failed debris.
- Armor layers are bonded to each other.
- The face shape of plates is the same on both sides of the shot line.
- Armor from an STL file is one material throughout.
- Contact acts about one grid cell before bodies touch.
- Explosive reactive armor, sabots and the flight of the projectile are
  not modelled.

## Troubleshooting

| Message | What to do |
|---|---|
| `material … is not defined` | check the identifier against `rap materials`, or define the material in the file |
| `the grid would have … nodes` | lower `cells_per_radius` or `armor_extent_radii`; for an oblique impact, consider `"oblique_model": "plane_strain"`; for armor from an STL file, set `max_depth` |
| `cannot read … .stl` | the mesh is looked for next to the scenario file; move it there or give its full path |
| `the mesh is not a closed surface` | repair the mesh in the program that exported it; every edge must be shared by two triangles |
| `the shot line misses the armor mesh` | check `aim_point`, `direction` and `metres_per_unit` against the coordinates of the file |
| `the armor mesh is smaller than one particle` | `metres_per_unit` is probably wrong: a file in millimetres read as metres is a thousand times too large, the reverse a thousand times too small |
| `no compatible GPU was found` | use the CPU solver; the GPU solver needs a Vulkan, DirectX 12 or Metal adapter |
| `the GPU solver cannot run three-dimensional models yet` | use the CPU solver, or `"oblique_model": "plane_strain"` |
| `material … uses a model the GPU solver does not implement` | the scenario contains a ceramic; use the CPU solver |
| `scenario format version … is not supported` | the file was written by a different version of the program |
| `unknown field …` | a field name is misspelt or does not belong where it is |
