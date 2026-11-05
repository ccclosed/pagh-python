#!/usr/bin/env python3
"""Run the pagh kernel under QEMU and drive pythonc over a serial socket.

The kernel's serial port is both the console and the keyboard (there is no
PS/2 in the `virt` machine), so commands are typed into the same socket that
carries the transcript. The tool saves every byte the kernel printed and is
deterministic enough to regenerate `docs/live-pythonc.txt`.

Example:

    python3 tests/live/qemu_driver.py \
        --disk /tmp/py-live.img \
        --commands tests/live/live.cmds \
        --transcript docs/live-pythonc.txt
"""

import argparse
import os
import socket
import subprocess
import sys
import time

DEFAULT_KERNEL = (
    "/home/oleg/pagh-riscv/target/riscv64gc-unknown-none-elf/debug/pagh"
)
PROMPT = b"pagh:rv64>"


def read_until(sock, buffer, marker, timeout):
    sock.settimeout(0.25)
    deadline = time.time() + timeout
    while time.time() < deadline:
        if marker in buffer:
            return True, buffer
        try:
            chunk = sock.recv(65536)
        except socket.timeout:
            continue
        except OSError:
            break
        if not chunk:
            break
        buffer += chunk
    return marker in buffer, buffer


def send(sock, text, delay=0.004):
    # The guest polls a small UART FIFO; one byte at a time avoids dropping
    # characters when a whole command is written at once.
    for byte in text.encode("utf-8"):
        sock.sendall(bytes([byte]))
        time.sleep(delay)


def connect(path, timeout=20.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        candidate = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            candidate.connect(path)
            return candidate
        except OSError:
            candidate.close()
            time.sleep(0.05)
    raise RuntimeError("cannot connect to serial socket %r" % path)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--kernel", default=DEFAULT_KERNEL)
    parser.add_argument("--disk", required=True)
    parser.add_argument("--commands", required=True)
    parser.add_argument("--transcript", required=True)
    parser.add_argument("--memory", default="256M")
    parser.add_argument("--timeout", type=float, default=60.0)
    parser.add_argument("--serial", default="/tmp/pagh-python-serial.sock")
    args = parser.parse_args()

    if os.path.exists(args.serial):
        os.unlink(args.serial)
    command = [
        "qemu-system-riscv64",
        "-machine",
        "virt",
        "-bios",
        "default",
        "-m",
        args.memory,
        "-device",
        "ramfb",
        "-kernel",
        args.kernel,
        "-serial",
        "unix:%s,server=on,wait=on" % args.serial,
        "-global",
        "virtio-mmio.force-legacy=false",
        "-display",
        "none",
        "-no-reboot",
        "-drive",
        "file=%s,format=raw,if=none,id=hd0" % args.disk,
        "-device",
        "virtio-blk-device,drive=hd0",
    ]
    process = subprocess.Popen(
        command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    transcript = bytearray()
    sock = None
    try:
        sock = connect(args.serial)
        ready, transcript = read_until(sock, transcript, PROMPT, args.timeout)
        if not ready:
            sys.stderr.write("boot prompt not seen\n")
            return 2
        # Drop the OpenSBI/kernel banner: the committed transcript starts at
        # the first shell prompt so it is small and reviewable.
        transcript = transcript[transcript.rfind(PROMPT):]

        commands = []
        with open(args.commands, "r", encoding="utf-8") as handle:
            for line in handle:
                line = line.rstrip("\n")
                if line and not line.startswith("#"):
                    commands.append(line)

        for entry in commands:
            baseline = transcript.count(PROMPT)
            send(sock, entry + "\n")
            sock.settimeout(0.25)
            deadline = time.time() + args.timeout
            while time.time() < deadline:
                if transcript.count(PROMPT) > baseline:
                    break
                try:
                    chunk = sock.recv(65536)
                except socket.timeout:
                    continue
                except OSError:
                    break
                if not chunk:
                    break
                transcript += chunk
            # Collect diagnostics printed immediately after the new prompt.
            sock.settimeout(0.2)
            while True:
                try:
                    chunk = sock.recv(65536)
                except socket.timeout:
                    break
                except OSError:
                    break
                if not chunk:
                    break
                transcript += chunk
    finally:
        if sock is not None:
            sock.close()
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)

    text = transcript.decode("utf-8", "replace")
    with open(args.transcript, "w", encoding="utf-8") as handle:
        handle.write(text)
    sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())