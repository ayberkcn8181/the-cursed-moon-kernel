ARCH ?= i386
MODE ?= release

ROOT_DIR := $(abspath .)
KERNEL_DIR := $(ROOT_DIR)/kernel
TARGET_DIR := $(ROOT_DIR)/target
ISO_DIR := $(ROOT_DIR)/build/isodir-$(ARCH)
ISO := $(ROOT_DIR)/build/tcmk-$(ARCH).iso

CARGO_FLAGS :=
CARGO_OUT_DIR := debug
ifeq ($(MODE),release)
CARGO_FLAGS += --release
CARGO_OUT_DIR := release
endif

# --- Mimariye ozel ayarlar (doc S.15: ARCH= ile hedef secimi) ---
ifeq ($(ARCH),i386)
  RUST_TARGET := i686-tcmk
  QEMU        := qemu-system-i386
  GRUB_CMD    := multiboot
else ifeq ($(ARCH),x86_64)
  RUST_TARGET := x86_64-tcmk
  QEMU        := qemu-system-x86_64
  GRUB_CMD    := multiboot2
else
  $(error Desteklenmeyen ARCH=$(ARCH). Gecerli: i386, x86_64)
endif

# JSON hedef tanimlari (`targets/*.json`) kararsiz bir ozelliktir ve
# 2026 ortasindaki nightly'lerden itibaren ACIK bir bayrak ister:
#
#   error: `.json` target specs require -Zjson-target-spec to be added
#          to the cargo invocation
#
# Bayrak KOSULSUZ verilir. Onceki surumde "cargo bunu taniyor mu"
# denetimi vardi; iyi niyetliydi ama kirilgan cikti -- denetimin kendisi
# bayrakla ilgisiz nedenlerle basarisiz olabiliyor ve o zaman bayrak hic
# eklenmiyordu, yani hatayi cozmesi gereken kod sessizce devre disi
# kaliyordu. Bir denetim, olcmesi gereken seyden daha kirilgan ise
# zarari faydasindan cok olur.
#
# Bayragi taniyan ama zorunlu kilmayan nightly'ler onu sorunsuz kabul
# eder, yani tek risk bir gun kararli hale gelip -Z listesinden
# cikmasidir. O gun geldiginde kapatmak icin:
#
#   make JSON_TARGET_FLAG=
#
# `?=` oldugu icin ortam degiskeni ya da komut satiri her zaman kazanir.
JSON_TARGET_FLAG ?= -Zjson-target-spec

TARGET_JSON := $(ROOT_DIR)/targets/$(RUST_TARGET).json
KERNEL_ELF := $(TARGET_DIR)/$(RUST_TARGET)/$(CARGO_OUT_DIR)/tcmk-kernel

.DEFAULT_GOAL := all
.PHONY: all iso run disk run-disk info check clean bootloader userland userland-rust userland-x86_64 userland-win userland-win64 userland-legacy

# --- Arac denetimi -------------------------------------------------------
#
# Eksik bir arac, aksi halde alakasiz gorunen bir hatayla ortaya cikar:
# `rust-src` yoksa hata "can't find crate for `core`" olur ve gercek neden
# hic gorunmez. Her hedef YALNIZCA kendi ihtiyacini denetler; cekirdegi
# derlemek icin QEMU ya da GRUB kurmak gerekmez.
#
# $(1) = komut, $(2) = ne ise yarar, $(3) = kurulum ipucu
define need
@command -v $(1) >/dev/null 2>&1 || { \
	echo ""; \
	echo "HATA: '$(1)' bulunamadi."; \
	echo "  gereken:  $(2)"; \
	echo "  kurulum:  $(3)"; \
	echo ""; \
	echo "  butun araclarin durumu icin: make check"; \
	echo ""; \
	exit 1; }
endef

# `-Z build-std` cekirdek kutuphanesini KAYNAKTAN derler; kaynak yoksa
# hata gorunuste crate cozumleme hatasidir.
define need_rust_src
@command -v cargo >/dev/null 2>&1 || { \
	echo ""; echo "HATA: 'cargo' bulunamadi."; \
	echo "  kurulum:  https://rustup.rs"; echo ""; exit 1; }
@cargo --version 2>/dev/null | grep -q nightly || { \
	echo ""; \
	echo "HATA: cargo NIGHTLY degil."; \
	echo "  bulunan:  $$(cargo --version 2>&1 | head -1)"; \
	echo "  konum:    $$(command -v cargo)"; \
	echo ""; \
	echo "  Bu cekirdek stable ile derlenemez: ozel JSON hedefleri,"; \
	echo "  -Z build-std ve abi_x86_interrupt gibi kararsiz ozellikler"; \
	echo "  kullanir. Bunlarin hicbirinin stable karsiligi yoktur."; \
	echo ""; \
	echo "  Depodaki rust-toolchain.toml nightly'yi zaten secer, AMA bunu"; \
	echo "  yalnizca rustup uygular. Dagitim paketinden gelen cargo"; \
	echo "  (Arch'ta 'rust', Debian'da 'cargo') o dosyayi yok sayar."; \
	echo ""; \
	if command -v rustup >/dev/null 2>&1; then \
		echo "  rustup KURULU ama cargo baska yerden geliyor."; \
		echo "  Muhtemel neden: PATH'te dagitim cargo'su once."; \
		echo "    rustup toolchain install nightly --component rust-src,llvm-tools"; \
		echo "    export PATH=\"\$$HOME/.cargo/bin:\$$PATH\"    # rustup shim'i one al"; \
		echo "  Arch'ta dagitim paketini kaldirmak da cozer: sudo pacman -Rs rust"; \
	else \
		echo "  rustup KURULU DEGIL. Kurulumu:"; \
		echo "    Arch:   sudo pacman -S rustup && sudo pacman -Rs rust"; \
		echo "    digeri: https://rustup.rs"; \
		echo "    sonra:  rustup toolchain install nightly --component rust-src,llvm-tools"; \
	fi; \
	echo ""; exit 1; }
@test -d "$$(rustc --print sysroot)/lib/rustlib/src/rust/library/core" || { \
	echo ""; \
	echo "HATA: rust-src bileseni yok."; \
	echo "  gereken:  -Z build-std, core ve compiler_builtins'i kaynaktan derler"; \
	echo "  kurulum:  rustup component add rust-src --toolchain nightly"; \
	echo ""; exit 1; }
endef

## Butun araclari denetler ve eksikleri listeler.
check:
	@echo "arac                 gerekli oldugu yer                      durum"
	@echo "-------------------- --------------------------------------- -----"
	@printf '%-20s %-39s ' "cargo/rustc" "cekirdek + userland derlemesi"; command -v cargo >/dev/null 2>&1 && echo "var" || echo "YOK"
	@printf '%-20s %-39s ' "cargo NIGHTLY mi" "kararsiz ozellikler (stable olmaz)"; cargo --version 2>/dev/null | grep -q nightly && echo "var" || echo "YOK"
	@printf '%-20s %-39s ' "rust-src" "-Z build-std (core, compiler_builtins)"; test -d "$$(rustc --print sysroot 2>/dev/null)/lib/rustlib/src/rust/library/core" && echo "var" || echo "YOK"
	@printf '%-20s %-39s ' "grub-mkrescue" "make iso / run"; command -v grub-mkrescue >/dev/null 2>&1 && echo "var" || echo "YOK"
	@printf '%-20s %-39s ' "xorriso" "make iso (grub-mkrescue kullanir)"; command -v xorriso >/dev/null 2>&1 && echo "var" || echo "YOK"
	@printf '%-20s %-39s ' "mformat (mtools)" "grub-mkrescue EFI imaji"; command -v mformat >/dev/null 2>&1 && echo "var" || echo "YOK"
	@printf '%-20s %-39s ' "llvm-dlltool" "make userland-win (PE ithal .lib)"; command -v llvm-dlltool >/dev/null 2>&1 && echo "var" || echo "YOK"
	@printf '%-20s %-39s ' "python3" "make disk / userland-legacy"; command -v python3 >/dev/null 2>&1 && echo "var" || echo "YOK"
	@printf '%-20s %-39s ' "qemu-system-i386" "make run (ARCH=i386)"; command -v qemu-system-i386 >/dev/null 2>&1 && echo "var" || echo "YOK"
	@printf '%-20s %-39s ' "qemu-system-x86_64" "make run (ARCH=x86_64)"; command -v qemu-system-x86_64 >/dev/null 2>&1 && echo "var" || echo "YOK"
	@echo ""
	@echo "Debian/Ubuntu icin hepsi:"
	@echo "  rustup toolchain install nightly --component rust-src,llvm-tools"
	@echo "  sudo apt install qemu-system-x86 grub-pc-bin grub-common xorriso mtools llvm"
	@echo ""
	@echo "NOT: grub-mkrescue Linux'a ozgudur. macOS/Windows'ta 'make' ve"
	@echo "     'make ARCH=x86_64' calisir, 'make iso' calismaz."
	@echo ""
	@echo "Derlenmis ikililer depoda hazir gelir; 'make userland' YALNIZCA"
	@echo "onlari yeniden uretmek istendiginde gerekir (llvm-dlltool ister)."

# Cekirdek 1. asamayi include_bytes! ile gomdugu icin onyukleyici
# cekirdekten ONCE uretilmelidir.
all: bootloader
	$(call need_rust_src)
	cd $(KERNEL_DIR) && cargo $(JSON_TARGET_FLAG) build $(CARGO_FLAGS) \
		--target $(TARGET_JSON) --target-dir $(TARGET_DIR)

# --- Onyukleyici ---------------------------------------------------------
#
# Iki asama da tamamen assembly'dir; Rust yalnizca derleyici/baglayici
# zinciri olarak kullanilir (harici assembler eklemeden). Duz ikili uretmek
# icin `--oformat=binary` verilir.
BOOT_DIR := $(ROOT_DIR)/boot/tcmkboot
BOOT_OUT := $(ROOT_DIR)/build/boot
BOOT_TARGET_DIR := $(TARGET_DIR)/tcmkboot

bootloader:
	$(call need_rust_src)
	@mkdir -p $(BOOT_OUT)
	@set -e; for s in stage1 stage2; do \
		( cd $(BOOT_DIR) && cargo $(JSON_TARGET_FLAG) rustc --release --bin $$s \
			--target $(ROOT_DIR)/targets/i686-tcmk.json \
			--target-dir $(BOOT_TARGET_DIR) \
			-- -C link-arg=-T$$s.ld -C link-arg=--oformat=binary ); \
		cp $(BOOT_TARGET_DIR)/i686-tcmk/release/$$s $(BOOT_OUT)/$$s.bin; \
		echo "  [boot] $$s.bin $$(stat -c%s $(BOOT_OUT)/$$s.bin) bayt"; \
	done

iso: all
	$(call need,grub-mkrescue,make iso: onyuklenebilir ISO uretir,sudo apt install grub-pc-bin grub-common)
	$(call need,xorriso,grub-mkrescue ISO9660 icin bunu cagirir,sudo apt install xorriso)
	mkdir -p $(ISO_DIR)/boot/grub
	cp $(KERNEL_ELF) $(ISO_DIR)/boot/tcmk-kernel.elf
	printf 'set timeout=0\nset default=0\n\nmenuentry "The Cursed Moon Kernel ($(ARCH))" {\n    $(GRUB_CMD) /boot/tcmk-kernel.elf\n    boot\n}\n' \
		> $(ISO_DIR)/boot/grub/grub.cfg
	grub-mkrescue -o $(ISO) $(ISO_DIR)

run: iso
	$(call need,$(QEMU),make run: emulatorde calistirir,sudo apt install qemu-system-x86)
	$(QEMU) -cdrom $(ISO) -serial stdio

# --- Disk imaji -----------------------------------------------------------
#
# grub-mkrescue hibrit bir imaj uretir (hem CD hem sabit disk olarak
# acilabilir). tools/make_disk.py bunun sonuna TCMKFS icin kalici bir
# bolum ekleyip MBR bolum tablosuna isler. Sonuc: root gerektirmeden
# uretilen, kendi kendine acilan, yazilabilir veri bolumu olan tek dosya.
DISK := $(ROOT_DIR)/build/tcmk-disk-$(ARCH).img
DISK_MIB ?= 64

disk: iso bootloader
	$(call need,python3,tools/make_disk.py disk imajini kurar,sudo apt install python3)
	python3 $(ROOT_DIR)/tools/make_disk.py $(ISO) $(DISK) $(DISK_MIB) \
		$(BOOT_OUT)/stage2.bin $(KERNEL_ELF)

# Diskten acilis (CD yok): kalicilik ancak boyle dogrulanabilir.
run-disk: disk
	$(call need,$(QEMU),make run-disk: emulatorde calistirir,sudo apt install qemu-system-x86)
	$(QEMU) -drive file=$(DISK),format=raw,if=ide -serial stdio

# --- Ring 3 uygulamalari ------------------------------------------------
#
# Her surec kendi adres uzayini aldigi icin (bkz. core/mmu_i386.rs) TUM
# uygulamalar ayni tabana linklenir. Onceki "slot" modeli -- her uygulamaya
# derleme aninda farkli bir taban -- artik gereksiz.
USERLAND_DIR := $(ROOT_DIR)/userland-rs
USERLAND_TARGET_DIR := $(TARGET_DIR)/userland
USER_BASE := 0x00C00000

USER_APPS := hello paint plasma crash hog spin notes menu twins relay echo2 sigdemo

# Windows (PE32) uygulamalari. Ayni kaynak agacindan, ayni `tcmk`
# kutuphanesiyle, yalnizca **baska bir hedefle** derlenir: rust-lld
# `msvc-lld` kipinde dogrudan PE32 uretir, yani Windows arac zinciri
# gerekmez. Taban 0x00400000 (Windows gelenegi) oldugu icin cekirdek
# imaji yuklerken taban yeniden yerlesimi uygular -- gercek bir Windows
# programinda oldugu gibi.
WIN_TARGET_DIR := $(TARGET_DIR)/userland-win
WIN_APPS := winclock winpad

# Ithal kutuphaneleri (Faz 7b). Ortada gercek bir DLL YOKTUR: bunlar
# yalnizca baglayiciya "bu adlar KERNEL32.dll'den gelecek" demenin
# bicimsel yoludur, boylece ikilinin icinde gercek bir ithal tablosu
# olusur. Cekirdek yukleme aninda adlari gomulu tablosunda cozer ve her
# biri icin surecin adres uzayina bir thunk yazar (nt_subsystem/dll.rs).
WIN_LIB_DIR := $(ROOT_DIR)/build/winlib
WIN_DEFS := kernel32 tcmkgui

userland: userland-rust userland-x86_64 userland-win userland-win64 userland-legacy

$(WIN_LIB_DIR)/stamp: $(patsubst %,$(USERLAND_DIR)/win/%.def,$(WIN_DEFS))
	$(call need,llvm-dlltool,PE ithal kutuphanelerini .def dosyasindan uretir,sudo apt install llvm)
	@mkdir -p $(WIN_LIB_DIR)
	@set -e; for d in $(WIN_DEFS); do \
		llvm-dlltool -m i386 --kill-at \
			-d $(USERLAND_DIR)/win/$$d.def \
			-l $(WIN_LIB_DIR)/$$d.lib; \
		echo "  [winlib] $$d.lib"; \
	done
	@# Ithal kutuphaneleri cargo'nun girdisi degildir; bir .def degistiginde
	@# kaynaklara dokunulmazsa cargo yeniden LINKLEMEZ ve ikili eski ithal
	@# tablosuyla kalir (sessiz, bulmasi zor bir tuzak).
	@touch $(USERLAND_DIR)/src/win/*.rs
	@touch $@

userland-rust:
	@mkdir -p $(ROOT_DIR)/userland
	@set -e; for app in $(USER_APPS); do \
		echo "  [userland] $$app (taban $(USER_BASE))"; \
		( cd $(USERLAND_DIR) && cargo $(JSON_TARGET_FLAG) rustc --release --bin $$app \
			--target $(ROOT_DIR)/targets/i686-tcmk.json \
			--target-dir $(USERLAND_TARGET_DIR) \
			-- -C link-arg=--image-base=$(USER_BASE) ); \
		cp $(USERLAND_TARGET_DIR)/i686-tcmk/release/$$app $(ROOT_DIR)/userland/$$app.elf; \
	done

# x86_64 Ring 3 uygulamalari. Ayni kaynak, ayni `tcmk` kutuphanesi;
# degisen yalnizca sistem cagrisi bicimi (`syscall` komutu) ve Linux
# numaralari -- ikisi de `sys.rs` icinde cfg ile ayrilmistir. Cekirdek
# hedefinin JSON'u aynen kullanilir: userland icin farkli olmasi gereken
# tek sey taban adresidir, o da bir baglayici argumani.
USER64_TARGET_DIR := $(TARGET_DIR)/userland64
# PE tarafi (winclock/winpad) i386'ya ozgu; gerisi iki mimaride de var.
USER64_APPS := hello plasma paint notes menu crash twins relay echo2 sigdemo

userland-x86_64:
	@mkdir -p $(ROOT_DIR)/userland
	@set -e; for app in $(USER64_APPS); do \
		echo "  [userland] $$app (ELF64, taban $(USER_BASE))"; \
		( cd $(USERLAND_DIR) && cargo $(JSON_TARGET_FLAG) rustc --release --bin $$app \
			--target $(ROOT_DIR)/targets/x86_64-tcmk.json \
			--target-dir $(USER64_TARGET_DIR) \
			-- -C link-arg=--image-base=$(USER_BASE) ); \
		cp $(USER64_TARGET_DIR)/x86_64-tcmk/release/$$app $(ROOT_DIR)/userland/$$app.elf64; \
	done

userland-win: $(WIN_LIB_DIR)/stamp
	@mkdir -p $(ROOT_DIR)/userland
	@set -e; for app in $(WIN_APPS); do \
		echo "  [userland] $$app.exe (PE32, taban 0x00400000)"; \
		( cd $(USERLAND_DIR) && cargo $(JSON_TARGET_FLAG) rustc --release --bin $$app \
			--target $(ROOT_DIR)/targets/i686-tcmk-win.json \
			--target-dir $(WIN_TARGET_DIR) \
			-- -L $(WIN_LIB_DIR) ); \
		cp $(WIN_TARGET_DIR)/i686-tcmk-win/release/$$app $(ROOT_DIR)/userland/$$app.exe; \
	done

# Windows (PE32+) uygulamalari -- ayni kaynak, ayni ithal kutuphaneleri,
# yalnizca baska bir hedef. Taban 0x140000000 (64-bit Windows gelenegi)
# kullanici bolgesinin cok uzerindedir, yani yeniden yerlesim burada
# **zorunludur**: delta negatiftir ve butun DIR64 girdileri duzeltilir.
WIN64_TARGET_DIR := $(TARGET_DIR)/userland-win64
WIN64_APPS := winclock winpad

userland-win64: $(WIN_LIB_DIR)/stamp64
	@mkdir -p $(ROOT_DIR)/userland
	@set -e; for app in $(WIN64_APPS); do \
		echo "  [userland] $$app.exe (PE32+, taban 0x140000000)"; \
		( cd $(USERLAND_DIR) && cargo $(JSON_TARGET_FLAG) rustc --release --bin $$app \
			--target $(ROOT_DIR)/targets/x86_64-tcmk-win.json \
			--target-dir $(WIN64_TARGET_DIR) \
			-- -L $(WIN_LIB_DIR)/x64 ); \
		cp $(WIN64_TARGET_DIR)/x86_64-tcmk-win/release/$$app $(ROOT_DIR)/userland/$$app.exe64; \
	done

# 64-bit ithal kutuphaneleri: ayni .def dosyalari, `-m i386:x86-64`.
#
# Tek fark `@N` suslemelerinin kirpilmasi. Win64'te `__stdcall` diye bir
# sey yoktur -- yigini her zaman cagiran temizler -- ve `@4` gibi bir ek
# adin parcasi sayilip cozulemeyen sembol olur. i386 tarafinda bu isi
# `--kill-at` yapar; llvm-dlltool o bayragi x86-64 hedefinde yok saydigi
# icin burada .def sed ile suslemesiz hale getirilir. Boylece iki mimari
# tek bir .def dosyasindan beslenir ve ordinaller birbirinden kaymaz.
$(WIN_LIB_DIR)/stamp64: $(patsubst %,$(USERLAND_DIR)/win/%.def,$(WIN_DEFS))
	$(call need,llvm-dlltool,PE32+ ithal kutuphanelerini .def dosyasindan uretir,sudo apt install llvm)
	@mkdir -p $(WIN_LIB_DIR)/x64
	@set -e; for d in $(WIN_DEFS); do \
		sed -E 's/^([A-Za-z_][A-Za-z0-9_]*)@[0-9]+/\1/' \
			$(USERLAND_DIR)/win/$$d.def > $(WIN_LIB_DIR)/x64/$$d.def; \
		llvm-dlltool -m i386:x86-64 \
			-d $(WIN_LIB_DIR)/x64/$$d.def \
			-l $(WIN_LIB_DIR)/x64/$$d.lib; \
		echo "  [winlib64] $$d.lib"; \
	done
	@touch $(USERLAND_DIR)/src/win/*.rs
	@touch $@

# Elle (Python ile) uretilen ikililer. Ikisi de yukleyicilerin **en dar
# yolunu** sinar: derleyicinin urettigi zengin ikililerin aksine burada
# ne oldugu bayt bayt bilinir, yani bir sorun ciktiginda hatanin
# yukleyicide mi ikilide mi oldugu tartisilmaz.
#   * PE32  -- import tablosuz, tek bolumlu, elle kodlanmis en kucuk PE.
#              Derlenmis PE icin bkz. `userland-win` (winclock.exe).
#   * ELF64 -- Rust userland'i su an yalnizca i386'yi hedefler; x86_64
#              tarafinda cekirdek ELF64 yukleyicisi bu ikiliyle dogrulanir.
userland-legacy:
	$(call need,python3,elle uretilen en kucuk PE32/ELF64 ikilileri,sudo apt install python3)
	python3 $(ROOT_DIR)/tools/gen_pe_hello.py $(ROOT_DIR)/userland/hello.exe
	python3 $(ROOT_DIR)/tools/gen_hello_elf64.py $(ROOT_DIR)/userland/hello64.elf

info:
	@echo "ARCH        = $(ARCH)"
	@echo "MODE        = $(MODE)"
	@echo "Rust target = $(TARGET_JSON)"
	@echo "GRUB komutu = $(GRUB_CMD)"
	@echo "QEMU        = $(QEMU)"
	@echo "Kernel ELF  = $(KERNEL_ELF)"
	@echo "cargo -Z    = $(JSON_TARGET_FLAG)"
	@echo "cargo       = $(shell cargo --version 2>&1 | head -1)"
	@echo "ISO         = $(ISO)"

clean:
	rm -rf $(TARGET_DIR) $(ROOT_DIR)/build
