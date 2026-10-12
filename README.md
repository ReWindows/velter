# Velter

Velter is a live Windows process lab aimed at offline Unity games. It combines a transparent overlay with an r2-style CLI over one Rust core.

## Current build

- Transparent always-on-top overlay; `Insert` switches interaction/click-through and `F9` pauses the target.
- Mono/IL2CPP detection from loaded modules.
- Exact and successive `i32`, `i64`, `f32`, and `f64` memory scans.
- Live table values, edits, freezing, and pinned HUD rows.
- Debug attach/detach that does not kill the target.
- DR0 hardware write watchpoints across target threads.
- Live x86-64 disassembly, bounded CFG recovery, raw byte patches, and reversible patch history.
- CLI process listing, memory IO, scans, disassembly, CFG, pause/resume, and write-watch logs.
- Native Mono ECMA-335 and IL2CPP v24.1 metadata browsing with normalized assemblies, types, fields, methods, tokens, and parameter counts.

This is intended for offline games. Do not attach it to multiplayer games or anti-cheat protected processes.

## Run

```powershell
.\target\release\velter.exe
.\target\release\velter-cli.exe ps unity
.\target\release\velter-cli.exe open Bonerooms
.\target\release\velter-cli.exe meta "D:\path\Assembly-CSharp.dll" Player
.\target\release\velter-cli.exe meta "D:\path\global-metadata.dat" Player
```

Inside the CLI:

```text
modules
s GameAssembly.dll+1234
px 64
pd 20
agf 512
wx 90 90 @ 0x7ff...
scan i32 300k
next changed
hits 20
watch 0x12345678 4
log
dp-
```

Games must use windowed or borderless presentation for the external overlay. Exclusive fullscreen bypasses desktop composition.

## Workspace

- `crates/velter-core`: processes, memory, scans, debugger/watchpoints, disassembly, CFG.
- `crates/velter-gui`: transparent egui/Glow overlay.
- `crates/velter-cli`: interactive terminal frontend.
- `crates/velter-runtime`: checked Mono and IL2CPP metadata parsers and a backend-neutral runtime model.
- `references/`: pinned Cpp2IL and Il2CppDumper source used as compatibility references.
- `Plugins/`: built first-party Cpp2IL plugins used for compatibility benchmarking.
- `benchmarks/`: real-game cross-check outputs and the parser accuracy report.

## Next runtime layers

1. Mono JIT address-to-`MonoMethod` resolution and IL/C# method browsing.
2. IL2CPP `GameAssembly.dll` code/metadata registration parsing and native method maps.
3. Register snapshots and backward value slicing on watchpoint hits.
4. Assembler/code-cave manager with relocation-aware trampolines.
5. Persistent project files for labels, patches, watches, and layouts.

## License

Velter is licensed under the GNU Lesser General Public License v3.0 only
(`LGPL-3.0-only`); see `COPYING.LESSER` and `COPYING`. Code ported from radare2
keeps radare2's LGPLv3 terms. radare2 plugins that are GPL-licensed must not be
ported into LGPL crates. `vendor/il2dump` remains under its own MIT license.
