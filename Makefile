ARCH ?= i386
MODE ?= release

ROOT_DIR := $(abspath .)
KERNEL_DIR := $(ROOT_DIR)/kernel
TARGET_DIR := $(ROOT_DIR)/target
ISO_DIR := $(ROOT_DIR)/build/isodir
ISO := $(ROOT_DIR)/build/tcmk.iso
CARGO_FLAGS :=
CARGO_OUT_DIR := debug
ifeq ($(MODE),release)
CARGO_FLAGS += --release
CARGO_OUT_DIR := release
endif

KERNEL_ELF := $(TARGET_DIR)/i686-tcmk/$(CARGO_OUT_DIR)/tcmk-kernel

.PHONY: all iso run info clean userland

# Ring 3 test ikilileri (ELF32 + PE32). Cekirdege include_bytes! ile gomulur.
userland:
	python3 $(ROOT_DIR)/tools/gen_hello_elf.py $(ROOT_DIR)/userland/hello.elf
	python3 $(ROOT_DIR)/tools/gen_pe_hello.py $(ROOT_DIR)/userland/hello.exe

all:
ifneq ($(ARCH),i386)
	$(error Faz 1 sadece ARCH=i386 destekliyor)
endif
	cd $(KERNEL_DIR) && cargo build $(CARGO_FLAGS) --target-dir $(TARGET_DIR)

iso: all
	mkdir -p $(ISO_DIR)/boot/grub
	cp $(KERNEL_ELF) $(ISO_DIR)/boot/tcmk-kernel.elf
	cp $(ROOT_DIR)/boot/grub/grub.cfg $(ISO_DIR)/boot/grub/grub.cfg
	grub-mkrescue -o $(ISO) $(ISO_DIR)

run: iso
	qemu-system-i386 -cdrom $(ISO) -serial stdio

info:
	@echo "ARCH        = $(ARCH) (Faz 1: sadece i386 desteklenir)"
	@echo "MODE        = $(MODE)"
	@echo "Target spec = targets/i686-tcmk.json"
	@echo "Kernel ELF  = $(KERNEL_ELF)"
	@echo "ISO         = $(ISO)"

clean:
	rm -rf $(TARGET_DIR) $(ROOT_DIR)/build
