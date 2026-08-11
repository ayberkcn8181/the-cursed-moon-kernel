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

/// Bir Ring 3 baglaminin tamami -- `arch_enter_user_mode_regs` bunu
/// oldugu gibi geri yukleyip `iretq` ile Ring 3'e doner.
///
/// Alan sirasi assembly ile **birebir** eslesmelidir (bkz. `usermode.rs`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct UserContext {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rsp: u64,
    pub rflags: u64,
}

impl UserContext {
    /// Bos baglam -- `static` dizilerde kullanilir (`Default` const degil).
    pub const ZERO: Self = UserContext {
        rax: 0, rbx: 0, rcx: 0, rdx: 0, rsi: 0, rdi: 0, rbp: 0,
        r8: 0, r9: 0, r10: 0, r11: 0, r12: 0, r13: 0, r14: 0, r15: 0,
        rip: 0, rsp: 0, rflags: 0,
    };

    /// Syscall donus degerini ayarlar (`fork`'ta cocuk icin 0).
    ///
    /// Mimariden bagimsiz: ust katman hangi registerin donus tasidigini
    /// bilmek zorunda kalmasin diye.
    pub fn set_return(&mut self, value: usize) {
        self.rax = value as u64;
    }

    /// Baglamin devam edecegi komut adresi (gunluk icin).
    pub fn instruction_pointer(&self) -> usize {
        self.rip as usize
    }
}

impl SyscallFrame {
    /// Cagiranin **tam** Ring 3 baglami.
    ///
    /// `syscall` donus RIP'ini RCX'e, RFLAGS'i R11'e koyar; ikisi de
    /// cercevede duruyor. Kullanici RSP'si ise cerceveden hemen SONRA,
    /// giris stub'inin ittigi kelimededir (bkz. `syscall_msr.rs`) --
    /// yani `pusha` blogunun ustunde, i386'daki kesme cercevesiyle ayni
    /// mantik.
    ///
    /// `fork` bunu ister: cocuk ebeveynin durdugu **tam** noktadan devam
    /// etmelidir, yalnizca RIP/RSP yetmez.
    ///
    /// # Safety
    /// Yalnizca Ring 3'ten gelen bir syscall cercevesi icin gecerlidir.
    pub unsafe fn user_context(&self) -> UserContext {
        // Cerceve 15 kelimedir; 16. kelime giris stub'inin ittigi
        // kullanici RSP'sidir.
        let user_rsp = (self as *const SyscallFrame as *const u64).add(15).read();
        UserContext {
            rax: self.rax,
            rbx: self.rbx,
            rcx: self.rcx,
            rdx: self.rdx,
            rsi: self.rsi,
            rdi: self.rdi,
            rbp: self.rbp,
            r8: self.r8,
            r9: self.r9,
            r10: self.r10,
            r11: self.r11,
            r12: self.r12,
            r13: self.r13,
            r14: self.r14,
            r15: self.r15,
            rip: self.rcx,
            rsp: user_rsp,
            rflags: self.r11,
        }
    }

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
