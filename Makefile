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
.PHONY: all iso run info clean userland

all:
	cd $(KERNEL_DIR) && cargo build $(CARGO_FLAGS) \
		--target $(TARGET_JSON) --target-dir $(TARGET_DIR)

iso: all
	mkdir -p $(ISO_DIR)/boot/grub
	cp $(KERNEL_ELF) $(ISO_DIR)/boot/tcmk-kernel.elf
	printf 'set timeout=0\nset default=0\n\nmenuentry "The Cursed Moon Kernel ($(ARCH))" {\n    $(GRUB_CMD) /boot/tcmk-kernel.elf\n    boot\n}\n' \
		> $(ISO_DIR)/boot/grub/grub.cfg
	grub-mkrescue -o $(ISO) $(ISO_DIR)

run: iso
	$(QEMU) -cdrom $(ISO) -serial stdio

# Ring 3 test ikilileri (ELF32 + PE32). Cekirdege include_bytes! ile gomulur.
userland:
	python3 $(ROOT_DIR)/tools/gen_hello_elf.py $(ROOT_DIR)/userland/hello.elf
	python3 $(ROOT_DIR)/tools/gen_pe_hello.py $(ROOT_DIR)/userland/hello.exe

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
