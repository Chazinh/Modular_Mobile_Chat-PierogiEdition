# Changelog

Project releases and changes are recorded here. Assessments do not represent
implemented features.

## Released

### ModularChat 3.5.0: Secondary Unique Key Convergence in Sync — 2026-10-20

- **Duplicate Logical Rows No Longer Break the Sync (`server-rust/src/store/sync.rs`)**:
  - Fixed the 409 `Colisão de IDs/chaves únicas.` that blocked the Android connection when both devices had created the same logical row with different ids (cast character by `(conv_id, name_key)`, message part by `(interaction_id, idx)` or generation by `(conv_id, op_id)`).
  - The merge now keeps one row per secondary unique key: the oldest record keeps the identity, the newest contributes the content and the replaced row only fills what the winner left empty (aliases are unioned and the confirmation flag is preserved).
  - The references of the replaced row (version, generation and operation) follow the surviving row, so no message, memory or checkpoint is lost.
  - The destination releases the key its own stale row still holds before applying the peer row; a row of another conversation that owns the key is refused by name.
  - When both sides changed the same logical row since the last common ancestor, the newest content wins in a deterministic and symmetric order, so both stores converge on the same row instead of refusing the batch.
  - Two rows that disagree about what the character is (`kind`) cannot be joined without changing who the user is: the batch is refused with the named reason `UNIQUE_KEY_CONFLICT:characters:<conv_id>:<name_key>` and nothing is overwritten.
- **Anti-Regression Coverage (`server-rust/tests/sync_contracts.rs`)**:
  - Added regressions for a duplicated character key converging with the newest text, for a duplicated key whose peer row replaces the local one, and for the named conflict when the two rows have different kinds.
  - Rust suite: **271 passed**, 5 explicitly ignored, 0 failed (25 targets, `sync_contracts` 35 passed).
  - JavaScript suite: 347 passed, 3 explicitly skipped, 0 failed.
### ModularChat 3.4.2: Pending Sync Receipt Recovery — 2026-10-20

- **Pending Receipt Recovery on the Android Client (`client/src/mobile-sync.js`)**:
  - Fixed the dead-end `PENDING_BATCH_CHANGED` ("O snapshot de um lote pendente mudou") that blocked reconnection after the 3.4.1 upgrade, when the stored pending receipt referenced a snapshot that no longer exists.
  - A deterministic atomic rejection (HTTP 4xx other than auth/conflict/rate-limit, and `BATCH_REUSE`) now discards the irreproducible receipt instead of blocking every later attempt; the source keeps the data and the next attempt replans the batch.
  - A legacy receipt without a recorded outcome is discarded only when the destination still holds the receipt's base version, which proves the batch was never applied; an absent conversation keeps the exact replay.
  - A receipt whose source snapshot changed is discarded so the fresh manifests reconcile; no conversation, memory or checkpoint is discarded.
- **Anti-Regression Coverage (`tests/mobile-sync.test.js`)**:
  - Added regressions for a stale receipt delivering the new snapshot with a new `batchId`, for a pending receipt with a divergent source hash, for a concurrent change during a normal transfer, for an atomic 4xx rejection and for a provably unapplied legacy receipt.
  - JavaScript suite: **347 passed**, 3 explicitly skipped, 0 failed.

### ModularChat 3.4.1: Public Settings Temperature Sync Hotfix — 2026-10-20

- **Android Sync Public Settings Validation (`server-rust/src/store/sync.rs`)**:
  - Fixed rejection of the Store's own `temperature: 1.0` during sync due to the JSON representation difference between `1` and `1.0`, preventing `Settings públicos fora dos limites do Store.` during Android connection.
  - Public settings validation remains strict: limits, private fields and snapshot hashes are still checked.
- **Regression Coverage (`server-rust/tests/sync_contracts.rs`)**:
  - Added three regressions covering settings roundtrip, hash preservation and replay; all 121 Store temperatures from 0.20 to 1.40; and rejection of invalid limits, private fields and invalid hashes.
- **Validation Status**:
  - Passed 268 Rust tests (including 32 sync contracts) and 343 JavaScript tests; 5 Rust and 3 JavaScript tests remain explicitly ignored/skipped.
  - Updated stale regression expectations for the existing adaptive history, configurable model effort and recoverable sync-conflict contracts without changing their production behavior.
  - Built the Windows server and signed ARM64 Android APK, verified the existing signing certificate, and installed Android versionCode 83 with an in-place update preserving application data.

### ModularChat 3.4.0: Mobile Sync Loop Resolution, Queue Convergence & Resilient Memory Extraction — 2026-10-20

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

- **Sync Loop Recovery & Client Conflict Resolution (`mobile-sync.js`)**:
  - Resolved infinite conflict loop when synchronizing prolonged conversation turns (~13–16+ turns) between mobile and PC.
  - When resolving conflicts with `prefer-pc`, now tracks preferred baseline without immediate re-trigger of conflicting pushback payloads.
  - Broadened auto-recovery triggers to handle `JOB_DEFINITION_DIVERGED`, `MERGE_VALIDATE`, and structural merge conflict warnings gracefully.
- **Deterministic Runtime Convergence & Job Metadata Normalization (`store/sync.rs`)**:
  - Fixed job runtime metadata merging (`merge_job_runtime`) across devices when execution `error` strings diverged.
  - Strictly isolated runtime-only fields (`status`, `attempts`, `updated_at`, `error`) so un-synced database columns like `error` are stripped before snapshot validation, eliminating `MERGE_VALIDATE: Campo/tabela não permitido no sync: error`.
  - Prevented completed receipts (`done` / `skipped`) from being overridden or invalidated by conflicting failed attempts.
  - Fixed conversation `character_rev` calculation during three-way merges to dynamically resolve to `max(rev)` across all merged character revisions, eliminating spurious `character_rev não aponta à última revisão` validation errors.
- **Resilient Memory Extraction & Fallback (`memory_backend.rs`)**:
  - Added automatic fallback retry for memory extraction when upstream model endpoints return HTTP 400 (`response_format unavailable` / schema rejection).
  - Retries first with `json_object` and subsequently without `response_format`, maintaining local client schema validation to prevent background job amnesia and failure cascades.
- **Anti-Regression Test Suite (`server-rust/src/store/sync.rs`, `sync_contracts.rs`)**:
  - Added `test_merge_job_runtime_handles_error_divergence_and_rank` to guarantee error string divergence convergence.
  - Added `test_merge_job_runtime_keeps_local_terminal_status` verifying terminal status retention.
  - Added `test_merge_snapshot_character_rev_calculation` verifying multi-peer character revision consistency.
  - Executed full 29-contract `sync_contracts` test suite with 100% pass rate.

### ModularChat 3.3.3: Preset Ordering, Native MiniMax Host Resolution & Kinship Perspective Memory Isolation — 2026-10-20

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

- **Model Preset Ordering ("Outro modelo" Always Last) (`SettingsDrawer.jsx`, `api.js`)**:
  - Enforced that "Outro modelo" (`generic`) is strictly sorted to the very end of the model list across both the API data merger (`mergeWithDefaults`) and the drawer UI (`ModelPage`).
  - Placed `minimax-m31` above `generic` among standard models.
- **Embedded ARM64 Host MiniMax Resolution & Client Fallback (`libmodularchat_server.so`, `api.js`, `SettingsDrawer.jsx`, `ChatView.jsx`)**:
  - Embedded `minimax-m31` into the native ARM64 Android host (`libmodularchat_server.so`) `presets.json` table, resolving `400 "Preset desconhecido."` natively.
  - Implemented automatic fallback retry in `createChat` and `updateChat` to handle older server runtimes transparently.
  - Normalized loaded chat presets via `normalizeChatPreset` so MiniMax routes display with their canonical label and full controls.
- **Kinship & Pronoun Perspective Memory Isolation ("sua mãe" vs "minha mãe") (`core-prompt.txt`, `extraction-prompt.txt`, `context.rs`, `jobs.rs`)**:
  - Added strict `DISTINÇÃO DE PARENTESCO E POSSE (MINHA / SUA)` to core prompts, forbidding characters from confusing the user's family with their own.
  - Updated memory extraction instructions to convert 1st/2nd person possessives into explicit subject names (e.g. "a mãe de {{persona}}"), preventing ambiguous pronouns in stored facts.
  - Injected kinship distinction reminders in the prompt tail immediately prior to generation.
  - Increased short-term memory verbatim cutoff from 5 to 15 interactions, preventing premature history truncation and conversational amnesia.
- **Anti-Regression Unit Tests (`tests/regression-effort-and-minimax.test.js`)**:
  - Added tests verifying generic preset terminal ordering and chat normalization.

### ModularChat 3.3.2: MiniMax Discovery, Deep Thinking Controls & Gemini Effort Fix — 2026-10-20

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

- **MiniMax M3.1 Preset Discovery & Cache Resilience (`api.js`, `default-presets.js`, `presets.json`)**:
  - Implemented `mergeWithDefaults()` in `api.js` (`cachedPresets()` and `presets()`) to ensure new presets and updated routes are automatically merged over stale IndexedDB cache.
  - Added route `oc/minimax-m3.1-flash-free` alongside `openrouter/minimax/minimax-m3.1-flash:free` under fallback group `MiniMaxM3.1`.
- **Deep Thinking Toggle & Effort Level Controls (`SettingsDrawer.jsx`, `default-presets.js`, `presets.json`)**:
  - Restored deep thinking toggle switch whenever models support thinking (`thinkingCanDisable || (thinking && thinking !== 'none')`).
  - Added `low`, `medium`, and `high` effort buttons for DeepSeek V4.1, GLM, Muse Spark, Nemotron 3.5, and MiniMax M3.1.
  - Removed `thinkingDefault: false` from Muse, Nemotron, and MiniMax so thinking begins enabled.
- **Gemini Effort Selection Fix (`SettingsDrawer.jsx`)**:
  - Fixed effort selection on the `Gemini` fallback group route, allowing switching between `low`, `medium`, and `high` without disabling buttons or forcing exit from group fallback.
- **Anti-Regression Unit Tests (`tests/regression-effort-and-minimax.test.js`)**:
  - Added regression test suite verifying MiniMax routes, low/medium/high efforts, thinking availability, and cache merging.

### ModularChat 3.3.1: MiniMax M3.1 Preset & 9Router Combo Integration — 2026-10-19

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

- **MiniMax M3.1 Model Preset (`presets.json`, `default-presets.js`, `api.js`, `presets.rs`)**:
  - Added `minimax-m31` preset with 9Router combo fallback route `MiniMaxM3.1` and upstream route `openrouter/minimax/minimax-m3.1-flash:free`.
  - Configured with 1.0M context (`1048576` tokens), 131k max output (`131072` tokens), reasoning/thinking toggle with omission when disabled, temperature sampling, and 16k provider reserve.
  - Added `minimax-m31` to `PUBLIC_PRESETS` in `client/src/api.js` and `DEFAULT_PRESETS` in `client/src/default-presets.js`.
  - Incremented `PRESET_VERSION` to 11 in `presets.rs`.

- **Anti-Regression Unit Tests (`default-presets.test.js`, `multiagent_rp_config.rs`)**:
  - Added automated tests verifying route resolution, groupRoute integrity, and public catalog availability for `minimax-m31`.

### ModularChat 3.3.0: Model Creativity Narrative Slider, Trash Purge & Clear Controls — 2026-10-19

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

- **Model Creativity Slider & Narrative Adherence Control (`StyleForm.jsx`, `context.rs`, `presets.rs`, `maintenance.rs`)**:
  - Added a global model creativity slider in the "Estilo" side menu to control narrative adherence ("termômetro do seguimento narrativo") ranging from strictly faithful (0.2–0.5) to balanced (0.8) and highly creative/inventive (1.0–1.4).
  - Injected dynamic narrative guidance into `core-prompt.txt` based on the configured temperature, balancing story continuity with character spontaneity and organic scene twists.
  - Linked global temperature settings to route sampling in `presets.rs`, dispatching temperature and `top_p: 0.95` to models when chats do not specify an individual override.
  - Synchronized `temperature` across PC and mobile devices via `PUBLIC_SETTINGS` and `SETTINGS`.

- **Trash Management & Complete Clear Functionality (`HomeDrawer.jsx`, `api.js`, `en.json`)**:
  - Added an "Esvaziar lixeira" (Empty trash) option to permanently clear all deleted chats and messages at once with confirmation.
  - Added per-item permanent deletion buttons alongside the restore button for selective trash cleanup.
  - Updated English locale dictionaries in `en.json` for all new creativity and trash controls.

- **Anti-Regression Unit Tests (`global-settings.test.js`, `style-settings.test.js`, `app-character-trash-backup.test.js`)**:
  - Added automated tests covering global temperature normalization, range clamping, queue persistence, and complete trash purging.

### ModularChat 3.1.27: Coral Multi-Agent RP Dynamics, Global Safety Controls, Muse Spark & Nemotron 3.5 Presets — 2026-10-19

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

- **Multi-Agent Roleplay Pacing & Coral Dynamics (`core-prompt.txt`, `presets.json`, `presets.rs`)**:
  - Adopted optimized multi-agent roleplay parameters for conversational fluidity and pacing (`temperature: 1.0` dynamic, `top_p: 0.95`, non-thinking mode for RP dialogue).
  - Switched `deepseek-v41-flash` and `glm-52` from `"sampling": "default"` to `"sampling": "temperature"`, activating real-time temperature slider controls in the UI and transmitting `top_p: 0.95`.
  - Injected global `ELENCO E MULTI-AGENTES` narrative guidelines into `core-prompt.txt`, ensuring vocal distinctiveness, authentic cross-character interaction, and natural colloquial prose across all models.

- **Muse Spark & Nemotron 3.5 Super Presets with 9Router Group Routes (`presets.json`, `default-presets.js`, `api.js`)**:
  - Added `nemotron-35` preset with 9Router combo fallback route `Nemotron`, upstream route `oc-router/nvidia/nemotron-3.5-super-v1:free`, 128k context, 64k max output, and temperature sampling.
  - Reactivated `muse-spark-13` as a selectable preset with 9Router combo fallback route `Muse`, upstream route `oc-router/meta/muse-spark-open`, 1.0M context, 131k max output, and temperature sampling.
  - Updated `PUBLIC_PRESETS` in `api.js` and `DEFAULT_PRESETS` in `default-presets.js` to surface both models in the UI model picker.

- **Global Unrestricted Safety Policy Forwarding (`generation.rs`, `presets.rs`, `providers.rs`)**:
  - Injected global safety configuration (`BLOCK_NONE` across harassment, hate speech, sexual content, and dangerous content) into model generation plans and chat requests across all compatible providers, preventing spurious content policy blocks in benign creative roleplay.

- **Automated Anti-Regression Unit Tests (`multiagent_rp_config.rs`, `unrestricted_safety.rs`, `contracts.rs`, `default-presets.test.js`)**:
  - Added test suites verifying temperature and top_p sampling propagation, system prompt multi-agent injection, unrestricted safety controls, and client default preset route ordering.

### ModularChat 3.1.26: Mobile Sync Conflict Resolution, UI Recovery, and Runner Test Isolation — 2026-10-18

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

- **Mobile Sync Conflict Resolution & UI Recovery (`mobile-sync.js`, `private-storage.js`, `server-connection.js`, `PcConnectionButton.jsx`, `ServerSetup.jsx`, `api.js`)**:
  - Implemented client-side conflict resolution handling for mobile synchronization divergences (`resolveConflict`, `clearConflict`, `resolveStrategy: 'prefer-pc'`).
  - Added `clearPrivateConflicts()` to purge persisted conflict batches from secure local storage upon resolution.
  - Added user-facing recovery banner and conflict resolution action in `PcConnectionButton.jsx` and `ServerSetup.jsx`, enabling one-click divergence recovery directly on mobile devices without manual data wipes.
  - Propagated conflict resolution parameters through `switchToPc` and `switchToLocal` connection transitions.

- **Integration Test Runner Isolation & Fallback Claiming (`server-rust/src/jobs.rs`, `server-rust/src/memory_backend.rs`)**:
  - Added explicit check for empty `MC_TEST_LOCAL_URL` in `memory_backend.rs` so integration tests isolating unavailable local models do not fall back to router mocks.
  - Updated desktop runner cycle in `jobs.rs` to fall back to `MemoryBackend::Local` when no upstream backend is resolved, enabling jobs to be claimed and deferred with "indisponível" as expected by runner integration tests.

- **Automated Anti-Regression Unit Tests (`tests/mobile-sync.test.js`)**:
  - Added automated test suite verifying conflict batch detection, resolution strategy execution, private storage cleanup, and store synchronization.


### ModularChat 3.1.25: Desktop Memory Upstream Fallback, Resilient JSON Extraction, and 9Router Key Discovery — 2026-10-18

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

- **Desktop Memory Upstream Fallback & Status Visibility (`jobs.rs`, `memory_backend.rs`, `api.rs`)**:
  - Replaced desktop-specific local-only runner gate with dynamic backend resolution via `MemoryBackend::resolve`, enabling background memory extraction on PC using upstream models (9Router / OpenRouter) when local GGUF models are not active.
  - Added `desktop_upstream_model` inspection and surfaced active upstream memory provider status in `/api/status` under `local.chat` on desktop environments.
  - Enabled memory policy inspection and updates on desktop without requiring mobile mode restrictions.

- **Resilient Memory Parsing & Portuguese Alias Support (`memory_backend.rs`, `jobs.rs`)**:
  - Enhanced `parse_model_content` and `parse_json_content` in `memory_backend.rs` to support SSE stream formats (`data: ...`), automatically strip `<think>...</think>` reasoning tokens and markdown code blocks, and locate balanced JSON boundaries.
  - Upgraded `sanitize_extraction` in `jobs.rs` to support Portuguese schema keys (`resumo`, `fatos`, `cena`, `personagens`, `relacionamentos`, `pendencias`, `resolvidas`) and synthesize summary from extracted facts when omitted, eliminating false 502 extraction errors.

- **Automated 9Router Key Discovery & Preflight Expansion (`providers.rs`)**:
  - Implemented `read_local_9router_key` in `providers.rs` to automatically discover active API keys from `%APPDATA%\9router\db\data.sqlite` when environment variables are unset.
  - Expanded preflight models response limit to 1MB and added case-insensitive model matching to seamlessly handle large upstream catalogs.

- **Dedicated Integration Test Suite (`regression_desktop_memory_fallback.rs`)**:
  - Added comprehensive regression tests validating desktop memory upstream resolution, thinking tag filtering, JSON code block extraction, Portuguese alias normalization, and status reporting.


### ModularChat 3.1.24: 9Router Group Fallback Routes, Mobile Sync Pushback, and Backup Isolation — 2026-10-17

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

- **9Router Combo Group Routes with Automatic Fallback (`SettingsDrawer.jsx`, `default-presets.js`, `presets.json`, `presets.rs`)**:
  - Integrated 9Router group routes (`Deepseek`, `Gemini`, `Sonnet`, `Opus`, `GLM`) across default presets with automatic fallback, prioritizing group endpoints first in public route ordering.
  - Added visual indicator `(fallback automático 9Router)` in `SettingsDrawer.jsx` to clearly identify combo group routes.
  - Retired inactive presets and routes (`nvidia/z-ai/glm-5.2`), streamlining the active model catalog to core maintained endpoints.

- **Mobile Sync Pushback Reconciler & Connection Resilience (`mobile-sync.js`, `server-connection.js`)**:
  - Added pushback reconciliation in `mobile-sync.js` when mobile store merges diverge from PC after pull, ensuring complete bidirectional convergence across synchronized stores.
  - Prevented active PC connections from dropping during periodic background sync checks upon encountering transient `HASH_DIVERGENCE`, scheduling retries without interrupting active chat sessions.

- **Local Runtime & Repository Hygiene (`local.rs`, `scripts/run-all-tests.mjs`, `identity-hygiene.test.js`)**:
  - Standardized local GGUF model home directory exclusively to `ModularChat-local` in `server-rust/src/local.rs`.
  - Added Windows `.cmd` shell invocation support in `scripts/run-all-tests.mjs`.

- **Comprehensive Git Protection for Backups, Artifacts, and Caches (`.gitignore`, `tests/identity-hygiene.test.js`)**:
  - Expanded `.gitignore` with strict blocking rules for backup directories (`*backup*/`, `*bckp*/`, `*copia*/`, `*copy*/`, `backups/`, `backup/`), backup extensions (`*.bak`, `*.backup`, `*.old`, `*.orig`), temporary directories and files (`tmp/`, `temp/`, `.task-temp/`, `*.tmp`, `*.temp`), session reports (`*relatorio*`), and local scratchpads, with explicit exemptions for test fixtures (`!tests/app-character-trash-backup.test.js`).
  - Added automated test coverage in `identity-hygiene.test.js` to prevent regressions in repository exclusion rules.


### ModularChat 3.1.23: Precise Model Routes, Argon Exclusion, and 9Router Environment Isolation — 2026-10-17

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

- **Model Preset & Route Alignment (`default-presets.js`, `presets.json`, `presets.rs`)**:
  - Removed artificial 9Router combo titles (`Deepseek`, `Opus`, `Sonnet`, `Gemini`, `Kimi`, `Muse`, `GLM`, `Step5-preview`) from model preset route listings, keeping strictly valid routing endpoints.
  - Added real screenshot-verified routes: `oc/deepseek-v4.1-flash-free` (DeepSeek V4.1 Flash), `nvidia/z-ai/glm-5.2` (GLM), `gemini/gemini-3.8-flash` (Gemini 3.8 Flash), and verified routes for Claude Opus 5.5, Sonnet 5.5, Kimi K3, Muse Spark 1.3, and Step 5 Preview.
  - Explicitly excluded Argon (`ag/gemini-4-argon-high`) and registered `"argon"` under `RETIRED_ROUTE_MARKS` in Rust to programmatically prevent Argon routes from being selected or exposed even if advertised by an upstream proxy.

- **9Router Environment Isolation (`start-server.ps1`)**:
  - Isolated the process environment when launching 9Router from `start-server.ps1` via a `try/finally` block that temporarily unsets `DATA_DIR`. This ensures 9Router always loads its canonical database at `%APPDATA%\9router\db\data.sqlite` rather than inheriting ModularChat's server data directory.

- **Automated Regression Test Suite (`regression_send_and_memory.rs`, `default-presets.test.js`, `start-server-9router.ps1`)**:
  - Added Rust test `regression_screenshot_model_routes_and_argon_exclusion` verifying Argon rejection, route availability, preset mapping, and exclusion from public presets.
  - Added JS test asserting presence of valid routes, exclusion of combo titles, and absence of Argon across all presets.
  - Added PowerShell Scenario 4d testing `DATA_DIR` environment isolation during 9Router process launch and restoring the caller's environment.


### ModularChat 3.1.22: CodeBuddy HTTP 400 Channel Verification Fix, Memory Deadlock Resolution, and 9Router Automation — 2026-10-16

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

- **Upstream CodeBuddy / DeepSeek HTTP 400 Fix (`presets.json`, `presets.rs`, `generation.rs`)**:
  - Enforced `user_prefix` mode for DeepSeek, CBAI and CodeBuddy routes, eliminating rejected `role: "system"` payload envelopes that triggered upstream channel verification error `11128 ("request illegal")`.
  - Added full response text inspection and JSON parsing for upstream provider errors in `generation.rs`, surfacing clear diagnostic messages instead of masking them behind generic HTTP status codes.

- **Memory Background Queue Deadlock & Thrashing Resolution (`jobs.rs`, `store/jobs.rs`, `local.rs`)**:
  - Pre-checked `backend.available()` before claiming jobs in `cycle_work`, preventing unavailable memory runtimes from thrashing tasks in and out of the `running` state.
  - Reset error states on job claim (`error = NULL`) and added a deferral ceiling (`attempts >= 5`) in `defer_job` to permanently prevent infinite retry loops.
  - Ensured `ModularChat-local` in `local.rs` discovers local GGUF models (`Qwen3.5-9B`, `embeddinggemma`) and `llama-server.exe` immediately.
  - Adapted `MemoryBackend::Api` to map `role: "system"` into compliant `user`/`assistant` instruction turns for external memory model APIs.

- **Automated 9Router Launcher & Dedicated Regression Test Suite (`start-server.ps1`, `start-server.bat`, `regression_send_and_memory.rs`)**:
  - Integrated `-With9Router` parameter and interactive launcher menu in `start-server.ps1` and `start-server.bat` to detect and spin up the 9Router gateway on port 20128 automatically.
  - Added dedicated regression test suite (`server-rust/tests/regression_send_and_memory.rs`) validating system-free payload emission, CodeBuddy route resolution, and memory claim safeguards.


### ModularChat 3.1.21: Model Providers, Crash-Free Error Handling, and Vault Baseline Recovery — 2026-10-16

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

- **Model Catalog, Provider Restoration & Crash Fix (`client/src/components/ChatView.jsx`, `api.js`, `SettingsDrawer.jsx`)**:
  - Fixed `ReferenceError: isNative is not defined` in `ChatView.jsx`, ensuring error messages render cleanly without unmounting React components or showing a blank screen. Message drafts and chat history are preserved.
  - Enabled generic and server-advertised model routes (including OpenCode DeepSeek variants like `oc/deepseek-v4.1-flash-free`) under "Outros modelos do servidor" without requiring hardcoded client presets or inventing unsupported thinking/effort controls.
  - Eliminated artificial cache invalidation that required `gemini-38-flash` in cached catalogs, maintaining loaded models across provider combinations.

- **Explicit Local vs Remote Execution & PC Reconnection (`server-rust/src/generation.rs`, `AutoHostStatus.jsx`, `PcConnectionButton.jsx`)**:
  - Eliminated silent fallback of remote model routes to local Qwen when running on the Android host. Unconfigured remote models now return HTTP 409 (`REMOTE_MODEL_UNAVAILABLE`), preventing physical context limits of local models from being erroneously applied to remote models.
  - Added dedicated **"Conectar ao PC"** action button to the Android status panel and chat error views, allowing one-tap reconnection to the paired PC with real-time feedback and cancellation without re-requesting a pairing code.

- **Vault Checkpoint Baseline Recovery & Queue Metadata Merge (`client/src/mobile-sync.js`, `server-rust/src/store/sync.rs`)**:
  - Safe baseline comparison recovery between PC and mobile vaults using verified historical checkpoint hashes (`checkpointHashes`), preventing divergence locks when the client's private baseline marker is lost.
  - Deterministic queue execution metadata merging (`merge_job_runtime`), resolving false `ROW_DIVERGED:jobs` conflicts between devices while ensuring completed/skipped tasks are never re-executed.


### ModularChat 3.1.17: Historical Relationship Links, Sync Snapshots, and Background Memory Prioritization — 2026-10-16

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

- **Historical Relationship Consistency & Snapshot Normalization (`server-rust/src/store/turns.rs`, `sync.rs`)**:
  - Reprocessing turns restores earlier relationship rows before removing a derived revision, eliminating dangling `superseded_by` links across sync cycles.
  - Wire snapshots normalize historical relationships whose replacement targets no longer exist, maintaining conversation vault integrity while strictly rejecting invalid cross-conversation references.
  - Added `sync-check` CLI diagnostic (`server-rust/src/main.rs`, `store/mod.rs`) for in-memory vault snapshot validation without exporting conversation plaintext or credentials.

- **Background Memory Preemption & Sync Prioritization (`server-rust/src/jobs.rs`, `api.rs`)**:
  - Authenticated sync requests preemptively pause background memory inference without spending job retry attempts, returning tasks to the queue safely.
  - Pause duration is bounded (30s) and shared across concurrent sync requests, automatically resuming tasks with a 1s grace margin once replication concludes.
  - Periodic connection checks observe busy workers (`&observe=1`) without interrupting active inference or disconnecting healthy PC sessions.

- **Real-Time Pairing Wait & Controlled Sync Retry (`client/src/pc-pairing.js`, `ServerSetup.jsx`)**:
  - When pairing succeeds while the server is processing background memory (`BUSY`), the client commits the pairing and safely retries only the synchronization phase every 10 seconds for up to 5 minutes without re-requesting or replaying the one-time pairing code.
  - Added real-time user feedback ("Pareamento salvo. Aguardando o trabalho ativo terminar...") with cancellation support.


### ModularChat 3.1.14: Public Proxy Pairing Bootstrap, Code Expiration Renewal, and Loopback Guidance — 2026-10-16

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

> [!NOTE]
> **Version Alignment Transition (`3.1.14`)**: Starting with this release, git tags, release headers, and commits transition from the arbitrary `2.x` numbering scheme to align strictly 1:1 with the internal mobile application package version (`3.1.14`, build 65). This guarantees direct traceability between the version reported inside the Android client UI ("About / Version") and repository tags/releases.


- **Public Proxy Pairing Bootstrap (`Caddyfile.template`)**:
  - Configured `@pareamento_publico` route in Caddy reverse proxy allowing `/api/health`, `/api/pair`, and `/api/pair/info` to reach the backend service without requiring a pre-existing Bearer authentication token.
  - Resolves "Aparelho não pareado" when bootstrapping connection from mobile over public domains or reverse proxies.

- **Pairing Code Retention & Window Renewal (`server-rust/src/auth.rs`)**:
  - Preserves the active 6-digit pairing code on repeated renewal requests while resetting the 300-second expiration window, preventing race conditions and invalidation while the user types the code on mobile.
  - Added unit and contract tests verifying pairing bootstrap ignores revoked tokens while preserving device data security.

- **Stale Credential Omission on Pairing (`client/src/api.js`)**:
  - Explicitly omits outdated `Authorization` headers during pairing initiation and code submission, ensuring clean authentication handshakes.

- **Mobile Loopback Detection & Guidance (`ServerSetup.jsx`, `server-address.js`, `server-connection.js`)**:
  - Rejects loopback addresses (`127.0.0.1`, `localhost`) on native Android platforms, presenting immediate localized guidance directing users to configure the PC's local network (LAN) IP.
  - Updated `client/src/locales/en.json` with comprehensive English guidance for connection setup.

- **LAN Server Connection Guidance (`start-server.ps1`)**:
  - Enhanced `Write-ConnectionAddresses` output formatting: highlights local LAN IP addresses in green specifically for mobile device pairing, and flags loopback in yellow for PC browser use only.


### ModularChat 2.2: LAN Connectivity, Real-Time Device Pairing, and Sync Conflict Recovery — 2026-10-16

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

- **LAN HTTP Connectivity & Server Discovery**:
  - Unified server address handling in `client/src/server-address.js`, supporting plain HTTP for recognized local subnet IPs (`192.168.*`, `10.*`, `172.16-31.*`, `localhost`, `*.local`) while enforcing HTTPS for internet hostnames.
  - Enabled local cleartext traffic support in Android (`network_security_config.xml` and `AndroidManifest.xml`), allowing fluid mobile pairing with local PC servers over home Wi-Fi without TLS certificate friction.
  - Added live latency measurement probe (`/api/health`) and real-time connectivity testing in `ServerSetup.jsx` and quick host switching in `GlobalSettings.jsx`.
  - Added `open-firewall.ps1` for one-click persistent Windows Firewall rule creation (TCP 3000, LocalSubnet, Private & Public profiles).

- **Paired Devices Management & Real-Time Sync (`PairedDevices.jsx`)**:
  - Extracted shared `PairedDevices.jsx` component used across both `HomeDrawer` and `GlobalSettings` with 44px touch targets, optimistic immediate deletion, live device counter updates, and touch debounce.
  - Added backend device revocation events (`devices` SSE broadcast in `server-rust/src/api.rs`) ensuring synchronized device list state across active sessions.
  - Robust HTTP 409 conflict recovery (`PUBLIC_SETTINGS_BATCH`) in `client/src/mobile-sync.js`, safely handling divergent batches, known-baseline prioritization, and idempotent receipts.

### ModularChat 2.1: Performance & Security Consolidation, Context Optimization, and Bounded HTTP — 2026-10-16

_Development assisted by Artificial Intelligence (multi-agent collaboration)._

- **Context, Memory & Extraction Optimizations**:
  - Excluded redundant raw messages already represented by recent or pending turn pairs (`context.rs`), preserving cross-speaker turn provenance.
  - Unicode-safe overlapping interval merging in UTF-16, memory deduplication in prompts (`memory_prompt.rs`), and Jaccard similarity curation protected with `checked_sub`.
  - Removed artificial `CONTEXT_TOO_LARGE` veto, preserving actual provider capacity limits and diagnostics.
  - Isolated runner shutdown synchronization (`runner_stop.rs` with `send_replace`).

- **Local Provider Discovery & Security Hardening**:
  - Bounded HTTP client (`bounded_http.rs`) with payload size caps (128 KiB models, 16 KiB health), 650ms deadlines, and single-flight queries.
  - Local discovery for loopback daemons (`local_discovery.rs`) with strict separation of native OpenCode serve vs OpenCode Zen HTTPS.
  - AES-256-GCM cipher scratch buffer zeroizing upon sealing error branches.
  - Scroll and reading position restoration fixes in `ChatView.jsx`, stabilizing bottom anchors across device scales and message retries.

### ModularChat 2.0: Android Keystore vault, adaptive context reduction, direct JNI, and FLAG_SECURE — 2026-10-15

- **Android Keystore Hardware Vault (`NativeVaultKey.java`)**: Integrated Android Keystore-backed AES-256-GCM envelope encryption (`MCKEYKS1`). The SQLite database key is protected directly inside hardware/TEE-backed Keystore storage with authenticated associated data. Implemented automatic transparent migration from legacy software envelopes (`MCKEY001`) with cryptographic round-trip verification, atomic replacement, and strict tampering rejection.
- **Adaptive Context History & Prompt Optimization**: Dynamically compresses conversation context from 10 down to 8 or 6 turn pairs under token budget pressure when preceding turns are covered by non-empty episodic summaries (~59.6% token reduction in benchmarks). Streamlined prompt assembly with unified textual provenance, clean narrative scene/time presentation, and isolated style repair prompts that avoid re-transmitting full conversation histories.
- **Direct Rust JNI Export & CMake Bridge Elimination**: Replaced the legacy intermediate C bridge (`native_host_bridge.c`, `mc_android_host.h`, and CMake build files) with direct JNI bindings via the `jni` crate (`server-rust/src/android_jni.rs`). `NativeHost.java` directly binds and executes `libmodularchat_server.so` with 16 KiB page alignment verification.
- **Android Security Hardening & Safe UUID Generation**: Restored mandatory `FLAG_SECURE` window protection prior to view construction on release builds. Restricted WebView debugging strictly to `FLAG_DEBUGGABLE` packages. Replaced UUID fallback generation in `client/src/api.js` with cryptographically secure `crypto.getRandomValues` (RFC 4122 v4 compliant), failing safely if secure random generation is unavailable.
- **Prompt Caching Support**: Added explicit cache breakpoint markers for `openrouter.ai` endpoints on the primary system prompt without disrupting compatible 9Router or third-party provider contracts.
- **PC Connection Loss Recovery & Local Fallback Modal**: Introduced `ConnectionLossModal.jsx` along with `mc:pc-connection-lost` event handling in `server-connection.js` and `App.jsx`, enabling graceful background retry or instant one-tap switching to local AutoHost when a paired PC server disconnects.



- **Mobile-to-PC Pairing & Network Connection Resiliency**: Parameterized probe deadlines in `client/src/server-connection.js` (`available(server, key, signal, timeoutMs)`), enforcing a 3,000ms minimum timeout on explicit PC targets to accommodate TLS 1.3 key exchange and handshake overhead over local Wi-Fi without false-positive `ORIGIN_UNAVAILABLE` aborts.
- **Android Host Card Suppression in PC Mode**: Automatically suppress the Android AutoHost status card (`AutoHostStatus.jsx`) and pause background status polling whenever connected to a PC server (`config.kind === 'pc'`), eliminating unnecessary polling overhead and UI clutter.
- **Local Mobile Model Route Isolation & Execution Safeguards**: Filtered out local mobile models (`mobile-local` and `mobile-api`) from preset listings in `client/src/api.js` and `client/src/components/SettingsDrawer.jsx` when connected to a PC. Added protective client-side interception in `client/src/components/ChatView.jsx` and `api.js` to prevent executing local mobile routes on PC backends.
- **Model Selector Grey/Black Screen Fix**: Resolved an unhandled `ReferenceError: config is not defined` in `SettingsDrawer.jsx` by explicitly importing `config` from `api.js`. Added confirmation prompts (`confirm(tr("Deseja criar uma nova conversa?"))`) before creating chats from the drawer.
- **Home Drawer & Cascading Navigation**: Introduced `client/src/components/HomeDrawer.jsx` providing a unified liquid-glass drawer experience for the main home screen (`ChatList.jsx`), with smooth spring animations, global style controls, reading settings, and server resets.
- **Comprehensive English Localization Overhaul**: Modernized English translations in `client/src/locales/en.json`, adopting cleaner naming conventions (e.g., "Chats" instead of "Conversations") and industry-standard character/AI terminology ("Character card", "Reasoning", "Model & effort", "Continue scene", "Regenerate response").
- **Application Showcase Banner**: Added 2560x1440 high-resolution presentation showcase graphic (`assets/showcase.png`, `showcase.png`) and updated `README.md` to display edge-to-edge mockup visuals without system bars or camera punch-holes.

### Home screen connection warning fix, TLS handshake deadline, and AutoHost settings — 2026-10-14

- **Ghost Connection Banner Elimination**: In `client/src/App.jsx`, ensured `setConnectionError('')` is explicitly triggered upon successful chat list refreshes (`refresh()`), pull-to-refresh actions (`checkNow()`), and connection state changes (`connectionChanged()`). Protected `ChatList` from rendering stale error banners when the active connection state is confirmed `online` (`error || (status === 'online' ? '' : connectionError)`).
- **TLS Handshake Probe Deadline Adjustment**: In `client/src/server-connection.js`, adjusted probe timeouts in `available()` to use a 2,500ms deadline when probing external PC endpoints (`const deadline = local() ? 400 : 2500;`), accommodating initial TLS 1.3 handshakes (ECC key exchange, certificate chain transmission, and Android TrustManager validation over physical Wi-Fi) without false-positive connection aborts, while preserving the 400ms budget for local embedded host fallback.
- **AutoHost Status & Mobile Provider Settings**: Integrated standalone AutoHost monitoring and local provider setup interfaces (`client/src/components/AutoHostStatus.jsx`, `client/src/components/LocalHostSettings.jsx`, `client/src/autohost-status.js`, `client/src/mobile-settings.js`), allowing manual server setup and direct model management on standalone mobile installations.

### Memory extraction limits, interpersonal relationships, job queue deadlock fix, and connection resilience — 2026-10-14

- **Memory Extraction Engine Optimization**: Capped fact extraction to 5–7 essential facts per turn (`server-rust/src/extraction-prompt.txt`, `server-rust/src/jobs.rs`). Restricted `knowledge` mapping strictly to characters directly involved in each fact rather than exhaustively citing all present entities in scenes with 8–9 characters, permanently resolving local LLM token budget overflow (`finish_reason: length` / HTTP 502).
- **Interpersonal Relationships & Narrative Threads**: Updated prompt contracts and background runners to emit explicit `{ "a": "...", "b": "...", "description": "..." }` relationship objects as well as open and resolved narrative threads (`open_threads` and `resolved_threads`).
- **Deleted Conversations & Job Queue Deadlock Fix**: In `server-rust/src/store/jobs.rs`, `store/repo.rs`, and `store/mod.rs`, immediately mark all associated pending/running/failed background tasks as `skipped` upon deleting a conversation (`delete_conv`). Return `gone: true` in `job_input` if a referenced conversation is missing from the database vault, instantly discarding stranded tasks. Automatically reset budget-failed jobs (`error LIKE '%orçamento%'`) back to `pending` on startup (`RequeueJobs`).
- **Camera and Scroll Anchor Stability on Chat Retry**: In `client/src/components/ChatView.jsx`, preserved bottom scroll pinning (`el.scrollTop = el.scrollHeight`) whenever `stuck` is active. Reset `readingRef.current` to `null` and anchored the view to the bottom upon sending, continuing, or retrying interrupted responses to eliminate backward scroll jumps.
- **Mobile Background Connection Resilience**: In `client/src/server-connection.js`, resolved function shadowing by renaming parameters to `newLocal` and `newRemote`. Updated the lifecycle listener to ignore `hidden` visibility transitions, keeping network connections, streams, and leases alive while minimized, and only triggering revalidation when the app returns to `visible`.
- **Cross-Device Mobile Sync & Standalone Host**: Integrated standalone local host adapter (`client/src/local-host.js`) and bidirectional SQLite replication protocol (`client/src/mobile-sync.js`, `server-rust/src/store/sync.rs`) with atomic conflict rollback and 17 dedicated connection test fixtures.

### Side drawer layout refinements and spacing polish — 2026-10-14

- **New Chat Button Layout**: Removed the outer rectangular wrapper container around the "New Chat" button in `SettingsDrawer.jsx`, embedding the button directly into the transparent panel with rounded clipping (`overflow-hidden rounded-full py-[18px]`).
- **Drawer Internal Spacing Polish**: Expanded internal padding (+2px) across main side drawer items in `SettingsDrawer.jsx` and `drawer.css` (`.mc-row` padding updated to `10px 16px 10px 10px`, avatar bubble to `py-3 pl-3 pr-[18px]`, thinking bubble to `px-[18px] py-3.5`, and sampling info to `px-[18px] py-4`), providing cleaner margins around labels and icons.

### Mobile UI refinements, compact composer capsule, gesture tuning and portal screens — 2026-10-14

- **Compact Composer Capsule**: Introduced a compact 68px floating input capsule displaying a keyboard icon when collapsed. Expands on tap with focus; collapses on tap outside, back navigation, or virtual keyboard dismissal while preserving message drafts.
- **Card-to-Chat Seamless Expansion**: Chat opening now expands smoothly from the tapped chat card's bounding rectangle (`press-feedback.js`), popping the glass rim at the target apex without disjoint circular overlays.
- **Custom Brain Vector Icon**: Added custom dual-hemisphere vector graphic component (`BrainIcon.jsx`) used for conversation memory access and pending memory indicators.
- **Portal Rendering for Sheets and Screens**: Rendered secondary views (`CastScreen`, `MemoryScreen`, `PersonaManager`, `StyleForm`) via React portals to `document.body` to decouple them from transformed drawer parent containers.
- **Gesture and Scroll Improvements**: Enforced single-owner native scrolling for reading history, removed competing elastic offsets from history and drawer panel, and prevented drawer unmount glitches during mid-gesture reversals.
- **Home List & Typography Refinements**: Adjusted chat card margins (12px horizontal, 8px vertical padding) and intrinsic layout estimates (76px). Synchronized conversation and archived list switching to prevent displaying stale responses.
- **Biometrics Activity Lifecycle Fix**: Prevented activity teardown on system cancellation (`ERROR_CANCELED`) in AndroidX `BiometricPrompt` during Recents task switching, resuming authentication on window focus.

### Android host-trust auto-detection and network security configuration — 2026-10-14

- Updated `client/android/app/build.gradle` to auto-detect the local `host-trust` directory (`${rootDir}/host-trust/xml/modularchat_host_security_config.xml`), embedding local CA anchors without requiring explicit environment variables.
- Added support for optional private identity configuration (`private-build.properties`, git-ignored) for local personal updates while preserving default open-source identifiers.
- Added `<certificates src="user" />` trust-anchor to `network_security_config.xml` to allow user-installed local certificates.
- Added `private-build.properties` and `.autohost-v4/` to `.gitignore`.

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
