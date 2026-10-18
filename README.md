# Urbarium

**Real cities, reconstructed from geographic data and procedural geometry.**

Urbarium turns source roads, building footprints and elevation into a compact,
walkable 3D city. It is built primarily in **Rust**, with a native data/mesh
pipeline and a browser viewer using **WebAssembly + wgpu (WebGL2/WebGPU)**.

**[Explore the live Downtown Los Angeles pilot](https://sguzman.github.io/urbarium/)**

## What works

- Source-derived buildings, terrain and road/path surfaces for a bounded
  **Downtown LA** area (approximately 1.5 × 1.2 km).
- An independently acquired **Little Tokyo / northern Arts District** section;
  two-section joined world **enabled by default** (`?world=0` for standalone diagnosis).
- First-person movement, fast-travel toggle, collision against building
  footprints, and a north-up minimap with real geographic landmark anchors.
- Progressive rendering: terrain and streets first, then buildings and
  optional neighboring geometry. Use `?progressive=0` for the full-scene
  startup fallback.
- WebGL2 by default, optional WebGPU, graphics diagnostics and persistent
  content-versioned geography/WASM caches.
- Rust exports for indexed building meshes, transport meshes and combined
  glTF 2.0 (`.glb`) scenes.

**Important limitations:** this is a geographic prototype, not a complete city
or accurate architectural recreation. Transport centerlines are not surveyed
curbs, collision is approximate, and **unfinished freeway geometry is hidden**
from the default view. The experimental freeway preview is `?bridges=1`;
it should not be mistaken for a faithful interchange. See
[geographic limitations](docs/DATA.md) and the
[freeway acceptance gate](docs/FREEWAY_RECONSTRUCTION.md).

## Getting started

To **use the city**, open the [browser viewer](https://sguzman.github.io/urbarium/).
No local installation is required.

For local Rust development and real-source asset generation:

```sh
cargo test
bash scripts/build-downtown-overture.sh
cargo run --release -- glb \
  --city data/processed/downtown-la.json \
  --out data/processed/downtown-la.glb
```

The data build needs the appropriate Overture tooling and network access.
Its generated inputs and assets are kept out of Git. For dependencies,
alternatives (including OSM), other exporters and deployment details, use the
[complete build guide](docs/GETTING_STARTED.md).

## Documentation

Start with the **[documentation index](docs/INDEX.md)**. Technical detail is
organized by purpose rather than accumulated in this README:

| Topic | Guide |
| --- | --- |
| Run, build and export | [Getting started](docs/GETTING_STARTED.md) |
| Controls, HUD and landmarks | [Viewer guide](docs/VIEWER.md) |
| World coordinates, tiles and geometry | [World and geometry](docs/WORLD.md) |
| Startup, caching and performance | [Performance and caches](docs/PERFORMANCE.md) |
| Testing and browser GPU diagnosis | [Testing](docs/TESTING.md) |
| Provenance, licenses and GIS limitations | [Geographic data](docs/DATA.md) |
| Optional source pipeline | [Overture](docs/OVERTURE.md) |
| Freeway fidelity and acceptance criteria | [Freeway reconstruction](docs/FREEWAY_RECONSTRUCTION.md) |
| Planned work and remaining gaps | [Roadmap](docs/ROADMAP.md) |

## Project boundaries

The pilot prioritizes **verifiable source geometry** over invented buildings,
road connections or bridge heights. Large raw and processed geographic
datasets are not committed to this repository. Source providers retain their
own attribution and redistribution requirements; the project code is
[MIT-licensed](LICENSE).

Further work targets camera-local chunked geometry, visible-section culling,
regional streaming and more faithful grade-separated transport. See the
[roadmap](docs/ROADMAP.md).
