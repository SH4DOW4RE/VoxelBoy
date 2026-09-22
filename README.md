# VoxelBoy

VoxelBoy is an experimental Game Boy-family emulator frontend that turns 2D
games into layered 3D voxel scenes. It is being designed for Windows 11 and
Linux.

The project intentionally separates emulation from presentation. Users will be
able to choose an emulator core, while the voxel renderer consumes either a
normal video frame or richer, optional scene metadata from an enhanced core
adapter.

## Current status

This repository contains the first architectural slice:

- a core-neutral emulator API;
- dynamic loading of software-rendered libretro cores;
- video, audio, input, timing, and save-state bridging for libretro;
- capability negotiation for optional scene metadata;
- a core-independent framebuffer-to-voxel fallback;
- tests for the fallback conversion;
- design notes for libretro and native core adapters.

A basic desktop runner can load a software-rendered libretro core and ROM,
display either the core's 2D framebuffer or a GPU-instanced voxel scene, pace
emulation from the core's reported timing, and forward keyboard input. Audio is
captured but is not played yet.

Run it with:

```sh
cargo run --release -- --core /path/to/core --rom /path/to/game.gb
```

On Windows, the core will normally be a `.dll`; on Linux it will normally be a
`.so`. VoxelBoy automatically detects GB, GBC, and GBA ROMs by extension and
the Game Boy Color cartridge flag. Use `--system` to override detection. The
selected core must advertise support for the ROM's extension.

By default, VoxelBoy exposes `boot/` as the libretro system directory and
`saves/` as the core-managed save directory. These can be changed with
`--system-directory` and `--save-directory`.

### Keyboard controls

| Game Boy input | Keyboard |
| --- | --- |
| D-pad | Arrow keys |
| A / B | Z / X |
| Start / Select | Enter / Backspace |
| GBA L / R | A / S |
| Pause | Space |
| Reset | F2 |
| Quit | Escape |

### Voxel controls

Voxel mode is enabled by default. The automatic converter identifies the most
common color in each frame, then removes only matching pixels connected to the
frame border. Enclosed matching pixels, such as white artwork inside a white
background, are preserved. Remaining pixels are extruded according to
luminance.

| Action | Keyboard |
| --- | --- |
| Toggle 2D / voxel view | V |
| Orbit left / right | Q / E |
| Tilt up / down | R / F |
| Zoom out / in | - / = |
| Decrease / increase depth | [ / ] |
| Reset camera | C |

## Planned system order

1. Game Boy and Game Boy Color
2. Game Boy Advance
3. Super Game Boy, which also requires SNES emulation

## Building

Install a current stable Rust toolchain, then run:

```sh
cargo test --workspace
```

No ROM files are included.

## Credits

VoxelBoy's voxel presentation is inspired in part by 3dSen from Geod Studio.
VoxelBoy is an independent project and contains no 3dSen code or assets.

See [docs/architecture.md](docs/architecture.md) for the core boundary and
voxel-data flow.
