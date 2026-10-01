# Browser demo (experimental)

The same gpui views as the desktop app, compiled to WebAssembly and drawn
into a canvas through gpui's web backend (WebGPU, with WebGL2 as the
fallback). Drop a `.fec` file on the page, use "Choose a file…", or use
"Load sample filing" (`1944135.fec`, bundled here). `?file=<url>` opens a
filing from the server at startup.

## Build and run

One-time setup:

```sh
rustup toolchain install nightly
rustup +nightly target add wasm32-unknown-unknown
# Must match the wasm-bindgen version in Cargo.lock:
cargo install wasm-bindgen-cli --version 0.2.129 --locked
# Optional, shrinks the bundle: brew install binaryen
```

Then, from the repo root:

```sh
make -C web serve              # release build, then http://localhost:8000/
make -C web serve PROFILE=dev  # faster build, much bigger and slower
```

`web/dist/` then holds plain static files that any static host can serve.
The build needs nightly only because `gpui_web` pulls in `wasm_thread`. The
native build stays on stable.

## Limitations

- One window: opening a filing replaces the current one. There are no
  native menus, tabs or dialogs.
- Everything runs on the page's main thread. Count and collect passes run in
  ~12 ms slices between frames, so very large filings take longer than they
  do natively, but the page stays responsive.
- The whole file is held in memory.
- The app draws only the bundled fonts (IBM Plex Sans, JetBrains Mono; see
  `fonts/OFL.txt`). Emoji and CJK text fall back to the browser's fonts.
- Text is drawn on a canvas, so browser find, text selection and screen
  readers don't see it.
