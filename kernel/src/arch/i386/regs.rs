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

    /// `user_context`'in tersi: cerceveye yazilan baglam, `popa` + `iret`
    /// ile Ring 3'e **oldugu gibi** doner.
    ///
    /// Sinyal teslimi bunu ister: kullanicinin EIP'si isleyiciye, ESP'si
    /// isleyicinin cercevesine cevrilir; `sigreturn` de saklanan baglami
    /// ayni yoldan geri koyar.
    ///
    /// CS/SS'e dokunulmaz -- ayricalik seviyesi degismiyor.
    ///
    /// # Safety
    /// `user_context` ile ayni kosul: yalnizca Ring 3'ten gelen bir
    /// cerceve icin gecerlidir.
    pub unsafe fn set_user_context(&mut self, ctx: &UserContext) {
        self.edi = ctx.edi;
        self.esi = ctx.esi;
        self.ebp = ctx.ebp;
        self.ebx = ctx.ebx;
        self.edx = ctx.edx;
        self.ecx = ctx.ecx;
        self.eax = ctx.eax;
        let iret = (self as *mut SyscallFrame as *mut u32).add(8);
        iret.write(ctx.eip);
        iret.add(2).write(ctx.eflags);
        iret.add(3).write(ctx.esp);
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

    /// Kullanici yigin isaretcisi -- sinyal cercevesi buraya kurulur.
    pub fn stack_pointer(&self) -> usize {
        self.esp as usize
    }

    /// Baglami baska bir kod adresine ve yigina cevirir (sinyal teslimi).
    pub fn redirect(&mut self, ip: usize, sp: usize) {
        self.eip = ip as u32;
        self.esp = sp as u32;
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

/// Bir CPU istisnasinda yigina konan **tam** cerceve.
///
/// `x86-interrupt` ABI'si istisna handler'ina genel registerlari vermez;
/// yalnizca EIP/CS/EFLAGS/ESP/SS gorunur. Bu, hatayi *raporlamak* icin
/// yetiyordu ama iki sey icin yetmez:
///
///   * Windows SEH bir **CONTEXT** kaydi ister -- yani butun registerlar.
///   * Isleyici o kaydi degistirip "devam et" derse, registerlarin geri
///     yazilabiliyor olmasi gerekir.
///
/// Bu yuzden istisna girisleri artik elle yazilmis stub'lardir (bkz.
/// `idt::i386`) ve su duzeni kurarlar (dusuk adresten yuksege):
///
/// ```text
///   pusha blogu (32 bayt)   <- cercevenin basi
///   vector                  <- stub itti
///   error_code              <- CPU itti (ya da stub 0 itti)
///   EIP, CS, EFLAGS         <- CPU itti
///   ESP, SS                 <- CPU itti, YALNIZCA Ring 3'ten gelirse
/// ```
///
/// Son iki alan Ring 0 istisnalarinda **yoktur**; `from_user()` yanlissa
/// okunmamalidir.
#[repr(C)]
#[derive(Debug)]
#[allow(dead_code)]
pub struct ExceptionFrame {
    pub edi: u32,
    pub esi: u32,
    pub ebp: u32,
    pub esp_dummy: u32,
    pub ebx: u32,
    pub edx: u32,
    pub ecx: u32,
    pub eax: u32,
    pub vector: u32,
    pub error_code: u32,
    pub eip: u32,
    pub cs: u32,
    pub eflags: u32,
    pub esp: u32,
    pub ss: u32,
}

impl ExceptionFrame {
    /// Hata Ring 3'ten mi geldi? (CS'in RPL'i 3 mu)
    pub fn from_user(&self) -> bool {
        self.cs & 3 == 3
    }

    /// Hataya yol acan komutun adresi.
    pub fn instruction_pointer(&self) -> usize {
        self.eip as usize
    }

    /// Istisna anindaki **tam** Ring 3 baglami.
    ///
    /// # Safety
    /// Yalnizca `from_user()` dogruyken gecerlidir: Ring 0 istisnalarinda
    /// CPU ESP/SS itmez ve o alanlar baska verinin uzerine denk gelir.
    pub unsafe fn user_context(&self) -> UserContext {
        UserContext {
            edi: self.edi,
            esi: self.esi,
            ebp: self.ebp,
            ebx: self.ebx,
            edx: self.edx,
            ecx: self.ecx,
            eax: self.eax,
            eip: self.eip,
            eflags: self.eflags,
            esp: self.esp,
        }
    }

    /// `user_context`'in tersi: yazilan baglam `popa` + `iretd` ile Ring
    /// 3'e oldugu gibi doner. CS/SS'e dokunulmaz.
    ///
    /// # Safety
    /// `user_context` ile ayni kosul.
    pub unsafe fn set_user_context(&mut self, ctx: &UserContext) {
        self.edi = ctx.edi;
        self.esi = ctx.esi;
        self.ebp = ctx.ebp;
        self.ebx = ctx.ebx;
        self.edx = ctx.edx;
        self.ecx = ctx.ecx;
        self.eax = ctx.eax;
        self.eip = ctx.eip;
        self.eflags = ctx.eflags;
        self.esp = ctx.esp;
    }
}

impl SyscallFrame {
    /// x86_64'teki ikiziyle ayni imza, ama i386'da **tek** bir giris yolu
    /// var: hem `int 0x80` hem `int 0x2E` birer kesme kapisidir ve ikisi
    /// de ayni cerceveyi kurar. Bayrak bu yuzden yok sayilir; imzanin
    /// ortak olmasi, ust katmanlarin mimariye gore dallanmasini onler.
    ///
    /// # Safety
    /// `user_context` ile ayni kosul.
    pub unsafe fn user_context_via(&self, _from_interrupt: bool) -> UserContext {
        self.user_context()
    }

    /// # Safety
    /// `set_user_context` ile ayni kosul.
    pub unsafe fn set_user_context_via(&mut self, _from_interrupt: bool, ctx: &UserContext) {
        self.set_user_context(ctx)
    }
}
