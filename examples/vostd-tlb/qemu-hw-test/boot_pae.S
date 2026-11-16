/* PAE variant: 2MB page -> split to 4K -> remap without invlpg.
 * 32-bit PAE (no long mode); same large-page TLB hazard as x86_64 2M/4K.
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

    movb $0xAA, 0x00800000
    movb $0xBB, 0x00A00000

    /* PDPT[0] -> PD, present. */
    movl $pd, %eax
    orl $1, %eax
    movl %eax, pdpt
    movl $0, pdpt+4

    /* PD: identity 0..1GB with 2MB pages. */
    mov $pd, %edi
    xor %ecx, %ecx
pd_loop:
    mov %ecx, %eax
    shl $21, %eax          /* i * 2MB */
    or $0x83, %eax
    mov %eax, (%edi,%ecx,8)
    movl $0, 4(%edi,%ecx,8)
    inc %ecx
    cmp $512, %ecx
    jb pd_loop

    /* PAE + CR3 + PG */
    mov %cr4, %eax
    or $0x20, %eax         /* CR4.PAE */
    mov %eax, %cr4
    mov $pdpt, %eax
    mov %eax, %cr3
    mov %cr0, %eax
    or $0x80000001, %eax
    mov %eax, %cr0

    /* Warm the 2MB TLB entry. */
    mov $0x00800000, %esi
    mov $8, %ecx
warm:
    movzbl (%esi), %eax
    loop warm
    movzbl (%esi), %eax
    mov %al, warm_val

    /* Split: PT identity-maps the 2MB range; entry 0 -> frame B. */
    mov $pt, %edi
    xor %ecx, %ecx
pt_loop:
    mov %ecx, %eax
    shl $12, %eax
    add $0x00800000, %eax
    or $0x03, %eax
    mov %eax, (%edi,%ecx,8)
    movl $0, 4(%edi,%ecx,8)
    inc %ecx
    cmp $512, %ecx
    jb pt_loop
    movl $0x00A00003, pt
    movl $0, pt+4

    /* PD[4] (VA 8MB) = PT pointer, PS=0. No invlpg. */
    mov $pt, %eax
    or $0x03, %eax
    mov %eax, pd+32         /* 4 * 8 */
    movl $0, pd+36

    movzbl (%esi), %eax
    mov %al, stale_val
    invlpg (%esi)
    movzbl (%esi), %eax
    mov %al, flushed_val

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
    mov $msg_verdict, %esi
    call puts
    mov stale_val, %al
    cmp $0xAA, %al
    jne verdict_no
    mov flushed_val, %al
    cmp $0xBB, %al
    jne verdict_inc
    mov $msg_confirmed, %esi
    call puts
    jmp done
verdict_no:
    mov $msg_negative, %esi
    call puts
    jmp done
verdict_inc:
    mov $msg_inconclusive, %esi
    call puts
done:
    call newline
    mov $0x10, %al
    mov $0xF4, %dx
    out %al, %dx
2:  hlt
    jmp 2b

/* serial helpers (same as PSE test) */
serial_init:
    mov $0x3F8+1, %dx
    xor %al, %al
    out %al, %dx
    mov $0x3F8+3, %dx
    mov $0x80, %al
    out %al, %dx
    mov $0x3F8, %dx
    mov $0x03, %al
    out %al, %dx
    mov $0x3F8+1, %dx
    xor %al, %al
    out %al, %dx
    mov $0x3F8+3, %dx
    mov $0x03, %al
    out %al, %dx
    mov $0x3F8+2, %dx
    mov $0xC7, %al
    out %al, %dx
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
msg_stale:   .asciz "after 2M split+remap (no invlpg): 0x"
msg_flushed: .asciz "after invlpg: 0x"
msg_verdict: .asciz "verdict: "
msg_confirmed:    .asciz "CONFIRMED stale 2M-page translation without flush"
msg_negative:     .asciz "NEGATIVE: no stale translation observed"
msg_inconclusive: .asciz "INCONCLUSIVE: unexpected values"

.section .bss
.align 4096
pdpt: .space 4096
pd:   .space 4096
pt:   .space 4096
.align 16
stack: .space 8192
stack_top:

.section .data
.align 8
gdt:
    .quad 0
    .quad 0x00CF9A000000FFFF
    .quad 0x00CF92000000FFFF
gdt_end:
gdt_desc:
    .word gdt_end - gdt - 1
    .long gdt
