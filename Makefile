# Host build (tests):   make
# Android build:        make CC=aarch64-linux-android24-clang
# Override runtime paths for testing, e.g.:
#   make CPPFLAGS='-DWS_LOG_PATH=\"/tmp/ws/sealmsg\" -DWS_PID_PATH=\"/tmp/ws/sealmsg.pid\" -DWS_LOCK_PATH=\"/tmp/ws/sealmsg.lock\"'

CC       ?= clang
CFLAGS   ?= -O2 -Wall -Wextra -std=gnu11
COMMON   := src/common.c src/i18n.c

all: watchseald watchsealctl

watchseald: src/watchseald.c $(COMMON) src/common.h src/i18n.h
	$(CC) $(CPPFLAGS) $(CFLAGS) $(LDFLAGS) -o $@ src/watchseald.c $(COMMON)

watchsealctl: src/watchsealctl.c src/sources.c $(COMMON) src/common.h src/i18n.h src/sources.h
	$(CC) $(CPPFLAGS) $(CFLAGS) $(LDFLAGS) -o $@ src/watchsealctl.c src/sources.c $(COMMON)

clean:
	rm -f watchseald watchsealctl

.PHONY: all clean
