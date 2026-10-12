# Runtime parser benchmarks

## NIGHT-RUNNERS PROLOGUE (Unity 2018.4.26f1, IL2CPP metadata 24.1)

Validated on the local offline game build against pinned Cpp2IL and Il2CppDumper checkouts.

| Measurement | Velter | Cpp2IL / Il2CppDumper |
|---|---:|---:|
| Images / assemblies | 64 | 64 |
| Type definitions | 9,640 | 9,640 |
| Field definitions | 55,439 | 55,439 metadata entries |
| Method definitions | 73,786 | 73,786 |
| Code registration VA | not mapped yet | `0x181EACEB0` |
| Metadata registration VA | not mapped yet | `0x181EACF70` |

The `CameraCollider` sample matched Il2CppDumper on method names, tokens
`0x0600005c` through `0x06000061`, and parameter counts/signatures.

Cpp2IL detected metadata 24.1 and reported `Processed 73786 OK`. Il2CppDumper's
`dump.cs` contains 9,640 `TypeDefIndex` entries. Generated reference output is
kept under `cpp2il-night-runners/` and `il2cppdumper-night-runners/`.

## The Bonerooms (Mono)

Velter parsed `Assembly-CSharp.dll` as 287 types, 2,740 fields, and 2,326
methods. Mono validation currently covers ECMA-335 metadata; live Mono JIT
address mapping remains a separate milestone.

## IL2CPP v31.1 coverage (Unity 2022.3)

Static metadata parsing and Cpp2IL method-map counts agree for all currently
available v31.1 benchmarks:

| Game | Assemblies | Types | Fields | Methods | Code registration | Metadata registration |
|---|---:|---:|---:|---:|---:|---:|
| Torque Drift | 105 | 15,260 | 71,211 | 109,687 | `0x18277CDB0` | `0x182D1B140` |
| Among Us | 92 | 18,955 | 84,858 | 136,348 | `0x182A1A450` | `0x182EDFA20` |
| Aim Lab | 244 | 41,033 | 198,134 | 295,777 | `0x1867B60B0` | `0x1871ED2D0` |

Among Us and Aim Lab are used only as static file-format fixtures. Velter does
not attach to or modify them. Drift Legends 2 was still downloading during the
inventory and had no installed runtime files to inspect yet.

## Reference plugin inventory

All six first-party Cpp2IL plugins are built and load from `D:/moddin/Plugins`:

- Build Report
- Control Flow Graph
- Mfuscator Support
- Orbis PKG
- PDB Output
- Stripped CodeReg Support

Il2CppDumper has no equivalent managed plugin loader. Its repository-bundled
IDA, Ghidra, Hopper, Binary Ninja, WebAssembly, header, and struct integration
scripts are present in `references/Il2CppDumper/Il2CppDumper`.
