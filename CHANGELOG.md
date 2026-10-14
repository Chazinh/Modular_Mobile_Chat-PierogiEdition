# Changelog

Project releases and changes are recorded here. Assessments do not represent
implemented features.

## Released

### Persistent animation cache and startup memory pre-warming — 2026-10-14

- Added a specialized animation cache module (`client/src/animation-cache.js`) generating lookup tables (LUT) for rubber-banding and precalculated spring trajectories for drawer open/close, top bar toggles, and elastic returns.
- Implemented single-save persistence to disk (`kv` / IndexedDB and `localStorage` under `mc.anim_cache.v1`), guaranteeing zero storage writes during animation frames or gestures.
- Added JIT warmup and compositor layer pre-allocation on application boot (`client/src/main.jsx`), compiling glass blur shaders and allocating GPU textures off-screen to eliminate startup hitching and first-gesture stutter.
- Configured Android WebView `LOAD_DEFAULT` caching and DOM/Database storage flags in `MainActivity.java` to enable persistent caching of V8 bytecode and pipeline shaders across application restarts.

### Backend performance optimizations, SQLite WAL pragmas and MSVC ICF/REF — 2026-10-14

- Configured MSVC linker optimizations (`/OPT:REF` to eliminate unused functions and `/OPT:ICF` for identical COMDAT folding) under `[target.x86_64-pc-windows-msvc]` in `.cargo/config.toml`.
- Configured high-performance SQLite PRAGMAs for WAL mode in `server-rust/src/store/mod.rs` (`synchronous=NORMAL; temp_store=MEMORY; mmap_size=268435456; cache_size=-32000;`), enabling memory-backed temporary operations, 256 MB memory-mapped I/O, and 32 MB page cache while retaining crash-safe WAL checkpoints.
- Optimized token count estimation hot-path in `server-rust/src/rules.rs` by replacing full character re-scanning (`p.chars().all(...)`) with an $O(1)$ initial character classification (`p.chars().next().is_some_and(char::is_alphanumeric)`), preserving exact token counting parity.
- Eliminated redundant URL parsing, query string reconstruction, and `HashMap` heap allocations in `server-rust/src/api.rs` for requests without query strings (`uri.query().is_none()`).

### Server build optimizations (mimalloc and x86-64-v3) — 2026-10-14

- Configured `mimalloc` as the global memory allocator for the Windows server binary (`main.rs`, scoped under `[target.'cfg(windows)'.dependencies]` in `server-rust/Cargo.toml`), improving allocation performance while keeping the Android JNI library safely on the default bionic allocator.
- Added `.cargo/config.toml` targeting `x86_64-pc-windows-msvc` with `-C target-cpu=x86-64-v3` to enable modern vector extensions (AVX, AVX2, BMI1/2, FMA) for cryptography, SQLite, and hashing on the host without affecting cross-compilation for ARM Android targets.
- Preserved `panic = "unwind"` behavior across the FFI boundary to prevent unhandled panics in the JNI layer.

### Status dot connection alignment, persona button styling and search cleanup — 2026-10-14

- Aligned the collapsed top bar status dot inside the chat (`ChatView.jsx`) with the PC/server connection state (`status` prop from `App.jsx`), matching the behavior of the main chat list (`ChatList.jsx`). The handle dot now turns green when connected to the server and pulses when checking connection, while the inner dot in the model menu continues to reflect confirmed AI model responses.
- Added localized tooltips for the top bar handle indicating connection state ("Connected to PC", "No connection to PC", "Checking connection").
- Fixed the "Add persona" button styling in `PersonaManager.jsx` by adopting the circular floating glass pill style (`mc-glass-pill mc-glass-accent`) matching the main screen. Updated `Screen` in `Sheet.jsx` to render actions directly if they already define their own `mc-glass` container, avoiding redundant double-capsule wrapping.
- Removed search elements from the home screen (`ChatList.jsx`), including the header search button, popup input container, debounced query states/timers, and search hits list. Fixed scroll padding and pull-to-refresh vertical offsets to maintain a clean and steady header layout.

### README icons — 2026-10-14

- Replaced the combined icon banner with the existing transparent Pierogi Dark
  and Pierogi Light icons, centered side by side. Each uses 40% of the available
  width so the pair fits on smaller screens without a black rectangular backdrop.

### Side menu performance and on-device measurement — 2026-10-13

- Fixed the side menu animation being expensive to render on the phone. A trace of
  the WebView on a real device showed the cause: on every frame of the pull/release
  animation, the staircase wrote `transform`/`opacity` straight onto the glass
  buttons, so the browser recalculated the style of each button **and** of its
  `::before`/`::after` pseudo-elements (gradients and shadows full of
  `var()`/`calc()`). That was about 69 elements per frame, roughly 4.2 ms of style
  recalculation against an 8.3 ms budget at 120 Hz. Blur, shadows, scrim and ring
  masks were tested one by one and were not the problem.
- Each menu item now sits inside a plain box (`StairList` in `SettingsDrawer.jsx`,
  `.mc-stair` in `drawer.css`) and the staircase moves the box, not the button.
  Appearance and animation are identical. Measured over repeated open/close cycles:
  elements restyled per frame 69 → 13, style recalculation 4.1 s → 0.8 s, main
  thread (`BeginMainFrame`) 5.8 s → 3.3 s, frames that miss their deadline
  51–52% → 2.8%, and about 72 → 123 frames per cycle (a smoother animation).
  Battery temperature did not change in the 3-minute test (29.6 → 30.5 °C), so
  the measured gain is in frames and CPU, not in degrees.
- The "Style and limits" summary in the menu no longer queries the PC every time
  the menu opens (`api.js`): the global settings are cached for 60 s and the cache
  is dropped on any save. This avoids an HTTPS request and a re-render in the
  middle of the animation.
- Added a `perf` Android build type, a measurement copy of the app. It has its own
  package (`<applicationId>.perf`, installed beside the real app with empty
  storage), is debug-signed and debuggable, and has no `FLAG_SECURE` and no
  biometric gate (`R.bool.mc_perf`, read in `MainActivity`; always `false` in
  `release`), so screenshots, `adb` measurements and WebView tracing work. It must
  never be distributed. `AppIconPlugin` now resolves its launcher aliases against
  the code namespace instead of the package name, which differs in this build.
- New tools: `scripts/build-perf-android.ps1` (builds and installs the measurement
  copy with the build output outside the project folder),
  `scripts/measure-device-perf.mjs` (real swipes in a `drawer`, `bar` or `scroll` mode,
  then `gfxinfo` frames, janky-frame percentage, percentiles and battery/AP/skin
  temperatures) and `scripts/trace-device-ui.mjs` (WebView trace over CDP). Set
  `MC_PERF_PACKAGE` if the app is installed under another package.
- Applied the same fix to the chat's top bar (green dot). `top-bar-motion.js`
  wrote `transform`/`opacity` on every frame straight onto the glass bubble, its
  ring and the four glass buttons (back, name, memory, options), restyling their
  gradients, shadows and pseudo-elements. The bubble, the ring and each button now
  sit inside plain boxes (`.mc-bar-bubble`/`.mc-bar-ring` and their `-face`
  elements in `glass.css`; each `[data-bar-item]` is now a `div` around its button
  in `ChatView.jsx`). The animation moves the box while the glass stays still
  inside it, and the layout is unchanged. Measured on the phone when pulling and
  releasing the dot: elements restyled per frame 18 → 8, style recalculation over
  8 cycles 2.8 s → 1.2 s, frames drawn over 40 cycles 2,849 → 5,827 (the animation
  used to lose about half of its frames) and frames missing their deadline
  9.1% → 5.8%.
- Known: a spike of about 10–14 ms remains at the start and end of the top bar
  animation, when the `mc-bar-animating` class on the `<header>` flips
  `pointer-events` (inherited, so the style of about 65 elements is recalculated at
  once). It does not affect the middle of the animation. About 250 raster tasks per
  cycle and about 1 ms/frame of painting also remain in the side menu, which is the
  next target if the phone still heats up.

### Server logs, memory and CPU diet — 2026-10-13

With the encrypted vault, the whole SQLite database lives in memory and every
`Immediate` write serializes, encrypts and rewrites all of it, so every byte of
diagnostic data kept in the database costs RAM permanently and I/O on each write.
The response-plan order is unchanged: `[PLANO DA RESPOSTA]` still goes before the
current message.

- Server-sent events:
  - the context report is now sent only in the `ack`, and the `context` event is
    emitted only when the report changes (before, it was repeated on every text
    delta and twice in the first frame);
  - the replay `ack` no longer carries `content` or `usage`, only IDs, mode,
    status and revision;
  - the delta is sliced by a remembered UTF-16 → byte offset instead of rescanning
    the whole buffer on every wake, which was O(n²) over a full reply;
  - the 2 s checkpoint only writes when the text has grown.
- Vault writes: `ProviderCheck` is now `Derived` and no longer bumps the
  revision, so it does not trigger an extra full vault rewrite and a conversation
  reload on the client after each reply. The redundant first `SetContext` per turn
  was removed.
- Embeddings: the vector BLOB is decoded straight to `f32` (`rows_with_vector`)
  instead of being turned into one JSON `Value` per byte, which made a 4 KB vector
  take about 130 KB while searching.
- `router_log`: filters in SQL (`model`, `instr`, `LIMIT 20`) and parses the logged
  requests with a typed struct and `RawValue`, instead of loading up to 60 rows of
  up to 512 KB as full `Value` trees on each of up to 5 attempts.
- Permanent database size:
  - `versions.usage_json` now stores only a UI summary; the full diagnostic data
    stays in `generations` and is pruned along with `request_json`/`context_json`.
    The idempotent `usage_summary_v1` migration slims existing rows, and the prune
    also summarizes;
  - the context report stores only counts; the lists of included memory/episode
    IDs, the embedding model and the sync data are kept only with
    `MC_DEBUG_CONTEXT=1`;
  - upstream error messages are clipped to 300 characters, as `jobs.error`
    already was. `vault-check` prints only the error category.
- Logging: `tracing` now emits a few events without conversation content (default
  filter `warn,modularchat_server=info`). The main binary writes logs to stderr and
  leaves stdout for the banner and the `snapshot`/`vault-check` JSON. A vault write
  failure still reaches stderr (`eprintln!`) when there is no `tracing` subscriber.
- CPU and allocations:
  - `context::assemble` no longer clones pairs, candidates, episodes, relationships
    and threads on each step of the compression ladder; a test over 6,000 random
    cases produced output byte-identical to the previous version;
  - the message list no longer runs N+1 queries or `SELECT *`;
  - `route-validation.json` is read again only when its modification time changes;
  - the review schema is parsed once (`OnceLock`);
  - `/models` and `/presets` cache the upstream `/models` answer for 60 s.
- Fixed `retrieve` being very slow on long conversations: the `active` CTE built
  from `json_each` was recomputed on every join comparison. It is now
  `MATERIALIZED`, which took the search from 25 s to 57 ms with 720 turns in a debug
  build and keeps it well inside the 6 s limit on context enrichment.
- `listCast` returns the **active** user persona first.
- `Cargo.toml`: `serde_json` with the `raw_value` feature.
- Memory extraction no longer gets slower on every turn (`store/context.rs`,
  `store/memory.rs`, `store/mod.rs`):
  - the timeline cache keeps the snapshots under a chained fingerprint
    (`Timeline`), so a new turn reuses the snapshots already in memory and only
    derives the new one, instead of rereading and reparsing all of them;
  - the context receives only the timeline summary (`summary()`); the full view
    (`view()`) is built only for the memory screen, with the same output as before;
  - eligibility went from 3 queries on the per-extraction path to 1
    (`active_path` + `Eligibility::from_path`);
  - a 720-turn simulation dropped from 87 s to 34 s in a debug build.
- The Rust build directory can live outside the project. The tests,
  `scripts/benchmark-context.mjs` and `start-server.ps1` find the executables
  through `cargo metadata` (new `tests/rust-target.js`), which honors
  `CARGO_TARGET_DIR` and `build.target-dir` in a `.cargo/config.toml`; the
  `MC_CONTRACT_EXE`/`MC_SERVER_EXE` overrides still win and the default remains
  `server-rust/target`. This lets a cloud-sync client be kept away from the files
  the linker writes, which caused intermittent LNK1104/LNK1201 errors. The README
  was updated.
- Tests: new `server-rust/tests/log_diet.rs`; `tests/memory.test.js` expects a
  sorted list to match the sorted result; the `waitFor` in `tests/api.test.js` was
  raised from 5 s to 10 s because the provider check retries for about 6 s.
- Removed the stray `package.json` at the repository root, a loose copy of
  `Modular-Mobile-Chat/package.json` whose scripts pointed to paths that do not
  exist at the root.
- Not done: the duplicated candidate fields (`content`/`text`, `sourceSeq`/`seq`,
  `sourceInteractionId`/`interactionId`) are still read by `context.rs`, the tests
  and the extraction prompt.

### Response limits, local memory model and UI fluidity — 2026-10-13

- Removed the yellow notice about the character ceiling being exceeded
  (`ChatView.jsx`, obsolete string removed from `en.json`). When the server's
  character ceiling is reached, the reply already ends at its last complete
  sentence. The notice now appears only when the **provider** truncates by tokens,
  i.e. when the reply is actually incomplete.
- The size limit is now given to the model **before** the user's message. A new
  `[PLANO DA RESPOSTA]` block (`context.rs`) is placed ahead of the message in the
  final turn, after the story context. It states the size target/ceiling and the
  visible token budget, the talk/narration composition, the monologue limit and
  the turn pace, and tells the model to decide what fits before writing. The
  `[Lembrete …]` after the message is shorter (language, format, where to start,
  agency) and points back to the plan. The system prompt keeps the mandatory limit.
- Stronger default local memory model (`local.rs`):
  - default is now **Qwen3.5-9B-Q4_K_M**, with an automatic fallback to
    `Qwen3.5-4B-Q5_K_M` when the 9B file is missing;
  - GPU/RAM split is automatic through `--fit` (`-ngl auto`): it fills the VRAM
    that fits, keeping a margin of 768 MiB (`LOCAL_FIT_MARGIN_MIB`), and puts the
    rest in RAM; the second attempt is CPU only;
  - context raised from 8192 to 16384 (`LOCAL_CHAT_CTX`);
  - the per-request deadline is configurable with `LOCAL_CHAT_TIMEOUT_S` (default
    600 s, previously a fixed 240 s). The 9B partly runs on the CPU on small GPUs
    and is slower than the 4B; set `LOCAL_CHAT_MODEL` to the 4B file to go back.
  - `scripts/download-memory-models.py` now downloads the 9B and the README
    variables table was updated.
- Fixed two bugs in the memory job queue:
  - a timeout was treated as "model unavailable", so a slow extraction returned
    the job to `pending` forever with no failure limit and blocked the queue on
    every 30 s cycle. It is now a normal error: retry with a smaller context, then
    `failed` after 3 attempts (`local.rs`);
  - waiting for the model to load or become available consumed attempts, so jobs
    ended up processed at reduced/minimal context or marked `failed`. A new
    `DeferJob` operation (`jobs.rs`, `store/jobs.rs`, `store/mod.rs`) returns the
    job to the queue and refunds the attempt.
- UI fluidity:
  - top bar (`top-bar-motion.js`, `glass.css`): the soap bubble no longer uses
    `backdrop-filter`, and the bar's islands drop the blur while animating and get
    it back at rest, since re-blurring every frame dropped the animation to a few
    frames; finger movements are coalesced into one write per frame;
  - `drawer-motion.js`: the first frame of every spring (drawer, sheets, top bar,
    elastic) was a repeated frame because of a negative `dt`; `frameDt` now always
    advances one frame. Drawer and sheet drags are also coalesced to one write per
    frame;
  - side drawer (`Staircase` in `drawer-motion.js`, `SettingsDrawer.jsx`,
    `drawer.css`): the menu bubbles move as a chain of springs (critically damped,
    about 60 ms of lag per link):
    - when pulling open, the **top** row leads and each row below chases the one
      above it; when retracting, the **bottom** row leads and each row above chases
      the one below, so the top leaves last and the menu is only unmounted when the
      last row is out;
    - the tip of the staircase is always the top or the bottom, never the middle,
      even when the direction is reversed mid-gesture; the leader only switches
      once the finger has really turned (about 20 px), not on every jitter;
    - while a finger holds the menu, the staircase is never unmounted mid-gesture,
      even when every bubble has caught up with the panel at one end. It used to be
      unmounted and remounted several times per second during quick back-and-forth
      drags (rows measured again, the widened box toggled, causing relayouts and
      bubbles jumping to the panel); it now unmounts only after release;
    - each bubble's opacity follows its own position (fainter near "closed");
    - no clipping: while moving, the list's clip box is widened beyond the panel
      (`mc-stair-free` class, without changing the rows' layout and without a
      scrollbar), so bubbles outside the panel are not cut by a straight edge;
    - only transform and opacity are animated, only for rows on screen, and
      nothing is left applied at rest.

    Earlier versions (delay tied to the opening progress, then a 32 ms delay line
    and a per-row wait of 55 ms) were imperceptible or stepped, so they were
    replaced;
  - `elastic.js`: when the finger reversed direction while stretched, the stretch
    dropped to 0 in one frame (a jump of up to 48 px); it now returns with a spring
    from where it is. In lists that do not scroll (the drawer menu) the stretch is
    a continuous function of the finger through zero. Grabbing the content while a
    spring is running continues from the current offset, and letting go no longer
    cuts off a spring that is still running.
- Tests: `server-rust/tests/context_contracts.rs` checks the order context → plan →
  message → reminder; `tests/api.test.js` checks that the final turn starts with
  `[PLANO DA RESPOSTA]` and that the message follows `[FIM DO PLANO]`;
  `tests/drawer-motion.test.js` covers the staircase (top-to-bottom cascade,
  reaching the panel, closing together, nothing left at rest) and the first frame
  of the springs.

### Home screen and chat interface — 2026-10-13

- Removed the purple dot over the "Go to end" arrow in the chat
  (`ChatView.jsx`). The arrow behaves as before: it appears when the view is not
  at the end of the conversation or when new text arrives.
- Added a connection indicator to the home screen title bubble, in the same
  style as the chat's `mc-bar-dot`: green when the last request to the PC
  succeeded, gray when offline (the app shows the copy saved on the device) and
  pulsing while checking. It has accessible labels ("Connected to the PC",
  "No connection to the PC", "Checking connection"), translated to English. The
  new `status` state (`checking`/`online`/`offline`) in `App.jsx` is refreshed on
  every list load: on launch, on returning to the app, on server events and on
  pull to refresh.
- Added pull to refresh to the chat list. With the list at the top, pulling down
  shows a glass bubble with a refresh icon that grows and rotates with the pull
  and turns lilac at the trigger point (about 96 px). Releasing past it discards
  the cached address choice (local or DuckDNS), checks the connection again and
  reloads the chat list and the indicator; releasing earlier only cancels. Touch
  listeners are passive, so native scrolling and the elastic overscroll are
  unchanged, and the gesture does not re-render the list on every move. Animations
  are disabled when the system "reduce motion" setting is on.

### Menu smoothness — 2026-10-13

- Fixed the stutter in the side menu when scrolling up and down, caused by the
  elastic physics shared by all lists (`elastic.js`):
  - the compositing layer was created and destroyed several times per gesture
    because `will-change` was toggled whenever the stretch crossed zero;
  - the movement was written on every touch event, out of step with the display,
    at subpixel positions, and the items were measured again with
    `getComputedStyle` each time;
  - touching during the bounce reset the offset at once, making the content jump
    under the finger.
- Touch moves are now coalesced into a single write per frame
  (`requestAnimationFrame`), the offset is aligned to physical pixels, elements
  are measured once per gesture, and the compositing layer is kept for the whole
  gesture and released only when the content is back at rest. Touching during the
  bounce now catches the content where it is.
- `drawer.css`: the menu page and sheet content (`.mc-elastic-wrapper`) has a
  permanent compositing layer, and the menu list no longer passes scrolling on to
  the chat behind it (`overscroll-behavior: contain`).

### Approximate length target — 2026-10-13

- Fixed the **Approximate target** mode cutting replies in the middle of a word
  with no clear notice. The server and the token budget used the target itself as
  a hard ceiling, so a reply with a target of 1,600 or 1,800 was stored with
  exactly that many characters and the "length" status. The **Up to this size**
  mode is unchanged.
- The approximate target now has an acceptable range of 85%–125% of the chosen
  size and a cut-off at 135% of it, for any size. For example, 2,000 gives a range
  of 1,700–2,500 and a cut-off at 2,700; 8,000 gives 6,800–10,000 and a cut-off
  at 10,800.
  - Prompt: "around N, acceptable range X–Y; it is a target, not a ceiling; end on
    a complete sentence; never go past Z". The mandatory limit line also uses Z.
  - API budget (`max_tokens`) is computed from the cut-off, so the provider does
    not stop a reply that is still within the range.
  - The style review accepts the whole range; previously anything above the
    target counted as an overrun.
  - If the text reaches the cut-off, the reply ends on the last complete sentence
    (or paragraph) instead of mid-word, as long as that sentence is in the second
    half of the text; otherwise the text is kept as received. This applies to
    both modes.
  - The warning is now a yellow box with an icon inside the message itself. It
    shows the ceiling that was reached ("went past the ceiling of 2,700
    characters and was ended on the last complete sentence. Tap Continue to
    carry on or increase the size in Style and limits") or that the provider
    stopped on tokens. It used to be a small line under the message.
  - Style and limits: the explanatory text shows the range and cut-off computed
    for the chosen size.
- The storage limit for **model replies** rose from 8,000 to **10,800** characters
  (8,000 + 35%, constant `REPLY_STORE_LIMIT`). It applies when a generation is
  saved, when it is finalized and when a reply is edited manually, and the app
  editor accepts 10,800 for replies. Unchanged: the user message limit (6,000)
  and the maximum selectable size (8,000).
- Files: `rules.rs`, `presets.rs`, `context.rs`, `style_policy.rs`,
  `store/turns.rs`, `generation.rs`, `ChatView.jsx`, `StyleForm.jsx` and
  `locales/en.json`. Tests: `response_controls.rs` covers the target limits, the
  budget over the cut-off, the accepted range and the cut at the last sentence;
  `contracts.rs` stores up to 10,800 and rejects 10,801; `api.test.js` covers the
  new range and an end-to-end case with a target of 500.
- These changes were ported from a validated copy of the project; the tests and
  builds were not rerun in this repository.

### README presentation — 2026-10-13

- Incorporated the supplied visual structure: a greeting and two original
  centered images, followed by the title and the updated documentation. Removed
  the duplicated presentation and the Unicode spaces used for alignment.
- Kept the current features and instructions for languages, controls, host,
  providers, GLM and Rust evaluation. Fixed the Android section that still
  described a direct HTTP connection, aligning it with the HTTPS gateway and
  certificate trust.
- Reconciled the README after an integration conflict: removed markers and diff
  fragments that had ended up in the text; recovered the complete provider/preset
  examples, the project structure and the security notes, keeping the original
  images, the two languages and the HTTPS guidance.

### Languages and visual controls — 2026-10-13

- Added the interface translation to Brazilian Portuguese and English, with
  immediate updates, the local `mc.language` preference, synchronization between
  tabs and Android biometric texts. Conversation content, character sheets,
  presets and reply language rules are not changed by the preference.
- Replaced the native language/memory type selectors and the color dialog with
  sheets that use the app's glass style, palette and buttons. The custom color
  keeps HEX input and hue, saturation and lightness controls.
- Delete/clear confirmations now use app sheets, cancel with Back/Escape and run
  only after confirmation. Sheet editing errors appear on the page itself. Added
  keyboard focus and dialog semantics to the sheets; the top layer receives
  Escape/Tab.
- Fixed the overlap of sheets opened inside cards: they are now rendered above
  the whole screen, so the controls that follow can no longer cover the selector.
  The new panels use more opaque glass for clear reading.
- Adjusted the native Android theme to the app's dark and lavender colors,
  including biometric prompts, preserving the system's strong authentication.
- Updated the README for both languages and centered the icon greeting/images with
  GitHub-compatible HTML.

### Private host and menu — 2026-10-13

- Incorporated generic HTTPS hosting support with a Caddy template, the browser's
  own login, Bearer token verification in the backend and blocking of pairing
  code creation through the proxy.
- Fixed relative login redirects in the Caddy template with an explicit matcher
  and preserved the Android client's CORS preflight checks.
- Added Windows helpers to prepare/control Caddy, update DuckDNS and manage only
  the IPv4 TCP 80/443 forwardings that belong to the configuration. Personal
  domain, IP, subnet, directory and credentials were not imported.
- Added the `-Loopback` option to run Rust behind the proxy without exposing it to
  the LAN.
- The Android connection now accepts local and remote HTTPS addresses of the same
  PC, with bounded probes, a choice shared for up to 30 seconds and re-evaluation
  on resume, on network changes and on failures. POSTs and open streams keep their
  destination and are not retried by endpoint selection.
- Android blocks external HTTP and mixed content. Private CA trust is optional and
  restricted to the configured host, generated outside the versioned files; no CA
  from the source host or private key ships in the default package. The app's
  policy takes precedence over the plugins' HTTP configuration in the merged
  manifest.
- Ported the final side menu fix: softer spring, a 12-pixel opening threshold,
  controls excluded from the edge gesture, cleanup of old events and protection
  against animation after destruction. Preserved opening by button, Back, drag,
  bottom sheets and existing styles.
- Updated the README with portable instructions, certificate
  requirements, local pairing and CGNAT/IPv6 limits. Deliveries and private
  settings of the source installation do not represent an installation of this
  project.
- Added isolated checks for selection, cancellation, POST preservation, the menu,
  host parameters, Android resources and the optional Caddy gateway.
- Verified the web build, translations/interpolation, color conversion, menu flows
  in a headless browser, the isolated Caddy gateway and Java/Android resource
  compilation in the default and optional CA modes. Installation on a device,
  issuance of public certificates, DNS and real port forwarding were not run in
  this update.

### Added — 2026-10-13

- Project changelog and an access link in the README.
- Rust migration policy in the README, with priorities, risks and criteria for
  accepting future changes.

### Assessed — 2026-10-13

- Confirmed in the code that the HTTP API, generation/SSE, context, memory, jobs,
  SQLite persistence and the encrypted vault are already implemented in Rust.
- Identified as candidates for future assessment: model installation/verification,
  parts of the Windows launcher and intensive tests/benchmarks that currently use
  Node-to-Rust communication.
- The scene parser was considered a conditional candidate for Rust/WebAssembly,
  only if client-side measurements show a bottleneck and a gain over the full
  path.
- Recommended keeping React, gestures/DOM, client transport, browser cache and the
  Android Keystore/biometric integrations in the current technologies.
- No runtime code migration was applied in this review. Performance, RAM or VRAM
  gains from a future migration have not been demonstrated.

### Changed — 2026-10-13

- Documented the assessment priorities: recover the missing test fixture, measure
  the current bottlenecks, assess the model installer, reduce IPC in intensive
  tests and only then consider the launcher and the Wasm parser.

### Known issues — 2026-10-13

- `tests/fixtures/world-card.txt` is empty in this copy of the project. The Rust
  test `structured_card_matches_world_contract` fails because it cannot find the
  expected characters. The original fictional character sheet must be recovered
  before claiming that the test base is fully green.
