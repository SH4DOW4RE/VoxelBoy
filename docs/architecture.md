# Architecture

## Design goals

- Emulator cores are replaceable at runtime.
- The minimum integration is a completed video frame plus audio.
- A core adapter may additionally expose semantic scene data.
- Game profiles remain independent of the selected emulator core.
- Windows 11 and Linux are first-class targets.
- Proprietary ROMs, BIOS images, emulator binaries, and reference-app assets
  are never shipped in the repository.

## Data flow

```text
ROM -> selected core -> frame/audio --------------------+
                     -> optional semantic scene data ---+-> voxelizer
game profile -------------------------------------------+      |
                                                             renderer
```

The frontend owns timing, input, persistence, core selection, rendering, and
profiles. A backend owns emulation. This prevents renderer behavior from being
tied to one core's lifecycle or internal data layout.

## Core integrations

The general-purpose adapter loads libretro-compatible dynamic libraries. This
supplies the common denominator needed for user-selectable cores: game loading,
software-rendered video, audio, input, timing, and save states.

Libretro's normal video callback supplies a composited frame. That is enough
for the fallback voxelizer but cannot reliably identify which pixels belong to
the Game Boy background, window, or sprites. VoxelBoy therefore defines an
optional `SemanticScene` capability. It can be implemented by:

1. a native adapter using a core's debugging or PPU inspection API;
2. an explicitly supported extension to a libretro core;
3. a VoxelBoy-owned core in the future.

An adapter that lacks this capability remains fully usable.

### Current libretro limitations

- Libretro's callback ABI is process-global, so VoxelBoy currently permits one
  active libretro core per process.
- Hardware-rendered cores are not accepted yet; Game Boy-family cores normally
  provide the software framebuffers supported by the adapter.
- Core options and system/save directories are not exposed yet.
- Persistent save RAM has not been connected to the frontend yet.
- Native cores are executable code and are not sandboxed.

## Voxel conversion tiers

### Tier 0: framebuffer fallback

Every core supports this mode. Pixels are quantized into cells, a profile
chooses the background color, and remaining cells are extruded. It works
without core-specific knowledge but can produce unstable geometry during
animation or scrolling.

### Tier 1: analyzed framebuffer

Temporal tracking, connected-component analysis, palette grouping, and
profile hints stabilize objects across frames. This remains core-independent.

### Tier 2: semantic scene

The adapter supplies planes, tiles, sprites, draw order, transforms, and stable
identifiers. Profiles can then assign depth by semantic layer or object. This
is the preferred path for polished game profiles.

## Profiles

A profile is keyed by ROM hashes and versioned independently of cores. It may
contain:

- crop and overscan rules;
- palette and transparency rules;
- depth rules for layers, tiles, sprites, or screen regions;
- object grouping and temporal-stability hints;
- camera and lighting presets;
- game-specific exceptions.

Profiles must not patch emulation correctness. If two cores produce materially
different game state, that remains a core compatibility issue.

## Security boundary

User-supplied native cores are executable code and are not a security sandbox.
The UI must clearly show a core's path and origin before loading it. Hashes can
identify known builds, but they do not make an untrusted library safe.
