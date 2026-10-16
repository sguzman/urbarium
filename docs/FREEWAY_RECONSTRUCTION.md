# Freeway reconstruction: evidence and acceptance gate

Status: **prototype retired from default browser view** (2026-10-16).

## Why the previous implementation failed

- Overture transport lines encode geographic centerlines and scoped bridge,
  tunnel and relative level tags. They do **not** give surveyed, continuous
  three-dimensional deck geometry or engineering clearances.
- The ~1.5 km × 1.2 km Downtown LA pilot clips US freeway corridors and their
  approaches. Raising individual tagged subsegments created arbitrary ends.
- The 5.8 m tier heights, fixed-width ribbons, supports and heuristic ramps
  generated disconnected slabs. Frame-count-only CI previously missed
  flat-looking or implausible views.
- Intersections by 2D proximity are *not* road junctions: grade-separated
  crossings must not be welded together.

## Present default

The normal browser viewer keeps buildings, terrain, actual road footprints
approximated from source centerlines, landmarks, minimap, and caching.
Bridge-tagged sections appear as ordinary terrain-draped roads. No unfinished
bridge-only meshes are allocated or drawn. This is **not** a trustworthy
representation of freeway altitude; it is an intentionally modest
cartographic fallback. The debug opt-in `?bridges=1` preserves the previous
experiment, isolated from the default render.

## Next implementation, in dependency order

1. **Source coverage and manifest.** Define connected US-101 / I-110 road
   corridors overlapping Downtown LA, including approaches beyond the
   current pilot boundary. Use versioned Overture transport features and
   stable IDs, plus provenance. Detect missing links and roads truncated
   by the acquisition box; never invent a continuation. Load by tiles /
   corridor chunks rather than increasing the entire city payload.
2. **Directed topological network.** Preserve endpoint/connector IDs and
   Overture connector/link semantics, turn constraints, bridge flags, level
   scopes and tunnel status. Split edges at evidence-backed changes.
   Determine motorway carriageway separation. Crossing lines at different
   levels remain topologically independent.
3. **Verified grade profile.** Create per-directed-edge longitudinal profiles
   that join continuously at *verified* connectors. Relative level alone
   cannot determine meters; collect surveyed transportation/road elevation
   where available. Mark estimated sections as uncertain. Explicitly
   distinguish bridge, embankment, cutting, tunnel and ground roads.
4. **Continuous actual mesh.** Generate joined deck strips/shoulders/
   guardrails for proven connected roads, with stable cross section across
   transitions and non-overlapping lane markings. Place supports beneath
   decks only where context allows, and omit where evidence is insufficient.
   Avoid visually inventing a bridge at unverified cross streets.
5. **QA beyond passing renders.** Count tagged and untagged corridor
   segments, connected components, unmatched interior endpoints,
   dangling ramps, cross-level welding, nonfinite/degenerate triangles,
   grade discontinuities, terrain intersections and open cuts.
   Save automated oblique/top/eye-height scene images for each interchange
   in both WebGPU and WebGL2. Check actual pixel variation and scene
   boundaries, not just FPS or frame count.
6. **GPU/stability budget.** Track per-tile vertex/index memory, visible
   draw calls and LOD impact. Do not regress the working default when
   enabling detailed freeway scenes. Continue diagnosing user Edge/Hyprland
   crashes independently of geographic correctness.

## Acceptance criteria before default reintroduction

- No dangling elevated interior end unless the source *actually* ends there;
  missing coverage must show as missing data, not a fabricated bridge.
- Grade profiles continuous at proven connectors, with no cliff-style
  elevation changes and no mismatched carriageway joins.
- Over-under crossing order grounded in source evidence; no false junctions
  between levels; no intersections with terrain/buildings or pedestrian
  surfaces that would be physically impossible.
- Complete chosen freeway corridor through the target area, **including**
  approaches outside the old pilot, rather than 20 isolated raised slivers.
- Screenshots of every chosen interchange inspected for plausible
  appearance from aerial and street-level viewpoints, in both backends.
- Default renderer remains stable enough for ordinary exploration.
  Passing headless tests is necessary but cannot prove hardware-specific
  Edge crashes are fixed.

Until these gates hold, keep experimental freeway geometry behind
`?bridges=1`. Do not describe the prototype as finished or realistic.
