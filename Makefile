APP := manty-imap-desktop

.PHONY: install dev test fmt clippy clean \
        package package-darwin package-darwin-universal \
        package-linux package-windows package-all \
        targets-install icon

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

package-darwin:
	npm run tauri build -- --target aarch64-apple-darwin
	npm run tauri build -- --target x86_64-apple-darwin

package-darwin-universal:
	npm run tauri build -- --target universal-apple-darwin

package-linux:
	npm run tauri build -- --target x86_64-unknown-linux-gnu

package-windows:
	npm run tauri build -- --target x86_64-pc-windows-gnu

package-all: package-darwin package-linux package-windows

targets-install:
	rustup target add aarch64-apple-darwin x86_64-apple-darwin \
	                  x86_64-unknown-linux-gnu \
	                  x86_64-pc-windows-gnu
