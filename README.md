<p align="center">Hi! &gt;&lt;</p>

<p align="center">
  <img width="671" alt="Pierogi Edition presentation" src="https://github.com/user-attachments/assets/1ba91d5f-9995-4cd1-975d-8217168f8048" />
</p>

<p align="center">
  <img width="1018" alt="ModularChat presentation" src="https://github.com/user-attachments/assets/643d8d69-cc95-4d62-a7ce-e21896ccfbe0" />
</p>

# modularchat-pierogiedition

A self-hosted character chat. Your **PC runs the server** and keeps all memory encrypted on disk; your **phone (Android app) or any browser** reads and writes the conversation. Text generation goes to any **OpenAI-compatible model provider** you configure (9Router, OpenRouter, a local llama.cpp/vLLM/Ollama server, …), and long-term memory is built **locally** by small GGUF models.

> The interface supports **Brazilian Portuguese** (pt-BR) and **English**, selectable in Settings. Code comments and this guide are in English.

Project changes and migration assessments are recorded in [CHANGELOG.md](CHANGELOG.md).

## Features

- **Rust server** (axum + SQLite) with a single encrypted vault file (AES-256-GCM, key protected by Windows DPAPI). Nothing is ever written to disk in plain text.
- **Pluggable providers**: several OpenAI-compatible endpoints at once, selected per conversation (`provider:model` routes).
- **Presets per model family** with locked reasoning effort and an on/off thinking switch where the provider supports it (DeepSeek, Claude, Gemini, Kimi, GLM, …). New models are added in a JSON file.
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

| Part | Needs |
| --- | --- |
| Server | **Windows 10/11** (the vault uses DPAPI), [Rust](https://rustup.rs) stable (edition 2024), Node.js ≥ 24 (build/test scripts only) |
| Generation | At least one OpenAI-compatible provider (API key if it requires one) |
| Local memory (optional, recommended) | [llama.cpp](https://github.com/ggml-org/llama.cpp) `llama-server` (CUDA build if you have an NVIDIA GPU) and two GGUF models (~5.6 GB) |
| Android app (optional) | JDK 21, Android SDK (API 36), a device with Android 7+ |

## Quick start (PC)

```powershell
git clone https://github.com/<you>/modularchat-pierogiedition.git
cd modularchat-pierogiedition

# 1. Build the web client (served by the Rust server)
cd client
npm ci
npm run build
cd ..

# 2. Build the server
cargo build --release --manifest-path server-rust/Cargo.toml

# 3. Configure at least one provider (see "Model providers")
mkdir data
copy providers.example.json data\providers.json   # then edit it
$env:OPENROUTER_API_KEY = "sk-or-..."              # whatever your providers.json references

# 4. Start
.\start-server.bat
```

Open `http://127.0.0.1:3000` on the PC. The console prints a **6-digit pairing code** (valid for 5 minutes, single use); pairing is required for every device, including the PC's own browser. You can also generate a new code later in **Configurações → Pareamento**.

`start-server.ps1` takes an encrypted, verified snapshot of the vault before opening it (`data\backups\`), stops a previous ModularChat instance on the same port, and prints the LAN addresses for your phone. Useful switches:

| Switch | Effect |
| --- | --- |
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

```json
{
  "default": "9router",
  "providers": [
    { "id": "9router", "baseUrl": "http://127.0.0.1:20128/v1", "apiKeyEnv": "NINEROUTER_API_KEY" },
    { "id": "openrouter", "baseUrl": "https://openrouter.ai/api/v1", "apiKeyEnv": "OPENROUTER_API_KEY",
      "models": ["deepseek/deepseek-v4.1-flash", "z-ai/glm-5.2"] },
    { "id": "local", "baseUrl": "http://127.0.0.1:8080/v1" }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `id` | Short name, used as the route prefix (no `:`) |
| `baseUrl` | OpenAI-compatible base URL (the server calls `/models` and `/chat/completions`) |
| `apiKeyEnv` | Name of the environment variable holding the key (preferred). `apiKey` is also accepted, but keeps the secret in the file |
| `models` | Optional fixed model list; without it the provider's `/models` is queried |
| `requestLog` | Optional path to 9Router's SQLite request log, used to confirm the effort the upstream model actually received |

**Routes.** A conversation stores a route: `provider:model` (e.g. `openrouter:deepseek/deepseek-v4.1-flash`). Routes without a known prefix go to the `default` provider, so plain names such as `cbai/deepseek-v4.1-flash` reach 9Router.

Without a `providers.json`, a single provider is configured from `MC_UPSTREAM_URL` and `MC_UPSTREAM_API_KEY` (`NINEROUTER_URL`/`NINEROUTER_API_KEY` are accepted as aliases). API keys stay on the PC; the phone only receives a pairing token.

## Presets (models)

The app offers a model only when its route matches a **preset** in `server-rust/src/presets.json`. A preset defines how to talk to a model family:

```json
{
  "id": "glm-52",
  "label": "GLM",
  "selectable": true,
  "family": "glm",
  "match": ["glm-5\\.2(?:[^0-9]|$)"],
  "exclude": [],
  "preferredRoutes": ["cbai/glm-5.2"],
  "efforts": ["high"],
  "defaultEffort": "high",
  "thinkingCanDisable": true,
  "thinkingDefault": false,
  "thinkingOffShape": "omit",
  "thinking": "enabled",
  "sampling": "default",
  "providerReserve": 16384,
  "contextWindow": 1000000,
  "maxOutput": 131072,
  "capacitySource": "documented"
}
```

- `match` / `exclude`: regular expressions tested against the lowercase route (provider prefix included). The first matching preset wins; the last entry (`generic`) catches everything else and is not offered.
- `efforts`, `defaultEffort`, `effortByRoute`: reasoning effort sent as `reasoning_effort` (or the provider-specific shape validated by `scripts/validate-routes.mjs`).
- `thinkingCanDisable`, `thinkingDefault`, `thinkingOffShape`: whether the chat menu shows the thinking switch, its default, and how "off" is sent (`reasoning_effort: "none"`, or `"omit"` to send no reasoning parameter, needed by GLM).
- `sampling`: `"default"` keeps the provider's sampling; `"temperature"` sends the conversation temperature.
- `providerReserve`, `contextWindow`, `maxOutput`: token budget planning.

To add a model: add a preset (before `generic`), add its `id` to `PUBLIC_PRESETS` in `client/src/api.js` (and optionally to `client/src/default-presets.js`, the offline fallback), rebuild client and server.

Presets included: DeepSeek V4.1 Flash, Claude Opus 5.5, Claude Sonnet 5.5, Gemini 3.8 Flash, Kimi K3, Muse Spark 1.3, Step 5 Preview, GLM (5.2).

## Local memory models

Memory extraction, curation and embeddings run on two local models through llama.cpp, started and stopped by the server on demand (ports 20481/20482, idle shutdown after 15 minutes):

```powershell
python scripts/download-memory-models.py   # downloads and verifies the pinned GGUF files into data\model-staging
```

Then place the files where the server looks for them (defaults under `%USERPROFILE%\ModularChat-local`), or point to them:

| Variable | Default |
| --- | --- |
| `MC_LOCAL_HOME` | `%USERPROFILE%\ModularChat-local` |
| `LLAMA_SERVER` | `<home>\llama-cuda\llama-server.exe` |
| `LOCAL_CHAT_MODEL` | `<home>\models\Qwen3.5-9B-Q4_K_M.gguf` (falls back to `Qwen3.5-4B-Q5_K_M.gguf` if the 9B is absent) |
| `LOCAL_CHAT_CTX` | `16384` |
| `LOCAL_FIT_MARGIN_MIB` | `768` (VRAM left free by `--fit`; the rest of the model goes to RAM) |
| `LOCAL_CHAT_TIMEOUT_S` | `600` |
| `LOCAL_EMBED_MODEL` | `<home>\models\embeddinggemma-300M-Q8_0.gguf` |
| `LOCAL_IDLE_MS` | `900000` |

Without local models, chatting works; memory jobs stay queued until the models are available.

## Android app

```powershell
cd client
npm run build
npx cap sync android
cd android
.\gradlew.bat assembleRelease
```

To sign the release, create `client/android/keystore.properties` (git-ignored) or point `MC_KEYSTORE_PROPS` to one:

```properties
storeFile=C:/path/to/release.jks
storePassword=...
keyAlias=...
keyPassword=...
```

Keep the keystore safe: Android refuses updates signed with a different key. Install `client/android/app/build/outputs/apk/release/app-release.apk` (or `adb install -r <apk>`), open it, enter the PC's **local HTTPS address** and the pairing code. You can also configure an optional internet HTTPS address for the same PC. Pair for the first time on the local network.

See [OPERACAO.md](OPERACAO.md) for the HTTPS gateway, certificate trust and firewall setup. For a private local CA, generate the host-specific trust resources before building the APK. The Rust backend can run on loopback HTTP behind the HTTPS gateway; Android connects to the gateway over HTTPS.

## Tests

```powershell
cargo test --manifest-path server-rust/Cargo.toml   # Rust unit and contract tests
npm test                                            # Node tests driving the Rust executables
node scripts/verify-mobile-ui.mjs                   # UI regression in headless Edge (needs client/dist)
```

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
client/            React + Vite web app, also packaged as the Android app (Capacitor)
  android/         Native shell: biometric lock, Keystore-backed cache, icon switching (Java)
  src/             UI, gestures and motion, glass material, eye care, API client
server-rust/       Server: HTTP API, generation, presets, providers, context assembly, memory jobs, vault
  src/store/       SQLite store (single owner thread), schema, migrations, trash, export
scripts/           Route validation, benchmarks, UI checks, model download
hosting/           Generic Caddy template (private runtime state stays outside the repository)
tests/             Node integration tests and fixtures (fictional data only)
start-server.ps1   Windows launcher (backup, sandbox, environment)
OPERACAO.md        Portable private HTTPS hosting and Android trust instructions
```

## Security notes

- The vault is encrypted at rest; the plain database exists only in memory.
- Every device must pair; tokens can be revoked in **Configurações → Pareamento**.
- Provider API keys never leave the PC.
- Conversation text is sent to the provider you choose for generation and to the local memory models.
