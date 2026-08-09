//! `extern "x86-interrupt"` handler'larinin CPU tarafindan yiginin ustune
//! itilen ortak alanlari. Faz 1'deki handler'lar (ISR0, IRQ0, IRQ1, int 0x80)
//! hicbiri hata kodu itmez, dolayisiyla tek bir frame tipi yeterlidir.

#[repr(C)]
#[allow(dead_code)]
pub struct InterruptStackFrame {
    pub instruction_pointer: u32,
    pub code_segment: u32,
    pub cpu_flags: u32,
    pub stack_pointer: u32,
    pub stack_segment: u32,
}
