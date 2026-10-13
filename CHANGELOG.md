# Changelog

Project changes are recorded here. The **Unreleased** section collects changes
without a published version; assessments do not represent implemented features.

## Unreleased

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
- Updated the README and OPERACAO.md with portable instructions, certificate
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
