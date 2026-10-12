# Pinned references

These checkouts are not committed (see `.gitignore`). Clone them at these
commits to reproduce benchmarks and parity tests.

| Path | Upstream | Commit |
|---|---|---|
| `engine/radare2` | https://github.com/radareorg/radare2 | `f914159025fd` |
| `references/Cpp2IL` | https://github.com/SamboyCoding/Cpp2IL | `b5ad444b8226` |
| `references/Il2CppDumper` | https://github.com/Perfare/Il2CppDumper | `4741d46ba9cd` |

`competition/` is a shallow research corpus; see `competition/README.md` in a
local checkout for how it was gathered.

## Reference radare2 (WSL)

The C build is only the parity oracle, so it is built on Linux where r2 and its
`r2r` test suite are supported, not with MSVC:

```sh
git clone https://github.com/radareorg/radare2 ~/r2-ref
cd ~/r2-ref && git checkout f914159025fd && ./sys/user.sh
```
