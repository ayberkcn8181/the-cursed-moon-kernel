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

TARGET_JSON := $(ROOT_DIR)/targets/$(RUST_TARGET).json
KERNEL_ELF := $(TARGET_DIR)/$(RUST_TARGET)/$(CARGO_OUT_DIR)/tcmk-kernel

.DEFAULT_GOAL := all
.PHONY: all iso run disk run-disk info clean bootloader userland userland-rust userland-legacy

# Cekirdek 1. asamayi include_bytes! ile gomdugu icin onyukleyici
# cekirdekten ONCE uretilmelidir.
all: bootloader
	cd $(KERNEL_DIR) && cargo build $(CARGO_FLAGS) \
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
	@mkdir -p $(BOOT_OUT)
	@set -e; for s in stage1 stage2; do \
		( cd $(BOOT_DIR) && cargo rustc --release --bin $$s \
			--target $(ROOT_DIR)/targets/i686-tcmk.json \
			--target-dir $(BOOT_TARGET_DIR) \
			-- -C link-arg=-T$$s.ld -C link-arg=--oformat=binary ); \
		cp $(BOOT_TARGET_DIR)/i686-tcmk/release/$$s $(BOOT_OUT)/$$s.bin; \
		echo "  [boot] $$s.bin $$(stat -c%s $(BOOT_OUT)/$$s.bin) bayt"; \
	done

iso: all
	mkdir -p $(ISO_DIR)/boot/grub
	cp $(KERNEL_ELF) $(ISO_DIR)/boot/tcmk-kernel.elf
	printf 'set timeout=0\nset default=0\n\nmenuentry "The Cursed Moon Kernel ($(ARCH))" {\n    $(GRUB_CMD) /boot/tcmk-kernel.elf\n    boot\n}\n' \
		> $(ISO_DIR)/boot/grub/grub.cfg
	grub-mkrescue -o $(ISO) $(ISO_DIR)

run: iso
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
	python3 $(ROOT_DIR)/tools/make_disk.py $(ISO) $(DISK) $(DISK_MIB) \
		$(BOOT_OUT)/stage2.bin $(KERNEL_ELF)

# Diskten acilis (CD yok): kalicilik ancak boyle dogrulanabilir.
run-disk: disk
	$(QEMU) -drive file=$(DISK),format=raw,if=ide -serial stdio

# --- Ring 3 uygulamalari ------------------------------------------------
#
# Cekirdekte henuz surec basina adres uzayi yok (doc Faz 9+): tum Ring 3
# uygulamalari ayni 2 MiB'lik kullanici bolgesini paylasir. Bu yuzden her
# uygulama kendi 256 KiB'lik "slotuna" linklenir. Slot tabani `cargo rustc`
# ile YALNIZCA ikili hedefe verilir; boylece kutuphane ve `core` bir kez
# derlenir, uygulama basina yalnizca baglama tekrarlanir.
USERLAND_DIR := $(ROOT_DIR)/userland-rs
USERLAND_TARGET_DIR := $(TARGET_DIR)/userland
USER_REGION := 0x00C00000
SLOT_SIZE   := 0x40000

# ad:slot
USER_APPS := hello:0 paint:1 plasma:2 crash:3 hog:4

userland: userland-rust userland-legacy

userland-rust:
	@mkdir -p $(ROOT_DIR)/userland
	@set -e; for entry in $(USER_APPS); do \
		app=$${entry%%:*}; slot=$${entry##*:}; \
		base=$$(printf '0x%08x' $$(( $(USER_REGION) + slot * $(SLOT_SIZE) ))); \
		echo "  [userland] $$app -> slot $$slot (taban $$base)"; \
		( cd $(USERLAND_DIR) && cargo rustc --release --bin $$app \
			--target $(ROOT_DIR)/targets/i686-tcmk.json \
			--target-dir $(USERLAND_TARGET_DIR) \
			-- -C link-arg=--image-base=$$base ); \
		cp $(USERLAND_TARGET_DIR)/i686-tcmk/release/$$app $(ROOT_DIR)/userland/$$app.elf; \
	done

# Elle uretilen ikililer:
#   * PE32  -- Rust'in i686-pc-windows hedefi bir Windows toolchain'i
#              gerektirir; TCMK'nin arac zinciri bilerek Rust + GRUB + QEMU
#              ile sinirli tutulmustur (bkz. README).
#   * ELF64 -- Rust userland'i su an yalnizca i386 slot modelini destekler;
#              x86_64 tarafinda cekirdek ELF64 yukleyicisi bu ikiliyle
#              dogrulanir.
userland-legacy:
	python3 $(ROOT_DIR)/tools/gen_pe_hello.py $(ROOT_DIR)/userland/hello.exe
	python3 $(ROOT_DIR)/tools/gen_hello_elf64.py $(ROOT_DIR)/userland/hello64.elf

info:
	@echo "ARCH        = $(ARCH)"
	@echo "MODE        = $(MODE)"
	@echo "Rust target = $(TARGET_JSON)"
	@echo "GRUB komutu = $(GRUB_CMD)"
	@echo "QEMU        = $(QEMU)"
	@echo "Kernel ELF  = $(KERNEL_ELF)"
	@echo "ISO         = $(ISO)"

clean:
	rm -rf $(TARGET_DIR) $(ROOT_DIR)/build
