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

There is no playable application yet. The libretro adapter is currently a
library; a desktop runner and renderer are the next milestone.

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
