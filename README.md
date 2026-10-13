@ -1,8 +1,16 @@
# modularchat-pierogiedition

<p align="center">Hi! &gt;&lt;</p>
<p align="center">
  <img src="client/public/icons/pierogi-dark-512.png" width="320" alt="Pierogi dark icon" />
  <img src="client/public/icons/pierogi-light-512.png" width="320" alt="Pierogi light icon" />
  <img width="1018" height="1545" alt="image" src="https://github.com/user-attachments/assets/643d8d69-cc95-4d62-a7ce-e21896ccfbe0" />
</p>

A self-hosted character chat. Your **PC runs the server** and keeps all memory encrypted on disk; your **phone (Android app) or any browser** reads and writes the conversation. Text generation goes to any **OpenAI-compatible model provider** you configure (9Router, OpenRouter, a local llama.cpp/vLLM/Ollama server, …), and long-term memory is built **locally** by small GGUF models.

> The user interface is in **Brazilian Portuguese** (pt-BR). Code comments and this guide are in English.
> The interface supports **Brazilian Portuguese** (pt-BR) and **English**, selectable in Settings. Code comments and this guide are in English.

Project changes and migration assessments are recorded in [CHANGELOG.md](CHANGELOG.md).

## Features

@ -12,7 +20,11 @@ A self-hosted character chat. Your **PC runs the server** and keeps all memory e
- **Local memory**: each turn is mined for facts, relationships, scene state and timeline by a local model (llama.cpp), with embeddings for retrieval, curation of near-duplicate memories and quote provenance checks.
- **Context budget**: keeps the last ~30 complete turns, adds the scene, who-knows-what, open threads and a few relevant memories, and shrinks gracefully under pressure.
- **Android app** (Capacitor) with biometric lock, screenshots blocked, encrypted local cache and selectable home-screen icons.
- **Private HTTPS hosting** with a configurable Caddy gateway, separate browser login, optional DuckDNS and controlled port-mapping helpers.
- **Automatic Android connection** between user-configured local and internet HTTPS endpoints, without automatic replay of writes or generation requests.
- **Liquid-glass UI**: side menu with spring/rubber-band physics, collapsible "soap bubble" top bar, eye-care options (text brightness/color, blue-light filter, paper texture).
- **Local interface language** (Portuguese/English), with live updates and persistence per device. Interface language does not translate saved conversations or change the model's response-language policy.
- **Consistent in-app controls**: language and memory-type choices, custom color palette (HEX and hue/saturation/lightness), and action confirmations use the app's glass sheets instead of Android's default web dialogs.
- Branches and versions of replies, continue/regenerate, 30-day trash, encrypted backups, export/import, device pairing with 6-digit codes.

## Requirements
@ -57,11 +69,51 @@ Open `http://127.0.0.1:3000` on the PC. The console prints a **6-digit pairing c
| `-Port 3000` | Listening port (default 3000) |
| `-Sandbox` | Works on an isolated copy of the vault (loopback only, port 3011) |
| `-NoUpstream` | Disables generation (no provider is contacted) |
| `-Loopback` | Binds Rust only to this PC when using the HTTPS gateway |
| `-Upstream http://host/v1` | Single provider URL when there is no `providers.json` |
| `-DataDir C:\path` | Data directory (default `.\data`, or `$env:MODULARCHAT_DATA`) |

Keep the vault (`modularchat.db.vault`) **and** its key file (`modularchat.db.key`) together: the key only opens on the same Windows account.

## Private HTTPS hosting

See [OPERACAO.md](OPERACAO.md) for the complete Windows/Caddy, DNS, pairing and
Android certificate workflow. Hosting is optional and no setup script runs
automatically. The repository ships no personal hostname, IP, credentials,
certificate, signing key or device identifier.

The Android setup screen accepts a local HTTPS address and an optional internet
HTTPS address for the same PC. It prefers the local endpoint, probes the remote
one when necessary, caches its choice for up to 30 seconds, and rechecks after
resume, network changes or failure. An open request or SSE stream keeps its
endpoint; endpoint selection never replays a POST. Pair initially on the LAN.

Android trusts system CAs by default and permits HTTP only for the internal
Capacitor `localhost` origin. An optional `MC_ANDROID_HOST_TRUST_DIR` resource
directory can add the public CA of the chosen local host for a private build.
Generate it with `scripts/configure-android-host.ps1`; the default output is
Git-ignored. Private keys and the originating host's certificate are not bundled.
Legacy HTTP endpoints require explicit HTTPS configuration.

The side drawer now settles more gently, needs a 12 px inward edge drag, ignores
controls as gesture origins and clears stale drag events. Button opening, Back,
glass styling, bottom sheets and reduced motion retain their existing flow.

## Interface language and controls

Open **Configurações → Idioma** to choose Portuguese (Brazil) or English. The
choice applies immediately, stays on this device/browser and synchronizes across
browser tabs. The Android biometric prompts use the same saved preference. Saved
messages, character sheets, persona names, provider routes and generation settings
keep their content; the backend's current response-language policy remains pt-BR.

Language and memory-type choices, custom colors and deletion/cleanup confirmations
open glass sheets inside the app, with its purple/lavender palette, instead of OS
select/color/JavaScript dialogs. Custom colors support HEX and hue, saturation and
lightness, with preview, validation, Apply and Cancel. Back/Escape cancels the top
sheet; destructive requests start only after explicit confirmation. Android keeps
the system's strong biometric authentication and required system file picker.

## Model providers

Providers are listed in `providers.json` (in the data directory, or the path in `MC_PROVIDERS_FILE`):
@ -97,7 +149,7 @@ The app offers a model only when its route matches a **preset** in `server-rust/
```json
{
  "id": "glm-52",
  "label": "GLM 5.2",
  "label": "GLM",
  "selectable": true,
  "family": "glm",
  "match": ["glm-5\\.2(?:[^0-9]|$)"],
@ -125,7 +177,7 @@ The app offers a model only when its route matches a **preset** in `server-rust/

To add a model: add a preset (before `generic`), add its `id` to `PUBLIC_PRESETS` in `client/src/api.js` (and optionally to `client/src/default-presets.js`, the offline fallback), rebuild client and server.

Presets included: DeepSeek V4.1 Flash, Claude Opus 5.5, Claude Sonnet 5.5, Gemini 3.8 Flash, Kimi K3, Muse Spark 1.3, Step 5 Preview, GLM 5.2.
Presets included: DeepSeek V4.1 Flash, Claude Opus 5.5, Claude Sonnet 5.5, Gemini 3.8 Flash, Kimi K3, Muse Spark 1.3, Step 5 Preview, GLM (5.2).

## Local memory models

@ -180,6 +232,39 @@ node scripts/verify-mobile-ui.mjs                   # UI regression in headless

`npm test` uses the debug executables in `server-rust/target/debug` (`cargo build --manifest-path server-rust/Cargo.toml --bins`); set `MC_CONTRACT_EXE`/`MC_SERVER_EXE` to use others. `scripts/validate-routes.mjs` makes a few real, paid calls to check that each preset's effort reaches the provider (9Router request log).

## Rust migration policy

The runtime backend is already Rust: HTTP/SSE, generation, providers, context,
memory jobs, SQLite and the encrypted vault live in `server-rust/`. Node runs
client build tools, test fixtures and verification scripts. Language percentages
do not measure runtime cost or justify replacing the client.

| Area | Assessment | Condition for proceeding |
| --- | --- | --- |
| Model installer (`scripts/download-memory-models.py`) | A Rust CLI could simplify installation by removing the Python requirement for this utility | Preserve pinned revisions, streamed downloads, SHA-256/size checks, staging and existing-file verification; do not activate models implicitly |
| Windows launcher (`start-server.ps1`) | Selected responsibilities could move into the existing Rust CLI for maintainability | Preserve verified backups, sandbox isolation, executable staging, port/process ownership checks, environment options and shutdown behavior |
| Repeated store tests and context benchmarks | Native Rust tests can avoid fixture IPC when that cost matters | Keep equivalent assertions and retain Node HTTP/SSE tests as checks from an independent client; a native context benchmark already exists |
| Scene parser (`client/src/format.js`) | Consider a small Rust/Wasm module only after client profiling | Compare complete JS/Wasm paths, including text transfer, output conversion, startup and rendering, with the same Unicode/streaming fixtures on PC and Android |
| React UI, DOM gestures, client networking and browser cache | Keep the current implementation | Optimize measured bottlenecks in place before considering a rewrite |
| Android Keystore, biometric lock and icon plugins | Keep the native Java integration | Preserve platform key handling, lifecycle and existing cache compatibility |

Prioritize profiling the existing Rust store and vault before expanding the
migration. Every proposed change must preserve API/JSON/SSE contracts, UTF-16
limits and reconnect offsets, operation idempotency, branches/versions, memory
provenance, encrypted storage and confirmed-write durability. Accept performance
claims only with comparable before/after measurements; accept packaging changes
only when their maintenance benefit outweighs the added compatibility risk.

This assessment does not implement a new migration or establish a speedup.

Recommended order: (1) restore the missing test input in
`tests/fixtures/world-card.txt` and obtain a passing baseline; (2) profile the
existing Rust vault/store and client; (3) evaluate the model installer CLI;
(4) move only IPC-heavy test/benchmark workloads that show a benefit;
(5) consider selected launcher responsibilities; (6) consider the scene parser
only if device measurements justify Wasm. The React UI and Android platform
plugins remain in their current stack.

## Project layout

```
@ -189,8 +274,10 @@ client/            React + Vite web app, also packaged as the Android app (Capac
server-rust/       Server: HTTP API, generation, presets, providers, context assembly, memory jobs, vault
  src/store/       SQLite store (single owner thread), schema, migrations, trash, export
scripts/           Route validation, benchmarks, UI checks, model download
hosting/           Generic Caddy template (private runtime state stays outside the repository)
tests/             Node integration tests and fixtures (fictional data only)
start-server.ps1   Windows launcher (backup, sandbox, environment)
OPERACAO.md        Portable private HTTPS hosting and Android trust instructions
```

## Security notes
