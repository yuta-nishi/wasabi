use core::arch::global_asm;

global_asm!(
    r#"
    // Vectors that push no error code: the hardware frame starts with RIP.
    .macro isr n
    .globl wasabi_isr_\n
    wasabi_isr_\n:
        push rax
        mov eax, \n
        mov qword ptr [rip + {vector}], rax
        mov rax, qword ptr [rsp + 8]
        mov qword ptr [rip + {rip}], rax
        mov qword ptr [rip + {error}], 0
        pop rax
        jmp wasabi_isr_common
    .endm

    // Vectors that push an error code: it comes first on the stack.
    .macro isr_err n
    .globl wasabi_isr_\n
    wasabi_isr_\n:
        push rax
        mov eax, \n
        mov qword ptr [rip + {vector}], rax
        mov rax, qword ptr [rsp + 8]
        mov qword ptr [rip + {error}], rax
        mov rax, qword ptr [rsp + 16]
        mov qword ptr [rip + {rip}], rax
        pop rax
        jmp wasabi_isr_common
    .endm

    isr     0
    isr     1
    isr     2
    isr     4
    isr     5
    isr     6
    isr     7
    isr_err 8
    isr     9
    isr_err 10
    isr_err 11
    isr_err 12
    isr_err 13
    isr_err 14
    isr     15
    isr     16
    isr_err 17
    isr     18
    isr     19
    isr     20
    isr_err 21
    isr     22
    isr     23
    isr     24
    isr     25
    isr     26
    isr     27
    isr     28
    isr_err 29
    isr_err 30
    isr     31

    // The breakpoint exception only counts itself and resumes.
    .globl wasabi_isr_3
    wasabi_isr_3:
        push rax
        mov eax, 3
        mov qword ptr [rip + {vector}], rax
        mov rax, qword ptr [rsp + 8]
        mov qword ptr [rip + {rip}], rax
        mov qword ptr [rip + {error}], 0
        pop rax
        lock inc qword ptr [rip + {breakpoints}]
        iretq

    // The timer interrupt returns to whatever it interrupted, so it saves the
    // registers it may clobber and acknowledges the PIC on the way out.
    .globl wasabi_isr_32
    wasabi_isr_32:
        push rbp
        push rax
        push rcx
        push rdx
        push rsi
        push rdi
        push r8
        push r9
        push r10
        push r11
        mov rbp, rsp
        and rsp, -16
        call {timer_tick}
        mov rsp, rbp
        pop r11
        pop r10
        pop r9
        pop r8
        pop rdi
        pop rsi
        pop rdx
        pop rcx
        pop rax
        pop rbp
        iretq

    .globl wasabi_isr_common
    wasabi_isr_common:
        and rsp, -16
        call {fatal}
        ud2

    // The IDT entries are filled from this table, so the stubs are listed once.
    .section .rodata
    .globl wasabi_isr_table
    wasabi_isr_table:
        .quad wasabi_isr_0
        .quad wasabi_isr_1
        .quad wasabi_isr_2
        .quad wasabi_isr_3
        .quad wasabi_isr_4
        .quad wasabi_isr_5
        .quad wasabi_isr_6
        .quad wasabi_isr_7
        .quad wasabi_isr_8
        .quad wasabi_isr_9
        .quad wasabi_isr_10
        .quad wasabi_isr_11
        .quad wasabi_isr_12
        .quad wasabi_isr_13
        .quad wasabi_isr_14
        .quad wasabi_isr_15
        .quad wasabi_isr_16
        .quad wasabi_isr_17
        .quad wasabi_isr_18
        .quad wasabi_isr_19
        .quad wasabi_isr_20
        .quad wasabi_isr_21
        .quad wasabi_isr_22
        .quad wasabi_isr_23
        .quad wasabi_isr_24
        .quad wasabi_isr_25
        .quad wasabi_isr_26
        .quad wasabi_isr_27
        .quad wasabi_isr_28
        .quad wasabi_isr_29
        .quad wasabi_isr_30
        .quad wasabi_isr_31
        .quad wasabi_isr_32
    "#,
    vector = sym crate::idt::LAST_VECTOR,
    error = sym crate::idt::LAST_ERROR,
    rip = sym crate::idt::LAST_RIP,
    breakpoints = sym crate::idt::BREAKPOINTS_HANDLED,
    fatal = sym crate::idt::fatal_exception,
    timer_tick = sym crate::idt::timer_tick,
);
