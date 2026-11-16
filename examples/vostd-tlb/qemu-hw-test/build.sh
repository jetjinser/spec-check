#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
as --32 boot.S -o boot.o
ld -m elf_i386 -T linker.ld -o kernel.elf boot.o
echo "built kernel.elf"
