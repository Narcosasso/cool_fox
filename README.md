# VIctory on an RTX 5070 Laptop

## The target of this fork

This fork is tuned for one machine, a gaming laptop:

| | |
|---|---|
| GPU | NVIDIA GeForce RTX 5070 Laptop, 8 GB GDDR7 (Blackwell), with an Intel integrated GPU beside it (hybrid graphics) |
| CPU | Intel Core i7-14650HX: 8 performance and 8 efficient cores, 24 threads |
| RAM | 64 GB DDR5-5600 |
| System | Windows 11 Pro 26H2 |

What that means for the work:

- **8 GB on the card is the limit.** The RTX 5090 runs of the bug journal used
  17-19 GB. What does not fit goes to the PC's shared memory and is read across
  the bus, so card memory comes first: `scripts\play.ps1 -LowVram` (Series S,
  texture levels on demand), `VICTORY_RESIDENCY_PRIORITY=1` (render targets
  stay on the card first).
- **The thread that prepares submissions is the wall** (docs/findings.md), and
  the 64 GB of RAM leave room to do more of its work early:
  `VICTORY_PREDICT=1` measures how much of it the frame before predicts.
- **Hybrid graphics:** the host device is made on the GPU that runs mesh
  shaders, not on Windows' default (`VICTORY_ADAPTER` overrides it). A
  discrete-only (MUX) mode, mains power and the maker's performance profile
  help.
- **Every change is measured**, from the same save, with logs compared by
  `victory compare`; new switches stay off until a comparison on this laptop
  says they help.

The sections below describe the builds and measurements this fork started
from (RTX 5090 and RX 7900 XTX); their figures are not of this laptop.

## Run the codec_d Windows build

Download and extract the Windows x64 ZIP from the [private releases](https://github.com/codec-d/cool_fox/releases/latest). Open PowerShell in the extracted folder and point the launcher at the executable of your own complete Xbox dump.

```powershell
$env:VICTORY_GAME_EXE = 'D:\Games\MyDump\<the title''s executable>.exe'
.\Play-VIctory.cmd pareto-residency
```

The launcher selects Series S and the RTX-tested residency settings. It builds its shader cache locally under `%LOCALAPPDATA%\VIctory\cache-pareto`. Set `VICTORY_CACHE_DIR` to reuse an existing cache outside the checkout. Windows x64, an AVX2 CPU, and a Direct3D 12 GPU with shader model 6.6 and mesh shader support are required.

To build from this checkout, install the pinned Rust toolchain and the MSVC C++ build tools with a Windows SDK, then run `cargo build --release --locked -p victory`. The same launcher finds `target\release\victory.exe`. The sections below describe the upstream AMD fork and its separate launcher.

A fork of [VIctory](https://github.com/Marvisak/VIctory), a compatibility layer
that runs an Xbox Series title natively on Windows from files the user supplies.
This one is about **speed on AMD RDNA 3**: the world at 30 frames a second on
one PC, without giving up the picture. It is not there yet.

**This repository contains no game files, code or assets.** You need your own
copy of the game. It is not affiliated with or endorsed by the game's developer
or publisher.

## Where it stands (2026-11-25)

Tested on one machine only: Windows 11, Intel i9-12900K, 64 GB RAM, Radeon
RX 7900 XTX, 1440p, the Series X profile.

| | |
|---|---|
| Menus, videos, loading screens | 30 fps |
| Prologue, indoors | 50-75 ms a frame (13-20 fps) |
| First mission with the boat | 46-53 ms (19-22 fps) |
| Street chase with many cars and people | 100-165 ms (6-10 fps), single frames far longer |
| Outdoors on foot, the camp | 60-90 ms |
| Flying | 42-57 ms |
| Longest session so far | 41 minutes, no crash |
| Memory | 40-46 GB of private memory: **64 GB of RAM recommended** |

It started at 280 ms a frame in the same scenes two days earlier.

What is known to be wrong:

- Trees, plants and far buildings flicker outdoors in some places.
- Rooms with little direct light are too dark without `-RayTracing`.
- Textures of what just came into view appear late while the title streams.
- The first run with a new driver or a new build stutters while the driver makes
  pipelines (`victory warm` makes them ahead).
- Recording a frame in the world makes the title's own watchdog end it.

## How to run it

You need Windows 10/11 x64, a CPU with AVX2, a Direct3D 12 GPU with shader
model 6.6 and mesh shaders, Rust (the toolchain is pinned), the MSVC build
tools, and your own dump of the title.

```powershell
git clone https://github.com/avatarmanz/VIctory-7900XTX
cd VIctory-7900XTX
copy victory.example.toml victory.toml      # then set your paths, outside this folder
cargo build --locked --profile diag -p victory --features diagnostics
scripts\play.ps1 -Exe <path of your copy's executable>
```

On a laptop with two GPUs the one that runs mesh shaders is taken, not
Windows' default (often the integrated one); the log names every GPU seen
and the one taken (`host GPU`). `VICTORY_ADAPTER` picks another, by its
number in that list or part of its name, e.g. `$env:VICTORY_ADAPTER='rtx'`.

With 8 GB on the card, `scripts\play.ps1 -LowVram` (Series S, texture levels
on demand). To compare two settings, run each from the same save with its log
redirected (`... 2> run-a.log`), then `target\diag\victory.exe compare run-a.log
run-b.log`: median, p90 and worst ms a frame over intervals of play (1,000
draws a frame or more), and the card's memory, of which the PC's shared memory
in use is what did not fit.

`scripts\play.ps1 -Safe` leaves out the newest, faster ways; try it first when
something is wrong. `-RayTracing` turns the title's ray-traced indirect light on.

Before the first run, ahead of time (both can be stopped and go on later):

```powershell
target\diag\victory.exe shaders <the shader archives>   # every shader translated, minutes
target\diag\victory.exe warm --used                     # pipelines earlier runs drew with, for a new driver or build
```

Commands, debugging switches and upstream's own description:
[docs/README-fxceless.md](docs/README-fxceless.md). Every `VICTORY_...` switch is
described in a comment where it is read.

## What is different here

In short; the reasons and the measurements are in [docs/findings.md](docs/findings.md).

- **List calls are written down, not carried out, on the title's threads**
  (`VICTORY_THIN`): eight threads of ours translate them.
- **Submissions are prepared by a thread of ours and joined**
  (`VICTORY_ASYNC_SUBMIT`, `VICTORY_JOIN_PARTS`), with what can be spread over
  threads spread: refreshes, compares (AVX2), descriptor records, landings.
- **What shaders wrote comes back beside the write guards**, by the dwords that
  changed, on a thread of its own (`VICTORY_RELAY`, `VICTORY_LAND`).
- **Buffer fills that need bytes still on the card are done on the card**
  (`VICTORY_CARD_STEPS`) instead of waiting for it.
- **The title's writes are caught more cheaply**: one view opened at a fault,
  blocks that rest, places in the title's code that look before they write.
- **Textures go up on a copy queue of their own** and a slot shows one only after
  its upload ran, which also ended a device loss that came every 15 minutes or so.
- **Boats float**: textures that shaders write and the title's CPU reads go back
  to title memory (from fxceless and Geckomacabre, on by default here).
- **Frame record and replay** with per-pass times and shader variants compared
  turn by turn ([docs/replay.md](docs/replay.md)).

## Credits

This is a tree of other people's work with more on top. The history is kept, and
commits taken from elsewhere name the commit and its author.

- **[VIctory](https://github.com/Marvisak/VIctory)** by Marvisak and its
  contributors: the loader, the runtime services, the Direct3D 12 back end and
  the shader recompiler.
- **[fxceless/victory-fork](https://github.com/fxceless/victory-fork)**: the fork
  this one started from, and the people whose pull requests it carries.
- **[DexNom/VIctory](https://github.com/DexNom/VIctory)**: ray tracing fixes,
  cube gradients, sparse textures.
- **[Belobug/VIctory-Belobug-AMD](https://github.com/Belobug/VIctory-Belobug-AMD)**:
  the ambient light idea, pipeline workers, and work by romainhedouin.
- **[FQY55kai/ZJOU-VICTORY](https://github.com/FQY55kai/ZJOU-VICTORY)**.
- **[myokex/victory-amd](https://github.com/myokex/victory-amd)**: the guard over
  every view of a page (with Spexivus, joroyt and tightfall), and Joshua Odom's
  reads into title memory.
- **Geckomacabre** and **cxy0072019-sketch**: the texture read-back.
- **[XCL](https://github.com/ivall/xcl)** (GPL-2.0-only) was read for how the same
  title behaves under another layer; no code of it is here.
- This fork's changes were made with [Claude Code](https://claude.com/claude-code).

## License

GPL-3.0-or-later, as VIctory. See [LICENSE](LICENSE); the one file that is not
covered is named in [docs/asset-policy.md](docs/asset-policy.md).
