# R5 — Desktop application stack for the live 6-max NLHE assistant

Date: 2026-09-10. Scope: single-user Windows 11 app; fast keyboard entry of table state; compute-heavy Rust/C++ solver (1–15 s, possibly GPU) running off the UI thread with progress; possible later exposure to a phone browser on the LAN. Installed: Python 3.12, Node 24; Rust can be installed.

Everything below is either cited or marked "not verified". Sizes/versions are as found on 2026-09-10.

---

## 0. Scorecard

Scale: A (best) … D. Criteria: (a) speed of building a keyboard-driven entry UI, (b) embedding a Rust/C++ solver off-thread with progress, (c) GPU from the solver, (d) single runnable Windows package, (e) how well AI coding agents work with it, (f) UI testability (unit + E2E).

| Stack | a | b | c | d | e | f | Notes |
|---|---|---|---|---|---|---|---|
| Tauri 2 (Rust + web UI) | A | A | A | A | B+ | B+ | Solver is a plain Rust crate in the same workspace; ~MB-size app; LLMs sometimes emit Tauri v1 code |
| Electron (Node + web UI) | A | B+ | A | B | A | A | Solver as child process (no ABI issues) or napi-rs addon; 100–250 MB bundle |
| Python + PySide6/Qt | B | B+ | A | B | B | B | PyO3 + QThread works well; packaging heavier; E2E tooling weaker |
| Python FastAPI/Flask + browser | B− | B | A | C | A | A | Browser-reserved shortcuts and tab management fight a hotkey UI; best E2E (Playwright); LAN trivial |
| egui/eframe (pure Rust) | B+ | A | A | A | B− | B | Fastest to a keyboard-first panel; least polished look; API churn between versions |
| Flutter + flutter_rust_bridge | B | B+ | A | B+ | B− | B+ | Adds Dart; fine but no advantage here |
| WinUI 3 / .NET MAUI | B− | B | A | B | B− | C | Windows-only, MSIX/VS-centric, weak E2E tooling |
| Slint / iced / Dioxus | B | A | A | A | C+ | C+ | Smaller communities; Dioxus native renderer still young |

---

## 1. Tauri 2.x (Rust backend + web frontend)

Status: v2 stable since 2024-10-02 (https://v2.tauri.app/blog/tauri-20/); the `tauri` crate is at 2.11.x on docs.rs (https://docs.rs/crate/tauri/latest).

(a) Keyboard-driven UI. The UI is ordinary HTML/TS in WebView2 (Chromium-based on Windows), so card hotkeys ("As Kd"), a stack/position command line, and an undo stack are standard DOM `keydown` handling plus an immutable state history — no framework-specific work. Two Windows caveats:
- WebView2 enables browser accelerators (F5, Ctrl+P, Ctrl+F, F12, zoom) by default; a Tauri feature request to switch them off via config is still open (https://github.com/tauri-apps/tauri/issues/7418). Workarounds: `preventDefault()` in a top-level keydown handler, or wry's Windows-only `with_browser_accelerator_keys(false)` (added in wry 0.23.2: https://v2.tauri.app/release/wry/v0.23.2/). Whether Tauri's `WebviewWindowBuilder` exposes that wry option directly: not verified.
- In-window shortcuts are JS listeners; the global-shortcut plugin is only needed for shortcuts while another window (e.g., the poker client) is focused (https://dev.to/hiyoyok/global-keyboard-shortcuts-in-tauri-v2-the-right-way-and-the-wrong-way-2h6d).

(b) Solver embedding and long solves. This is Tauri's strongest point for this project:
- The solver is a normal crate in the same Cargo workspace; a `#[tauri::command]` calls it directly. Commands marked `async` run "on a separate async task using `async_runtime::spawn`"; non-async commands "are executed on the main thread unless defined with `#[tauri::command(async)]`" (https://v2.tauri.app/develop/calling-rust/). Tauri's runtime is Tokio and exposes `spawn`, `spawn_blocking`, `block_on` (https://docs.rs/tauri/latest/tauri/async_runtime/index.html).
- For a CPU-bound solve, do not run it on the Tokio worker threads: Tokio's docs say `spawn_blocking` is for bounded blocking work, "Specialized CPU-bound executors, such as rayon, may also be a good fit", and long-running loops belong on "a dedicated thread created with `thread::spawn`" (https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html). Concretely: `std::thread::spawn` (or a rayon pool) inside the command, cancellation via `AtomicBool`/`CancellationToken`.
- Progress: `tauri::ipc::Channel<T>` — "the recommended mechanism for streaming data ... to the frontend" (https://v2.tauri.app/develop/calling-rust/); the event system also works (https://v2.tauri.app/develop/calling-frontend/).
- C++ core instead of Rust: wrap with the `cxx` crate or `bindgen` and link statically; same command surface. Not exercised here (not verified for this project).
- Process isolation, if wanted (e.g., a GPU driver fault must not kill the UI): ship the solver as a sidecar binary and read its stdout as `CommandEvent::Stdout` lines via the shell plugin, with `shell:allow-spawn` permission (https://v2.tauri.app/develop/sidecar/).

(c) GPU. Independent of Tauri — it is whatever the Rust crate links (see §6).

(d) Packaging. `tauri build` produces an NSIS `-setup.exe` and/or a WiX MSI (https://v2.tauri.app/distribute/windows-installer/). WebView2 "is distributed as part of the operating system" on Windows 10 (April 2018+) and Windows 11 (same page; Microsoft: "The Evergreen WebView2 Runtime will be included as part of the Windows 11 operating system", https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution). Installer size impact: downloadBootstrapper 0 MB, embedBootstrapper ~1.8 MB, offlineInstaller ~127 MB. Frontend assets are compiled into the binary, so the bare `target/release/*.exe` is runnable without an installer, but then "deep links, file associations, sidecar, resources, updater" are unavailable (maintainer, https://github.com/tauri-apps/tauri/discussions/11167). Measured example bundle: 8.6 MiB Tauri vs 244 MiB Electron (macOS; https://www.gethopp.app/blog/tauri-vs-electron) — Windows numbers not verified.

(e) AI coding agents. Frontend is mainstream TS/React. Known pitfall: LLMs "frequently suggest code from v1" of Tauri; the docs team has open requests for `llms.txt` (https://github.com/tauri-apps/tauri-docs/issues/3668, https://github.com/tauri-apps/tauri-docs/issues/3144). Mitigation: pin `tauri = "2"`, keep a short v2 cheat-sheet (commands, `Channel`, capabilities/permissions, `@tauri-apps/api/core` `invoke`) in CLAUDE.md, and let `cargo check` reject v1 idioms. Rust's strict compiler gives agents a tight feedback loop (my judgment, not verified by data).

(f) Testability.
- Frontend unit: `@tauri-apps/api/mocks` — `mockIPC()` intercepts `invoke`, `clearMocks()` between tests; examples use Vitest (https://v2.tauri.app/develop/tests/mocking/).
- Rust: `cargo test` on the solver crate; `tauri::test` (crate feature `test`) provides `MockRuntime`, `mock_builder()`, `get_ipc_response()`/`assert_ipc_response()` to test commands without a window (https://docs.rs/tauri/latest/tauri/test/index.html).
- E2E: `tauri-driver` + Microsoft Edge WebDriver on Windows ("only Windows and Linux are supported on desktop", https://v2.tauri.app/develop/tests/webdriver/); `@wdio/tauri-service` automates driver setup, default provider "Embedded" (https://webdriver.io/docs/wdio-tauri-service/). Playwright alternative: WebView2 accepts `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` and Playwright attaches via `connectOverCDP` (https://playwright.dev/docs/webview2); whether this coexists with Tauri's own browser args is not verified.

---

## 2. Electron (Node backend + web frontend)

Status: Electron 43.x per Wikipedia (July 2026; https://en.wikipedia.org/wiki/Electron_(software_framework)) — not verified against electronjs.org.

(a) Keyboard-driven UI. Same web UI as Tauri, plus better key control: menu `accelerator`s, renderer `keydown`, and `webContents` `before-input-event` which "is emitted before dispatching keydown and keyup events in the renderer" (https://www.electronjs.org/docs/latest/tutorial/keyboard-shortcuts). Consistent Chromium rendering, no WebView2 accelerator issue.

(b) Solver embedding. Three options, in order of least friction:
1. Child process: Rust solver binary speaking JSON lines over stdio (`child_process.spawn`); no ABI concerns; progress is just lines on stdout. Electron's `utilityProcess` is the Chromium-managed equivalent of `child_process.fork` and can pass MessagePorts straight to the renderer (https://www.electronjs.org/docs/latest/api/utility-process).
2. Native addon via napi-rs: `AsyncTask` runs `compute` on the libuv thread pool; `ThreadsafeFunction` lets any Rust thread call a JS callback (progress) with `'static` payloads (https://napi.rs/docs/concepts/async-task, https://napi.rs/docs/concepts/threadsafe-function). Electron officially lists napi-rs/neon/node-bindgen for Rust (https://www.electronjs.org/docs/latest/tutorial/native-code-and-electron). Caveat: Electron says native modules "will need to be recompiled for Electron" because its ABI differs from Node (https://www.electronjs.org/docs/latest/tutorial/using-native-node-modules); Node-API is ABI-stable across Node versions (https://nodejs.org/api/n-api.html). Whether a napi-rs addon built for Node 24 loads unmodified in Electron: not verified — budget for `@electron/rebuild` (Forge runs it automatically).
3. `worker_threads` in the main process for JS-side work only.

(c) GPU. Same as §6; prefer option 1 (separate process) if using CUDA, so a device fault cannot take down the UI process.

(d) Packaging. electron-builder targets NSIS, portable exe, MSI, AppX (https://www.electron.build/); Electron Forge is the official all-in-one (https://www.electronjs.org/docs/latest/tutorial/tutorial-packaging). Bundles ship Chromium + Node: 244 MiB measured vs 8.6 MiB for Tauri (macOS, https://www.gethopp.app/blog/tauri-vs-electron); ~100 MB+ is typical on Windows (not verified). Irrelevant for a single user, but startup and RAM are higher.

(e) AI coding agents. The most mainstream option: JavaScript is used by 66% of respondents in the 2025 Stack Overflow survey (https://survey.stackoverflow.co/2025/technology); Electron's API has been stable for years, so agent output is rarely version-confused.

(f) Testability. Vitest/Jest for the renderer; Playwright has "experimental support for Electron automation" via `_electron.launch` (https://playwright.dev/docs/api/class-electron; Electron's own guide: https://www.electronjs.org/docs/latest/tutorial/automated-testing); WebdriverIO also supported. Best-documented E2E story of the native-shell options.

---

## 3. Python + PySide6/Qt (solver via PyO3/pybind11 or subprocess)

Status: PySide6 6.11.2 (2026-08-18), license "LGPL-3.0-only OR GPL-2.0-only OR GPL-3.0-only" (https://pypi.org/project/PySide6/). LGPL is fine for a private, dynamically linked app.

(a) Keyboard-driven UI. Qt widgets have `QShortcut`/`QKeySequence`, validators, and full key control without browser interference. Building a dense grid-of-six-seats panel is fast in Python; visual polish (custom card widgets, animations) is more work than CSS. Undo is app-level (or `QUndoStack`).

(b) Solver embedding. PyO3 + maturin: release the interpreter lock around the solve with `Python::detach` (renamed from `allow_threads` in PyO3 0.26; https://pyo3.rs/latest/parallelism.html), run it in a `QThread`/`QThreadPool`, report progress with Qt signals. Rust-side rayon works as long as the call is wrapped in `detach` (same page). C++ core: pybind11/nanobind. Or run the solver as a subprocess (JSON over stdio) — same as the Electron path. Python 3.12 is already installed.

(c) GPU. Same as §6 (Rust crate) — Python's own CuPy/Numba are options but unnecessary.

(d) Packaging. `pyside6-deploy` (wrapper around Nuitka) or PyInstaller (https://doc.qt.io/qtforpython-6/deployment/deployment-pyside6-deploy.html, https://doc.qt.io/qtforpython-6/deployment/deployment-pyinstaller.html). PyInstaller one-file "is a little slower to start than a one-folder app" because it extracts to a temp folder (https://pyinstaller.org/en/stable/operating-mode.html). Resulting size tens to 100+ MB (Qt DLLs) — exact figure not verified. Needs the compiled Rust extension (`.pyd`) bundled.

(e) AI coding agents. Python is the fastest-growing language in the 2025 survey (https://survey.stackoverflow.co/2025/technology); PySide6 is documented but agents commonly mix PyQt5/PyQt6/PySide2 signal and enum syntax (anecdotal, not verified).

(f) Testability. pytest-qt: `qtbot`, `waitSignal` for threads ("blocks until a signal is emitted or 10 seconds has elapsed"; https://pytest-qt.readthedocs.io/en/latest/signals.html). There is no Playwright-class E2E tool for Qt widgets; tests drive widgets in-process.

---

## 4. Python FastAPI/Flask backend + browser UI (localhost), no native shell

(a) Keyboard-driven UI. Web UI as in §1/§2, but inside a real browser: tab/window chrome, and some accelerators (Ctrl+W/Ctrl+T/Ctrl+N, F11) cannot be intercepted by page JS in Chromium-based browsers (widely reported; not verified against a spec). An accidental Ctrl+W closes the assistant mid-hand. `msedge.exe --app=http://localhost:PORT` gives a chromeless window (https://textslashplain.com/2022/01/05/edge-command-line-arguments/) but browser accelerators remain. pywebview is the middle ground (native window around the local server; https://pywebview.flowrl.com/).

(b) Solver. PyO3 extension in a worker process (`ProcessPoolExecutor`) or a subprocess; progress via WebSocket/SSE. Works, but adds a network/serialization hop for every keystroke-driven state update.

(c) GPU. Same as §6.

(d) Packaging. PyInstaller for the server + a launcher that opens the browser (example: https://github.com/iancleary/pyinstaller-fastapi). Two moving parts (server process + browser), no single app window, port conflicts to handle.

(e) AI coding agents. Excellent — FastAPI + React is among the most common agent-generated stacks (my judgment, not verified by data).

(f) Testability. Best of all: pytest + `TestClient` for the API, Playwright against the real browser for E2E (https://playwright.dev/).

LAN exposure: trivial — bind `0.0.0.0`. This is the option's main structural advantage, but §7 shows the same is achievable from Tauri without a rewrite.

---

## 5. Other credible options (brief)

- egui/eframe (pure Rust, immediate mode). Fastest path to a keyboard-first data-entry panel in a single ~few-MB exe; eframe defaults to wgpu rendering; AccessKit accessibility on Windows (https://github.com/emilk/egui). Official test harness `egui_kittest` (automation + snapshot tests; https://crates.io/crates/egui_kittest). Downsides: less polished visuals, immediate-mode layout quirks, and version churn (egui 0.36 now) that trips LLMs the way Tauri v1/v2 does (not verified). Would need a separate web UI later for the phone.
- iced: Elm-style, clean but "documentation has gaps" (https://wrenlearnsrust.com/posts/2026-03-11-rust-gui-landscape-2026.html). No advantage over egui here.
- Slint: declarative DSL; GPLv3 or a royalty-free license for proprietary desktop apps (https://slint.dev/pricing). Smaller ecosystem; agents less familiar.
- Dioxus 0.7 (2025-09-08): React-like Rust UI; desktop via webview; the new GPU-native renderer (Blitz) is "still very young" per the release post (https://dioxuslabs.com/blog/release-070/). Interesting, but a Tauri app with a TS frontend is the same architecture with a larger ecosystem.
- Flutter desktop + flutter_rust_bridge 2.13.0 (2026-08-23; https://github.com/fzyzcjy/flutter_rust_bridge/releases). Windows stable since Flutter 2.10 (https://docs.flutter.dev/platform-integration/desktop). Solid, but adds Dart for no gain.
- WinUI 3 / Windows App SDK (Microsoft's recommended native path, https://learn.microsoft.com/en-us/windows/apps/winui/winui3/) or .NET MAUI: C# P/Invoke into a Rust `cdylib`; packaged (MSIX) by default; Windows-only; E2E tooling (WinAppDriver) is weak (not verified). Not recommended.

---

## 6. GPU from the solver (common to every stack)

The UI shell never touches the GPU; the solver crate does. On Windows the options are:
- cudarc 0.19.9: safe bindings to the CUDA driver/NVRTC/cuBLAS APIs, supports CUDA 11.4–13.3 via feature flags, dynamic loading by default (https://crates.io/crates/cudarc). NVIDIA only.
- wgpu (v30.x in 2026): compute shaders in WGSL over Vulkan/DX12, vendor-neutral (https://rustify.rs/articles/rust-gpu-computing-wgpu-2026 — third-party summary; primary source https://github.com/gfx-rs/wgpu).
- cuda-oxide: NVIDIA Labs' rustc backend to write CUDA kernels in Rust; its docs say the PTX it produces can be launched with cudarc (https://nvlabs.github.io/cuda-oxide/appendix/ecosystem.html). Maturity/support guarantees: not verified.
- C++ core: plain CUDA C++ compiled with nvcc, linked into the Rust crate or the sidecar.

Practical rule: put the GPU path behind a Cargo feature (`--features cuda`) and keep the CPU path default. If CUDA is used, prefer running the solver in its own process (Tauri sidecar / Electron child) so a device fault cannot crash the UI — a recommendation, not verified for this codebase.

---

## 7. Exposing the same backend to a phone browser on the LAN later

Design once so the shell is thin:
1. `poker-core` = pure Rust library (state model, hand parser, solver), no Tauri/Electron types.
2. `poker-proto` = serde request/progress/result types shared by IPC and HTTP.
3. Desktop shell (Tauri commands) and a `poker-server` crate (axum: REST + WebSocket for progress) are both ~50-line adapters over `poker-core`.
4. Frontend talks to a `Backend` interface with two implementations: `invoke`/`Channel` (Tauri) and `fetch`/WebSocket (HTTP). The same Vite build is served by axum to the phone.

Running axum inside the Tauri process is done via `tokio::spawn` before `tauri::Builder::run` (https://github.com/tauri-apps/tauri/discussions/2751); maintainers caution to understand the exposure implications, which for a LAN-only, bind-on-demand server are acceptable. Sharing one codebase between Tauri and an axum web target is a known pattern (https://github.com/tauri-apps/tauri/discussions/11399). Expect a Windows Firewall prompt on first bind (not verified). Tauri 2 can also build the same frontend as an Android/iOS app (https://v2.tauri.app/), but the phone-browser route needs no app store and no Kotlin/Swift.

With Electron the equivalent is an Express/Fastify server in the main process over the same child-process solver; with the Python options it is native.

---

## 8. RECOMMENDATION

Primary: Tauri 2 + Vite/React/TypeScript frontend + Rust workspace (`poker-core` solver crate, optional `cuda`/`wgpu` features).
Why: the solver and the app are one Cargo workspace (no FFI, no ABI rebuilds); `Channel<T>` gives progress with three lines of code; the whole app is a ~10 MB installer that relies on the WebView2 already in Windows 11; unit tests on both sides (`mockIPC`, `tauri::test`) and Windows E2E via `@wdio/tauri-service` are documented; and the LAN/phone phase is an added axum crate, not a rewrite. The known costs — WebView2 accelerator keys (handle via `preventDefault`/wry option) and LLM v1/v2 confusion (handle via CLAUDE.md pinning) — are small and one-time. Run the solve on a dedicated thread or rayon pool, never on the Tokio workers.

Fallback: Electron + the same React/TS frontend + the same `poker-core` compiled as a standalone binary driven over stdio JSON lines (`child_process`/`utilityProcess`).
Why this fallback: it reuses the two expensive parts (frontend, solver) unchanged, has the most mainstream tooling for agents, Playwright E2E, and zero native-addon/ABI work. Choose it if Tauri's WebView2 quirks or Rust compile times prove annoying in the first week. (Python/PySide6 is viable but reuses nothing if abandoned; the browser-only option loses too much keyboard control for live play.)

Project layout (primary):

```
PokerAI/
├─ Cargo.toml                  # [workspace] members = crates/*, src-tauri
├─ crates/
│  ├─ poker-core/              # pure Rust: cards, hand parser ("AsKd"), ranges, solver; features: cuda, wgpu
│  ├─ poker-proto/             # serde types: TableState, SolveRequest, Progress, Recommendation
│  └─ poker-server/            # axum REST+WS over poker-core; serves ui/dist to the phone (phase 2)
├─ src-tauri/
│  ├─ src/{main.rs,lib.rs,commands.rs,solver_job.rs}   # commands = thin wrappers; solver on std::thread/rayon
│  ├─ capabilities/default.json
│  ├─ tauri.conf.json
│  └─ Cargo.toml
├─ ui/                          # Vite + React + TS
│  ├─ src/backend/{Backend.ts,tauriBackend.ts,httpBackend.ts}
│  ├─ src/state/{table.ts,history.ts,cardInput.ts}      # immutable state + undo stack + key parser
│  ├─ src/components/
│  └─ src/__tests__/            # vitest + @tauri-apps/api/mocks
├─ e2e/                         # WebdriverIO + @wdio/tauri-service (wdio.conf.ts)
├─ package.json
└─ CLAUDE.md                    # Tauri v2 API cheat-sheet, "no v1 idioms", build/test commands
```

First-week checklist: (1) `cargo test` on `poker-core` with a fixed-seed solve; (2) a `solve` command that spawns a thread and streams `Progress` over `Channel`; (3) keydown handler with `preventDefault` on F5/Ctrl+P/Ctrl+F and the card grammar; (4) `tauri build` NSIS installer to confirm packaging early.

---

## Sources

- https://v2.tauri.app/blog/tauri-20/
- https://docs.rs/crate/tauri/latest
- https://v2.tauri.app/develop/calling-rust/
- https://v2.tauri.app/develop/calling-frontend/
- https://docs.rs/tauri/latest/tauri/async_runtime/index.html
- https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html
- https://v2.tauri.app/develop/sidecar/
- https://v2.tauri.app/distribute/windows-installer/
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution
- https://github.com/tauri-apps/tauri/discussions/11167
- https://github.com/tauri-apps/tauri/issues/7418
- https://v2.tauri.app/release/wry/v0.23.2/
- https://dev.to/hiyoyok/global-keyboard-shortcuts-in-tauri-v2-the-right-way-and-the-wrong-way-2h6d
- https://v2.tauri.app/develop/tests/mocking/
- https://docs.rs/tauri/latest/tauri/test/index.html
- https://v2.tauri.app/develop/tests/webdriver/
- https://webdriver.io/docs/wdio-tauri-service/
- https://playwright.dev/docs/webview2
- https://github.com/tauri-apps/tauri-docs/issues/3668
- https://github.com/tauri-apps/tauri-docs/issues/3144
- https://github.com/tauri-apps/tauri/discussions/2751
- https://github.com/tauri-apps/tauri/discussions/11399
- https://v2.tauri.app/
- https://www.gethopp.app/blog/tauri-vs-electron
- https://en.wikipedia.org/wiki/Electron_(software_framework)
- https://www.electronjs.org/docs/latest/tutorial/keyboard-shortcuts
- https://www.electronjs.org/docs/latest/api/utility-process
- https://www.electronjs.org/docs/latest/tutorial/native-code-and-electron
- https://www.electronjs.org/docs/latest/tutorial/using-native-node-modules
- https://nodejs.org/api/n-api.html
- https://napi.rs/docs/concepts/async-task
- https://napi.rs/docs/concepts/threadsafe-function
- https://www.electron.build/
- https://www.electronjs.org/docs/latest/tutorial/tutorial-packaging
- https://playwright.dev/docs/api/class-electron
- https://www.electronjs.org/docs/latest/tutorial/automated-testing
- https://survey.stackoverflow.co/2025/technology
- https://pypi.org/project/PySide6/
- https://pyo3.rs/latest/parallelism.html
- https://doc.qt.io/qtforpython-6/deployment/deployment-pyside6-deploy.html
- https://doc.qt.io/qtforpython-6/deployment/deployment-pyinstaller.html
- https://pyinstaller.org/en/stable/operating-mode.html
- https://pytest-qt.readthedocs.io/en/latest/signals.html
- https://textslashplain.com/2022/01/05/edge-command-line-arguments/
- https://pywebview.flowrl.com/
- https://github.com/iancleary/pyinstaller-fastapi
- https://playwright.dev/
- https://github.com/emilk/egui
- https://crates.io/crates/egui_kittest
- https://wrenlearnsrust.com/posts/2026-03-11-rust-gui-landscape-2026.html
- https://slint.dev/pricing
- https://dioxuslabs.com/blog/release-070/
- https://github.com/fzyzcjy/flutter_rust_bridge/releases
- https://docs.flutter.dev/platform-integration/desktop
- https://learn.microsoft.com/en-us/windows/apps/winui/winui3/
- https://crates.io/crates/cudarc
- https://github.com/gfx-rs/wgpu
- https://rustify.rs/articles/rust-gpu-computing-wgpu-2026
- https://nvlabs.github.io/cuda-oxide/appendix/ecosystem.html
