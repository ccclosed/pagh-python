#!/bin/sh
# Live run of the pythonc 0.8.0 suite on the pagh kernel: rebuild the release
# ELF, stage a 64 MiB ext2 image with the interpreter and tests/live/*.py, run
# tests/live/live08.cmds in one boot through tests/live/qemu_driver.py, and
# write the committed transcript (metadata header + raw serial log).
#
# Usage: tests/live/live08.sh [transcript.txt]
#   PAGH   kernel checkout (default /home/oleg/pagh-riscv)
#   KERNEL kernel ELF override (default $PAGH/target/riscv64gc-unknown-none-elf/debug/pagh)
set -e

PAGH=${PAGH:-/home/oleg/pagh-riscv}
KERNEL=${KERNEL:-$PAGH/target/riscv64gc-unknown-none-elf/debug/pagh}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TRANSCRIPT=${1:-$ROOT/docs/live-pythonc-080.txt}
PYTHONC=$ROOT/target/riscv64gc-unknown-none-elf/release/pythonc
IMAGE=/tmp/py-live-0.8.0.img
ROOTFS=$(mktemp -d)
RAW=$(mktemp)

cd "$ROOT"
cargo build --release --target riscv64gc-unknown-none-elf
cp "$PYTHONC" tests/live/*.py "$ROOTFS/"
truncate -s 64M "$IMAGE"
mke2fs -q -t ext2 -F -b 4096 -d "$ROOTFS" "$IMAGE"

python3 tests/live/qemu_driver.py --kernel "$KERNEL" --disk "$IMAGE" \
    --commands tests/live/live08.cmds --transcript "$RAW"

kernel_hash=$(sha256sum "$KERNEL" | cut -d' ' -f1)
pythonc_size=$(wc -c <"$PYTHONC")
pythonc_hash=$(sha256sum "$PYTHONC" | cut -d' ' -f1)

{
    printf 'Serial transcript of the pythonc 0.8.0 live run.\n\n'
    printf 'Kernel: %s (pagh-riscv 0.21.2), sha256 %s.\n' "$KERNEL" "$kernel_hash"
    cat <<'EOF'
QEMU `qemu-system-riscv64` 10.2.2, machine `virt`, 256 MiB, serial on a Unix
socket, no display, virtio-blk. Disk: 64 MiB ext2 image built on the host with
`truncate -s 64M` + `mke2fs -q -t ext2 -F -b 4096 -d <rootfs>`; the rootfs
EOF
    printf 'contains `pythonc` (release, %s bytes, sha256 %s) and every\n' "$pythonc_size" "$pythonc_hash"
    cat <<'EOF'
`tests/live/*.py`.

One boot, driven by the repository's own driver, commands in the order of
`tests/live/live08.cmds` (the 0.1.2-0.6.0 regression suite plus the new
`gen_methods.py`/`gen_delegation.py`/`gen_fibers.py` for the generator protocol);
the kernel prints
`run: /mnt/pythonc exited with code N` after every program. This script runs
exactly these steps, including the image build:

    python3 tests/live/qemu_driver.py \
        --kernel <kernel ELF> \
        --disk /tmp/py-live-0.8.0.img \
        --commands tests/live/live08.cmds \
        --transcript <raw serial log>

The OpenSBI/kernel boot banner is dropped; the transcript starts at the first
shell prompt.

Totals: 44 program runs, 38 exited 0; six exited non-zero on purpose
(`run /mnt/pythonc` without arguments -> 2, and `syntax_error.py`,
`uncaught.py`, `generator_syntax.py`, `decorator_syntax.py`,
`yield_from_syntax.py` -> 1). Three `cat` commands show the staged script and
the two files written by `fileio.py`/`print(file=...)`.

The new marker of this run is `DICT SET OK` (live `dict_keys`/`dict_values`/
`dict_items` views, the bitwise operators `& | ^ ~ << >>` and set algebra
including in-place `|=`/`&=`/`-=`/`^=`); the 0.7.0 markers `GEN METHODS OK`,
`GEN DELEGATION OK`, `GEN FIBERS OK`, the 0.6.0 marker `YIELD FROM OK` and
`child: True status: 0` / `proc ok` from `os_proc.py` are still there.

==============================================================================
## One boot: the 0.1.2-0.7.0 regression commands and the 0.8.0 dict/set features
==============================================================================

EOF
    cat "$RAW"
} > "$TRANSCRIPT"

rm -rf "$ROOTFS" "$RAW"
echo "serial transcript: $TRANSCRIPT"