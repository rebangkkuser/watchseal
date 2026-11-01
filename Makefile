# Host build (tests):   make
# Android build:        make CC=aarch64-linux-android24-clang
# Override runtime paths for testing, e.g.:
#   make CPPFLAGS='-DWS_LOG_PATH=\"/tmp/ws/sealmsg\" -DWS_PID_PATH=\"/tmp/ws/sealmsg.pid\" -DWS_LOCK_PATH=\"/tmp/ws/sealmsg.lock\"'

CC       ?= clang
CFLAGS   ?= -O2 -Wall -Wextra -std=gnu11
COMMON   := src/common.c src/i18n.c
MODULEDIR ?= magisk
WATCHSEAL_BINDIR := $(MODULEDIR)/system/lib64/watchseal
ZIPFILE ?= watchseal.zip

all: watchseald watchsealctl zip

watchseald: src/watchseald.c $(COMMON) src/common.h src/i18n.h
	$(CC) $(CPPFLAGS) $(CFLAGS) $(LDFLAGS) -o $@ src/watchseald.c $(COMMON)

watchsealctl: src/watchsealctl.c src/sources.c $(COMMON) src/common.h src/i18n.h src/sources.h
	$(CC) $(CPPFLAGS) $(CFLAGS) $(LDFLAGS) -o $@ src/watchsealctl.c src/sources.c $(COMMON)

clean:
	rm -f watchseald watchsealctl

.PHONY: all clean zip

zip: watchseald watchsealctl module.prop
	mkdir -p $(WATCHSEAL_BINDIR)
	mkdir -p $(MODULEDIR)/system/bin
	cp module.prop $(MODULEDIR)/module.prop
	cp watchseald $(WATCHSEAL_BINDIR)/watchseald
	cp watchsealctl $(WATCHSEAL_BINDIR)/watchsealctl
	ln -sf ../lib64/watchseal/watchseald $(MODULEDIR)/system/bin/watchseald
	ln -sf ../lib64/watchseal/watchsealctl $(MODULEDIR)/system/bin/watchsealctl
	cd $(MODULEDIR) && zip -r -FS ../$(ZIPFILE) .
