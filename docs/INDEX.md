# Urbarium documentation

[Project README](../README.md) · [Live viewer](https://sguzman.github.io/urbarium/)

The README is a short introduction. All implementation notes, commands, data
limitations and historical validation details have homes below.

## Start here

| Document | Contents |
| --- | --- |
| [Getting started](GETTING_STARTED.md) | Setup, real-data builds, OBJ/GLB exports and deployment |
| [Viewer guide](VIEWER.md) | Navigation, keyboard/mouse controls, HUD, minimap, landmarks and sky |
| [World and geometry](WORLD.md) | Joined regions, geographic frames, collisions, terrain and mesh fidelity |
| [Performance and caches](PERFORMANCE.md) | WASM/geography caching, startup phases, GPU safety, diagnostics |
| [Testing and GPU troubleshooting](TESTING.md) | Validation milestones and adapter diagnostic procedures |

## Geographic data and development direction

| Document | Contents |
| --- | --- |
| [Data sources and fidelity](DATA.md) | Licenses, source quality, ingestion constraints and GIS accuracy |
| [Overture pipeline](OVERTURE.md) | Optional Overture buildings acquisition and provenance |
| [Freeway reconstruction](FREEWAY_RECONSTRUCTION.md) | Why unfinished freeway slabs are hidden and the reintroduction gate |
| [Roadmap](ROADMAP.md) | Completed work, outstanding dependencies and regional-expansion plan |

## Working conventions

- Shell commands and paths in the technical guides are **relative to the repository root**, unless otherwise stated.
- The public viewer is a bounded Downtown LA pilot with an adjacent Little Tokyo / northern Arts District section. It is **not** a complete LA reconstruction.
- Geographic source material and generated large datasets remain outside Git; the MIT software license does not replace source-data licensing obligations.
- Date-stamped technical notes record the state or reasoning at that point. Consult the current viewer and roadmap for present behavior.
