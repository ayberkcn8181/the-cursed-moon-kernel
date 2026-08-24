# TCMK -- Dosya Rehberi

Bu belge deponun **her kaynak dosyasinin ne ise yaradigini** anlatir.
Amac bir API referansi degil, bir **harita**: bir sey degistirmek
istediginde nereye bakacagini bilmek.

Depo dort ana parcaya bolunur ve dordu de dokumanin katman
isimlendirmesini izler:

```text
  boot/         onyukleyici (GRUB yapilandirmasi + TCMK'nin kendi zinciri)
  kernel/       cekirdek -- Level-0b2, Level-0b1, Level-0a
  userland-rs/  Ring 3 kutuphanesi ve uygulamalari (Level-1)
  tools/        gelistirme araclari (Python), calisma zamaninda kullanilmaz
```

Katmanlarin anlami (doc S.2):

| Katman | Rol | Dizin |
|---|---|---|
| **Level-0b2** | Merkezi Denetleyici: her kesme/istisna/syscall once buraya duser | `kernel/src/level0b2/` |
| **Level-0b1** | Ceviri katmani: POSIX ve NT alt sistemleri, ikili yukleyiciler | `kernel/src/level0b1/` |
| **Level-0a** | Ana cekirdek: zamanlayici, MMU, VFS, suruculer, kabuk, pencere yoneticisi | `kernel/src/level0a/` |
| **Level-1** | Ring 3 kullanici katmani | `userland-rs/` |

---

## Kok dizin

| Dosya | Amaci |
|---|---|
| `Makefile` | Butun yapinin tek giris noktasi. `make ARCH=i386`, `make iso`, `make run`, `make disk`, `make userland`. Iceride `cargo` cagirir; hangi uygulamalarin derlenecegi `USER_APPS` / `WIN_APPS` / `USER64_APPS` / `WIN64_APPS` listelerinde. |
| `README.md` | Projenin anlatisi: her yetenegin **neden** oyle yapildigi, olcumlerin bulduğu hatalar ve bilerek yapilmayanlar. 4500 satir; bu depodaki en uzun belge. |
| `rust-toolchain.toml` | Nightly surumunu sabitler. `-Z build-std` ve `abi_x86_interrupt` nightly gerektirdigi icin zorunlu. |
| `LICENSE` | MIT. |
| `.gitignore` | `build/`, `target/` gibi uretilen dizinler. |

## `targets/` -- ozel hedef tanimlari

Rust'in bilmedigi dort hedef. Hicbiri isletim sistemi varsaymaz
(`"os": "none"`), yani `std` yok.

| Dosya | Amaci |
|---|---|
| `i686-tcmk.json` | ELF32 Ring 3 uygulamalari (Linux ABI tarafi). |
| `x86_64-tcmk.json` | ELF64 karsiligi. Cekirdek de bu hedeflerle derlenir. |
| `i686-tcmk-win.json` | PE32 uretir (`rust-lld` `msvc-lld` kipinde). **Windows arac zinciri gerektirmez.** |
| `x86_64-tcmk-win.json` | PE32+ karsiligi; taban `0x140000000`, yani yeniden yerlesim zorunlu. |

## `boot/` -- acilis

| Dosya | Amaci |
|---|---|
| `grub/grub.cfg` | ISO icin GRUB menusu. `multiboot` (i386) ya da `multiboot2` (x86_64) ile cekirdegi yukler. |
| `linker/i386.ld` | Cekirdek 1 MiB'a linklenir; bolum sirasi Multiboot basliginin ilk 8 KiB'da kalmasini garanti eder. |
| `linker/x86_64.ld` | Ayni is, uzun mod icin. |
| `tcmkboot/` | **TCMK'nin kendi onyukleyicisi** -- GRUB'a alternatif. Diskten acilis icin. |
| `tcmkboot/src/bin/stage1.rs` | 512 baytlik MBR. Tek isi stage2'yi okumak; boyut siniri yuzunden neredeyse tamami assembly. |
| `tcmkboot/src/bin/stage2.rs` | Gercek is: A20, korumali mod, cekirdek ELF'ini diskten okuyup 1 MiB'a yerlestirmek, Multiboot bilgi yapisini uydurmak. |
| `tcmkboot/stage1.ld`, `stage2.ld` | Ikisinin yerlesimi (0x7C00 ve 0x8000). |
| `tcmkboot/Cargo.toml`, `.cargo/config.toml` | 16-bit/32-bit karisik derleme ayarlari. |

---

# `kernel/` -- cekirdek

| Dosya | Amaci |
|---|---|
| `Cargo.toml` | `no_std` binary crate. |
| `.cargo/config.toml` | `-Z build-std=core,compiler_builtins` ve hedef secimi. |
| `src/main.rs` | Acilis sirasi ve **gomulu dosya tablosu**. Butun Ring 3 ikilileri buradan `include_bytes!` ile cekirdege gomulur ve acilista RAMFS'e baglanir. |

## `src/boot/` -- mimariye ozel acilis kodu

| Dosya | Amaci |
|---|---|
| `i386.rs` | Multiboot1 basligi + `_start`: yigin kurar, `kernel_main`e atlar. `global_asm!` ile yazilmis. |
| `x86_64.rs` | Multiboot2 basligi + uzun moda gecis: PAE, sayfa tablolari, `EFER.LME`, `CR0.PG`. En zor acilis kodu burada. |
| `multiboot.rs` | Onyukleyicinin biraktigi bilgi yapisini ayristirir: bellek haritasi ve **framebuffer**. |
| `mod.rs` | Mimariye gore dogru modulu secer. |

## `src/arch/` -- CPU'ya dokunan her sey

Ustteki katmanlar `crate::arch::cpu` diye tek bir ada bakar; hangi
mimari oldugu derleme aninda kapanir.

| Dosya | Amaci |
|---|---|
| `mod.rs` | `cpu` takma adi -- ortak kodun mimariden bagimsiz kalmasini saglayan tek satir. |
| `i386/mod.rs` | Port G/C (`inb`/`outb`), kesme acma/kapama, `hlt`. |
| `i386/regs.rs` | `SyscallFrame` (pusha duzeni), `ExceptionFrame` (tam istisna cercevesi), `UserContext`. Ring 3 baglaminin okunup yazildigi yer. |
| `i386/context.rs` | Gorev degistirme (`arch_context_switch`) -- callee-saved registerlari yigina koyup yigin isaretcisini takas eder. |
| `i386/usermode.rs` | Ring 3'e gecis (`iret`) ve geri donus (setjmp/longjmp cifti), sinyal cercevesi kurulumu. |
| `x86_64/mod.rs` | Ayni ilkeler + MSR okuma/yazma. |
| `x86_64/regs.rs` | i386 ikizi. Ek olarak `user_context_via`: `syscall` komutu ile `int 0x80` **farkli cerceve duzeni** kullanir, ayrim burada. |
| `x86_64/context.rs` | Gorev degistirme, 64-bit. |
| `x86_64/usermode.rs` | `iretq` ile Ring 3. FS/GS secicisi **bilerek yuklenmez**: secici yuklemek taban MSR'sini sifirlar. |

---

# Level-0b2 -- Merkezi Denetleyici

Dokumanin merkezinde duran katman. Her sistem cagrisi, her istisna once
buradan gecer.

| Dosya | Amaci |
|---|---|
| `dispatcher.rs` | Cagriyi siniflandirir, yuk olcumune bildirir, Level-0a saglikliysa cevirmene devreder -- degilse fallback'e. Sinyal teslim noktasi da burasi. |
| `state_monitor.rs` | Level-0a'nin nabzini (PIT tiki) izler. Belirli sure artis yoksa "olu" ilan eder. |
| `load_balancer.rs` | Kanal basina cagri sayaci. Bir surec kotayi asarsa **geri baski** uygulanir (kuyruga alma degil: gorev sirasini bekler). |
| `fallback.rs` | Level-0a olduginde devreye giren acil durum yuzu: temel syscall'lari kendisi taklit eder, ekrana durum yazar. |
| `ipc.rs` | Level-0b2 ile Level-0a arasindaki halka tampon. Kabuktaki sistem gunlugu penceresi bunu okur. |
| `mod.rs` | Modul agaci. |

---

# Level-0b1 -- Ceviri katmani

Buradaki hicbir modul donanima dokunmaz. Yalnizca **cagri sozlesmesi
cevirisi** yapip Level-0a'nin ortak API'sine devreder.

## Ortak

| Dosya | Amaci |
|---|---|
| `process.rs` | Bir ikiliyi yukleyip Ring 3'e sokan akis. Adres uzayi kurar, yigin yerlesimini yapar (POSIX'te `argc/argv/envp/auxv`, Win32'de tek komut satiri), TSS'i ayarlar. |
| `argv.rs` | **Arguman vektoru.** POSIX `argv[]` dizisi ile Win32 komut satiri arasindaki ortak tasiyici (NUL ayrilmis blok) ve `CommandLineToArgvW`nin alintilama kurallari. |
| `fork.rs` | `fork`: adres uzayini copy-on-write kopyalar, cocuk gorevi kurar, ebeveynin baglamini 0 donusuyle cocuga verir. `execve` zinciri de burada yurutulur. |
| `signal.rs` | POSIX sinyalleri: yerlestirme tablosu, maske, ic ice teslim, `sigreturn`. Cekirdek kullanici yiginina bir cerceve kurup baglami isleyiciye cevirir. |

## `binary_loader/` -- ikili yukleyiciler

| Dosya | Amaci |
|---|---|
| `elf32.rs` | ELF32 yukleyici. `PT_LOAD` segmentlerini kopyalar, `.bss`i sifirlar, program baslik tablosunun **bellekteki** adresini raporlar (`auxv` icin). |
| `elf64.rs` | ELF64 karsiligi. |
| `pe32.rs` | PE32 yukleyici: bolumler, **taban yeniden yerlesimi**, ithal tablosu cozumu. Her ithal fonksiyon icin surecin adres uzayina bir thunk yazar. |
| `pe64.rs` | PE32+ karsiligi. Taban `0x140000000` oldugu icin yeniden yerlesim zorunlu; thunk'lar Win64 cagri gelenegini kullanir. |
| `mod.rs` | Modul agaci. |

## `linux_subsystem/` -- POSIX cevirisi

| Dosya | Amaci |
|---|---|
| `posix_syscalls.rs` | `int 0x80` / `syscall` ile gelen cagrilarin tamami. Numaralar **mimariye gore degisir** ve iki ayri tabloda tutulur -- bu ayrimi yapmamak Faz 4'te gercek bir hataya yol acmisti. 60 cagri. |
| `mod.rs` | Modul agaci. |

## `nt_subsystem/` -- Windows NT cevirisi

| Dosya | Amaci |
|---|---|
| `nt_syscalls.rs` | `int 0x2E` ile gelen cagrilar. Uc aralik: `0x1000` (ham NT), `0x2000` (win32k), `0x3000` (Win32 API, yigin argumanli). Deponun en uzun kaynak dosyasi. |
| `dll.rs` | **Gomulu DLL tablosu.** Diskte `KERNEL32.dll` yok; bu tablo adi bir NT servis numarasina cevirir ve `emit_thunk` surecin adres uzayina cagri stub'i yazar. |
| `teb.rs` | Is Parcacigi Ortam Blogu. `fs:[0x18]` / `gs:[0x30]`den ulasilan yapi; SEH zinciri, son hata kodu, yigin sinirlari. PEB ve modul listesi de bu blokta kurulur. |
| `seh.rs` | **Windows istisna dagitimi.** VEH listesi, `fs:[0]` zinciri, `UnhandledExceptionFilter`. Cekirdek kullanici yiginina `EXCEPTION_RECORD` + `CONTEXT` yazip cerceveyi isleyiciye cevirir. |
| `modules.rs` | Modul tablosu: `GetModuleHandleA`, `GetProcAddress`, `LoadLibraryA`. Ithal edilmemis bir fonksiyon istendiginde thunk'i **o anda** uretir. |
| `mapping.rs` | Dosya esleme nesneleri: `CreateFileMapping` + `MapViewOfFile`. POSIX'in tek cagrisina karsilik iki adim; gorunum uzunlugu cekirdekte tutulur. |
| `mod.rs` | Modul agaci. |

---

# Level-0a -- Ana cekirdek

## Ust duzey

| Dosya | Amaci |
|---|---|
| `kernel_api.rs` | Level-0b1'in cagirdigi **ortak API**. Dosya, dizin, ortam, saat, surec islemleri. Iki alt sistem de buraya iner -- ayrisan yalnizca sozlesme. |
| `shell.rs` | Metin kabugu: `ls`, `cat`, `run`, `ps`, `mem`, `faults`, `env`, `disk`... Kabuk komutlari ayri bir gorevde kosar. |
| `wm.rs` | Pencere yoneticisi ve birlestirici. Pencere listesi, odak, surukleme, cizim sirasi. |
| `gui_api.rs` | Ring 3'ten gelen pencere cagrilarinin karsiligi: pencere ac, tampon adresi ver, tus olayi teslim et. |
| `launcher.rs` | Uygulama baslatma: kisa adi tam yola cevirir, gorev yaratir, `execve` isteklerini kuyruklar. |
| `keyboard.rs` | PS/2 klavye: tarama kodu -> ASCII, shift/caps durumu. |
| `input.rs` | PS/2 fare (IRQ12): konum ve dugme durumu. |
| `pic.rs` | 8259A yeniden haritalama. Slave denetleyici `0x70`e alindigi icin IRQ12/14 vektorleri 116/118. |
| `pit.rs` | 8253/8254 zamanlayici, 100 Hz. Nabiz sayaci burada artar. |
| `exceptions.rs` | Butun 32 CPU istisnasinin ortak govdesi. Kurtarilabilir mi (COW, talep uzerine sayfalama), SEH'e devredilebilir mi, yoksa olumcul mu. |
| `syscall_msr.rs` | x86_64'un `syscall` komutu icin MSR kurulumu (`STAR`, `LSTAR`, `SFMASK`). |
| `installer.rs` | Sistemi diske kuran akis (`install` komutu). |
| `messages.rs` | Acilista ekrana yazilan metinler; tek yerde toplanmis. |
| `mod.rs` | Modul agaci. |

## `gdt/` ve `idt/`

| Dosya | Amaci |
|---|---|
| `gdt/i386.rs` | GDT: Ring 0/3 segmentleri, TSS, ve **is-parcacigi tanimlayicilari** (FS/GS tabanlari). |
| `gdt/x86_64.rs` | GDT64 + TSS. Uzun modda segment tabani yok; TLS MSR ile. |
| `idt/i386.rs` | 256 vektorluk IDT. Istisna girisleri elle yazilmis stub'lar (`pusha` + cerceve adresi) -- `x86-interrupt` ABI'si genel registerlari vermiyor. |
| `idt/x86_64.rs` | Ayni is, 16 baytlik girdilerle. |
| `gdt/mod.rs`, `idt/mod.rs` | Mimariye gore secim. |

## `core/` -- cekirdegin ic makinesi

| Dosya | Amaci |
|---|---|
| `scheduler.rs` | Gorev tablosu, oncelikli round-robin, preemption, uyku/bekleme durumlari, `waitpid` destegi. Deponun en yogun dosyalarindan. |
| `mmu_i386.rs` | Iki seviyeli sayfalama, surec basina adres uzayi, copy-on-write, talep uzerine sayfalama, `mmap` penceresi. |
| `mmu_x86_64.rs` | Dort seviyeli karsiligi. |
| `frames.rs` | Fiziksel cerceve havuzu + **basvuru sayaci** (COW icin sart). |
| `kmalloc.rs` | Cekirdek yigini. Bump degil: serbest liste tutar, cunku pencere tamponlari acilip kapaniyor. |
| `vfs.rs` | Dugum tablosu ve iki arka uc: salt okunur RAMFS (gomulu dosyalar) ve yazilabilir TCMKFS (disk). |
| `tcmkfs.rs` | Kalici dosya sistemi: superblok, inode, dogrudan blok isaretcileri, dizin agaci, `truncate`. |
| `fd.rs` | Tanimlayici tablosu. `dup`/`dup2`, `fork`'ta kopyalama, boru tanimlayicilari. |
| `pipe.rs` | Borular. Okuma bloke etmez -- GUI dongusunu durdurmamak icin. |
| `dir.rs` | Dizin gezinmesi (`getdents` / `FindFirstFile` ortak zemini). |
| `cwd.rs` | Calisma dizini **surec basina**. `fork`ta devralinir, `execve`de korunur. |
| `env.rs` | Ortam degiskenleri, surec basina tablo. Oturum tablosu kabuga ait. |
| `tls.rs` | Is-parcacigi tabanlari. i386'da GDT tanimlayicisi, x86_64'te MSR; Linux ile Windows'un register secimi **capraz**. |
| `init.rs` | Acilis sirasinin `core` ayagi. |
| `mod.rs` | Mimariye gore `mmu` takma adi. |

## `drivers/`

| Dosya | Amaci |
|---|---|
| `vga.rs` | 80x25 metin modu -- grafik acilmadan onceki tek cikis. |
| `gfx.rs` | Framebuffer ilkelleri: dikdortgen, metin, kopyalama. |
| `font_data.rs` | 8x16 bitmap yazi tipi (uretilmis; bkz. `tools/gen_font.py`). |
| `console.rs` | Grafik konsol: kaydirma, imlec, satir tamponu. |
| `serial.rs` | COM1. Butun olcumler bu gunluge yazilir. |
| `ata.rs` | ATA PIO surucusu (IRQ14 ile bekler, yoklamaz). |
| `block.rs` | Blok katmani: sektor okuma/yazma soyutlamasi. |
| `partition.rs` | MBR bolum tablosu ayristirma. |
| `rtc.rs` | Gercek zaman saati -- `time()` ve `GetSystemTime` buradan besleniyor. |
| `mod.rs` | Modul agaci. |

---

# `userland-rs/` -- Level-1

Ayni kaynak agaci hem ELF hem PE olarak derlenir. Cizim kodu **ortak**;
ayrisan yalnizca cekirdege hangi kapidan girildigi.

## Kutuphane

| Dosya | Amaci |
|---|---|
| `src/lib.rs` | `entry!` makrosu, panik isleyicisi, `exit_process`. Iki ABI'nin ayrildigi tek yer. |
| `src/sys.rs` | POSIX sistem cagrilari (`int 0x80` / `syscall`). `syscall6` i386'da elle yazilmis: altinci arguman EBP'ye gider ve EBP derleyicinin cerceve isaretcisi. |
| `src/nt.rs` | Ham NT cagrilari (`int 0x2E`), `0x1000` araligi. |
| `src/winapi.rs` | **Ithal tablosu uzerinden** Win32 API. Cagrilar IAT'den gecer -- yani derleyicinin urettigi siradan bir Windows ikilisi gibi. |
| `src/win32.rs` | `winapi`nin elle `int 0x2E` yapan ikizi; ithal tablosu olmadan calisan yol. |
| `src/args.rs` | `argc`/`argv` (POSIX yigini) ve `GetCommandLineA` (Win32 dizesi) ayristirmasi + **yardimci vektor** (`auxv`) erisimi. |
| `src/env.rs` | Ortam degiskenleri; iki ABI icin tek yuz. |
| `src/io.rs` | `Stdout`/`Stdin`, `core::fmt::Write` uyarlamasi. |
| `src/gui.rs` | POSIX tarafinin pencere API'si. |
| `src/canvas.rs` | Cizim ilkelleri -- **iki ABI'nin paylastigi kod**. |
| `src/font.rs` | Yazi tipi (cekirdektekiyle ayni veri). |
| `src/signal.rs` | Sinyal isleyicisi kaydi ve `sigreturn` tramplenler. |
| `src/tls.rs` | POSIX is-parcacigi tabani (`set_thread_area` / `arch_prctl`). |
| `src/teb.rs` | Windows TEB erisimi: kimlik, son hata, SEH zincirinin basi. |
| `src/seh.rs` | Windows istisna yapilari: `EXCEPTION_RECORD`, CONTEXT register erisimi, zincir kaydi (`ChainGuard`). |
| `win/kernel32.def` | Ithal kutuphanesinin tanimi. Ordinaller **acikca** verilmistir; cekirdegin gomulu tablosuyla ayni numaralar. |
| `win/tcmkgui.def` | Pencere cagrilarinin karsiligi (`TCMKGUI.dll`). |
| `Cargo.toml` | Her uygulama ayri bir `[[bin]]`. |

## ELF uygulamalari (`src/bin/`)

Isaretlenenler **sinav programlari**: sonuclarini hem ekrana hem seri
gunluge yazarlar ve olcum bunlardan okunur.

| Dosya | Amaci |
|---|---|
| `hello.rs` | En kucuk uygulama; Ring 3'un calistiginin kaniti. |
| `paint.rs`, `plasma.rs` | Grafik animasyonlari; pencere yoneticisini yuk altinda gosterir. |
| `notes.rs` | Not defteri: yaz, kaydet, ac. Disk yolunun ucundan ucuna sinavi. |
| `browse.rs` | Dosya gezgini; `getdents` ve `getcwd` kullanir. |
| `menu.rs` | Uygulama baslatici (`execve`). |
| `echo2.rs` | Argumanlari yazar -- en basit argv sinavi. |
| `crash.rs` | **Bilerek coker**: hata izolasyonunun sinavi. Sistem ayakta kalmali. |
| `hog.rs`, `spin.rs` | CPU yiyen gorevler; yuk dengeleyici ve preemption sinavi. |
| `twins.rs`, `race.rs` | Es zamanli gorevler. |
| `relay.rs`, `mux.rs`, `redirect.rs` | Boru, `poll` ve `dup2` sinavlari. |
| `sigdemo.rs`, `masked.rs`, `nested.rs` (4 sinav) | Sinyal teslimi, maskeleme, ic ice isleyiciler. |
| `reaper.rs`, `waiter.rs`, `heir.rs` | `fork`/`waitpid`/`execve` ve calisma dizini devri. |
| `arena.rs` | `mmap`/`munmap` -- cerceve geri verme sinavi. |
| `seeker.rs` | `lseek` ve rastgele erisim. |
| `bequest.rs` (6 sinav) | Ortam degiskenleri: `setenv`, `fork` mirasi, kardes yalitimi. |
| `probe.rs` (16 sinav) | POSIX yuzeyinin genis sinavi: `stat`, `access`, `uname`, saat, `writev`, TLS, **yardimci vektor**. |
| `quoted.rs` (4 sinav) | `execve(yol, argv[], envp[])`: dizi bicimi, `argv[0]` korunmasi, `envp` degistirmesi. |
| `mapped.rs` (4 sinav) | Dosya destekli `mmap`: icerik, hizasiz ofset reddi, dosya sonu sifirlamasi. |

## PE uygulamalari (`src/win/`)

| Dosya | Amaci |
|---|---|
| `clock.rs` | Saat; `GetSystemTime` ve pencere cagrilari. |
| `notepad.rs` | Not defteri, Win32 yuzuyle. `WriteFile` ile kaydeder. |
| `explorer.rs` | Dosya gezgini; `FindFirstFileA`/`FindNextFileA`. |
| `envtest.rs` (4 sinav) | Win32 ortam sozlesmesi: donus boyu, NULL ile silme. |
| `probe.rs` (12 sinav) | Win32 yuzeyi: dosya ozellikleri, surum, saat, TEB, **`CreateProcessA` ile bir ELF baslatma**. |
| `seh.rs` (9 sinav) | Istisna dagitimi: VEH, `fs:[0]` zinciri, `RaiseException`, sahipsiz istisna filtresi. |
| `modules.rs` (6 sinav) | PEB ve modul tablosu; `GetProcAddress` ile **ithal edilmemis** bir fonksiyonu bulup cagirma. |
| `argv.rs` (4 sinav) | Komut satiri alintilamasi ve `lpEnvironment` blogu. |
| `mapping.rs` (4 sinav) | `CreateFileMapping` + `MapViewOfFile`; adlandirilmis eslemenin acikca reddi. |

---

# `tools/` -- gelistirme araclari

Python; yalnizca **derleme aninda** kullanilir, calisan sistemde yer
almazlar.

| Dosya | Amaci |
|---|---|
| `gen_font.py` | Bitmap yazi tipini uretir. |
| `sync_font.py` | Ayni yazi tipini cekirdek ve userland kopyalarina yayar. |
| `gen_hello_elf64.py` | Elle kodlanmis en kucuk ELF64. Yukleyicinin **dar yolunu** sinar -- derleyici cikti bir ikilinin gizledigi varsayimlari acar. |
| `gen_pe_hello.py` | Ayni is, PE32 icin. |
| `make_disk.py` | ISO'nun arkasina TCMKFS bolumu ekleyip MBR bolum tablosunu yazar. Root gerektirmez. |

---

# `userland/` ve `docs/`

| Dizin | Icerik |
|---|---|
| `userland/` | **Uretilen** Ring 3 ikilileri (`.elf`, `.elf64`, `.exe`, `.exe64`). Cekirdek bunlari `include_bytes!` ile gomer, o yuzden depoda tutuluyorlar. |
| `docs/` | Ekran goruntuleri (README'nin refere ettigi) ve bu belge. |

---

# Nereden baslamali

| Ilgi alanin | Once bak |
|---|---|
| Acilis | `boot/x86_64.rs`, `main.rs` |
| Bir sistem cagrisi eklemek | `posix_syscalls.rs` ya da `nt_syscalls.rs`, sonra `kernel_api.rs` |
| Bellek yonetimi | `mmu_i386.rs`, `frames.rs` |
| Zamanlama | `scheduler.rs`, `pit.rs` |
| Windows uyumlulugu | `pe32.rs`, `dll.rs`, `teb.rs`, `seh.rs` |
| Linux uyumlulugu | `elf32.rs`, `posix_syscalls.rs`, `signal.rs` |
| Bir sinav programi yazmak | `userland-rs/src/bin/probe.rs` ornegine bak |
