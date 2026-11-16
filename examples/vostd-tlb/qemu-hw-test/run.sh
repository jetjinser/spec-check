#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
timeout 30 qemu-system-i386 -kernel kernel.elf -m 64M -serial stdio -display none \
  -device isa-debug-exit,iobase=0xf4,iosize=4 -no-reboot 2>&1 | tee qemu.log || true
