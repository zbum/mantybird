APP := manty-imap-desktop
UNAME_S := $(shell uname -s)

.PHONY: install dev test fmt clippy clean \
        package package-darwin package-darwin-universal \
        package-linux package-windows package-all package-cross-all \
        require-darwin require-linux require-windows \
        target-darwin-aarch64 target-darwin-x86_64 target-linux-x86_64 \
        target-windows-x86_64 targets-install icon

install:
	npm install

dev:
	npm run tauri dev

test:
	cd src-tauri && cargo test

fmt:
	cd src-tauri && cargo fmt

clippy:
	cd src-tauri && cargo clippy --all-targets -- -D warnings

clean:
	cd src-tauri && cargo clean
	rm -rf dist node_modules

# Regenerate icon variants from src-tauri/icons/icon.png
icon:
	npx tauri icon src-tauri/icons/icon.png

# --- Distribution packaging ----------------------------------------------------
# All bundle artifacts land in src-tauri/target/<triple>/release/bundle/.
# For .dmg/.deb/.msi etc. you must build on (or cross-build to) the target OS.

package:
	npm run tauri build

package-darwin: require-darwin target-darwin-aarch64 target-darwin-x86_64
	npm run tauri build -- --target aarch64-apple-darwin
	npm run tauri build -- --target x86_64-apple-darwin

package-darwin-universal: require-darwin target-darwin-aarch64 target-darwin-x86_64
	npm run tauri build -- --target universal-apple-darwin

package-linux: require-linux target-linux-x86_64
	npm run tauri build -- --target x86_64-unknown-linux-gnu

package-windows: require-windows target-windows-x86_64
	npm run tauri build -- --target x86_64-pc-windows-gnu

ifeq ($(UNAME_S),Darwin)
package-all: package-darwin
else ifeq ($(UNAME_S),Linux)
package-all: package-linux
else
package-all: package-windows
endif

package-cross-all: target-darwin-aarch64 target-darwin-x86_64 \
                   target-linux-x86_64 target-windows-x86_64
	$(MAKE) ALLOW_CROSS=1 package-darwin
	$(MAKE) ALLOW_CROSS=1 package-linux
	$(MAKE) ALLOW_CROSS=1 package-windows

require-darwin:
	@if [ "$(ALLOW_CROSS)" != "1" ] && [ "$(UNAME_S)" != "Darwin" ]; then \
		echo "package-darwin must run on macOS, or set ALLOW_CROSS=1 with a configured cross toolchain."; \
		exit 1; \
	fi

require-linux:
	@if [ "$(ALLOW_CROSS)" != "1" ] && [ "$(UNAME_S)" != "Linux" ]; then \
		echo "package-linux must run on Linux, or set ALLOW_CROSS=1 with Linux sysroot/pkg-config configured."; \
		exit 1; \
	fi

require-windows:
	@if [ "$(ALLOW_CROSS)" != "1" ] && [ "$(OS)" != "Windows_NT" ]; then \
		echo "package-windows must run on Windows, or set ALLOW_CROSS=1 with a configured cross toolchain."; \
		exit 1; \
	fi

target-darwin-aarch64:
	rustup target add aarch64-apple-darwin

target-darwin-x86_64:
	rustup target add x86_64-apple-darwin

target-linux-x86_64:
	rustup target add x86_64-unknown-linux-gnu

target-windows-x86_64:
	rustup target add x86_64-pc-windows-gnu

targets-install: target-darwin-aarch64 target-darwin-x86_64 \
                 target-linux-x86_64 target-windows-x86_64
