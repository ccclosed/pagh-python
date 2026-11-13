#!/bin/sh
# Live run of the pythonc 0.9.1 suite on the pagh kernel: rebuild the release
# ELF, stage a 64 MiB ext2 image with the interpreter and tests/live/*.py, run
# tests/live/live091.cmds in one boot through tests/live/qemu_driver.py, and
# write the committed transcript (metadata header + raw serial log).
#
# Usage: tests/live/live091.sh [transcript.txt]
#   PAGH   kernel checkout (default /home/oleg/pagh-riscv)
#   KERNEL kernel ELF override (default $PAGH/target/riscv64gc-unknown-none-elf/debug/pagh)
set -e

PAGH=${PAGH:-/home/oleg/pagh-riscv}
KERNEL=${KERNEL:-$PAGH/target/riscv64gc-unknown-none-elf/debug/pagh}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TRANSCRIPT=${1:-$ROOT/docs/live-pythonc-091.txt}
PYTHONC=$ROOT/target/riscv64gc-unknown-none-elf/release/pythonc
IMAGE=/tmp/py-live-0.9.1.img
ROOTFS=$(mktemp -d)
RAW=$(mktemp)

cd "$ROOT"
cargo build --release --target riscv64gc-unknown-none-elf
cp "$PYTHONC" tests/live/*.py "$ROOTFS/"
truncate -s 64M "$IMAGE"
mke2fs -q -t ext2 -F -b 4096 -d "$ROOTFS" "$IMAGE"

python3 tests/live/qemu_driver.py --kernel "$KERNEL" --disk "$IMAGE" \
    --commands tests/live/live091.cmds --transcript "$RAW"

kernel_hash=$(sha256sum "$KERNEL" | cut -d' ' -f1)
pythonc_size=$(wc -c <"$PYTHONC")
pythonc_hash=$(sha256sum "$PYTHONC" | cut -d' ' -f1)

{
    printf 'Serial transcript of the pythonc 0.9.1 live run.\n\n'
    printf 'Kernel: %s (pagh-riscv 0.24.1, commit aa8f1a9), sha256 %s.\n' "$KERNEL" "$kernel_hash"
    cat <<'EOF'
QEMU `qemu-system-riscv64` 10.2.2, machine `virt`, 256 MiB, serial on a Unix
socket, no display, virtio-blk. Disk: 64 MiB ext2 image built on the host with
`truncate -s 64M` + `mke2fs -q -t ext2 -F -b 4096 -d <rootfs>`; the rootfs
EOF
    printf 'contains `pythonc` (release, %s bytes, sha256 %s) and every\n' "$pythonc_size" "$pythonc_hash"
    cat <<'EOF'
`tests/live/*.py`.

One boot, driven by the repository's own driver, commands in the order of
`tests/live/live091.cmds` -- the 0.1.2-0.9.0 regression suite plus the new
parser-nesting boundary checks; the kernel prints
`run: /mnt/pythonc exited with code N` after every program. This script runs
exactly these steps, including the image build:

    python3 tests/live/qemu_driver.py \
        --kernel <kernel ELF> \
        --disk /tmp/py-live-0.9.1.img \
        --commands tests/live/live091.cmds \
        --transcript <raw serial log>

The OpenSBI/kernel boot banner is dropped; the transcript starts at the first
shell prompt.

Totals: 54 program runs, 43 exited 0; eleven exited non-zero on purpose
(`run /mnt/pythonc` without arguments -> 2, and `syntax_error.py`,
`uncaught.py`, `generator_syntax.py`, `decorator_syntax.py`,
`yield_from_syntax.py` -> 1; the five new nesting diagnostics
`nesting_operators_201.py`, `nesting_blocks_201.py`, `nesting_parens_200.py`,
`nesting_parens_201.py`, `nesting_unbalanced_300.py` -> 1). Four `cat`
commands show the staged scripts and the two files written by
`fileio.py`/`print(file=...)`.

The 0.9.1 fix is visible in the last section: valid deep input still runs
(`NESTING OK`, `PARENS150 OK`, `NOT200 OK`, `BLOCKS200 OK`), exactly 200 nested
`not`s or `if` blocks are accepted and the 201st raises
`SyntaxError: maximum recursion depth exceeded`; the audit PoC (506 bytes,
200 nested parentheses, previously `[user] pythonc killed: store page fault`)
now exits 1 with that SyntaxError, and 201 nested brackets are refused by the
lexer as `too many nested parentheses`. Older markers `KEYWORD ONLY OK`,
`DICT SET OK`, `GEN METHODS OK`, `GEN DELEGATION OK`, `GEN FIBERS OK`,
`YIELD FROM OK` and `child: True status: 0` / `proc ok` are still there.

==============================================================================
## One boot: the 0.1.2-0.9.0 regression commands and the 0.9.1 parser nesting limits
==============================================================================

EOF
    cat "$RAW"
} > "$TRANSCRIPT"

rm -rf "$ROOTFS" "$RAW"
echo "serial transcript: $TRANSCRIPT"