# Experimental browser build of FEC Viewer. See the README's "Browser demo".
#
#   make -C web serve            # release build, then serve on :8000
#   make -C web serve PROFILE=dev
#
# Needs nightly Rust with the wasm32-unknown-unknown target (gpui's web
# backend uses unstable features), and wasm-bindgen-cli matching the
# wasm-bindgen version in Cargo.lock. wasm-opt (binaryen) is used if present.

CARGO ?= cargo +nightly
PROFILE ?= release
PORT ?= 8000
DIST := dist
TARGET := wasm32-unknown-unknown
OUT_DIR := $(if $(filter dev,$(PROFILE)),debug,$(PROFILE))
WASM := ../target/$(TARGET)/$(OUT_DIR)/fec-viewer.wasm

.PHONY: build wasm serve clean

build: wasm
	cp index.html 1944135.fec $(DIST)/

wasm:
	cd .. && $(CARGO) rustc --bin fec-viewer --target $(TARGET) --profile $(PROFILE) \
		-- -C link-arg=-zstack-size=8388608
	wasm-bindgen $(WASM) --out-dir $(DIST) --target web --no-typescript
	@if command -v wasm-opt >/dev/null && [ "$(PROFILE)" != dev ]; then \
		echo "wasm-opt -O2"; \
		wasm-opt -O2 --enable-bulk-memory --enable-nontrapping-float-to-int \
			$(DIST)/fec-viewer_bg.wasm -o $(DIST)/fec-viewer_bg.wasm; \
	fi
	@ls -l $(DIST)/fec-viewer_bg.wasm

serve: build
	@echo "Serving http://localhost:$(PORT)/"
	python3 -m http.server --directory $(DIST) $(PORT)

clean:
	rm -rf $(DIST)
