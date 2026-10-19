// Cortex-A53 843419: ADRP at page offset 0xff8, an independent load, then
// a load using the ADRP result. The program inspects the vulnerable load
// and exits successfully only when LLD has replaced it with a B thunk.
.text
.balign 4096
.global _start
.type _start, %function
_start:
    adr x9, vulnerable_load
    ldr w10, [x9]
    lsr w10, w10, #26
    cmp w10, #5
    cset x0, ne
    mov x8, #93
    svc #0
.balign 4096
.space 4088
hazard:
    adrp x0, datum
    ldr x1, [x1]
vulnerable_load:
    ldr x0, [x0, :lo12:datum]
    ret
.data
.balign 8
datum:
    .quad 0
