#!/usr/bin/env bash
# Semantic round-trip harness for the Luau decompiler.
#
#   harness.sh [corpus_dir] [out_dir] [profile] [opt_levels] [debug_levels]
#
# profile:
#   v9      Roblox-format canonical bytecode (all Roblox-compat fflags off, LBC v9)
#   v9k<N>  same, then every opcode byte multiplied by odd key N mod 256
#           (what a live Roblox client ships: stored = canonical * encode_key)
#   v14     current upstream default (call feedback + fastpcall), for reference
#
# For each case:
#   1. compile   src -> .luac          (the ground-truth program)
#   2. execute   .luac with luacrun    -> EXPECTED behaviour
#   3. decompile .luac -> .dec.lua
#   4. execute   .dec.lua with luau    -> ACTUAL behaviour
#   5. require an exact match of stdout, stderr and exit status.
#
# Exit 0 iff every case matches.
set -u

CORPUS="${1:-$(dirname "$0")/corpus}"
OUT="${2:-./out}"
PROFILE="${3:-v9}"
OPTS="${4:-0 1 2}"
DEBUGS="${5:-0 2}"

# Point LUAU_DIR at a luau-lang/luau checkout built with `make config=release`.
LUAU_DIR="${LUAU_DIR:-./luau}"
LUAU_COMPILE="${LUAU_COMPILE:-$LUAU_DIR/luau-compile}"
LUAU_VM="${LUAU_VM:-$LUAU_DIR/luau}"
LUACRUN="${LUACRUN:-$(dirname "$0")/luacrun}"
DEC="${DEC:-./target/release/luau-decompiler}"
TMO="${TMO:-15}"

# Roblox-compat fflags: produced bytecode must be LBC v9 (what Studio ships).
ROBLOX_FLAGS="LuauCompileFastpcall=false,LuauCompileEmitVectorDouble=false,LuauBytecodeCostModel=false,LuauEmitCallFeedback=false"

KEY=""
case "$PROFILE" in
    v9)    CFLAGS="--fflags=$ROBLOX_FLAGS" ;;
    v9k*)
        # NOT IMPLEMENTED. Applying the client's `* encode_key` scramble needs a
        # full chunk re-serialiser (opcode bytes live inside per-proto code
        # arrays, and skipping AUX words requires the canonical lengths first).
        # Refusing loudly beats silently measuring canonical chunks under a
        # "shuffled" label.
        echo "profile v9k: opcode-key scramble is not implemented yet;" >&2
        echo "  use v9 (Roblox format, canonical numbering) for now." >&2
        exit 2
        ;;
    v14)   CFLAGS="" ;;
    *) echo "unknown profile: $PROFILE" >&2; exit 2 ;;
esac

mkdir -p "$OUT"
REPORT="$OUT/report.tsv"
: > "$REPORT"

total=0; pass=0; fail_compile=0; fail_decomp=0; fail_run=0; fail_mismatch=0
failed_cases=()

run() { # $1 = runner cmd prefix, $2 = file -> canonical "exit/stdout/stderr" blob
    local out err rc
    out=$("$@" 2>"$OUT/.err"); rc=$?
    err=$(cat "$OUT/.err")
    # Normalise provenance noise so only BEHAVIOUR is compared:
    #  - chunk name / path / line numbers differ between a .luac and a .lua run
    #  - stacktrace frames are runner-specific
    # Strip provenance lines whether the path is relative (./x.lua:5) or
    # ABSOLUTE (/home/u/out/x.lua:5). Only the relative form was handled, so
    # passing an absolute output directory silently turned every passing
    # runtime-error case into RUN_FAIL - an apparent whole-corpus regression
    # that was purely the harness comparing file paths.
    err=$(grep -v '^stacktrace:' <<<"$err" \
        | grep -Ev '^(\.)?/[^[:space:]]*:[0-9]+$' \
        | grep -v '^[[:space:]]*$')
    err=$(sed -E -e "s#\[string \"[^\"]*\"\]#<chunk>#g" -e "s#^\.?/?[^ :]*:[0-9]+: ##" <<<"$err")
    printf 'exit:%d\n--stdout--\n%s\n--stderr--\n%s\n' "$rc" "$out" "$err"
}

for src in "$CORPUS"/*.luau "$CORPUS"/*.lua; do
    [ -e "$src" ] || continue
    name=$(basename "$src"); name="${name%.*}"
    for O in $OPTS; do for G in $DEBUGS; do
        total=$((total+1))
        tag="$name.O${O}g${G}"
        bc="$OUT/$tag.luac"; dec="$OUT/$tag.dec.lua"

        if ! timeout 60 "$LUAU_COMPILE" --binary $CFLAGS -O"$O" -g"$G" "$src" > "$bc" 2>"$OUT/.cerr"; then
            printf '%s\tCOMPILE_FAIL\t%s\n' "$tag" "$(head -c 200 "$OUT/.cerr" | tr '\n' ' ')" >> "$REPORT"
            fail_compile=$((fail_compile+1)); failed_cases+=("$tag:compile"); continue
        fi



        if ! timeout 60 "$DEC" "$bc" -o "$dec" > /dev/null 2>"$OUT/.derr"; then
            printf '%s\tDECOMPILE_FAIL\t%s\n' "$tag" "$(head -c 300 "$OUT/.derr" | tr '\n' ' ')" >> "$REPORT"
            fail_decomp=$((fail_decomp+1)); failed_cases+=("$tag:decompile"); continue
        fi

        exp=$(run "$LUACRUN" "$bc")
        act=$(run "$LUAU_VM" "$dec")

        if [ "$exp" = "$act" ]; then
            printf '%s\tPASS\t\n' "$tag" >> "$REPORT"
            pass=$((pass+1))
        else
            if ! grep -q '^exit:0' <<<"$act"; then
                printf '%s\tRUN_FAIL\t\n' "$tag" >> "$REPORT"
                fail_run=$((fail_run+1)); failed_cases+=("$tag:run")
            else
                printf '%s\tMISMATCH\t\n' "$tag" >> "$REPORT"
                fail_mismatch=$((fail_mismatch+1)); failed_cases+=("$tag:mismatch")
            fi
            mkdir -p "$OUT/fail"
            printf '%s' "$exp" > "$OUT/fail/$tag.expected.txt"
            printf '%s' "$act" > "$OUT/fail/$tag.actual.txt"
        fi
    done; done
done

echo "=================================================================="
echo " SEMANTIC ROUND-TRIP [$PROFILE]: $pass / $total match"
echo "=================================================================="
printf ' compile-fail  : %d\n decompile-fail: %d\n run-fail      : %d\n mismatch      : %d\n' \
    "$fail_compile" "$fail_decomp" "$fail_run" "$fail_mismatch"
[ ${#failed_cases[@]} -gt 0 ] && { echo "--- failing cases ---"; printf '%s\n' "${failed_cases[@]}"; }
echo "report: $REPORT"
[ "$pass" -eq "$total" ] && exit 0 || exit 1
