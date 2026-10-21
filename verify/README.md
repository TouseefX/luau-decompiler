# verify — semantic round-trip harness (Roblox-format bytecode)

This directory turns *"does the decompiler produce correct code?"* into a
number instead of an opinion.

Every case runs the same five steps:

1. **compile** the source to bytecode with the official `luau-compile`
   (in the profile's format — see below),
2. **execute the bytecode** itself with `luacrun` (a ~90-line C++ harness over
   `luau_load`) — this is the ground-truth behaviour,
3. **decompile** the bytecode with `luau-decompiler`,
4. **execute the decompiled source** with the `luau` interpreter,
5. **compare stdout, stderr and exit status exactly**.

Anything less than an exact match is a failure. A case that "looks right" but
prints a different value counts as a failure, which is the entire point: the
bugs this catches (a swap emitted as two sequential copies, `-7 % 3` folded to
`-1`, a dropped `local n = start`) all parse cleanly and look plausible.

## Profiles

| profile | what it produces |
|---|---|
| `v9`    | **Roblox-format bytecode**: LBC v9, the version Luau's `LBC_VERSION_TARGET` still targets and the numbering described by the disassembled client `BytecodeBuilder.c`. |
| `v9k<N>` | **planned**: the same chunk with every opcode byte multiplied by odd key `N` mod 256 — the client's per-version scramble (`stored = canonical * encode_key`). Needs a chunk re-serialiser; the harness refuses rather than mislabel canonical chunks. |
| `v14`   | current upstream default (call feedback + fastpcall), for reference. |

`v9` is the profile that matches what a Roblox client actually executes.

## Usage

```sh
# 1. build the Luau tools this harness drives (compiler + VM)
git clone https://github.com/luau-lang/luau
make -C luau -j"$(nproc)" config=release luau luau-compile

# 2. build the bytecode runner against that checkout
g++ -O2 -std=c++17 verify/luacrun.cpp \
    -Iluau/VM/include -Iluau/Common/include \
    -Lluau/build/release -lluauvm -lpthread -o verify/luacrun

# 3. build the decompiler
cargo build --release

# 4. measure
LUAU_DIR=./luau ./verify/harness.sh verify/corpus ./out-v9 v9
```

The harness prints a pass count and writes `report.tsv` plus per-case expected /
actual behaviour for every failure.

## Adding a case

Drop a `.luau` file in `corpus/`. Every file is compiled at `-O0/-O1/-O2` and
`-g0/-g2`, so one file yields six cases. Keep cases portable across the plain
Luau VM: `bit32`, `game`, `workspace` and friends are Roblox-only globals that
do not exist in the standalone interpreter, so a case using them fails on both
sides for reasons that have nothing to do with decompilation (`io` is also not
available).
