#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
as --32 boot_pae.S -o boot_pae.o
ld -m elf_i386 -T linker.ld -o kernel_pae.elf boot_pae.o
echo "built kernel_pae.elf"
