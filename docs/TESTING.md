# Validation history and GPU troubleshooting

Validated data/renderer milestones and how to investigate browser adapter issues. Numerical outputs and linked artifacts are **dated validation records**, not promises of current transfer speed or universal GPU stability.

[Documentation index](INDEX.md) · [Project README](../README.md)

---

## Status

**Data ingestion and first 3D massing build validated on real Downtown LA data.**
Rust CI passes, and a successful live Overture build on October 15, 2026
produced a geographic city JSON and a low-poly 3D building OBJ:

- **1,587** clipped transportation segments (including pedestrian/rail/path features).
- **563** building footprints, of which **537** have recorded height values.
- **563 / 563** imported buildings meshed successfully (zero skipped).
- **38,210** mesh vertices and **21,888** triangles.
- **26** render-only estimated heights for buildings without recorded heights.

- **1,540** transportation spans meshed (472 road, 943 pedestrian, 114 steps, 11 rail); **47** unknown/underground spans intentionally excluded.
- A single combined glTF 2.0 binary (.glb) contains **34,660 triangles** in six layers (reference ground, buildings, roads, pedestrian paths, steps, rail).
- **1,719,448 bytes** total for the uncompressed GLB, independently loaded through a separate glTF-capable 3D library.

**October 15 browser milestone:** headed Chromium on software Vulkan
confirmed genuinely rendered Downtown LA rooftops, streets and building
facades (not merely a running WASM process). The automated pixel check saw
separate sky, building, street and terrain colors and passed keyboard camera
interaction. [Validated build and screenshot](https://github.com/sguzman/urbarium/actions/runs/38725915571).
The shader and geometry are procedural Rust/WebGPU, with no screenshot
substitution or static fake viewport. The dataset remains a small downtown
pilot, not LA County coverage.

The sample covers approximately 1.5 × 1.2 km around Bunker Hill / Civic
Center, not the whole city. The outputs include real indexed 3D geometry and a first
**Rust/WASM/WebGPU flythrough viewer**. Chromium automated QA successfully
loaded the Downtown LA scene and exercised camera movement on October 15, 2026.
The viewer now has footprint-aware walking and a sampled terrain surface; roads
remain centerlines, and fidelity of individual facades is not established.

[Validated real-data build, OBJ and combined GLB artifacts](https://github.com/sguzman/urbarium/actions/runs/38680594189)
(artifact expires after seven days). No source data is committed to Git.

## Browser GPU adapter diagnosis (October 17, 2026)

Open `gpu-test.html` from the live viewer, then select **Run adapter test and
download JSON**. This is a standalone test: it does not load WASM or city
geography, launch an animation loop, or intentionally engage the RTX 3090.
It checks the hardware-facing WebGL2 renderer and browser WebGPU low-power
adapter, and writes `urbarium-gpu-report-*.json` locally. The optional
high-performance adapter test is disabled by default because it could wake
an additional GPU. Upload the downloaded JSON for analysis; no network
telemetry or personal files are collected. A browser may hide adapter names.

Normal WebGPU viewer startup now prefers a low-power adapter on the web and
**automatically switches to WebGL2 if wgpu reports a CPU/software WebGPU
adapter**. The `?allow-software=1` override is deliberately for CI / debugging
only and may use significant CPU power. Automated browser QA exercises the
standalone downloadable report and the software-adapter fallback separately.
A result from software Chromium CI cannot establish real AMD hardware selection.

If the normal Edge probe returns `google/swiftshader`, run the optional
`bash ~/Downloads/urbarium-edge-vulkan-probe.sh` launcher (download it from
the repository or the chat artifact). It opens a **separate temporary Edge
profile**, explicitly enables Chromium Vulkan and keeps the same
`/dev/dri/renderD128` iGPU render-node override as the normal Edge session.
It does **not** overwrite flags, settings or the normal user profile. Close
it immediately if unstable; this is a diagnostic, not a permanent graphics
configuration. Click **Run adapter test and download JSON** in the opened
gpu-test page, then upload the report. The report includes
`launchVariant: isolated-vulkan` to distinguish the environment. This probe
never starts Urbarium 3D rendering or requests a high-performance adapter
unless the user explicitly enables that checkbox.

### Hardware WebGPU rendering diagnostic (isolated X11/Vulkan)

The earlier low-power adapter report identified `google/swiftshader` in
normal Wayland Edge, while the isolated Vulkan adapter probe identified
`amd/rdna-2`. The first attempt at a real X11/XWayland rendering test
failed before Edge launched, because Jobflick's Hyprland session did not
provide `DISPLAY`. The fixed launcher therefore uses **`--ozone-platform=wayland`**,
the same isolated Vulkan adapter-selection path that demonstrably returned
AMD hardware. Chromium may warn that Vulkan is incompatible with Wayland;
that remains a real concern, so **actual rendering and errors must be checked**
before recommending this setup. It keeps the normal AMD render-node override
and does not intentionally use the NVIDIA RTX 3090.

Run the one-time `scripts/edge-vulkan-render.sh` launcher through Jobflick
(Execarium job, `session: hyprland`, `gpu: inherit`). It opens the **real
Urbarium 3D viewer** with `renderer=webgpu`, `quality=low`, and
`launch=wayland-vulkan-render`. Wait about 15 seconds, then click **Download 3D
rendering report** and upload the generated `urbarium-render-report-*.json`.
The downloaded JSON contains selected adapter, WebGPU device class, frames,
measured presented FPS samples, errors and resolution; it does not measure
wall power. If it crashes before a report can download, record that symptom,
close the separate window, and keep using normal WebGL2. This is a controlled
hardware rendering experiment, **not** a default Edge setting.

## Plans

See [roadmap](ROADMAP.md). Data is stored only on the developer's machine under ignored `data/raw` and `data/processed`; code is MIT, **source datasets retain their separate licenses**.

