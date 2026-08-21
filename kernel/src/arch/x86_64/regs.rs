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

    /// Kullanici yigin isaretcisi -- sinyal cercevesi buraya kurulur.
    pub fn stack_pointer(&self) -> usize {
        self.rsp as usize
    }

    /// Baglami baska bir kod adresine ve yigina cevirir (sinyal teslimi).
    pub fn redirect(&mut self, ip: usize, sp: usize) {
        self.rip = ip as u64;
        self.rsp = sp as u64;
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

    /// `user_context`'in tersi: cerceveye yazilan baglam `sysretq` ile
    /// Ring 3'e **oldugu gibi** doner.
    ///
    /// RIP ve RFLAGS'in RCX/R11 uzerinden gitmesi burada bir kolaylik:
    /// `sysretq` zaten o ikisini kullanir, yani ayri bir yol gerekmez.
    ///
    /// # Safety
    /// `user_context` ile ayni kosul: yalnizca Ring 3'ten gelen bir
    /// cerceve icin gecerlidir.
    pub unsafe fn set_user_context(&mut self, ctx: &UserContext) {
        self.rax = ctx.rax;
        self.rbx = ctx.rbx;
        self.rdx = ctx.rdx;
        self.rsi = ctx.rsi;
        self.rdi = ctx.rdi;
        self.rbp = ctx.rbp;
        self.r8 = ctx.r8;
        self.r9 = ctx.r9;
        self.r10 = ctx.r10;
        self.r12 = ctx.r12;
        self.r13 = ctx.r13;
        self.r14 = ctx.r14;
        self.r15 = ctx.r15;
        // RCX = donus RIP'i, R11 = donus RFLAGS'i (sysretq boyle ister).
        self.rcx = ctx.rip;
        self.r11 = ctx.rflags;
        (self as *mut SyscallFrame as *mut u64).add(15).write(ctx.rsp);
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

/// Bir CPU istisnasinda yigina konan **tam** cerceve (x86_64).
///
/// Gerekce ve duzen i386'daki ikiziyle aynidir (bkz. `arch::i386::regs`):
/// `x86-interrupt` ABI'si genel registerlari vermedigi icin istisna
/// girisleri elle yazilmis stub'lardir. Alan sirasi `SyscallFrame` ile
/// bilincli olarak aynidir -- iki giris yolu ayni push desenini kullanir.
///
/// ```text
///   R15..RAX (15 kelime)    <- stub itti
///   vector                  <- stub itti
///   error_code              <- CPU itti (ya da stub 0 itti)
///   RIP, CS, RFLAGS         <- CPU itti
///   RSP, SS                 <- CPU itti (x86_64'te ayricalik degismese
///                              bile HER ZAMAN itilir)
/// ```
///
/// Son satir x86_64'un i386'dan gercek bir farkidir: uzun modda `iretq`
/// cercevesi her zaman bes kelimedir, yani `rsp`/`ss` Ring 0
/// istisnalarinda da gecerlidir.
#[repr(C)]
#[derive(Debug)]
#[allow(dead_code)]
pub struct ExceptionFrame {
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
    pub vector: u64,
    pub error_code: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

impl ExceptionFrame {
    /// Hata Ring 3'ten mi geldi? (CS'in RPL'i 3 mu)
    pub fn from_user(&self) -> bool {
        self.cs & 3 == 3
    }

    /// Hataya yol acan komutun adresi.
    pub fn instruction_pointer(&self) -> usize {
        self.rip as usize
    }

    /// Istisna anindaki **tam** Ring 3 baglami.
    ///
    /// # Safety
    /// Yalnizca `from_user()` dogruyken anlamlidir.
    pub unsafe fn user_context(&self) -> UserContext {
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
            rip: self.rip,
            rsp: self.rsp,
            rflags: self.rflags,
        }
    }

    /// `user_context`'in tersi; yazilan baglam `iretq` ile geri doner.
    ///
    /// Burada RIP/RFLAGS **dogrudan** kendi alanlarina yazilir -- syscall
    /// yolundaki RCX/R11 dolayisi yok, cunku `iretq` onlari cerceveden
    /// okur.
    ///
    /// # Safety
    /// `user_context` ile ayni kosul.
    pub unsafe fn set_user_context(&mut self, ctx: &UserContext) {
        self.rax = ctx.rax;
        self.rbx = ctx.rbx;
        self.rcx = ctx.rcx;
        self.rdx = ctx.rdx;
        self.rsi = ctx.rsi;
        self.rdi = ctx.rdi;
        self.rbp = ctx.rbp;
        self.r8 = ctx.r8;
        self.r9 = ctx.r9;
        self.r10 = ctx.r10;
        self.r11 = ctx.r11;
        self.r12 = ctx.r12;
        self.r13 = ctx.r13;
        self.r14 = ctx.r14;
        self.r15 = ctx.r15;
        self.rip = ctx.rip;
        self.rflags = ctx.rflags;
        self.rsp = ctx.rsp;
    }
}

impl SyscallFrame {
    /// Cagiranin baglami -- **hangi kapidan girildigine gore**.
    ///
    /// x86_64'te bir sistem cagrisi iki ayri yoldan gelebilir ve ikisi
    /// donus bilgisini **baska yerde** tasir:
    ///
    /// ```text
    ///   syscall komutu   RIP -> RCX,  RFLAGS -> R11,  RSP -> stub itti
    ///   int 0x80/0x2E    RIP/RFLAGS/RSP -> CPU'nun kesme cercevesinde
    /// ```
    ///
    /// Ayrimi yapmamak sessiz ve yikici bir hataya yol acar: kesme
    /// yolundan gelen bir cerceveye `sysretq` duzeniyle yazmak, RSP'yi
    /// RIP yuvasina koymak demektir -- surec veriye dallanir.
    ///
    /// PE thunk'lari **her zaman** `int 0x2E` kullanir (bkz.
    /// `dll::emit_thunk`), yani Windows tarafinda gecerli olan hep ikinci
    /// satirdir.
    ///
    /// # Safety
    /// Yalnizca Ring 3'ten gelen bir cerceve icin gecerlidir ve
    /// `from_interrupt` cercevenin gercek giris yolunu anlatmalidir.
    pub unsafe fn user_context_via(&self, from_interrupt: bool) -> UserContext {
        let mut ctx = self.user_context();
        if from_interrupt {
            let iret = (self as *const SyscallFrame as *const u64).add(15);
            ctx.rip = iret.read();
            ctx.rflags = iret.add(2).read();
            ctx.rsp = iret.add(3).read();
        }
        ctx
    }

    /// `user_context_via`'nin tersi.
    ///
    /// # Safety
    /// `user_context_via` ile ayni kosul.
    pub unsafe fn set_user_context_via(&mut self, from_interrupt: bool, ctx: &UserContext) {
        if !from_interrupt {
            self.set_user_context(ctx);
            return;
        }
        self.rax = ctx.rax;
        self.rbx = ctx.rbx;
        self.rcx = ctx.rcx;
        self.rdx = ctx.rdx;
        self.rsi = ctx.rsi;
        self.rdi = ctx.rdi;
        self.rbp = ctx.rbp;
        self.r8 = ctx.r8;
        self.r9 = ctx.r9;
        self.r10 = ctx.r10;
        self.r11 = ctx.r11;
        self.r12 = ctx.r12;
        self.r13 = ctx.r13;
        self.r14 = ctx.r14;
        self.r15 = ctx.r15;
        let iret = (self as *mut SyscallFrame as *mut u64).add(15);
        iret.write(ctx.rip);
        iret.add(2).write(ctx.rflags);
        iret.add(3).write(ctx.rsp);
    }
}
