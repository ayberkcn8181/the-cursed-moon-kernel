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
| 4 | **x86_64 portu**: Long Mode, 4 seviyeli sayfalama, ELF64, `syscall` | ✅ |
| 13 | **Framebuffer/grafik**: 1024x768x32, bitmap font, cift tampon | ✅ (i386) |
| 14 | **Pencere yoneticisi**: kompozitor, fare, surukleme, GUI syscall'lari | ✅ (i386) |
| 6 | AArch64 portu (EL1/EL0, GIC, `svc #0`) | ⏳ yapilmadi |
| 8 | **Surec basina adres uzayi** (cerceve ayirici + CR3 degisimi) | ✅ (i386) |
| 8 | **Preemptive zamanlama** + uyku durumu (`sleep`) | ✅ |
| 9 | **Kalici depolama**: ATA PIO, MBR, TCMKFS (yazilabilir) | ✅ (i386) |
| — | **Kendi onyukleyicisi** + diske kurulum (`install`) | ✅ (i386) |
| 8+ | fork/execve + sinyaller, musl/busybox | ⏳ yapilmadi |

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
  | sistem | `ps` `top` `kill <id>` `svc` `health` `uptime` `ver` |
  | Level-0b2 | `load` `ipc` `faults` `stall <sn>` |
  | bellek/disk | `mem` `disk` `df` `format onayla` `sync` `install onayla` |
  | dosya | `ls` `cat <yol>` `save <yol> <metin>` `cp <kaynak> <hedef>` `rm <yol>` |
  | uygulama/pencere | `apps` `run <ad>` `win` `focus <id>` `mouse` |
  | diger | `echo <metin>` `clear` `help` |
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

Pencere tamponu **yalnizca sahibinin** adres uzayina eslenir (bkz. "Surec
basina adres uzayi"); uygulama piksellerini kendisi yazar, cekirdek
yalnizca kompozisyon yapar.

## Desteklenen mimariler

| | i386 | x86_64 |
|---|---|---|
| Boot | Multiboot1 | Multiboot2 + Long Mode gecisi |
| Sayfalama | 2 seviye, 4 KiB, 4 MiB identity | 4 seviye, 2 MiB huge + 4 KiB split, 1 GiB identity |
| Linux syscall | `int 0x80` | `syscall` komutu (MSR: EFER/STAR/LSTAR/SFMASK) |
| Windows syscall | `int 0x2E` | — (PE32+ Faz 7'nin 64-bit ayagi) |
| Ikili formatlar | ELF32 + PE32 | ELF64 |
| Kesme denetleyici | PIC 8259A | PIC 8259A (APIC ileride) |

```
make ARCH=i386   run
make ARCH=x86_64 run
```

Ortak katmanlar (`level0a`, `level0b1`, `level0b2`) **tek bir kod tabanidir**;
mimariye ozel her sey `arch/<arch>/`, `level0a/gdt/<arch>.rs`,
`level0a/idt/<arch>.rs` ve `level0a/core/mmu_<arch>.rs` icinde izole
(doc S.15 ilke 2). Ortak kod donanima her zaman `arch::cpu` uzerinden erisir.

## Gereksinimler

```
rustup toolchain install nightly --component rust-src,llvm-tools
sudo apt install qemu-system-x86 grub-pc-bin grub-common xorriso mtools
```

(`rust-toolchain.toml` bu depoda nightly'yi otomatik secer.)

## Derleme / ISO / Calistirma

```
make ARCH=i386       # cargo build (freestanding, custom i686-tcmk target)
make iso             # grub-mkrescue ile bootable ISO
make run             # ISO'dan calistir (CD-ROM)
make bootloader      # iki asamali TCMK onyukleyicisi (duz ikili)
make disk            # kalici TCMKFS bolumu + onyukleyici olan disk imaji
make run-disk        # DISKTEN acilis (CD yok) -- kalicilik boyle dogrulanir
make info            # secili ayarlari yazdirir
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

### Userland ikililerini yeniden uretme

```
make userland          # Rust uygulamalari + PE32/ELF64 (elle uretilenler)
make userland-rust     # yalnizca Rust uygulamalari
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

### Bolunemez isler: `preempt_disable`

Kesmeleri kapatmak yerine yalnizca **baglam degisimini** erteleyen bir
sayac var. Su an tek kullanicisi disk: bir ATA komutu (surucu secimi ->
LBA -> komut -> veri) bir butundur; ortasinda baska bir gorev ikinci bir
komut baslatirsa denetleyicinin durumu bozulur. Kesmeleri kapatmak da ise
yarardi ama 2 MiB'lik bir okuma boyunca kesmesiz kalmak saati ve girdiyi
dondururdu.

`PreemptGuard` `Drop` ile birakilir; boylece `?` ile erken donen her hata
yolu da kilidi geri verir.

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
- **GUI yalnizca i386'da.** x86_64 cekirdegi calisir ve framebuffer'i
  eslerse de GUI uygulamalari su an yalnizca ELF32 olarak uretiliyor.
- **`fork`/`execve` yok.** Her surec bos bir adres uzayinda dogar; sureci
  kopyalamak (fork) ve calisan surecin uzerine yeni imaj yuklemek (execve)
  Faz 8'in kalan yarisi.
- **Surec basina 512 KiB eslenir**, talep uzerine sayfalama yok.
- **TCMKFS duz bir isim uzayidir**, gercek dizin agaci degil: `/home/x`
  bir yol degil, dosyanin **adidir**. Azami 64 dosya, dosya basina 160 KiB
  (yalnizca dogrudan blok isaretcileri).
- **Disk erisimi yoklamalidir** (ATA PIO, IRQ14 baglanmadi): buyuk bir
  yazma sirasinda sistem o sure boyunca duraklar.

## Kapsam Disi (sonraki fazlar)

x86_64 (Faz 4) ve AArch64 (Faz 6) portlari, NT/PE uyumlulugu (Faz 7),
fork/execve + sinyaller (Faz 8), ext2/tmpfs ve genis POSIX (Faz 9-10),
musl/busybox + shell (Faz 11-12), framebuffer/virtio-net (Faz 13-14).
