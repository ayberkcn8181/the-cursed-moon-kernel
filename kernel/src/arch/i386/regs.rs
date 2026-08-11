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

    /// Cagiranin **tam** Ring 3 baglami (registerlar + EIP/ESP/EFLAGS).
    ///
    /// `pusha` blogunun hemen ustunde CPU'nun kesme girisinde ittigi
    /// cerceve durur: EIP, CS, EFLAGS ve -- ayricalik degistigi icin --
    /// kullanicinin ESP/SS'i. Duzen `syscall_entry`'de sabitlenmistir
    /// (`pusha` + `push esp`), bu yuzden `pusha` blogundan 32 bayt
    /// ileride guvenle okunabilir.
    ///
    /// `fork` bunu ister: cocuk, ebeveynin durdugu **tam** noktadan
    /// devam etmelidir; yalnizca EIP/ESP yetmez, cunku derleyici
    /// `int 0x80` sonrasinda EBX/ESI/EDI/EBP'nin korundugunu varsayar.
    ///
    /// # Safety
    /// Yalnizca **Ring 3'ten** gelen bir syscall cercevesi icin
    /// gecerlidir. Ring 0'dan gelen bir kesmede CPU SS/ESP itmez ve
    /// okunan degerler anlamsiz olur.
    pub unsafe fn user_context(&self) -> UserContext {
        let iret = (self as *const SyscallFrame as *const u32).add(8);
        UserContext {
            edi: self.edi,
            esi: self.esi,
            ebp: self.ebp,
            ebx: self.ebx,
            edx: self.edx,
            ecx: self.ecx,
            eax: self.eax,
            eip: iret.read(),
            eflags: iret.add(2).read(),
            esp: iret.add(3).read(),
        }
    }
}

impl UserContext {
    /// Bos baglam -- `static` dizilerde kullanilir (`Default` const degil).
    pub const ZERO: Self = UserContext {
        edi: 0, esi: 0, ebp: 0, ebx: 0, edx: 0, ecx: 0, eax: 0,
        eip: 0, esp: 0, eflags: 0,
    };

    /// Syscall donus degerini ayarlar (`fork`'ta cocuk icin 0).
    ///
    /// Mimariden bagimsiz: ust katman hangi registerin donus tasidigini
    /// bilmek zorunda kalmasin diye.
    pub fn set_return(&mut self, value: usize) {
        self.eax = value as u32;
    }

    /// Baglamin devam edecegi komut adresi (gunluk icin).
    pub fn instruction_pointer(&self) -> usize {
        self.eip as usize
    }
}

/// Bir Ring 3 baglaminin tamami -- `arch_enter_user_mode_regs` bunu
/// oldugu gibi geri yukleyip `iretd` ile Ring 3'e doner.
///
/// Alan sirasi assembly ile **birebir** eslesmelidir (bkz. `usermode.rs`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct UserContext {
    pub edi: u32,
    pub esi: u32,
    pub ebp: u32,
    pub ebx: u32,
    pub edx: u32,
    pub ecx: u32,
    pub eax: u32,
    pub eip: u32,
    pub esp: u32,
    pub eflags: u32,
}
