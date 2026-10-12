# Velter engine architecture

The reverse-engineering engine is the primary product. The overlay, CLI, and
future IDE are consumers of the same tested engine.

## Dependency policy

Approved external foundations for the engine:

- `iced-x86` for x86/x64 instruction decoding and encoding.
- `goblin` for executable container formats.
- ReWindows-owned platform libraries and small reviewed C ABI shims.
- The Rust and C standard libraries.

Runtime-format parsers, metadata models, analysis passes, registration search,
method mapping, IL recovery, CFG construction around managed semantics, and
output formats are implemented and maintained here.

`il2dump`, `dnfile`, `thiserror`, Il2CppDumper, and Cpp2IL are competitors and
validation references, not architectural dependencies. Existing dependencies
outside the approved set are migration debt and must not spread into new engine
code.

## Correctness policy

Every supported layout needs:

1. Checked offsets, sizes, multiplication, addition, indices, and strings.
2. An explicit version/layout selection path. No silent closest-version guess.
3. Corrupt and truncated input tests.
4. Real-game fixtures represented by hashes and expected counts, without
   redistributing game files.
5. Cross-checks against at least two independent tools where possible.
6. Benchmarks for parse time and memory use.

Unsupported formats fail with a precise error identifying the missing layout.
Partial output must be labelled as partial.

## Competition gates

Parity claims are feature-specific:

- Metadata: table counts, ownership, names, tokens, signatures, and defaults.
- Binary mapping: code/metadata registration, method pointers, generic methods,
  field offsets, and platform relocations.
- Recovery: native CFG, call graph, IL/pseudocode quality, and confidence data.
- Outputs: stable JSON, C headers, dummy assemblies, PDBs, graphs, and reports.
- Live mode: process mapping, JIT/native addresses, tracing, editing, rollback,
  and crash isolation.

A feature is complete only when its fixtures and competitor comparisons pass.
