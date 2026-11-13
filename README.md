# Realistic armor penetration by SernikNaZimno

A physics simulator of armor penetration in which both the armor and the
penetrator deform. It uses the Material Point Method (MPM) with
Johnson–Cook strength and damage and a Mie–Grüneisen equation of state.

Status: **M3 — 2D penetration on the CPU.** The solver handles plane-strain
and axisymmetric models, frictional contact between bodies, ductile damage
and spall. Scenario files, the command line and the user interface are not
implemented yet.

## Validation

The solver is checked against analytical solutions in
`crates/rap-solver-cpu/tests` (4 particles per cell):

| Case | Quantity | Simulated | Reference |
|---|---|---|---|
| Steel plates, ±500 m/s | shock pressure | 20.74 GPa | 20.80 GPa (Hugoniot) |
| | shocked density | 8637.5 kg/m³ | 8643.3 kg/m³ |
| | total energy drift | −0.13 % | 0 |
| Steel plates, ±5 m/s | elastic stress | 228.5 MPa | 228.2 MPa |
| | wave front after 2 µs | 11.4 mm | 11.7 mm |
| Taylor bar, plane strain, 200 m/s | final / initial length | 0.866 | 0.862 (Wilkins–Guinan estimate) |
| Taylor cylinder, axisymmetric, 200 m/s | final / initial length | 0.859 | 0.842 (Wilkins–Guinan estimate) |
| | total energy drift | −0.0001 % | 0 |
| Elastic bar striking an equal bar, 20 m/s | speed handed to the struck bar | 90 % | 100 % (1D wave theory) |
| | momentum drift | 10⁻¹⁴ | 0 |
| Tungsten rod, L/D = 10, into steel, 1500 m/s | penetration / rod length | 0.98 | 0.87 (Alekseevskii–Tate, Y_p = 1.5 GPa, R_t = 4.5 GPa) |
| | crater / rod radius | 1.9 | — |

The long-rod case uses 1 mm cells, so the rod is only four cells in radius;
the Tate strengths are estimates of the flow stress of the two alloys, not
fitted values.

## Known limitations

- Contact is detected on the grid, so bodies start to interact about one
  cell before they touch.
- Material parameters are literature values that have not been checked
  against a primary source.
- Fracture is local: there is no regularisation, so crack patterns depend on
  the grid.

## Layout

| Crate | Responsibility |
|---|---|
| `rap-domain` | projectile, armor, layer, scenario, units |
| `rap-materials` | strength, equation of state and damage models |
| `rap-geometry` | parametric shapes and particle sampling |
| `rap-solver` | solver abstraction |
| `rap-solver-cpu` | reference `f64` MPM solver |
| `rap-solver-gpu` | `f32` MPM solver in wgpu compute shaders |
| `rap-analysis` | post-processing of results |
| `rap-io` | scenarios, checkpoints, export |
| `rap-app` | use cases shared by the front ends |
| `rap-ui` | desktop interface (egui) |
| `rap-cli` | headless front end (`rap` binary) |

Dependencies point strictly downwards: front ends → `rap-app` → services → `rap-domain`.
Design decisions are recorded in [docs/adr](docs/adr).

## Development

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p rap-cli
```

## Roadmap

- M0 — skeleton, lints, CI
- M1 — domain and material models
- M2 — 2D plane-strain CPU solver, validated with Taylor impact and plate impact
- M3 — 2D penetration: contact, damage, spall, axisymmetry, comparison with the Tate model
- M4 — scenario files and CLI
- M5 — user interface: projectile editor (APBC, APFSDS), armor editor, viewer
- M6 — GPU solver and 3D
- M7 — ceramics, STL import, user documentation
