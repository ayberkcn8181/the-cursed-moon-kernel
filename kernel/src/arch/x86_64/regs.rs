//! x86_64 kesme/syscall register cerceveleri.

/// `iretq`'in bekledigi/CPU'nun ittigi cerceve (hata kodu itmeyen vektorler).
#[repr(C)]
#[allow(dead_code)]
pub struct InterruptStackFrame {
    pub instruction_pointer: u64,
    pub code_segment: u64,
    pub cpu_flags: u64,
    pub stack_pointer: u64,
    pub stack_segment: u64,
}

/// Syscall girisinde elle kaydedilen registerlar.
///
/// Alan **sirasi**, `arch/x86_64/syscall_entry` asm'inin push sirasiyla
/// birebir eslesmelidir (dusuk adresten yuksege).
///
/// Linux x86_64 ABI (doc S.6): RAX=numara, RDI/RSI/RDX/R10/R8/R9=arg1..6,
/// donus RAX. `syscall` komutu RCX'e donus adresini, R11'e RFLAGS'i koyar --
/// bu ikisi geri donusde sarttir, bu yuzden cerceveye dahildir.
#[repr(C)]
#[derive(Debug)]
#[allow(dead_code)]
pub struct SyscallFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
}

impl SyscallFrame {
    /// x86_64 Linux ABI: RAX = syscall numarasi.
    pub fn number(&self) -> u32 {
        self.rax as u32
    }

    /// x86_64 Linux ABI: RDI, RSI, RDX, R10, R8 = arg1..5.
    ///
    /// NOT: arg4 icin RCX **degil** R10 kullanilir; `syscall` komutu RCX'i
    /// donus adresi icin ezdiginden Linux bu degisikligi yapmistir.
    ///
    /// Donus tipi `usize`: ortak katmanlar (POSIX/NT cevirmenleri) boylece
    /// i386 ve x86_64'te ayni kodla calisir.
    pub fn args(&self) -> [usize; 5] {
        [
            self.rdi as usize,
            self.rsi as usize,
            self.rdx as usize,
            self.r10 as usize,
            self.r8 as usize,
        ]
    }

    pub fn set_return(&mut self, value: usize) {
        self.rax = value as u64;
    }
}
