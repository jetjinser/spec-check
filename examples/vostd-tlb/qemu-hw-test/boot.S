/* Minimal Multiboot1 test: 4MB PSE page -> split to 4K -> remap without invlpg.
 * Real QEMU execution; NOT ostd. AT&T syntax, 32-bit.
 */
.set MULTIBOOT_MAGIC, 0x1BADB002
.set MULTIBOOT_FLAGS, 0x00000003
.set MULTIBOOT_CHECKSUM, -(MULTIBOOT_MAGIC + MULTIBOOT_FLAGS)

.section .multiboot, "a"
.align 4
.long MULTIBOOT_MAGIC
.long MULTIBOOT_FLAGS
.long MULTIBOOT_CHECKSUM

.section .text
.global _start
.code32
_start:
    cli
    lgdt gdt_desc
    ljmp $0x08, $1f
1:
    mov $0x10, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %fs
    mov %ax, %gs
    mov %ax, %ss
    mov $stack_top, %esp

    call serial_init

    /* Fill frame A (8MB) with 0xAA and frame B (12MB) with 0xBB. */
    movb $0xAA, 0x00800000
    movb $0xBB, 0x00C00000

    /* Page directory: identity map 0..64MB with 4MB PSE pages. */
    mov $pd, %edi
    xor %ecx, %ecx
pd_loop:
    mov %ecx, %eax
    shl $22, %eax          /* i * 4MB */
    or $0x83, %eax         /* present | rw | ps */
    mov %eax, (%edi,%ecx,4)
    inc %ecx
    cmp $16, %ecx
    jb pd_loop

    /* CR4.PSE, CR3, CR0.PG|PE */
    mov %cr4, %eax
    or $0x10, %eax
    mov %eax, %cr4
    mov $pd, %eax
    mov %eax, %cr3
    mov %cr0, %eax
    or $0x80000001, %eax
    mov %eax, %cr0

    /* Warm the 4MB TLB entry for VA 0x00800000. */
    mov $0x00800000, %esi
    mov $8, %ecx
warm:
    movzbl (%esi), %eax
    loop warm
    movzbl (%esi), %eax
    mov %al, warm_val

    /* Split: PT identity-maps the 4MB range, entry 0 -> frame B. */
    mov $pt, %edi
    xor %ecx, %ecx
pt_loop:
    mov %ecx, %eax
    shl $12, %eax
    add $0x00800000, %eax  /* PA = 8MB + i*4K */
    or $0x03, %eax
    mov %eax, (%edi,%ecx,4)
    inc %ecx
    cmp $1024, %ecx
    jb pt_loop
    movl $0x00C00003, (%edi)   /* VA 8MB -> PA 12MB, present|rw */

    /* Replace the PDE with a page-table pointer (PS=0). No invlpg. */
    mov $pt, %eax
    or $0x03, %eax
    mov %eax, pd+8             /* PDE[2] */

    /* Access without invlpg. */
    movzbl (%esi), %eax
    mov %al, stale_val

    /* Control: invlpg then access. */
    invlpg (%esi)
    movzbl (%esi), %eax
    mov %al, flushed_val

    /* Report */
    mov $msg_warm, %esi
    call puts
    mov warm_val, %al
    call put_hex
    call newline

    mov $msg_stale, %esi
    call puts
    mov stale_val, %al
    call put_hex
    call newline

    mov $msg_flushed, %esi
    call puts
    mov flushed_val, %al
    call put_hex
    call newline

    /* Classify */
    mov $msg_verdict, %esi
    call puts
    mov stale_val, %al
    cmp $0xAA, %al
    jne verdict_no_stale
    mov flushed_val, %al
    cmp $0xBB, %al
    jne verdict_inconclusive
    mov $msg_confirmed, %esi
    call puts
    jmp done
verdict_no_stale:
    mov $msg_negative, %esi
    call puts
    jmp done
verdict_inconclusive:
    mov $msg_inconclusive, %esi
    call puts
done:
    call newline
    /* isa-debug-exit */
    mov $0x10, %al
    mov $0xF4, %dx
    out %al, %dx
2:  hlt
    jmp 2b

/* ---------- serial 16550 ---------- */
serial_init:
    mov $0x3F8+1, %dx
    xor %al, %al
    out %al, %dx            /* disable interrupts */
    mov $0x3F8+3, %dx
    mov $0x80, %al
    out %al, %dx            /* DLAB */
    mov $0x3F8, %dx
    mov $0x03, %al
    out %al, %dx            /* 115200 */
    mov $0x3F8+1, %dx
    xor %al, %al
    out %al, %dx
    mov $0x3F8+3, %dx
    mov $0x03, %al
    out %al, %dx            /* 8N1 */
    mov $0x3F8+2, %dx
    mov $0xC7, %al
    out %al, %dx            /* FIFO */
    ret

putc:
    push %eax
    push %edx
1:  mov $0x3F8+5, %dx
    in %dx, %al
    test $0x20, %al
    jz 1b
    pop %edx
    pop %eax
    mov $0x3F8, %dx
    out %al, %dx
    ret

puts:
    push %eax
1:  movzbl (%esi), %eax
    test %al, %al
    jz 2f
    call putc
    inc %esi
    jmp 1b
2:  pop %eax
    ret

put_hex:
    push %eax
    mov %al, %ah
    shr $4, %al
    call hex_digit
    call putc
    mov %ah, %al
    and $0x0F, %al
    call hex_digit
    call putc
    pop %eax
    ret

hex_digit:
    cmp $10, %al
    jb 1f
    add $('A'-10), %al
    ret
1:  add $'0', %al
    ret

newline:
    mov $'\n', %al
    call putc
    mov $'\r', %al
    call putc
    ret

.section .data
warm_val:    .byte 0
stale_val:   .byte 0
flushed_val: .byte 0
msg_warm:    .asciz "warm read: 0x"
msg_stale:   .asciz "after split+remap (no invlpg): 0x"
msg_flushed: .asciz "after invlpg: 0x"
msg_verdict: .asciz "verdict: "
msg_confirmed:    .asciz "CONFIRMED stale TLB translation without flush"
msg_negative:     .asciz "NEGATIVE: no stale translation observed"
msg_inconclusive: .asciz "INCONCLUSIVE: unexpected values"

.section .bss
.align 4096
pd: .space 4096
pt: .space 4096
.align 16
stack: .space 8192
stack_top:

.section .data
.align 8
gdt:
    .quad 0
    .quad 0x00CF9A000000FFFF  /* code: base 0, limit 4G, 32-bit */
    .quad 0x00CF92000000FFFF  /* data */
gdt_end:
gdt_desc:
    .word gdt_end - gdt - 1
    .long gdt
