# The Cursed Moon Kernel (TCMK) — Rust Portu

Yuksek hata toleransli, cift uyumluluklu (Linux/Windows) mikro-cekirdek.
Bu depo, onceden C ile planlanan TCMK mimarisinin Rust'a tasinmis halidir —
mimari degismedi, sadece uygulama dili degisti.

Katmanlar (Ring donanim izolasyonuna dayanir):

- **Level-1** (Ring 3): ELF + PE uygulamalari.
- **Level-0b2**: Merkezi Denetleyici — Dispatcher, State Monitor (heartbeat),
  Load Balancer, Fallback Interface. Tum interrupt/exception/syscall
  mantiksal olarak once buraya duser.
- **Level-0b1**: Uyumluluk/ceviri katmani — POSIX/NT subsystem, ELF/PE loader.
- **Level-0a** (Ring 0): Ana cekirdek — GDT/IDT/PIC/PIT/klavye/VGA,
  ileride scheduler/VMM/VFS.

Roadmap ve teknik detaylar icin proje dokumantasyonuna bakin.

**Tamamlanan fazlar:**

| Faz | Icerik | Durum |
|-----|--------|-------|
| 1 | Boot & Level-0b2 temeli (GDT/IDT/PIC/PIT/VGA/klavye) | ✅ |
| 2 | Level-0a cekirdek temeli (kmalloc/paging/scheduler/syscall zinciri) | ✅ |
| 3 | Level-0b1 ELF32 yukleyici + Ring 3 userland (TSS/iret) | ✅ |
| 5 | POSIX dosya cagrilari + VFS/RAMFS + FD tablosu + brk | ✅ |
| 7 | **Windows NT/PE**: PE32 yukleyici + reloc + `int 0x2E` | ✅ (i386) |
| 7b | **Derlenmis PE32 GUI uygulamasi** + `win32k` cagri tablosu | ✅ (i386) |
| 7b | **Ithal tablosu (IAT)**: `KERNEL32.dll` cozumu + thunk uretimi | ✅ (i386) |
| 7c | **Ordinal ile ithal** (adsiz ihracatlar) | ✅ (i386) |
| 7d | **PE32+ (64-bit Windows)**: DIR64 reloc + Win64 thunk | ✅ (x86_64) |
| 4 | **x86_64 portu**: Long Mode, 4 seviyeli sayfalama, ELF64, `syscall` | ✅ |
| 4b | **x86_64 Rust userland**: ayni kaynak, `syscall` ABI, GUI | ✅ |
| 13 | **Framebuffer/grafik**: 1024x768x32, bitmap font, cift tampon | ✅ (i386) |
| 14 | **Pencere yoneticisi**: kompozitor, fare, surukleme, GUI syscall'lari | ✅ (i386) |
| 6 | AArch64 portu (EL1/EL0, GIC, `svc #0`) | ⏳ yapilmadi |
| 8 | **Surec basina adres uzayi** (cerceve ayirici + CR3 degisimi) | ✅ (i386 + x86_64) |
| 8 | **Preemptive zamanlama** + uyku durumu (`sleep`) | ✅ |
| 9 | **Kalici depolama**: ATA PIO, MBR, TCMKFS (yazilabilir) | ✅ (i386) |
| 9b | **TCMKFS dizinleri** (gercek agac, `mkdir`/`rmdir`) | ✅ |
| 9c | **IRQ14**: disk kesmeyle bekler (`TaskState::IoWait`) | ✅ |
| — | **Kendi onyukleyicisi** + diske kurulum (`install`) | ✅ (i386) |
| 8 | **`execve`** (surec kendi yerine program yukler) | ✅ (i386 + x86_64) |
| 8 | **`fork`** (adres uzayi kopyasi + iki kez donen cagri) | ✅ (i386 + x86_64) |
| 8 | **`waitpid`** (`Waiting` durumu, cikis kodu, `WNOHANG`) | ✅ (i386 + x86_64) |
| 8 | **`pipe`** + surec basina fd tablosu | ✅ |
| 8 | **Surec basina program break** (`brk`) + **`read(0)` = klavye** | ✅ |
| 8 | **POSIX sinyalleri** (`kill`/`signal`/`sigreturn`, isleyici cagrisi) | ✅ (i386 + x86_64) |
| 8+ | musl/busybox | ⏳ yapilmadi |

## Grafiksel Alfa

`make ARCH=i386 run` ile acilan masaustu:

![TCMK masaustu](docs/screenshot-desktop.png)

Ekranda gorunenler:

- **TCMK Paint** ve **TCMK Plasma** -- Ring 3'te calisan iki ayri **kullanici
  uygulamasi** (Rust ile yazildi, bkz. `userland-rs/`). Her biri kendi
  scheduler gorevinde kosar, kendi piksel tamponuna **dogrudan yazar**
  (cizim cekirdekten gecmez) ve `win_poll_key`/`mouse_state` ile olay okur.
  Paint fareyle gercekten cizim yapar:

  ![TCMK Paint](docs/screenshot-paint.png)
- **TCMK Shell** -- etkilesimli kabuk. Komutlar:

  | Grup | Komutlar |
  |---|---|
  | sistem | `ps` `top` `kill <id>` `signal <id> <sinyal>` `sigs` `svc` `health` `uptime` `date` `ver` |
  | Level-0b2 | `load` `ipc` `faults` `stall <sn>` |
  | bellek/disk | `mem` `disk` `df` `format onayla` `sync` `install onayla` |
  | dosya | `ls [dizin]` `mkdir <yol>` `rmdir <yol>` `cat <yol>` `save <yol> <metin>` `cp <kaynak> <hedef>` `rm <yol>` |
  | uygulama/pencere | `apps` `run <ad>` `win` `focus <id>` `mouse` |
  | uygulamalar (ELF) | `paint` `plasma` `notes` `menu` `twins` `relay` `echo2` `sigdemo` `crash` `hog` `spin` |
  | uygulamalar (PE) | `winclock` (ham `int 0x2E`) `winpad` (IAT) -- i386'da PE32, x86_64'te PE32+ |
  | diger | `echo <metin>` `pipes` `clear` `help` |
- **Sistem Gunlugu** -- cekirdek kaydinin canli goruntusu (konsol halka
  tamponu her karede pencereye cizilir).
- Ust cubukta canli gorev/pencere/tick/nabiz sayaclari, altta fare imleci.

Pencereler baslik cubugundan **suruklenebilir**, tiklama ile one alinir ve
odak degistirir. Klavye odakli pencereye yonlendirilir.

### GUI sistem cagrilari

POSIX/NT'de karsiligi olmayan islevler icin `0x500+` araligi ayrildi:

| Numara | Cagri | Islev |
|---|---|---|
| 0x500 | `win_create(baslik, (x<<16)\|y, (w<<16)\|h)` | pencere ac |
| 0x501 | `win_buffer(id)` | piksel tamponunun adresi |
| 0x502 | `win_size(id)` | (genislik<<16)\|yukseklik |
| 0x503 | `win_flush(id)` | kareyi bitir, CPU'yu birak |
| 0x504 | `win_poll_key(id)` | bekleyen tus (0 = yok) |
| 0x505 | `mouse_state()` | fare konumu + tus durumu |
| 0x506 | `yield()` | CPU'yu birak (pencere gerektirmez) |
| 0x507 | `win_pos(id)` | (x<<16)\|y -- pencere suruklendiginde tazelenir |
| 0x508 | `sleep(ms)` | sureci uyut (zamanlayici hic secmez) |
| 0x509 | `execve(yol)` | surecin yerine baska bir program yukle |

Pencere tamponu **yalnizca sahibinin** adres uzayina eslenir (bkz. "Surec
basina adres uzayi"); uygulama piksellerini kendisi yazar, cekirdek
yalnizca kompozisyon yapar.

Windows uygulamalari ayni islevlere `int 0x2E` uzerinden, `NtUser*`/
`NtGdi*` adlariyla ve 0x2000 araligindan ulasir (bkz. "win32k: pencere
cagrilari icin ayri tablo"). Iki tablo da altta ayni `gui_api`'ye baglanir,
yani PE ve ELF uygulamalari **ayni pencere yoneticisini** paylasir.

## Desteklenen mimariler

| | i386 | x86_64 |
|---|---|---|
| Boot | Multiboot1 | Multiboot2 + Long Mode gecisi |
| Sayfalama | 2 seviye, 4 KiB, 4 MiB identity | 4 seviye, 2 MiB huge + 4 KiB split, 1 GiB identity |
| Linux syscall | `int 0x80` | `syscall` komutu (MSR: EFER/STAR/LSTAR/SFMASK) |
| Windows syscall | `int 0x2E` | `int 0x2E` (ayni vektor) |
| Ikili formatlar | ELF32 + PE32 | ELF64 + PE32+ |
| Ring 3 uygulamalari | Rust (ELF32) + Rust (PE32) | Rust (ELF64) + Rust (PE32+) |
| DLL thunk gelenegi | `__stdcall` (`ret imm16`) | Win64 (golge alan) |
| Surec modeli | fork / execve / waitpid / pipe | fork / execve / waitpid / pipe |
| Kesme denetleyici | PIC 8259A (slave 0x70) | PIC 8259A (slave 0x70) |

```
make ARCH=i386   run
make ARCH=x86_64 run
```

### x86_64 userland

`make userland-x86_64` ayni `userland-rs` kaynagini ELF64 olarak derler.
Uygulama kodunun tek satiri degismez; degisen yalnizca `sys.rs`'in icidir:

| | i386 | x86_64 |
|---|---|---|
| giris | `int 0x80` | `syscall` komutu |
| numara | EAX | RAX |
| arg1..3 | EBX/ECX/EDX | RDI/RSI/RDX |

Linux numaralari da mimariye gore ayrilir (`write` i386'da 4, x86_64'te
1) -- bunu tek kumeyle gecistirmek cekirdek tarafinda daha once gercek
bir hataya yol acmisti, o yuzden userland'de de ayri tutuluyor.

![x86_64 Rust userland](docs/screenshot-x86_64-userland.png)

Ekrandaki Plasma bir **ELF64** ikilisidir, Ring 3'te kosar ve cizim
cagrilarini `syscall` ile yapar. Onceki durum yalnizca elle kodlanmis bir
"hello" idi; artik i386'daki uygulamalarin aynisi calisiyor.

### x86_64'te surec basina adres uzayi

![x86_64 iki surec](docs/screenshot-x86_64-multiproc.png)

Artik x86_64'te de her surec kendi adres uzayini alir; iki uygulama ayni
anda kosabilir:

```
tcmk> ps
  id durum      ad         cagri  adres-uzayi
   2 uyuyor     plasma       735  0x01000000 (187 sayfa)
   3 uyuyor     paint        647  0x01085000 (191 sayfa)
```

Fikir i386'nin aynisi, bir seviye daha derinde: surec kendi
PML4/PDPT/PD ucusunu alir, **cekirdek girdileri kopyalanir** (heap,
cerceve havuzu ve LAPIC her uzayda gorunur kalir) ve yalnizca kullanici
bolgesini kaplayan PD girdileri surecin kendi sayfa tablolarina
yonlendirilir.

Kullanici bolgesi neden **iki** PD girdisi tutar: bir PD girdisi 2 MiB
kapsar. Program imaji 0x00C00000'da durur, ama pencere piksel tamponlari
0x00D00000'dan baslayip dort yuvayla 0x00F00000'a uzanir -- yani ikinci
girdiye tasar. i386'da bir PDE 4 MiB kapsadigi icin bu sorun hic
cikmamisti.

Uzay **kopyalanabildigi** icin (`clone_user_space`) `fork` ve `waitpid`
de x86_64'te calisir:

```
[LEVEL-0b1] fork: gorev #2 -> #3 (eip=0x00c01adb, 128 sayfa kopyalandi)
[twins] fork dondu: 3 -- ben ebeveyn
[twins] fork dondu: 0 -- ben cocuk
[twins] ebeveyn: cocuk 42 ile bitti.
```

Cocugu Ring 3'te canlandirmak icin `sysretq` degil **`iretq`** kullanilir:
`sysretq` RCX ve R11'i kendi sozlesmesi icin ister (donus RIP'i ve
RFLAGS), oysa `fork`'ta ikisi de geri yuklenmesi gereken gercek kullanici
degerleridir.

`execve` zaten mimariden bagimsizdi (0x500 araligindaki TCMK cagrilarindan
biri, `launcher` dongusune dayanir); yalnizca `menu` uygulamasi x86_64'e
derlenmedigi icin gosterilemiyordu:

```
[launcher] '/bin/menu' sonlandi.
[launcher] execve -> '/bin/plasma'
[launcher] '/bin/plasma' Ring 3'te baslatiliyor.
```

Hata izolasyonu da ayni: `run crash` sureci page fault alir, Level-0b2
onu sonlandirir, sistem calismaya devam eder.

Ortak katmanlar (`level0a`, `level0b1`, `level0b2`) **tek bir kod tabanidir**;
mimariye ozel her sey `arch/<arch>/`, `level0a/gdt/<arch>.rs`,
`level0a/idt/<arch>.rs` ve `level0a/core/mmu_<arch>.rs` icinde izole
(doc S.15 ilke 2). Ortak kod donanima her zaman `arch::cpu` uzerinden erisir.

## Gereksinimler

Once neyin eksik oldugunu sorun -- `make check` her araci, ne ise
yaradigini ve durumunu listeler:

```
$ make check
arac                 gerekli oldugu yer                      durum
-------------------- --------------------------------------- -----
cargo/rustc          cekirdek + userland derlemesi           var
rust-src             -Z build-std (core, compiler_builtins)  YOK
grub-mkrescue        make iso / run                          var
...
```

Debian/Ubuntu icin hepsi:

```
rustup toolchain install nightly --component rust-src,llvm-tools
sudo apt install qemu-system-x86 grub-pc-bin grub-common xorriso mtools
sudo apt install llvm            # llvm-dlltool: PE ithal kutuphaneleri
```

Her hedef **yalnizca kendi ihtiyacini** denetler: cekirdegi derlemek icin
QEMU ya da GRUB kurmaniz gerekmez, eksikse yalnizca `make iso`/`make run`
uyarir.

### Hangisi ne zaman gerekir

| arac | olmadan calismayan |
|---|---|
| `cargo` + **`rust-src`** | her sey (`make`) |
| `grub-mkrescue`, `xorriso`, `mtools` | `make iso`, `make run` |
| `qemu-system-i386` / `-x86_64` | `make run`, `make run-disk` |
| `python3` | `make disk`, `make userland-legacy` |
| `llvm-dlltool` | `make userland-win`, `make userland-win64` |

**En sik takilan yer `rust-src`.** Cekirdek `-Z build-std` ile `core` ve
`compiler_builtins`'i **kaynaktan** derler; bilesen yoksa hata
"can't find crate for `core`" olur ve gercek neden hic gorunmez. Bu
yuzden `make` bunu onceden denetleyip acikca soyler.

`rust-toolchain.toml` nightly'yi otomatik secer, ama bunun icin **rustup**
gerekir; dagitimin `rustc` paketiyle (rustup'siz) derleme yapilamaz.

### Derlenmis ikililer depoda hazir gelir

`userland/*.elf`, `*.elf64` ve `*.exe` surumlenmis durumda; cekirdek
onlari `include_bytes!` ile gomer. Yani **`make userland` calistirmaniz
gerekmez** -- yalnizca uygulama kaynagini degistirdiyseniz gerekir, ve o
zaman `llvm-dlltool` istenir.

### Platform

`make` ve `make ARCH=x86_64` (cekirdek derlemesi) Rust'in calistigi her
yerde calisir. `make iso` **Linux'a ozgudur**: `grub-mkrescue` macOS ve
Windows'ta yoktur. macOS/Windows'ta ISO uretmek icin bir Linux konteyneri
ya da WSL kullanin.

Windows tarafi icin **hicbir Windows arac zinciri gerekmez**: `rust-lld`
zaten PE32/PE32+ uretebiliyor, `llvm-dlltool` da ithal kutuphanelerini
`.def`'ten olusturuyor.

## Derleme / ISO / Calistirma

```
make ARCH=i386       # cargo build (freestanding, custom i686-tcmk target)
make iso             # grub-mkrescue ile bootable ISO
make run             # ISO'dan calistir (CD-ROM)
make bootloader      # iki asamali TCMK onyukleyicisi (duz ikili)
make disk            # kalici TCMKFS bolumu + onyukleyici olan disk imaji
make run-disk        # DISKTEN acilis (CD yok) -- kalicilik boyle dogrulanir
make info            # secili ayarlari yazdirir
make check           # gereken araclari ve eksikleri listeler
```

## Faz 1 Dogrulama Listesi

QEMU'da (`make run`, veya headless `qemu-system-i386 -cdrom build/tcmk.iso
-serial stdio -display none`) su bes madde dogrulandi:

1. ✅ GRUB menusunden cekirdek yuklenir (Multiboot1).
2. ✅ Ekranda/seri portta `[LEVEL-0b2] Central Controller: Active` gorunur.
3. ✅ Klavyeden basilan tuslar ekrana/seri porta yazilir (QMP `send-key` ile
   dogrulandi).
4. ✅ Cekirdegin kendi tetikledigi `int 0x80` self-test'i sonucu
   `[LEVEL-0b2] Dispatcher: syscall vektoru 0x80 alindi ...` mesaji gorunur.
5. ✅ **Heartbeat/Fallback:** `kernel/src/level0a/idt.rs` icindeki
   `pit_handler`'da `pit::on_tick()` cagrisi gecici olarak yorum satirina
   alinip yeniden derlendiginde, ~5 saniye sonra State Monitor Level-0a'yi
   "olu" sayip Fallback Interface'i tetikliyor
   (`Level-0a YANIT VERMIYOR (heartbeat kayboldu)` mesaji).

## Faz 2 Dogrulama Listesi

`make run` ciktisinda:

1. ✅ **Sayfalama:** `paging=acik identity=4 MiB` -- 0-4 MiB identity map,
   CR0.PG acik (kernel@1M, heap@2M, VGA@0xB8000 hepsi bu araliktadir).
2. ✅ **kmalloc:** 1 MiB bump heap (0x00200000-0x002FFFFF); worker gorevinin
   16 KiB yigini buradan tahsis edilir.
3. ✅ **Scheduler:** idle + worker arasinda round-robin gecis
   (`gecis=` sayaci artar); `sys_exit` sonrasi worker `Terminated` olur ve
   bir daha secilmez.
4. ✅ **Tam syscall zinciri:** worker'in `int 0x80`'i
   Level-0b2 dispatcher -> Level-0b1 POSIX -> Level-0a `kernel_api`
   yolunu izler; `sys_write` 38 bayt doner, gecersiz fd `-9` (-EBADF) doner,
   `sys_exit` gorevi sonlandirir.
5. ✅ **Servis yonetimi:** `vmm`, `kmalloc`, `scheduler` servisleri
   `[ACTIVE]` olarak raporlanir (systemd-benzeri kayit tablosu).
6. ✅ **Co-Service / Hata Toleransi:** scheduler durursa (nabiz=0) State
   Monitor Level-0a'yi `Dead` isaretler; syscall'lar Level-0b1'e HIC
   ugramadan Fallback Interface'in `emulate_syscall`'ina duser ve
   `[co-service]` onekiyle islenmeye devam eder -- sistem ayakta kalir.

## Faz 3 / Faz 5 Dogrulama Listesi

1. ✅ **Ring 3 gercekten CPL=3:** `qemu -d int` ciktisi
   `v=80 cpl=3 IP=001b:00300068 SP=0023:...` -- CS=0x1B (user code, RPL 3),
   SS=0x23, kullanici yigininda.
2. ✅ **ELF32 yukleme:** `/bin/hello` VFS'ten okunur, PT_LOAD segmenti
   kullanici bolgesine kopyalanir, `.bss` sifirlanir.
3. ✅ **Sayfa izolasyonu:** `user@0x300000=true`, `kernel@0x100000=false`,
   `heap@0x200000=false` -- Ring 3 cekirdek belleğine erisemez.
4. ✅ **POSIX dosya zinciri:** Ring 3 programi `sys_open("/boot/msg.txt")`
   -> `sys_read` -> `sys_write(stdout)` -> `sys_close` -> `sys_brk` ->
   `sys_exit` yapar; dosya icerigi ekrana basilir.
5. ✅ **FD sizintisi yok:** `sys_exit` sonrasi acik tanimlayici sayisi 0.
6. ✅ **Guvenlik:** `sys_open`'a cekirdek isaretcisi verilirse `-EFAULT`
   (-14) doner -- kullanici alanindan gelen her isaretci
   `mmu::is_user_accessible` ile dogrulanir.

### Faz 2 manuel testi (Co-Service)

`scheduler.rs::yield_now` icindeki `pit::beat()` cagrisini yorum satirina
alip yeniden derleyin: ~5 saniye sonra Level-0a `Dead` olur ve syscall'lar
`[co-service]` onekiyle Fallback tarafindan islenmeye baslar.

## Bilinen Tuzaklar (debugging notlari)

Faz 1 gelistirilirken iki kritik hataya rastlandi; ileride x86_64/AArch64
portlarinda benzer tuzaklara dikkat:

- **`mov esp, stack_top` bir BELLEK DEREFERANSIDIR, adres yuklemesi degil.**
  LLVM'in Intel-syntax `global_asm!`'inde bare bir sembol adi `mov reg, sembol`
  seklinde kullanilirsa, sembolun ADRESI degil, o ADRESTEKI DEGER yuklenir.
  Sonuc: `esp` rastgele/ROM-shadow bolgesine (`0xFFFFFFxx`) isaret eder ve ilk
  `push`/`pop`'ta sessizce her seyi bozar (triple fault'a kadar fark
  edilmeyebilir). **Dogrusu:** `lea esp, [stack_top]`.
- **Bootloader'in IF=0 birakacagina guvenilmemeli.** Kesmeler IDT/PIC hazir
  olmadan asla kabul edilmemeli; `kernel_main`'in ilk satiri kosulsuz
  `disable_interrupts()` olmalidir (bkz. `main.rs`).
- Yukaridaki stack hatasi aktifken belirti, VGA ekraninin TAMAMEN
  `0xFE` (gecersiz karakter placeholder'i) ile dolmasiydi -- `write_str`'in
  filtre mantigi dogruydu, ama bozuk `esp` yuzunden `pushf`/`pop` ile
  kaydedilen "kesme durumu" ROM-shadow bolgesinden okunan rastgele/kod
  baytlariydi. Boyle bir belirti gorulurse once ESP/stack kurulumunu
  supheli goruntuleyin.
- **Heartbeat "gecis sayisi" degil, "dongu ilerliyor mu" olcmelidir.**
  Faz 2'de nabiz once yalnizca baglam DEGISIMINDE artiriliyordu; worker
  bitip geriye tek basina idle kalinca gecis olmadigi icin nabiz durdu ve
  Fallback tamamen saglikli bir sistemde yanlislikla devreye girdi. Dogrusu:
  `yield_now`'un basinda, erken donusten once `beat()`.
- `extern "x86-interrupt"` syscall girisi icin YETERSIZDIR: Linux ABI'si
  numara/argumanlari registerlarda tasir, o ABI ise registerlara erisim
  vermez. int 0x80 girisi `pusha` + frame isaretcisi ile elle yazilmalidir
  (bkz. `level0a/idt/i386.rs::syscall_entry`).

Faz 4 (x86_64) sirasinda cikan uc hata:

- **Linux syscall numaralari mimariye gore DEGISIR.** i386'da `write`=4,
  `exit`=1; x86_64'te `write`=1, `exit`=60. Tek kumeyle gecistirilince
  x86_64 userland `write`(=1) cagirdi, cekirdek bunu i386'nin `exit`'i
  sandi ve programi "cikis kodu 1" ile sonlandirdi -- program hicbir sey
  yazmadan "basariyla" bitmis gibi gorundu.
- **`syscall` komutu Ring 0'dan cagrilamaz.** Donusu `sysretq`'tir ve
  `sysretq` HER ZAMAN Ring 3'e doner; Ring 0'daki bir cagiran kendini
  Ring 3'te bulur. Cekirdek ici `syscall3` yardimcisi bu yuzden x86_64'te
  de `int 0x80` kullanir (bkz. `arch/x86_64/mod.rs`).
- **Boot stub'inda User biti dagitmayin.** Long Mode'a gecerken kurulan
  2 MiB'lik identity girdilerine User biti konursa cekirdegin TAMAMI
  Ring 3'e acilir. Dogrusu: bit verilmez, kullanici bolgesi sonradan
  `mmu::protect_user_range` ile ilgili 2 MiB girdisi 4 KiB'lik tabloya
  bolunerek sayfa sayfa acilir (ayni 2 MiB'i paylasan cekirdek heap'i
  boylece kapali kalir).

## Neden Rust, neden bu araclar

- Multiboot1 basligi ve `_start` giris kodu `global_asm!` ile Rust icinde
  yazildi (NASM'e gerek kalmadi).
- GDT/IDT tamamen elle yazildi (harici crate yok); `x86-interrupt` ABI
  (nightly `abi_x86_interrupt` feature) IRQ/exception/syscall handler'lari
  icin kullanildi.
- `grub-mkrescue` + `xorriso` + `qemu-system-i386` orijinal C projesindeki
  gibi aynen korunuyor.
- Tum `println!` ciktisi VGA'nin yaninda COM1 (0x3F8) seri porta da
  yansitilir, boylece `-serial stdio` ile GUI olmadan da dogrulanabilir.

## Cift Uyumluluk (projenin cekirdek vaadi)

Ayni cekirdek, ayni Ring 3 ortami, iki farkli isletim sistemi ikilisi.
`make run` ciktisindan:

```
[LEVEL-0b1] VFS'ten yukleniyor: /bin/hello
[LEVEL-0b1] format: ELF32 (Linux POSIX alt sistemi)
Hello from Ring 3 userland!
/boot/msg.txt: VFS uzerinden okundu (RAMFS).

[LEVEL-0b1] VFS'ten yukleniyor: /bin/hello.exe
[LEVEL-0b1] format: PE32 (Windows NT alt sistemi)
Hello from Ring 3 PE (Windows NT uyumluluk katmani)!
/boot/msg.txt: VFS uzerinden okundu (RAMFS).
```

Iki program da **ayni dosyayi ayni VFS uzerinden** okur; tek fark
Level-0b1'deki cevirmendir:

| | Linux ikilisi | Windows ikilisi |
|---|---|---|
| Format | ELF32 | PE32 (`.reloc` ile) |
| Kesme | `int 0x80` (vektor 128) | `int 0x2E` (vektor 46) |
| ABI | EAX=numara, EBX/ECX/EDX=arg | EAX=servis, EBX/ECX/EDX=arg |
| Cevirmen | `linux_subsystem::posix_syscalls` | `nt_subsystem::nt_syscalls` |
| Hata kodu | negatif errno (`-EBADF`) | NTSTATUS (`0xC0000008`) |
| Ortak hedef | `level0a::kernel_api` | `level0a::kernel_api` |

`qemu -d int` ile dogrulanmis Ring 3 kaniti:

```
v=80 cpl=3 IP=001b:00300068  EAX=00000004   (POSIX sys_write)
v=2e cpl=3 IP=001b:00301014  EAX=00001001   (NT NtWriteConsole)
```

PE ikilisinin `ImageBase`'i 0x00400000, cekirdek onu 0x00300000'e
yukluyor; IP'nin 0x00301014 olmasi taban yeniden yerlesiminin (`.reloc`)
gercekten uygulandigini gosterir.

### Derlenmis bir Windows uygulamasi: `winclock.exe`

Yukaridaki `hello.exe` elle kodlanmis, tek bolumlu bir PE'dir --
yukleyicinin dogrulugunu kanitlar ama "Windows uygulamasi yazilabiliyor"
demez. Bu yuzden TCMK artik **gercek bir derleyiciyle uretilmis** PE32
GUI uygulamasi da tasiyor:

```
make userland-win     # -> userland/winclock.exe   (PE32)
make userland-win64   # -> userland/winclock.exe64 (PE32+)
```

`rust-lld`'nin `msvc-lld` kipi COFF/PE uretebildigi icin bunun icin
**Windows arac zinciri gerekmez**: `targets/i686-tcmk-win.json` hedefi
`/subsystem:console`, `/base:0x00400000` ve `/fixed:no` ile linkler.
Sonuc, `file` komutunun tanidigi sahici bir ikilidir:

```
winclock.exe: PE32 executable (console) Intel 80386, for MS Windows, 4 sections
  .text  vaddr=0x1000   .rdata vaddr=0x3000
  .eh_fram vaddr=0x4000 .reloc vaddr=0x5000  (104 bayt, derleyici uretimi)
```

`.reloc` bolumunun **derleyici tarafindan** uretilmis olmasi onemli:
yukleyicinin yeniden yerlesim motoru artik elle yazilmis bir tabloyla
degil, gercek bir linker'in ciktisiyla sinaniyor.

![PE32 ve ELF32 yan yana](docs/screenshot-pe32.png)

Ekranda solda ELF32 uygulamalari (Paint, Plasma, Shell), sagda PE32
uygulamasi. Sistem gunlugunde iki yukleyici arka arkaya gorunur:

```
[LEVEL-0b1] Pe32  entry=0x00c0249b  user_stack=0x00c08000
[LEVEL-0b1] Elf32 entry=0x00c0144d  user_stack=0x00c04000
```

Windows uygulamasi burada bir uyumluluk katmaninin konugu degil,
cekirdegin **birinci sinif surecidir**: kendi adres uzayi (`ps` ciktisinda
kendi CR3'u), kendi pencere tamponu ve zamanlayicida esit hakki vardir.

### win32k: pencere cagrilari icin ayri tablo

Windows'ta sistem cagrilari tek bir tabloda degildir -- cekirdek
yurutucusu (`ntoskrnl`, `Nt*`) ile pencere/cizim cagrilari
(`win32k.sys`, `NtUser*`/`NtGdi*`) ayrilir ve **donus sozlesmeleri de
farklidir**. TCMK bu ayrimi korur:

| Aralik | Tablo | Donus | Ornekler |
|---|---|---|---|
| 0x1000+ | yurutucu | NTSTATUS | `NtCreateFile` `NtWriteFile` `NtQuerySystemTime` `NtDelayExecution` |
| 0x2000+ | win32k | tutamac/deger | `NtUserCreateWindowEx` `NtGdiGetBits` `NtUserGetMessage` |

Yani pencere acan cagri NTSTATUS degil **HWND** dondurur -- bir Windows
programcisinin bekledigi anlam degismez. Iki tablo da altta Level-0a'nin
ayni `gui_api`/`kernel_api`'sine baglanir; POSIX tarafiyla paylasilan sey
tam olarak burasidir.

`winclock.exe` diske de yazar ve bunu yalnizca NT cagrilariyla yapar
(`NtCreateFile(FILE_CREATE)` -> `NtWriteFile` -> `NtClose`). Yazdigi
dosya, makine kapatilip yeniden acildiktan sonra kabuktan okunabiliyor:

```
tcmk> ls
  disk  /clock.txt                18 bayt  2026-08-11 00:37
tcmk> cat /clock.txt
26-08-11 00:37:15
```

TCMKFS dosyanin hangi ABI'den geldigini bilmez; Level-0b1'de ayrisan iki
dunya Level-0a'da tek bir dosya sisteminde bulusur.

### Ithal tablosu: `KERNEL32.dll` (Faz 7b)

Yukaridaki iki PE de sistem cagrilarini **elle** yapiyor (`int 0x2E`).
Gercek bir Windows programi bunu asla yapmaz: `WriteConsoleA` cagirir,
cagri `KERNEL32.dll`'in **ithal tablosu** (IAT) uzerinden cozulur. Ithal
tablosunu cozmeyen bir cekirdek, derleyicinin urettigi siradan hicbir
Windows ikilisini calistiramaz -- bu yuzden Faz 7b, "Wine'in yapamadigini
yapmak" iddiasinin gercek esigidir.

**Cozum: DLL'i yuklerken var et.** Diskte `KERNEL32.dll` diye bir dosya
yok ve olmasi da gerekmiyor. Yukleyici ithal edilen her fonksiyon icin
**surecin kendi adres uzayina** kucuk bir thunk yazar ve IAT girdisini
oraya yonlendirir:

```
    mov eax, <servis numarasi>
    lea edx, [esp+4]          ; cagiranin yigin argumanlarina isaretci
    int 0x2E
    ret <bayt>                ; stdcall: yigini cagirilan temizler
```

`EDX = arguman blogu` sozlesmesi Windows'un kendi secimidir (gercek NT
stub'i `mov edx, esp; sysenter` yapar) ve onemli bir sey saglar:
**parametre sayisi sinirsizdir**. Uc registere sigdirma zorunlulugu
olsaydi `CreateFileA`'nin yedi parametresi ya da `WriteConsoleA`'nin
*cikti* parametresi desteklenemezdi. Su an cozulen adlar:

| DLL | Ihracatlar |
|---|---|
| `KERNEL32.dll` | `ExitProcess` `Sleep` `GetTickCount` `CloseHandle` `WriteConsoleA` `CreateFileA` `ReadFile` |
| `TCMKGUI.dll` | `TcmkCreateWindow` `TcmkGetWindowBits` `TcmkGetClientRect` `TcmkGetWindowRect` `TcmkUpdateWindow` `TcmkGetMessage` `TcmkGetCursorPos` |

`KERNEL32.dll` adlari ve **parametre sayilari gercek Win32 imzalarinin
aynisidir**. GUI tarafi bilerek `Tcmk` onekli: `CreateWindowExA` on iki
parametre alir ve bir pencere sinifi + `WndProc` bekler; TCMK'de pencere
sinifi yoktur. Gercek adi farkli bir imzayla ihrac etmek yaniltici
olurdu.

Ikili tarafta ortada gercek bir DLL yok; `llvm-dlltool` ile
`userland-rs/win/*.def`'ten uretilen **ithal kutuphaneleri** yalnizca
baglayiciya "bu adlar `KERNEL32.dll`'den gelecek" demenin bicimsel
yoludur. Sonuc, ikilinin icinde sahici bir ithal tablosudur:

```
DLL: KERNEL32.dll   INT=0x38ac IAT=0x38dc
    CloseHandle  CreateFileA  ReadFile  Sleep  WriteConsoleA
DLL: TCMKGUI.dll   INT=0x38c4 IAT=0x38f4
    TcmkCreateWindow  TcmkGetClientRect  TcmkGetMessage ...
```

Acilista cekirdek bunlari cozer:

```
[LEVEL-0b1] PE ithal: KERNEL32.dll -- 7 fonksiyon baglandi.
[LEVEL-0b1] PE ithal: TCMKGUI.dll -- 5 fonksiyon baglandi.
[winpad] IAT uzerinden KERNEL32.dll + TCMKGUI.dll kullaniliyor.
```

`winpad` (`userland-rs/src/win/notepad.rs`) bunun gosterimidir: **tek bir
elle yazilmis sistem cagrisi icermez**. Pencereyi `TcmkCreateWindow` acar,
tuslari `TcmkGetMessage` okur, notu `CreateFileA` + `WriteConsoleA`
yazar, acilista `ReadFile` geri okur, cikisi `ExitProcess` yapar.
Yazilan not makine kapatilip acildiktan sonra yerindedir.

![PE32 ithal tablosu](docs/screenshot-pe-imports.png)

`ps` ciktisinda yedi gorev var: iki **PE32** (`winclock` ham `int 0x2E`,
`winpad` ithal tablosu uzerinden) ve uc **ELF32** uygulama, hepsi ayni
zamanlayicida, ayni pencere yoneticisinde, her biri kendi adres uzayinda.

Cozulemeyen bir ad surecin baslatilmamasina yol acar -- Windows'un
"The procedure entry point could not be located" davranisinin karsiligi:

```
[LEVEL-0b1] PE ithal: KERNEL32.dll!HeapAlloc bulunamadi -- surec baslatilmiyor.
```

#### Ordinal ile ithal (Faz 7c)

Bazi DLL'ler fonksiyonlari **adsiz**, yalnizca sira numarasiyla ihrac
eder; o zaman ikilide ad hic gecmez ve IAT girdisinin ust biti isaretli
olur. Gomulu tablo bu yuzden ada ek olarak bir de ordinal tasir ve
numaralar `userland-rs/win/*.def` icinde **acikca** verilmistir -- iki
taraf birbirinden bagimsiz kaymasin diye.

`GetTickCount` bilerek `NONAME` olarak ihrac ediliyor, boylece her
derlemede ordinal yolu da sinaniyor:

```
KERNEL32.dll
    CloseHandle  CreateFileA  ExitProcess  [ordinal] #3  ReadFile ...
```

```
[LEVEL-0b1] PE ithal: KERNEL32.dll -- 7 fonksiyon baglandi (1 ordinal ile).
```

Ekranin alt cubugundaki calisma suresi bu cagriyla geliyor -- yani
ordinal yolu yalnizca cozulmuyor, gercekten kullaniliyor.

### PE32+ : ayni ikili, 64 bit

`make ARCH=x86_64` ile ayni iki uygulama **PE32+** olarak da derlenir ve
ayni cekirdek tarafindan yuklenir:

![winclock, PE32+](docs/screenshot-winclock64.png)

Bicim adi "PE32+"tir, "PE64" degil: dosya duzeni buyuk olcude aynidir,
degisen yalnizca isaretci genisligine bagli alanlardir.

| | PE32 | PE32+ |
|---|---|---|
| optional header magic | `0x010B` | `0x020B` |
| `BaseOfData` alani | var | **yok** |
| `ImageBase` | u32 | u64 |
| veri dizinleri nerede | `opt+96` | `opt+112` |
| yeniden yerlesim turu | `HIGHLOW` (3) | `DIR64` (10) |
| ordinal isareti | `1 << 31` | `1 << 63` |
| IAT yuvasi | 4 bayt | 8 bayt |

`BaseOfData`'nin kaybolmasiyla `ImageBase`'in 8 bayta cikmasi birbirini
goturur; dizinlere kadar olan fark tam 16 bayttir. Bu sayiyi yanlis
almak **sessiz** bir hatadir: dizin RVA'lari sifir gorunur, program
ithalsiz sanilir ve ilk `call [IAT]`'te cakar.

Taban 0x140000000 secilir (64-bit Windows gelenegi) -- kullanici
bolgesinin cok uzerinde, yani yeniden yerlesim burada **zorunludur** ve
her derlemede sinanir.

#### Win64 thunk: golge alan hilesi

Ithal edilen fonksiyonlar icin uretilen thunk'ta i386 ile gercek bir
ayrim var. Win64'te ilk dort arguman registerdadir (RCX, RDX, R8, R9);
cekirdegin "argumanlar tek blokta" sozlesmesi boylece bozulur -- ta ki
Win64'un kendi kurali kullanilana kadar:

> Cagiran, register argumanlari icin de yiginda 32 baytlik yer (**golge
> alan**) ayirmak zorundadir.

O alan tam olarak register argumanlarinin dokulecegi yerdir ve besinci
argumanin hemen oncesindedir. Dort registeri oraya dokmek butun
argumanlari kesintisiz tek bir diziye cevirir:

```asm
    mov [rsp+8],  rcx        ; arg1  \
    mov [rsp+16], rdx        ; arg2   |  golge alan -- cagiran ayirdi
    mov [rsp+24], r8         ; arg3   |
    mov [rsp+32], r9         ; arg4  /
    mov eax, <servis>
    lea rdx, [rsp+8]         ; arg5, arg6... zaten [rsp+40]'tan devam eder
    int 0x2E
    ret                      ; Win64: yigini CAGIRAN temizler
```

Bunun dogrulugu `winpad`'in kaydetme yolunda sinaniyor: `CreateFileA`
**yedi** parametre alir, yani son ucu golge alanin otesinden gelir.
Diske yazilan not geri okunabiliyorsa thunk butun listeyi dogru
tasimistir:

```
tcmk> cat /winpad.txt
pe32 plus calisiyor
```

#### Yakalanan hata: yanlis servis numarasi

Bu is sirasinda gomulu tabloda gercek bir hata cikti: `ExitProcess`,
0x3000 araligindaki `NT_EXIT_PROCESS_W32` yerine 0x1000 araligindaki
`NtTerminateProcess`'e baglanmisti. Ikisinin **cagri sozlesmesi
farklidir** -- ilki cikis kodunu thunk'in kurdugu yigin blogundan,
ikincisi ECX/RCX'ten okur. Sonuc: IAT uzerinden `ExitProcess(0)` cagiran
bir program, o an ECX'te ne varsa onunla cikiyordu (pratikte bir
isaretci: `cikis kodu 12595365`). Iki mimaride de ayni sekilde
gorulduyse de belirtisi yalnizca cikis kodundaydi, bu yuzden uzun sure
fark edilmemisti.

## Level-1: Rust userland (`userland-rs/`)

Ring 3 uygulamalari artik **Rust ile yaziliyor**. `userland-rs` ayri bir
`no_std` crate'tir ve `tcmk` adinda kucuk bir TCMK libc'si sunar
(doc S.2.3'teki "libc / win32_api" ayagi):

| Modul | Icerik |
|---|---|
| `tcmk::sys` | ham `int 0x80` sarmalayicilari + syscall numaralari |
| `tcmk::io` | `print!`/`println!`, `File` (RAII: `Drop` ile `close`) |
| `tcmk::gui` | `Window` (piksel tamponu, `fill`/`disc`/`put`), `mouse()` |
| `tcmk::entry!` | `_start` uretir, `main` dondugunde `exit(0)` cagirir |

Bir uygulama artik siradan Rust'tir:

```rust
#![no_std]
#![no_main]
tcmk::entry!(main);

fn main() {
    let mut win = tcmk::gui::Window::open("Ornek", 30, 60, 320, 200).unwrap();
    loop {
        win.clear(0x0010_2030);
        if win.poll_key() == b'q' { break; }
        win.flush();
    }
}
```

**Baglama.** Taban adres `cargo rustc ... -- -C link-arg=--image-base=`
ile **yalnizca ikili hedefe** verilir; kutuphane ve `core` bir kez
derlenir.

**Slot modeli kaldirildi.** Surec basina adres uzayindan sonra tum
uygulamalar ayni tabana (`0x00C00000`) linkleniyor:

```
cargo rustc --release --bin paint -- -C link-arg=--image-base=0x00C00000
```

`rust-lld` `ET_EXEC`/`EM_386` uretir; Level-0b1'in ELF32 yukleyicisi bu
ikilileri hicbir degisiklik olmadan yukler.

### Tek kutuphane, iki ABI

`tcmk` kutuphanesi hem ELF hem PE uygulamalarina hizmet eder; hangi
tarafta oldugunu **hedef** belirler (`lib.rs` icinde `cfg(target_os)`):

| | ELF (`i686-tcmk`) | PE (`i686-tcmk-win`) |
|---|---|---|
| sistem cagrisi | `int 0x80` -- `sys` | `int 0x2E` -- `nt`, ya da **IAT** -- `winapi` |
| pencere | `gui::Window` | `win32::Hwnd` / `winapi::Window` |
| konsol | `io::Stdout` | `win32::Console` / `winapi::Console` |
| cikis | `sys_exit` | `NtTerminateProcess` / `ExitProcess` |
| **cizim** | `canvas::Canvas` | `canvas::Canvas` |

PE tarafinda iki secenek var: `win32` sistem cagrilarini elle yapar,
`winapi` ise onlari `KERNEL32.dll`/`TCMKGUI.dll`'den **ithal eder**.
Ikincisi gercek bir Windows programinin yaptigidir (bkz. "Ithal
tablosu").

Son satir kasitli: cizim kodu **ortaktir**. `Window` da `Hwnd` de ayni
`Canvas`'a `Deref` eder, yani `win.text(...)`, `win.fill(...)`,
`win.glyph_scaled(...)` iki tarafta ayni fonksiyonlardir. Bir uygulamayi
Windows ikilisi olarak derlemek cizim kodunun tek satirini degistirmez --
ayrisma yalnizca pencere acan ve olay okuyan cagrilarda olur, ki bu da
zaten Level-0b1'in var olma sebebidir.

Kaynak duzeni bu ayrimi yansitir: `src/bin/` ELF uygulamalari (kendiliginden
bulunur), `src/win/` PE uygulamalari (Cargo.toml'da bildirilir).

### Userland ikililerini yeniden uretme

```
make userland          # hepsi
make userland-rust     # ELF32 uygulamalari (src/bin/)
make userland-x86_64   # ayni kaynak, ELF64
make userland-win      # PE32 uygulamalari (src/win/)
make userland-win64    # ayni kaynak, PE32+
make userland-legacy   # elle uretilen en kucuk PE32/ELF64
python3 tools/gen_font.py   # 8x16 bitmap fontu yeniden uret
```

## Level-0b2: Merkezi Denetleyici (ayirt edici ozellikler)

TCMK'yi siradan bir cekirdekten ayiran sey, **her cagrinin tek bir
denetim noktasindan gecmesidir** (doc S.2.2.A "Trafik Polisi"). Bu tek
nokta uc gercek islev sagliyor:

### 1. Yuk Dengeleyici -- olcum + geri baski

Her sistem cagrisi bir **kanala** ve bir **goreve** yazilir; pencere
saniyede bir devrilir ve hizlar hesaplanir.

![Yuk dengeleyici](docs/screenshot-load-balancer.png)

```
tcmk> load
  kanal          toplam    /sn     tepe  durum
  posix-dosya        33      0       33  normal
  posix-bellek        3      0        3  normal
  gui             61273   4486     4498  normal
  nt                  6      0        6  normal
  istisna             0      0        0  normal
toplam cagri: 61315  kisitlama: 897
```

**Neden hala gerekli:** preemption CPU'yu adil bolusturur ama **cagri
hacmini** sinirlamaz. Saniyede on binlerce syscall yapan bir uygulama,
zaman dilimini asmasa bile VFS'i, kompozitoru ve disk yolunu bogar.
Yuk Dengeleyici darbogazi kanal bazinda gorunur kilar ve kotayi asan
gorevi geri baskiya alir.

Bir gorev pencere kotasini (`4000 cagri/sn`) asarsa Dispatcher cagriyi
**islemeden once** o goreve `yield` ettirir. Cagri kaybolmaz, sirasini
bekler.

Kanit: `run hog` -- kasten acgozlu bir test uygulamasi
(`userland-rs/src/bin/hog.rs`). Ekran goruntusunde hog 4486 cagri/sn ile
kosarken kabuk yanit veriyor, Plasma animasyonu suruyor:

```
tcmk> top
  id ad            cagri/sn   kisitlama
   2 paint              243           0
   3 plasma             162           0
   4 hog               4081        1121
```

Normal uygulamalar kotanin cok altinda kalir; yalnizca gercekten donen
bir dongu kisitlanir.

### 2. IPC -- mesaj kuyrugu + paylasimli bolge

Doc S.10 Faz 4+: "Mesaj kuyrugu (ring buffer) ile Level-0b2 <-> Level-0a
iletisimi; heartbeat sayaci paylasimli bellek bolgesinde tutulur."

![IPC](docs/screenshot-ipc.png)

Nabzin paylasimli bolgeden okunmasi mimari bir zorunluluk: State Monitor
Level-0a'nin bir **fonksiyonunu cagirsaydi**, Level-0a kilitlendiginde
izleyici de kilitlenirdi. Izleyicinin izlenene bagimli olmamasi hata
tolerans motorunun onkosuludur.

Olaylar (istisna, kisitlama, saglik degisimi, uygulama yasam dongusu)
uretildikleri yerde -- bazen kesme baglaminda -- kuyruga birakilir ve
masaustu dongusunde, kesmeler acikken tuketilir. Uretici asla beklemez:
kuyruk doluysa mesaj dusurulur ve sayilir.

```
tcmk> ipc
paylasimli bolge: 0x001fb100  (gecerli)
kuyruk: 0/31   tepe: 5
gonderilen: 9  tuketilen: 9  dusen: 0
```

### 3. Co-Service ve toparlanma

Nabiz 300 tick (3 sn) sessiz kalirsa Level-0a "Olu" sayilir ve Fallback
Interface devralir. Minimal kume bilincli olarak kucuktur; amac tam bir
isletim ortami degil, **sistemi uyanik ve etkilesime acik tutmaktir**:
konsola yazma, CPU birakma, kareyi bitirme, olay sorgularina "olay yok"
cevabi.

![Co-Service](docs/screenshot-co-service.png)

Test etmek icin (doc S.12: "Heartbeat timeout simulasyonunda fallback
devreye girer"):

```
tcmk> stall 6        # nabzi 6 saniye bastir
tcmk> health
Level-0a durumu: OLU
nabiz sessizligi: 565 tick (olu esigi 300)
co-service: DEVREDE (Level-0a olu)
olu olayi: 1  toparlanma: 0
```

Ekran goruntusunde goruldugu gibi masaustu donmaz, kabuk yanit verir,
Plasma cizmeye devam eder -- **Level-0a olu sayilirken bile**.

Nabiz geri geldiginde denetim ana katmana devredilir (doc S.11
"Level-0a yeniden baslatma"): TCMK'de Level-0a ayri bir adres uzayinda
olmadigi icin bu, servislerin yeniden yuklenmesi degil **denetimin geri
verilmesidir** -- Co-Service modundan cikilir, cagrilar yeniden
Level-0b1 cevirmenlerine yonlendirilir.

```
[LEVEL-0b2][FALLBACK] Level-0a nabzi geri geldi -- denetim ana katmana devrediliyor.
[LEVEL-0b2][FALLBACK] Co-Service modu kapandi.
[IPC] saglik: Level-0a toparlandi -- normal moda donuldu
```

## Kalici depolama: disk + TCMKFS

TCMK artik **kendi diskinden acilir ve verisini kalici olarak saklar**.

![TCMKFS](docs/screenshot-tcmkfs.png)

```
tcmk> ls
   ram   /bin/paint               3316 bayt
   ram   /bin/plasma              5968 bayt
   disk  /home/notlar.txt           21 bayt
   disk  /home/plasma             5968 bayt
tcmk> cat /home/notlar.txt
merhaba kalici dunya
tcmk> df
tcmkfs: 12 / 65516 KiB kullanimda
```

Yukaridaki ekran goruntusu **ikinci acilistandir**: QEMU tamamen
kapatilip yeniden baslatildi, dosyalar yerinde.

### Katman katman

| Katman | Dosya | Islev |
|---|---|---|
| Surucu | `drivers/ata.rs` | ATA PIO (LBA28), IDENTIFY, oku/yaz/flush |
| Blok aygiti | `drivers/block.rs` | "LBA'dan sektor oku/yaz" soyutlamasi |
| Bolum | `drivers/partition.rs` | MBR ayristirma, TCMKFS bolumunu bulma |
| Dosya sistemi | `core/tcmkfs.rs` | superblock + inode + blok bitmap, yazma |
| Isim uzayi | `core/vfs.rs` | RAMFS ve TCMKFS'i tek isim uzayinda birlestirir |
| Saat | `drivers/rtc.rs` | CMOS RTC -> zaman damgalari icin Unix zamani |

**ATA PIO neden:** port I/O disinda hicbir sey gerektirmez -- PCI taramasi,
DMA, kesme yonetimi yok. Hem QEMU'da hem gercek (eski) donanimda calisir.
Blok katmani soyut oldugu icin ileride AHCI/virtio surucusu ayni arayuzun
altina takilabilir; dosya sistemi degismez.

**Kendi dosya sistemi neden:** doc S.7 Faz 9 ext2 diyordu. ext2'nin
okunmasi kolaydir ama **yazilmasi** degildir (blok gruplari, dolayli blok
agaclari, dizin karma indeksleri). Bu asamadaki ihtiyac "Linux diskini
okumak" degil "kendi verisini saklamak" oldugu icin, ilk gunden yazma
destegi olan kucuk bir dosya sistemi secildi. ext2 okuyucusu ileride VFS'e
**ikinci bir arka uc** olarak eklenebilir -- katman tam bunun icin var.

### TCMKFS yerlesimi (bolum baslangicina gore, sektor)

```
   0        superblock ("TCMK" imzasi, kapasite, etiket)
   1..32    inode tablosu   (64 inode x 256 bayt)
  33..36    blok bitmap'i   (16384 bit)
  40..      veri bloklari   (blok = 4096 bayt = 8 sektor)
```

Dosya basina 40 **dogrudan** blok isaretcisi -> azami 160 KiB. Inode
tablosu ve bitmap bellekte onbelleklenir, degisiklikler aninda diske
yazilir (write-through), her yazmadan sonra ATA cache-flush verilir.

### IRQ14: disk artik kesmeyle bekliyor

Surucu bugune kadar **yoklamali**ydi: bir sektor icin durum registeri
milyonlarca kez okunuyordu ve o sure boyunca CPU baska hicbir is
yapmiyordu.

Bunun sebebi tembellik degil, bir **vektor catismasiydi**. Alisilmis PIC
yerlesiminde (master 0x20, slave 0x28) IRQ14 vektor 46'ya duser -- ve 46,
TCMK'de `int 0x2E`nin, yani **Windows NT sistem cagrisinin** yeridir. Ayni
IDT girdisini paylasamazlar.

Ilk akla gelen "NT vektorunu tasi"dir, ama 0x2E bir tercih degil **ABI**:
Windows ikilileri onu bekler. Tasinabilir olan taraf PIC'ti, bu yuzden
slave denetleyici 0x70'e alindi:

| hat | eski vektor | yeni vektor |
|---|---|---|
| IRQ0 (PIT) | 32 | 32 |
| IRQ1 (klavye) | 33 | 33 |
| IRQ12 (fare) | 44 | **116** |
| IRQ14 (ATA) | 46 (catisiyordu) | **118** |
| `int 0x2E` (NT) | 46 | 46 (artik yalniz) |

Bekleme yolu icin zamanlayiciya ucuncu bir engelleme durumu eklendi:
`TaskState::IoWait`. `Blocked`'tan farki uyanma kaynagidir -- zaman degil,
donanim. Ayri olmasi sart: yoksa "5 tik sonra uyan" ile "disk hazir olunca
uyan" ayni yuvayi paylasir ve biri otekini yanlislikla uyandirir.

Iki incelik:

* **Kacirilmis uyandirma.** Kesme, gorev `IoWait`'e gecmeden hemen once
  gelebilir. `wait_for_io` bu yuzden uyumadan once kosulu kesmeler kapaliyken
  bir kez daha denetler; aksi halde surec sonsuza kadar beklerdi.
* **Kesme hic gelmezse.** Sure dolunca yoklama yoluna dusulur. Yani kesme
  bir **hizlandirici**dir, tek dogruluk kaynagi degil; arizali bir aygit
  sistemi kilitlemez.

Iki yer bilerek yoklamada kaldi: acilis (zamanlayici henuz yok) ve **idle
gorevi** -- ki o gorev ayni zamanda masaustu dongusudur, uyutmak ekrani
dondururdu. Kabuktan verilen disk komutlari bu yola duser.

```
tcmk> df
tcmkfs: 4 / 63488 KiB kullanimda
bos blok: 15871 / 15872  (blok 4096 bayt)
dosya: 2 / 64  (isim uzayinda 17)  blok yazimi: 1  IRQ14: 202  io beklemesi: 1
```

Son iki sayi olcumun kendisi: `IRQ14` gelen kesme sayisi, `io beklemesi`
ise bir gorevin gercekten uyudugu kez sayisi. Ikincisi yalnizca Ring 3
surecleri icin artar (yukaridaki 1, `notes`in diske yazdigi an).

### Dizinler (bicim surumu 2)

TCMKFS onceden **duz** bir isim uzayiydi: `/home/x` bir yol degil,
dosyanin adiydi. Artik gercek bir agac var.

Yontem, dizin icerigini ayri bir blokta tutmak **degil**: her inode
iceren dizininin numarasini (`parent`) tasir, yani agac **cocuktan
ebeveyne** dogru saklanir.

```
inode 0  dizin  ""         parent=0   (kok)
inode 3  dizin  "notlar"   parent=0
inode 5  dizin  "2026"     parent=3
inode 7  dosya  "ocak.txt" parent=5   ->  /notlar/2026/ocak.txt
```

Kazanci: bir dizinin icerigi sabit boyutlu bir listeye sigmak zorunda
kalmaz -- "X'in cocuklari" sorusu inode tablosunun taranmasiyla
cevaplanir ve 64 inode'da bu, bir dizin blogu okumaktan ucuzdur.
Bedeli: tam yolu uretmek icin ebeveyn zinciri **yukari** yurunur
(`path_of`), ki VFS'in mount aninda yaptigi da budur.

```
tcmk> mkdir /notlar
tcmk> mkdir /notlar/2026
tcmk> mkdir /yok/olan
yol gecersiz (ara dizin yok ya da dizin degil)
tcmk> save /notlar/2026/ocak.txt merhaba dizin
14 bayt yazildi: /notlar/2026/ocak.txt

tcmk> ls /notlar
  dizin /notlar/2026
  disk  /notlar/2026/ocak.txt    14 bayt  2026-08-12 09:18
  disk  /notlar/kok.txt          12 bayt  2026-08-12 09:18
 toplam 2 dosya, 1 dizin
```

`rmdir` bos olmayan dizini reddeder; `rm` dizinleri hic gormez. Ara
dizinler **kendiliginden yaratilmaz** -- POSIX `open(O_CREAT)` de boyle
davranir.

`format` yeni bir diskte `/home` ve `/tmp` dizinlerini de yaratir. Duz
isim uzayi doneminde `/home/notes.txt` gecerli bir **addi**; artik gecerli
bir **yol** olmasi icin `/home`in var olmasi gerekiyor. `mkfs`in
`lost+found` yaratmasiyla ayni turden bir kolaylik -- ve mevcut
uygulamalarin (`notes`) calismaya devam etmesini sagliyor.

Bicim degistigi icin superblock surumu 2'ye cikti ve **surum 1 imajlari
baglanmaz**: eski inode'larda `parent` alani yoktur ve `name` tam yolu
tasir, oldugu gibi baglamak agaci rastgele kurardi. `mount` bunu sessizce
denemek yerine acikca reddeder (`format onayla` gerekir).

VFS tarafinda dizin kavrami **yoktur**: agac diskte yasar, VFS mount
aninda her dosyanin tam yolunu `path_of` ile duzlestirip isim uzayina
koyar. Boylece `/bin/paint` gibi RAMFS yollari ile disk yollari ayni
tabloda, ayni bicimde durur.

### Duvar saati (CMOS RTC)

PIT sistemin **ne kadar suredir** calistigini olcer; hangi gunde
oldugumuzu bilmez. Bir dosyanin ne zaman yazildigini kaydetmek icin gercek
tarih gerekir, bu yuzden `drivers/rtc.rs` CMOS saatini okur ve inode'un
`mtime` alani **Unix zamani** olarak doldurulur.

RTC saniyede bir kendini gunceller ve bu guncelleme atomik degildir --
tam o anda okunan alanlar farkli anlara ait olabilir (dakika yeni, saniye
eski). Iki savunma birlikte kullanilir: Status A'nin "guncelleme suruyor"
biti beklenir, **ve** okuma iki kez ust uste ayni sonucu verene kadar
tekrarlanir. Degerler BIOS'a gore BCD veya ikili, saat 12 ya da 24
saatlik olabilir; Status B bunu soyler ve donusum surucude yapilir.

```
tcmk> date
2026-08-11 00:16:08  (UTC, CMOS RTC)
unix zamani: 1786407368

tcmk> ls
  disk  /saat.txt        14 bayt  2026-08-11 00:16
```

`uptime` ayni saatten ikinci bir olcum verir:

```
tcmk> uptime
tick: 2416  (~24 saniye)
duvar saati: 0:00:28  (RTC)
acilis: 2026-08-11 00:15:43 (UTC)
```

Ikisinin farki bilerek gosteriliyor: PIT tick'i **kacirilan kesmeleri**
sayamaz. Aradaki dort saniye, cekirdegin acilista kesmeler kapaliyken
(ATA PIO bekleyisleri, bicimlendirme) gecirdigi suredir -- yani bu iki
satir birlikte ucuz bir saglik gostergesidir.

### Disk imaji nasil uretiliyor (root gerekmeden)

`grub-mkrescue` **hibrit** bir imaj uretir: ayni dosya hem CD hem sabit
disk olarak acilabilir ve gecerli bir MBR tasir. `tools/make_disk.py` bu
imajin sonuna bos bir bolge ekleyip MBR bolum tablosuna ikinci girdiyi
yazar:

```
bolum 1  0xCD  ISO hibrit  -> GRUB + cekirdek (salt okunur)
bolum 2  0x7F  TCMKFS      -> kalici, yazilabilir veri
```

Cekirdek yazilabilir bolumu **sabit bir LBA'ya gomerek degil, bolum
tablosundan bularak** acar; imajin duzeni degistiginde cekirdegi yeniden
derlemek gerekmez.

### Uygulama "kurmak"

`cp` diskteki bir kopyayi olusturur, `run` onu oradan calistirir:

```
tcmk> cp /bin/plasma /home/plasma
5968 bayt kopyalandi -> /home/plasma
tcmk> run /home/plasma
```

Launcher artik sabit bir uygulama listesine bagli degil: VFS'te var olan
her yol calistirilabilir. Yani diske kopyalanan uygulamalar cekirdegi
yeniden derlemeden calisir.

## `execve`: surec kendi yerine baska bir program yukluyor

![execve](docs/screenshot-execve.png)

`run menu` bir baslatici acar; bir numaraya basinca secilen uygulama
**menunun yerine** yuklenir. Yeni gorev acilmaz -- ayni gorev numarasi,
yeni bir adres uzayi:

```
[launcher] '/bin/menu' sonlandi.
[launcher] execve -> '/bin/notes'
[launcher] '/bin/notes' Ring 3'te baslatiliyor.
```

### Neden "cik ve yerine sunu yukle"

Imaj **yerinde** degistirilemez: surec o anda kendi kodunun icinde
kosuyor, altindaki sayfalari degistirmek calisan komut akisini yok
ederdi. Bu yuzden `execve` istegi cekirdekte gorev basina bir yuvaya
yazilir, surec Ring 3'ten cikar (`sys_exit` ile ayni yol) ve
`launcher`'in dongusu yeni imaji yukler:

```rust
while let Some(path) = next {
    run_from_vfs_dynamic(path);
    next = take_exec(current_id());   // execve istendi mi?
}
```

Adres uzayi bu arada dogal olarak birakilip sifirdan kuruluyor -- zaten
execve'nin istedigi sey. Surec basina adres uzayi olmadan bu temiz
olmazdi: eski imajin kalintilari yeni programin altinda kalirdi.

## `fork`: tek cagri, iki donus

![fork](docs/screenshot-fork.png)

`run twins` **tek** bir program baslatir. Program `fork()` cagirir ve o
noktadan sonra iki surec olur; ekrandaki iki pencere ayni ikilinin iki
ayri kopyasidir.

```
[LEVEL-0b1] fork: gorev #4 -> #5 (eip=0x00c018ba, 128 sayfa kopyalandi)
[twins] fork dondu: 5 -- ben ebeveyn
[twins] fork dondu: 0 -- ben cocuk
```

### Neden diger cagrilardan farkli

Butun syscall'lar "bir sey yap, don" kalibindadir. `fork` **iki kez
doner**: ebeveynde cocugun kimligiyle, cocukta sifirla. Bunun icin cagri
anindaki Ring 3 baglaminin eksiksiz yakalanip ikinci bir gorevde
canlandirilmasi gerekir.

Baglam `pusha` blogunun hemen ustunde durur: CPU kesme girisinde EIP, CS,
EFLAGS ve -- ayricalik degistigi icin -- kullanicinin ESP/SS'ini iter.
`SyscallFrame::user_context` bu duzeni okur. Cocuk icin `eax` sifirlanir;
"hangisiyim?" sorusunun cevabi budur.

**Sadece EIP/ESP yetmez.** Derleyici `int 0x80` sonrasinda
EBX/ESI/EDI/EBP'nin korundugunu varsayar, dolayisiyla cocuk bunlari
ebeveyninkiyle ayni gormezse syscall'dan sonraki ilk erisimde coker. Bu
yuzden Ring 3'e donus icin **butun** genel registerlari yukleyen ayri bir
giris yazildi (`arch_enter_user_mode_regs`).

### Bellek gercekten kopyalaniyor mu?

Ekrandaki iki sayac bunun kanitidir. Ikisi de `fork`'tan **once** 1000'e
kuruldu; sonra ebeveyn 1'er, cocuk 7'ser artiriyor. Goruntudeki degerler
1087 ve 1609: ikisi de 87 kare cizmis, ama **kendi** kopyalarini
artirmis. Bellek paylasilsaydi tek bir sayac gorunurdu.

Kopyalama sanal adresler uzerinden yapilamaz -- kaynak ve hedef ayni
sanal adreste (0x00C00000) ama farkli CR3'lerde durur, yani ikisi ayni
anda gorunmez. Bunun yerine **fiziksel** adresler kullanilir: cerceve
havuzu cekirdegin identity haritasinin icinde oldugu icin her iki
cerceveye de dogrudan yazilabilir, CR3 degistirmeye ya da gecici pencere
eslemeye gerek kalmaz.

Cocuk kendi cekirdek yiginini `scheduler::spawn`'dan alir; ebeveyninkini
paylasmasi, ikisi ayni anda syscall yaptiginda birbirinin cercevesini
ezmek demek olurdu.

### `waitpid`: cocugu toplamak

![waitpid](docs/screenshot-waitpid.png)

`twins`'te cocuk 60 kare sonra `42` ile cikar. Ebeveyn bunu toplar ve
cikis kodunu ekranda gosterir; goruntude cocugun penceresi kapanmis,
ebeveyn `cocuk bitti, kod:42` yaziyor.

```
[twins] cocuk 42 ile cikiyor.
[LEVEL-0a] Ring 3 sureci cikis kodu 42 ile sonlandi.
[twins] ebeveyn: cocuk 42 ile bitti.
```

Zamanlayiciya bunun icin bir `Waiting` durumu eklendi. Bekleyen gorev
`pick_next` tarafindan **atlanir** -- yani bekleyen bir surec CPU
harcamaz. `Blocked`'dan farki uyanma kosuludur: `Blocked` bir PIT
tick'ini bekler, `Waiting` **baska bir gorevin durumunu**. Ikisi de ayni
yerde, `wake_expired` icinde cozulur.

Ebeveyn dongusunde `WNOHANG` kullanir; boylece cocugu her karede yoklar
ama bloke olmaz ve penceresi akici kalir. Kullanici erken cikarsa
ebeveyn bu kez bloke olarak bekler -- iki yol da `twins` icinde.

Durum kelimesi Linux'un kodlamasini kullanir (`(kod & 0xFF) << 8`), yani
`WEXITSTATUS` beklendigi gibi calisir.

## `pipe`: ayri adres uzaylarindaki iki surec konusuyor

![pipe](docs/screenshot-pipe.png)

`twins` iki surecin **ayri** oldugunu gosteriyordu: bellek kopyalanir,
sayaclar ayrisir. `run relay` tam tersini gosterir -- ayri adres
uzaylarindaki iki surec nasil konusur.

Kalip UNIX'in klasigidir: **once boru, sonra catallanma**.

```text
  pipe()  ->  (okuma_fd, yazma_fd)
  fork()
    cocuk  : okuma ucunu kapat, olctugu degerleri yazma ucuna yaz
    ebeveyn: yazma ucunu kapat, okuma ucundan okuyup ekrana ciz
```

```
[relay] boru: okuma fd=3 yazma fd=4
[LEVEL-0b1] fork: gorev #4 -> #5 (128 sayfa kopyalandi)
[relay] cocuk 40 olcum gonderiyor.
[relay] cocuk bitirdi, yazma ucu kapandi.
[relay] ebeveyn 40 olcum aldi, cocuk 0 ile bitti.
```

Ekrandaki cubuk grafik cocugun urettigi 40 degerin aynisidir; toplam
1268, `(i * 37 + 11) % 64` dizisinin toplamiyla birebir tutuyor. Yani
veri yalnizca akmiyor, **bozulmadan** akiyor.

Tampon cekirdektedir, `fork`'ta kopyalanmaz -- iki surec de ayni halka
tamponu gorur. Paylasilan bir degisken olsaydi zaten kopyalanirdi ve is
gormezdi; `twins`'teki sayaclarin ayrismasi bunun kanitiydi.

### Surec basina fd tablosu (borularin zorunlu kildigi degisiklik)

Tanimlayici tablosu uzun sure **globaldi**; tek kullanici sureci varken
sorun degildi. Borular bunu surdurulemez hale getirdi: yukaridaki kalipta
her taraf kullanmadigi ucu kapatir, ve paylasilan bir tabloda cocugun
kapattigi tanimlayici ebeveyninkini de yok ediyordu. Ilk denemede
ebeveyn tam olarak bu yuzden **sifir bayt** aldi.

POSIX semantigi zaten dogru olani soyluyor: `fork` tanimlayicilari
*kopyalar*. Tablo surec basina tasindi, `fork` ebeveynin tablosunu
cocuga klonluyor ve boru uclarinin sayaclarini artiriyor. Bir uc ancak
**iki taraf da** kapatinca oluyor; gorev sonlandiginda tanimlayicilari
otomatik birakiliyor, yoksa kapanmayan bir yazma ucu okuyan tarafta
"dosya sonu"nun hic gorunmemesi demek olurdu.

Kabuktan izlenebilir:

```
tcmk> pipes
acik boru: 1 / 4  (tampon 1024 bayt)
  #0  bekleyen:    0  yazan:1  okuyan:1
```

### Bloke etmeyen okuma

Gercek POSIX'te bos bir borudan okumak veri gelene kadar bloke olur.
TCMK'de okuma bloke etmez; veri yoksa `0` doner. Nedeni GUI'dir: bloke
olan bir surec penceresini de dondurur, oysa buradaki uygulamalar kendi
cizim dongulerini surer. `relay`'in ebeveyni her karede yoklar ve grafik
akici kalir.

## Sinyaller: cekirdek uygulamanin akisini kesip isleyicisini cagiriyor

Butun diger sistem cagrilarinda **kullanici cagirir, cekirdek doner**.
Sinyalde bu ters doner: cekirdek cagirir, kullanici doner. Bir surece
sinyal geldiginde cekirdek onun Ring 3 baglamini kenara koyar, yigininin
ustune sahte bir cagri cercevesi kurar ve donusu isleyicinin adresine
cevirir. Surec hicbir sey cagirmadigi halde kendini isleyicinin icinde
bulur.

```
tcmk> run sigdemo
tcmk> signal 4 10        # SIGUSR1
tcmk> signal 4 12        # SIGUSR2
tcmk> signal 4 15        # SIGTERM -- isleyici kuruldugu icin uygulama YASAR
tcmk> kill 4             # SIGKILL -- yakalanamaz, aninda sonlanir
```

![sigdemo](docs/screenshot-sigdemo.png)

Ekrandaki "kare" sayaci sinyalin nerede yakalandigini gosterir: sayac
kesintisiz artmaya devam eder, cunku isleyici dondugunde surec **tam
kaldigi komuttan** devam eder. "son sinyal kare" alani ise isleyicinin
cizim dongusunun ortasinda calistigini kanitlar.

### Isleyiciden nasil geri donuluyor

Isleyici siradan bir fonksiyondur; bitince `ret` yapar. Ama donulecek bir
"cagiran" yoktur -- cagri gercek bir cagri degildi. Cozum, cekirdegin
yigina bir donus adresi koymasi: kullanici kutuphanesindeki kucuk bir
**tramplen**. Tek isi `sigreturn` cagirmaktir; cekirdek de saklanan
baglami geri koyar.

```asm
__tcmk_sigreturn:            ; i386
    mov eax, 119             ; sys_sigreturn
    int 0x80
```

Tramplenin kullanici tarafinda olmasi bilincli: aksi halde cekirdegin
surecin adres uzayina **kod yazmasi** gerekirdi. Gercek i386 Linux'ta da
cozum aynidir (`sigaction.sa_restorer`).

Cerceve duzeni mimariye gore degisir ve iki incelik tasir:

| | i386 | x86_64 |
|---|---|---|
| sinyal no | `[esp+4]` (cdecl) | `RDI` (SysV) |
| donus adresi | `[esp]` | `[rsp]` |
| hizalama | `esp % 16 == 12` | `rsp % 16 == 8` |
| kirmizi bolge | yok | `rsp`'nin 128 bayt alti **atlanir** |

Kirmizi bolge atlanmazsa, cerceve o anda calisan yaprak fonksiyonun
yerel degiskenlerinin ustune kurulur ve surec sinyalden sonra bozuk
verilerle devam eder -- teshisi cok zor bir hata. (Linux de ayni 128 bayti
atlar.)

### Teslim ne zaman olur

Sinyal aninda calismaz; bekleyenler maskesine yazilir ve surec bir
sonraki sefer **cekirdekten Ring 3'e donerken** teslim edilir -- yani
teslim noktasi bir syscall donusudur (`level0b2::dispatcher`). TCMK
uygulamalari her karede `win_flush` cagirdigi icin gecikme bir kareden
kucuktur.

`SIGKILL` bu kuralin disindadir: beklemeye alinmaz, gonderen taraf hedefi
dogrudan sonlandirir. Beklemeye alinsaydi hicbir syscall yapmayan bir
surec (`spin` gibi) oldurulemezdi -- yani "her seyi durdurabilen komut"
olma ozelligi kaybolurdu.

### `fork` ve `execve` ile iliskisi

* **`fork`**: isleyiciler cocuga kopyalanir (POSIX), bekleyen sinyaller
  kopyalanmaz -- cocuk temiz baslar.
* **`execve`**: butun isleyiciler sifirlanir. Kayitli adresler artik var
  olmayan bir programa aittir; devralinsalardi surec kendi kodunun
  ortasina dallanirdi.

Kabuk tarafinda `sigs` komutu hangi gorevin hangi sinyalleri yakaladigini
ve bekleyenleri listeler.

## Standart girdi: `read(0, ...)` gercekten klavye

Bu ana kadar TCMK uygulamalari tuslari **TCMK'ye ozgu** bir cagriyla
(`win_poll_key`, 0x502) aliyordu. Artik ayni tuslar POSIX'in kendi
yolundan, `read(0, ...)` ile de okunabiliyor.

Baglanti sudur: her pencerenin bir **sahibi** (gorev kimligi) vardir;
`read(FD_STDIN)` cagiran gorevin ilk penceresini bulur
(`wm::first_window_of`) ve o pencerenin tus kuyrugunu bosaltir. Yani
"standart girdi" = odakli pencereye giden tuslar. Once bu cagri kosulsuz
`Ok(0)` donuyordu ("stdin henuz bir cihaza bagli degil"); simdi bagli.

`echo2` bunu gosterir: pencere acmak disinda **hicbir** TCMK cagrisi
kullanmaz, girdiyi `read(0, ...)`, ciktiyi `write(1, ...)` ile yapar --
yani yazilan metin hem pencerede hem seri konsolda gorunur.

```
tcmk> run echo2
tcmk> focus 4          # odagi kabuktan uygulamaya ver
merhaba stdin          # -> pencerede goruntulenir
                       # -> seri gunlukte de "merhaba stdin" cikar
```

![echo2 -- read(0) ile klavye](docs/screenshot-echo2.png)

Sag alttaki sayac gerceklesen `read` cagrisi sayisidir: 13 karakter, 13
cagri -- kuyruk her karede bosaltildigi icin cagri basina bir tus dusuyor.

Ikisi de aynen x86_64'te de calisir; `sys.rs` cagri numarasini ve
mekanizmasini (`int 0x80` / `syscall`) `cfg` ile ayirir, geri kalan kod
tektir.

### Bloke etmeyen `read`

Gercek bir terminalde `read(0)` veri gelene kadar bekler. Burada beklemez:
o an ne varsa doner, yoksa `0`. Neden borularla ayni: bloke olan bir surec
kendi penceresini de dondurur. Terminal disiplini de yok -- satir tamponu,
otomatik yankilama, `termios` yok; yankiyi uygulama kendi yapar (`echo2`
okudugunu hem ekrana cizer hem `write(1)` ile geri yazar).

### Surec basina program break

Bu isle birlikte `brk` de surec basina tasindi. Onceden tek bir global
`PROGRAM_BREAK` vardi: iki surec ayni anda `brk` cagirdiginda birinin
yigin ustu otekinin heap tabanini kaydiriyordu -- fd tablosunda yasanan
hatanin tipki aynisi. Artik gorev basina bir `Break { current, start,
limit }` var ve `fork` bunu cocuga **kopyalar** (adres uzayi zaten
kopyalandigi icin degerler oldugu gibi gecerlidir).

## Notes: yaz, kaydet, kapat, ac

Ring 3 uygulamasi metnini kendi ciziyor, POSIX `open(O_CREAT)`/`write` ile
kalici dosya sistemine yaziyor ve acilista geri okuyor.

![Notes](docs/screenshot-notes.png)

Ekran goruntusu **ikinci acilistan**: makine tamamen kapatildi, yeniden
acildiginda Notes dosyayi diskten yukledi ("diskten yuklendi") ve kabuktaki
`cat /home/notes.txt` ayni metni gosteriyor.

Bu uygulama TCMK'nin parcalarini tek yerde birlestiriyor:

| Parca | Nerede |
|---|---|
| Pencere + **kendi cizdigi metin** | `tcmk::gui::Window::text/glyph` |
| Klavye olaylari | `win_poll_key` (0x504) |
| Kalici yazma | `File::create` -> `open(O_CREAT)` + `write` |
| Uyku | `win.frame(30)` -> `sleep` (0x508) |

**Metin cizimi bilerek uygulamada.** Cekirdekte bir "metin ciz" syscall'i
olsaydi her karakter icin Ring 0'a gecilirdi. Font cekirdekle ayni
(`tools/sync_font.py` kopyalar), yani kabuktaki yaziyla uygulama yazisi
ayni gorunur.

**Dosyaya yazma yolu.** `kernel_api::write` artik stdout disindaki
tanimlayicilar icin VFS'e gidiyor; `open` POSIX `O_CREAT` bayragini
tanidigi icin uygulama olmayan bir dosyayi kendisi olusturabiliyor.
Yazma yalnizca TCMKFS dugumlerinde calisir -- RAMFS dosyalarinin icerigi
cekirdek imajinin `.rodata`'sindadir.

## Preemptive zamanlama

Zamanlayici kesmesi artik gorevi **kendi istegi olmadan** birakiyor.

![Preemption](docs/screenshot-preemption.png)

`run spin` -- hicbir sistem cagrisi yapmayan saf hesap dongusu
(`userland-rs/src/bin/spin.rs`). Ekran goruntusunde spin kosarken:

```
tcmk> ps
   id durum      ad            cagri  adres-uzayi
 *  0 calisiyor  idle              0     cekirdek
    2 hazir      paint          7356  0x01000000 (128 sayfa)
    3 hazir      plasma         4905  0x01082000 (187 sayfa)
    4 hazir      spin             59  0x01104000 (164 sayfa)
baglam degisimi: 7769  (zorla: 2835)
```

spin yalnizca **59** cagri yapmis (plasma 4905) -- yani neredeyse tum
zamanini cekirdegi hic cagirmadan geciriyor. Buna ragmen kabuk `ps`'e
yanit verdi ve Plasma cizmeye devam etti. Isbirlikci modelde masaustu bu
dongu boyunca donardi; Yuk Dengeleyici de caresizdi, cunku kisitlayacak
bir cagri yoktu.

### Nasil calisiyor

Yapisal olarak syscall yolundan farki yok: orada da baglam degisimi kesme
yigininda (Ring 3 icin `TSS.esp0`) yapiliyor ve donuste ayni noktaya
donulup `iret` calisiyor. Zamanlayici kesmesi de aynisini yapiyor --
`pit_handler` once EOI gonderiyor (baglam degisiminden **once**, yoksa PIC
hala hizmet bekler ve bir daha IRQ0 gelmez), sonra
`scheduler::preempt_from_timer()`.

Kesme kapisi IF=0 ile girildigi icin ic ice preemption olmaz;
`without_interrupts` bolgeleri de dogal olarak korunur.

### Uyku: `TaskState::Blocked`

Preemption CPU'yu adil bolusturur ama uygulamalarin **istemedigi** zamani
geri vermez: sadece `flush` cagiran bir GUI dongusu sirasi geldiginde
hemen bir kare daha cizer, yani ekranin yenilenme hizindan bagimsiz olarak
CPU'yu doldurur.

`sleep` (0x508) bunu kapatir. Uyuyan gorev `pick_next` tarafindan **hic
secilmez**; suresi dolunca `wake_expired` onu yeniden hazir yapar.
Uyandirmayi secim aninda yapmak ayri bir zamanlayici kuyruguna gerek
birakmiyor -- gorev sayisi kucuk oldugu icin dogrusal tarama kuyruk
yonetiminden ucuz.

Userland tarafinda tek satir:

```rust
win.frame(30);   // flush + 30 ms uyku
```

```
tcmk> ps
   id durum      ad            cagri  adres-uzayi
    2 uyuyor     paint          3635  0x01000000 (191 sayfa)
    3 hazir      plasma         2018  0x01082000 (187 sayfa)
    4 uyuyor     plasma          405  0x01104000 (187 sayfa)
    5 uyuyor     spin             81  0x01186000 (164 sayfa)
baglam degisimi: 6056  (zorla: 2943)  uyuyan: 3
```

Idle gorevi uyutulmaz: masaustu dongusudur ve uyanacak baska gorev
kalmadiginda sistemi ilerletecek tek akistir. `sleep_ticks`'in dongusu de
bu garantiye dayanir -- idle her zaman hazir oldugu icin `yield_now`
mutlaka baska bir goreve gecer.

### Bolunemez isler: aygit kilidi

Kesmeleri kapatmak yerine yalnizca **baglam degisimini** erteleyen bir
sayac var (`preempt_disable`). Bir donem tek kullanicisi diskti: bir ATA
komutu (surucu secimi -> LBA -> komut -> veri) bir butundur ve ortasinda
baska bir gorev ikinci bir komut baslatirsa denetleyicinin durumu bozulur.

O kullanim **kaldirildi**. "Kimse calismasin" demek, korunmasi gereken sey
yalnizca *bir aygit* iken fazla genis bir cozumdu -- ve kesmeyle beklemeyi
bastan imkansiz kiliyordu. Artik surucu kendi **aygit kilidini** tutuyor
(`DeviceLock`): yalnizca ATA'ya dokunmak yasak, baska gorevler serbestce
kosuyor. Kilit `Drop` ile birakiliyor, yani `?` ile erken donen her hata
yolu da onu geri veriyor.

## Surec basina adres uzayi

Her Ring 3 sureci artik **kendi sayfa tablosunu** alir. Ayni ikilinin uc
kopyasi ayni anda, hepsi `0x00C00000`'de, birbirine dokunmadan calisiyor:

![Adres uzaylari](docs/screenshot-address-spaces.png)

```
tcmk> ps
  id durum      ad            cagri  adres-uzayi
 * 0 calisiyor  idle              0     cekirdek
   1 bitti      worker           42     cekirdek
   2 hazir      paint          9126  0x01000000 (128 sayfa)
   3 hazir      plasma         6085  0x01082000 (128 sayfa)
   4 hazir      plasma         1723  0x01104000 (128 sayfa)
   5 hazir      plasma          907  0x01186000 (128 sayfa)
```

### Neden ucuz

Kullanici bolgesi (`0x00C00000`, 2 MiB) tam olarak **tek bir PDE'nin**
(indeks 3) icine duser. Her surec yalnizca kendi PDE 3 sayfa tablosunu
alir; cekirdek, heap ve MBR/MMIO eslemeleri tum adres uzaylarinda
**paylasilir**. Baglam degisiminde tek yapilan CR3'u degistirmek.

| Katman | Dosya | Islev |
|---|---|---|
| Cerceve ayirici | `core/frames.rs` | 4 KiB'lik cerceveler uzerinde bit haritasi, 16 MiB havuz |
| Adres uzayi | `core/mmu_i386.rs` | `create_user_space` / `map_user_range` / `switch_to` / `destroy_user_space` |
| Zamanlayici | `core/scheduler.rs` | `Task.address_space`, baglam degisiminde CR3 |
| Surec | `level0b1/process.rs` | yuklemeden **once** uzay kurar, cikista cerceveleri havuza verir |

### Ne degisti

**Slot modeli bitti.** Onceden her uygulama derleme aninda farkli bir
taban adrese linkleniyordu (`--image-base=0x00C40000` gibi), cunku hepsi
ayni adres uzayini paylasiyordu. Iki bedeli vardi: ayni ikilinin iki
kopyasi birbirinin **kodunu eziyordu** ve her uygulama digerinin
bellegini **okuyabiliyordu**. Artik hepsi `0x00C00000`'e linkleniyor.

**Kullanici bolgesi cekirdek uzayinda hic yok.** Acilis testi bunu
gosteriyor:

```
[worker] izolasyon: user@0xc00000=false kernel@0x100000=false heap@0x800000=false
```

Uc deger de `false`: kullanici bolgesi yalnizca bir surecin adres
uzayinda **vardir**. Onceki modelde ilk deger `true` idi -- bolge tum
sistemde acikti.

**Cerceveler geri veriliyor.** `kmalloc` bir bump ayiricidir (serbest
birakma yok) ve cekirdek yapilari icin dogru secim. Surecler ise gelip
gidiyor; bu yuzden ayri bir cerceve ayiricisi eklendi. `mem` sayaci
tutuyor: dort surec = 520 cerceve (surec basina 128 veri + 1 sayfa dizini
+ 1 sayfa tablosu).

### Surec olurken

Bir surec bittiginde (normal cikis ya da `kill`) uc sey birakilir:
adres uzayi, cerceveleri ve **pencereleri**. Pencere kapatilmazsa
ekranda artik kimsenin cizmedigi olu bir dikdortgen kalir ve tamponu
bosuna tutulur.

```
tcmk> mem                       tcmk> kill 4
cerceve havuzu: 520 / 4096      tcmk> kill 5
                                tcmk> mem
                                cerceve havuzu: 260 / 4096 (tepe 520)
```

Iki surec = 260 cerceve, birebir geri dondu. Pencere yuvalari da serbest
kalir; `create` once bos yuva arar, aksi halde uygulamalar acilip
kapandikca sekiz pencerelik tablo dolardi.

### Sinirlar

- Surec basina **512 KiB** eslenir (`USER_MAP_SIZE`), tum 2 MiB degil.
  Talep uzerine sayfalama (demand paging) olmadigi icin pesin esleme
  havuzu bosuna tuketirdi.
- Pencere tamponlari da **surece ozeldir**: WM tamponu cekirdek
  heap'inden ayirir ama Ring 3'e yalnizca **sahibinin** adres uzayinda,
  `0x00D00000`'den itibaren eslenir. Cekirdek ayni bellegi identity
  adresinden gormeye devam eder -- kompozitor oradan okur. Eskiden tampon
  identity haritasinda aciliyordu, yani her uygulama her pencerenin
  piksellerini okuyabiliyordu.

  ```
  tcmk> win
    id  boyut     sahip     tampon (surecin adresi)
     0  632x390   cekirdek  (kernel tamponu)
     2  320x200   paint     0x00d00000
     3  300x200   plasma    0x00d00000
  ```

  Ayni adres, farkli adres uzaylari -- yani farkli fiziksel bellek.
  `win_buffer` cagrisi sahiplik denetimi de yapar.
- x86_64 portu **tek adres uzayinda** kaldi; dort seviyeli tablo + 2 MiB
  huge page bolunmesi ayni isi daha fazla muhasebeyle gerektiriyor.
  Ust katmanlar farki gormuyor: `create_user_space()` orada `None` doner
  ve cagiran paylasimli yola duser.

## Kendi onyukleyicisi: TCMK kendi kendini aciyor

Buraya kadar acilisi hep GRUB yapiyordu. Artik TCMK'nin **kendi iki asamali
onyukleyicisi** var (`boot/tcmkboot/`) ve `install` komutundan sonra disk
GRUB'a hic ugramadan aciliyor.

![Kurulum](docs/screenshot-install.png)

```
tcmk> install onayla
1. asama yazildi (148 bayt, MBR 0..446).
2. asama: lba 14376, 32 sektor.
cekirdek imaji: lba 14408
Kurulum tamam -- makineyi diskten yeniden baslatin.
```

Yeniden baslatildiginda ekrandaki her sey TCMK'nin kendi zinciriyle geliyor:

![Kendi onyukleyicisiyle acilis](docs/screenshot-selfboot.png)

### Zincir

| Asama | Nerede | Ne yapiyor |
|---|---|---|
| BIOS | — | MBR'nin ilk 446 baytini 0x7C00'e yukler |
| **1. asama** | MBR (148 bayt) | int 0x13 AH=0x42 ile 2. asamayi 0x8000'e okur |
| **2. asama** | bolum + 40 (548 bayt) | VBE modu kurar, korumali moda gecer, cekirdegi ATA PIO ile 1 MiB'a okur, `.bss`'i sifirlar, Multiboot1 sozlesmesiyle atlar |
| Cekirdek | 0x100000 | degismedi -- GRUB'dan gelmis gibi acilir |

**Cekirdek tek satir degismedi.** 2. asama, cekirdegin bekledigi
Multiboot1 bilgi yapisini (flags bit 12 + framebuffer alanlari) kendisi
kuruyor; cekirdek acisindan GRUB ile TCMK onyukleyicisi ayirt edilemez.

### Tasarim kararlari

**Cekirdek diske "cozulmus" yaziliyor.** ELF'in PT_LOAD segmentleri
kurulum ortami uretilirken (`tools/make_disk.py`) bellekteki yerlerine gore
tek bir ardisik bloga yerlestiriliyor. Boylece 2. asamanin ELF
ayristirmasina ihtiyaci yok: "su LBA'dan su kadar sektoru 0x100000'e oku"
yetiyor.

**Diski BIOS yerine ATA PIO ile okuyoruz.** Cekirdek imaji 1 MiB'i asiyor;
BIOS int 0x13 ise gercek mod segment:ofset ile yalnizca ilk 1 MiB'a
yazabilir. Klasik cozum "unreal mode"dur; korumali moda bastan gecip diski
cekirdegin zaten kullandigi port dizisiyle okumak hem daha kisa hem daha
az kirilgan. Bedeli: 2. asama IDE disk varsayiyor.

**Onyukleyici alani dosya sisteminin icinde.** TCMKFS bolumunun 40..4095
sektorleri (2 MiB) bitmap'e hic girmez; ayirici oraya asla dosya yazamaz.
Boylece MBR'nin dort bolum yuvasindan biri daha harcanmiyor.

**Assembler eklenmedi.** Iki asama da `global_asm!` icinde `.code16` /
`.code32` ile yazildi, `rust-lld --oformat=binary` ile duz ikiliye
linklendi. Arac zinciri hala Rust + GRUB + QEMU.

### 16-bit assembly tuzaklari (yasananlar)

- LLVM'in Intel sozdiziminde **ciplak sembol bir bellek operandidir**:
  `mov si, msg` sembolun adresini degil, o adresteki **degeri** yukler.
  Adres icin `offset` sart. (Cekirdegin `_start`'indaki
  `mov esp, stack_top` tuzaginin aynisi.)
- `.code16` icinde `call`/`ret`/`retf` LLVM tarafindan **32-bit** uretilir
  (0x66 onekiyle) -- gercek mod yigininda 4 baytlik itme/cekme demektir ve
  donus adresi bozulur. `callw`/`retw` da kabul edilmiyor; cozum
  altyordamdan tamamen vazgecmek oldu.
- 16->32 bit uzak atlama ham baytla kodlandi (`66 EA <ofset32> <segment16>`).

### Neden GRUB yerine kendi zinciri

Alternatif FAT bolumu + `grub-install` yoluydu; ama GRUB'un 1. asamasi yine
**host araclariyla** yazilmak zorunda kalirdi -- yani "kurulum" aslinda
kurulum olmazdi. Kendi zinciri TCMK'yi gercekten kendi kendini kurar hale
getiriyor.

## Hata izolasyonu (kararlilik)

Tum 32 CPU istisna vektoru baglidir. Bir Ring 3 uygulamasi hata uretirse
**yalnizca o surec sonlandirilir**, sistem calismaya devam eder:

![Hata izolasyonu](docs/screenshot-fault-isolation.png)

`run crash` ile baslatilan test uygulamasi kasten cekirdek adresine yazar:

```
[LEVEL-0b2] ISTISNA #14 (page-fault) -- Ring 3 kaynakli
            IP=0x00cc00d5  hata_kodu=0x7
            adres=0x00100000  koruma-ihlali / yazma / Ring 3
[LEVEL-0b2] Surec sonlandirildi; sistem calismaya devam ediyor.
```

Ekran goruntusunde goruldugu gibi Paint ve Plasma animasyonlarina devam
eder, kabuk yanit verir. **Bu degisiklikten once ayni hata triple fault
uretip tum makineyi yeniden baslatiyordu** -- cunku yalnizca vektor 0
bagliydi ve baglanmamis bir istisnanin gidecek yeri yoktu.

Hata Ring 0'dan gelirse bu bir cekirdek hatasidir; Level-0b2 Fallback
Interface devreye girip sistemi guvenli duruma alir.

Kabuktan `faults` komutu istatistikleri gosterir.

## Alfa'nin bilinen sinirlari

Durustce: bu **minimal grafiksel alfa**dir, masaustu ortami degil.

- **Zamanlama round-robin ve onceliksiz.** Preemption var ama gorevler
  esit; oncelik, gercek zamanli sinif ve `nice` yok.
- **PE ikililerinde `.reloc` zorunlu.** Yukleyici imaji her zaman
  kullanici bolgesinin basina koyar; `/fixed:no` ile linklenmemis, yani
  yeniden yerlesim tablosu tasimayan bir ikili yuklenemez. Windows'ta bu
  ikilinin tercih ettigi tabana bagli olarak calisabilirdi.
- **`fork` copy-on-write degil.** Sayfalar cagri aninda tamamen
  kopyalanir; COW icin sayfalari salt okunur isaretleyip page fault'ta
  ayirmak gerekir. Dogruluk degil, maliyet farki.
- **Boru okumasi bloke etmez** ve boru sayisi dorttur; `dup`/`dup2` ve
  `select`/`poll` yok.
- **Standart girdi bloke etmez.** `read(0, ...)` sahibinin penceresinde
  biriken tuslari **o an ne varsa** dondurur, tus yoksa 0. Bloke eden bir
  `read` GUI dongusunu de dondururdu; terminal disiplini (satir tamponu,
  yankilama, `termios`) da yok -- yankiyi uygulama kendi yapar.
- **Sinyal teslimi syscall donusune baglidir.** Hicbir syscall yapmayan
  saf hesap dongusu sinyali gormez (`spin` boyle); `SIGKILL` ise
  isbirligi gerektirmedigi icin her zaman calisir. Ayrica maskeleme
  (`sigprocmask`), `siginfo`/`sigaction` bayraklari, `alarm` ve
  gercek-zamanli sinyaller yok; isleyici icinde ikinci bir sinyal teslim
  edilmez (ic ice cagri yok).
- **`waitpid` yalnizca belirli bir cocugu bekler.** `pid = -1` ("herhangi
  bir cocuk") ve surec gruplari yok; oksuz kalan gorevler de
  toplanmiyor -- gorev yuvasi surec bitse de tabloda kalir.
- **Surec basina 512 KiB eslenir**, talep uzerine sayfalama yok.
- **TCMKFS'te toplam 64 inode var** (dizinler de sayilir), dosya basina
  160 KiB (yalnizca dogrudan blok isaretcileri) ve azami 8 seviye
  derinlik. Dizin agaci gercek, ama `.`/`..` bilesenleri, sembolik
  baglar, izinler ve sahiplik yok; kabugun bir "calisma dizini" kavrami
  da yok -- yollar her zaman mutlaktir.
- **Kabuktan verilen disk komutlari hala yoklamali.** Kabuk idle
  gorevinde (masaustu dongusu) kosar ve o gorev uyutulamaz -- uyutmak
  ekrani dondururdu. Ring 3 sureclerinin disk erisimi kesmeyle bekler.
  Ayrica DMA yok: veri hala `in/out` ile kelime kelime tasinir.

## Kapsam Disi (sonraki fazlar)

AArch64 portu (Faz 6), ext2/tmpfs ve genis POSIX (Faz 9-10),
musl/busybox + shell (Faz 11-12), framebuffer/virtio-net (Faz 13-14).
