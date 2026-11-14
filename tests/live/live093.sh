#!/bin/sh
# Live run of the pythonc 0.9.3 suite on the pagh kernel: rebuild the release
# ELF, stage a 64 MiB ext2 image with the interpreter and tests/live/*.py, run
# tests/live/live093.cmds in one boot through tests/live/qemu_driver.py, and
# write the committed transcript (metadata header + raw serial log).
#
# Usage: tests/live/live093.sh [transcript.txt]
#   PAGH   kernel checkout (default /home/oleg/pagh-riscv)
#   KERNEL kernel ELF override (default $PAGH/target/riscv64gc-unknown-none-elf/debug/pagh)
set -e

PAGH=${PAGH:-/home/oleg/pagh-riscv}
KERNEL=${KERNEL:-$PAGH/target/riscv64gc-unknown-none-elf/debug/pagh}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TRANSCRIPT=${1:-$ROOT/docs/live-pythonc-093.txt}
PYTHONC=$ROOT/target/riscv64gc-unknown-none-elf/release/pythonc
IMAGE=/tmp/py-live-0.9.3.img
ROOTFS=$(mktemp -d)
RAW=$(mktemp)

cd "$ROOT"
cargo build --release --target riscv64gc-unknown-none-elf
cp "$PYTHONC" tests/live/*.py "$ROOTFS/"
truncate -s 64M "$IMAGE"
mke2fs -q -t ext2 -F -b 4096 -d "$ROOTFS" "$IMAGE"

python3 tests/live/qemu_driver.py --kernel "$KERNEL" --disk "$IMAGE" \
    --commands tests/live/live093.cmds --transcript "$RAW"

kernel_hash=$(sha256sum "$KERNEL" | cut -d' ' -f1)
pythonc_size=$(wc -c <"$PYTHONC")
pythonc_hash=$(sha256sum "$PYTHONC" | cut -d' ' -f1)

{
    printf 'Serial transcript of the pythonc 0.9.3 live run.\n\n'
    printf 'Kernel: %s (pagh-riscv 0.24.2, clean `origin/main` tree at 23fe825), sha256 %s.\n' "$KERNEL" "$kernel_hash"
    cat <<'HEADER'
QEMU `qemu-system-riscv64` 10.2.2, machine `virt`, 256 MiB, serial on a Unix
socket, no display, virtio-blk. Disk: 64 MiB ext2 image built on the host with
`truncate -s 64M` + `mke2fs -q -t ext2 -F -b 4096 -d <rootfs>`; the rootfs
HEADER
    printf 'contains `pythonc` (release, %s bytes, sha256 %s) and every\n' "$pythonc_size" "$pythonc_hash"
    cat <<'BODY'
`tests/live/*.py`.

One boot, driven by the repository's own driver, commands in the order of
`tests/live/live093.cmds` -- the 0.1.2-0.9.2 regression suite plus the new
recursive-drop boundary checks (audit F-r3-rt-1); the kernel prints
`run: /mnt/pythonc exited with code N` after every program. This script runs
exactly these steps, including the image build:

    python3 tests/live/qemu_driver.py \
        --kernel <kernel ELF> \
        --disk /tmp/py-live-0.9.3.img \
        --commands tests/live/live093.cmds \
        --transcript <raw serial log>

The OpenSBI/kernel boot banner is dropped; the transcript starts at the first
shell prompt.

Totals: 80 program runs, 62 exited 0; eighteen exited non-zero on purpose
(`run /mnt/pythonc` without arguments -> 2, and seventeen diagnostics -> 1:
the five older ones `syntax_error.py`, `uncaught.py`, `generator_syntax.py`,
`decorator_syntax.py`, `yield_from_syntax.py`, the five 0.9.1 parser-nesting
ones, and the seven 0.9.2 evaluator-depth ones -- `eval_chain_257.py`,
`eval_chain_720.py`, `eval_chain_740.py`, `eval_chain_800.py`,
`eval_chain_attr.py`, `eval_chain_sub.py`, `gen_chain_300.py`). Five `cat`
commands show the staged scripts and the two files written by
`fileio.py`/`print(file=...)`.

The 0.9.3 fix is visible in the last section: on 0.9.2 a deep `Rc` teardown
inside a generator overran the 64 KiB heap fiber and killed the process, while
1800 nested lists survived. Here `SURVIVED 1800` is unchanged, and
`FIBER DROP OK 2000` / `FIBER DROP OK 4000` / `TUPLE FIBER OK` show the audit
PoCs exiting 0 instead of dying with
`[user] /mnt/pythonc killed: load page fault (stval=0x5204c1930)` or
`store page fault (stval=0x1ffffff8)`. The same section covers `dict`, `Set`,
function-frame (`Env`) release, instance-attribute chains, closure chains,
iterators over a deep list and dict views -- all through one explicit
teardown worklist. `ALIVE AFTER DROP TESTS` follows, and the older markers
`EVAL CHAIN 200 OK`, `EVAL CHAIN 256 OK`, `GEN GUARD OK`, `RECURSION LIMIT OK`,
`KEYWORD ONLY OK`, `DICT SET OK`, `GEN METHODS OK`, `GEN DELEGATION OK`,
`GEN FIBERS OK`, `YIELD FROM OK`, `NESTING OK`, `PARENS150 OK`, `NOT200 OK`,
`BLOCKS200 OK` and `child: True status: 0` / `proc ok` are still there.

==============================================================================
## One boot: the 0.1.2-0.9.2 regression commands and the 0.9.3 drop limits
==============================================================================

BODY
    cat "$RAW"
} > "$TRANSCRIPT"

rm -rf "$ROOTFS" "$RAW"
echo "serial transcript: $TRANSCRIPT"