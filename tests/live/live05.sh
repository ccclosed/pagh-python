#!/bin/sh
# Live run of the pythonc 0.5.0 feature scripts on the pagh kernel, driven by
# the kernel repository's own tool (tools/run.py), which boots QEMU, types the
# commands into the serial console and saves the transcript.
#
# Usage: tests/live/live05.sh [serial-log]
set -e

PAGH=${PAGH:-/home/oleg/pagh-riscv}
PYTHONC=${PYTHONC:-target/riscv64gc-unknown-none-elf/release/pythonc}
ROOTFS=$(mktemp -d)
IMAGE=/tmp/py-live-0.5.0.img
LOG=${1:-/tmp/live-pythonc-0.5.0.txt}

cargo build --release --target riscv64gc-unknown-none-elf
cp "$PYTHONC" tests/live/*.py "$ROOTFS/"
truncate -s 64M "$IMAGE"
mke2fs -q -t ext2 -F -b 4096 -d "$ROOTFS" "$IMAGE"

# One boot, every new feature script in order, then two front-end rejections.
KEYS=""
while read -r line; do
    case "$line" in
        run\ *) KEYS="$KEYS$line\n" ;;
    esac
done < tests/live/live05.cmds

python3 "$PAGH/tools/run.py" --disk "$IMAGE" --keys "$KEYS" --serial-log "$LOG"
echo "serial transcript: $LOG"
rm -rf "$ROOTFS"
