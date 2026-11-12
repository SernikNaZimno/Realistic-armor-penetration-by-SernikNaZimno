# Realistic armor penetration by SernikNaZimno

A physics simulator of armor penetration in which both the armor and the
penetrator deform. It uses the Material Point Method (MPM) with
Johnson–Cook strength and damage and a Mie–Grüneisen equation of state.

Status: **M0 — repository skeleton.** No physics is implemented yet.

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
- M2 — 2D CPU solver, validated with Taylor impact and plate impact
- M3 — 2D penetration: contact, damage, comparison with Tate and Forrestal models
- M4 — scenario files and CLI
- M5 — user interface: projectile editor (APBC, APFSDS), armor editor, viewer
- M6 — GPU solver and 3D
- M7 — ceramics, STL import, user documentation
