#!/bin/sh
# Live run of the pythonc 0.9.2 suite on the pagh kernel: rebuild the release
# ELF, stage a 64 MiB ext2 image with the interpreter and tests/live/*.py, run
# tests/live/live092.cmds in one boot through tests/live/qemu_driver.py, and
# write the committed transcript (metadata header + raw serial log).
#
# Usage: tests/live/live092.sh [transcript.txt]
#   PAGH   kernel checkout (default /home/oleg/pagh-riscv)
#   KERNEL kernel ELF override (default $PAGH/target/riscv64gc-unknown-none-elf/debug/pagh)
set -e

PAGH=${PAGH:-/home/oleg/pagh-riscv}
KERNEL=${KERNEL:-$PAGH/target/riscv64gc-unknown-none-elf/debug/pagh}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TRANSCRIPT=${1:-$ROOT/docs/live-pythonc-092.txt}
PYTHONC=$ROOT/target/riscv64gc-unknown-none-elf/release/pythonc
IMAGE=/tmp/py-live-0.9.2.img
ROOTFS=$(mktemp -d)
RAW=$(mktemp)

cd "$ROOT"
cargo build --release --target riscv64gc-unknown-none-elf
cp "$PYTHONC" tests/live/*.py "$ROOTFS/"
truncate -s 64M "$IMAGE"
mke2fs -q -t ext2 -F -b 4096 -d "$ROOTFS" "$IMAGE"

python3 tests/live/qemu_driver.py --kernel "$KERNEL" --disk "$IMAGE" \
    --commands tests/live/live092.cmds --transcript "$RAW"

kernel_hash=$(sha256sum "$KERNEL" | cut -d' ' -f1)
pythonc_size=$(wc -c <"$PYTHONC")
pythonc_hash=$(sha256sum "$PYTHONC" | cut -d' ' -f1)

{
    printf 'Serial transcript of the pythonc 0.9.2 live run.\n\n'
    printf 'Kernel: %s (pagh-riscv 0.24.1, clean `origin/main` tree at a3f3a70), sha256 %s.\n' "$KERNEL" "$kernel_hash"
    cat <<'EOF'
QEMU `qemu-system-riscv64` 10.2.2, machine `virt`, 256 MiB, serial on a Unix
socket, no display, virtio-blk. Disk: 64 MiB ext2 image built on the host with
`truncate -s 64M` + `mke2fs -q -t ext2 -F -b 4096 -d <rootfs>`; the rootfs
EOF
    printf 'contains `pythonc` (release, %s bytes, sha256 %s) and every\n' "$pythonc_size" "$pythonc_hash"
    cat <<'EOF'
`tests/live/*.py`.

One boot, driven by the repository's own driver, commands in the order of
`tests/live/live092.cmds` -- the 0.1.2-0.9.1 regression suite plus the new
evaluator-recursion boundary checks (audit F-r2-py-1/2); the kernel prints
`run: /mnt/pythonc exited with code N` after every program. This script runs
exactly these steps, including the image build:

    python3 tests/live/qemu_driver.py \
        --kernel <kernel ELF> \
        --disk /tmp/py-live-0.9.2.img \
        --commands tests/live/live092.cmds \
        --transcript <raw serial log>

The OpenSBI/kernel boot banner is dropped; the transcript starts at the first
shell prompt.

Totals: 69 program runs, 51 exited 0; eighteen exited non-zero on purpose
(`run /mnt/pythonc` without arguments -> 2, and seventeen diagnostics -> 1:
the five older ones `syntax_error.py`, `uncaught.py`, `generator_syntax.py`,
`decorator_syntax.py`, `yield_from_syntax.py`, the five 0.9.1 parser-nesting
ones, and the seven new evaluator-depth ones -- `eval_chain_257.py`,
`eval_chain_720.py`, `eval_chain_740.py`, `eval_chain_800.py`,
`eval_chain_attr.py`, `eval_chain_sub.py`, `gen_chain_300.py`). Five `cat`
commands show the staged scripts and the two files written by
`fileio.py`/`print(file=...)`.

The 0.9.2 fix is visible in the last section: `EVAL CHAIN 200 OK` and
`EVAL CHAIN 256 OK` prove that 200 and exactly 256 left-associative terms
(EXPR_RECURSION_LIMIT evaluation levels; the limit sits above the parser's
200 so 200 nested unary operators are not rejected at run time) still run,
and the 257th is a clean `RecursionError: maximum recursion depth exceeded
(256 expression levels)` with exit 1. The audit's 720/740/800-term
chains -- 720 was the last size that merely worked on 0.9.1 and 740/800 killed
the interpreter at `USER_STACK_FLOOR` (stval=0x3feffd68) -- are the same clean
RecursionError now, and `cat` of the 1626-byte 800-term PoC is followed by
`EVAL RECOVERED OK` (the exception is catchable) and `PARSE OK` (a 3000-term
chain in an uncalled function still parses and exits 0). `INTERLEAVE OK`
shows a generator suspended inside an expression keeping its own stack's
depth. The F-r2-py-2 fiber variants are clean as well: a 20-term chain works
(`GEN CHAIN 20 OK`), while the 100-term bomb that used to die with
wild-pointer faults (`stval=0x1/0x0/0x44800000008/0x1fffffe8`) is
`GEN GUARD OK: RecursionError: generator stack exhausted (...)` followed by
`ALIVE AFTER GEN BOMB`, and the uncaught 300-term variant exits 1.
`RECURSION LIMIT OK` is the plain-call regression: 55 frames work and the
56th raises RecursionError. Older markers `KEYWORD ONLY OK`, `DICT SET OK`,
`GEN METHODS OK`, `GEN DELEGATION OK`, `GEN FIBERS OK`, `YIELD FROM OK`,
`NESTING OK`, `PARENS150 OK`, `NOT200 OK`, `BLOCKS200 OK` and
`child: True status: 0` / `proc ok` are still there.

==============================================================================
## One boot: the 0.1.2-0.9.1 regression commands and the 0.9.2 evaluator limits
==============================================================================

EOF
    cat "$RAW"
} > "$TRANSCRIPT"

rm -rf "$ROOTFS" "$RAW"
echo "serial transcript: $TRANSCRIPT"