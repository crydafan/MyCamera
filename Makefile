CARGO_NDK_TARGETS	= armeabi-v7a arm64-v8a
CARGO_NDK_OUTPUT_PATH	= $(shell pwd)/app/src/main/jniLibs

all: build_native

build_native:
	@cd raw_processor && cargo ndk build --release

.PHONY: all build_native
