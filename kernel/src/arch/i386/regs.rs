//! `extern "x86-interrupt"` handler'larinin CPU tarafindan yiginin ustune
//! itilen ortak alanlari. Faz 1'deki handler'lar (ISR0, IRQ0, IRQ1) hicbiri
//! hata kodu itmez, dolayisiyla tek bir frame tipi yeterlidir.

#[repr(C)]
#[allow(dead_code)]
pub struct InterruptStackFrame {
    pub instruction_pointer: u32,
    pub code_segment: u32,
    pub cpu_flags: u32,
    pub stack_pointer: u32,
    pub stack_segment: u32,
}

/// `pusha` komutunun yigina itme SIRASI ile birebir ayni duzende
/// (dusuk adresten yuksek adrese): EDI, ESI, EBP, ESP, EBX, EDX, ECX, EAX.
///
/// int 0x80 girisinde bu yapinin adresi Rust tarafina verilir; boylece
/// Level-0b1 hem syscall numarasini (EAX) hem de argumanlari (EBX/ECX/EDX/
/// ESI/EDI) okuyabilir. Donus degeri `eax` alanina yazilir -- `popa`
/// registerlari bu frame'den geri yukledigi icin kullaniciya EAX olarak
/// ulasir (i386 Linux ABI, doc S.6).
#[repr(C)]
#[derive(Debug)]
#[allow(dead_code)]
pub struct SyscallFrame {
    pub edi: u32,
    pub esi: u32,
    pub ebp: u32,
    /// `pusha`'nin kaydettigi orijinal ESP -- `popa` bunu yok sayar.
    pub esp_dummy: u32,
    pub ebx: u32,
    pub edx: u32,
    pub ecx: u32,
    pub eax: u32,
}

impl SyscallFrame {
    /// i386 Linux ABI: EAX = syscall numarasi.
    pub fn number(&self) -> u32 {
        self.eax
    }

    /// i386 Linux ABI: EBX, ECX, EDX, ESI, EDI = arg1..arg5.
    ///
    /// Donus tipi `usize`: ortak katmanlar (POSIX/NT cevirmenleri) boylece
    /// i386 ve x86_64'te ayni kodla calisir.
    pub fn args(&self) -> [usize; 5] {
        [
            self.ebx as usize,
            self.ecx as usize,
            self.edx as usize,
            self.esi as usize,
            self.edi as usize,
        ]
    }

    /// Donus degeri EAX uzerinden verilir.
    pub fn set_return(&mut self, value: usize) {
        self.eax = value as u32;
    }
}
