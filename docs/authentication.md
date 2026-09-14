# Authentication and owner-controlled live validation

Large accounts: READY no longer shares the 4,000-entry account ceiling or the 4 MiB
ordinary-event queue limit. A bounded startup path retains all admitted navigation and
permissions, while failed optional read-state/settings/presence/emoji sections produce a
feature warning without aborting valid login. Account identity, relationships, session ID,
resume address, navigation and permissions remain required and validated. Safety-budget
failures still stop with a static explanation; neither an unlimited payload nor guaranteed
interoperability is promised. See [storage policy](storage-policy.md#large-account-startup-september-14-2026)
for component limits. The reported issue #154 build and live trigger are not yet verified.

Linux uses GTK4/WebKit6 with a fresh ephemeral NetworkSession and persistent credential
storage disabled. Normal TLS validation remains enabled. Scripts run at document start only
in the top Discord frame. Navigation permits HTTPS hcaptcha.com and its subdomains on
port 443 for embedded challenges, using the same origin validation as invite verification.
The main-document response and candidate origin checks still restrict login to
https://discord.com. Website-data access requests are allowed only for hcaptcha.com
or its subdomains embedded in discord.com, while the main page is still Discord.
This uses the existing ephemeral session and does not enable persistent storage.
It removes a possible challenge-state blocker; live acceptance and the reported Linux
QR/CAPTCHA loop remain unverified. Popups, downloads, file choosers, other permission
requests, HTTP-auth, notifications and printing are denied; embedded challenge
availability remains unverified.

WebKit6 script-message callbacks lack trusted sender-frame metadata. The callback accepts
only a boolean wake signal. A protected main-frame closure retains one ASCII candidate of at
most 2113 bytes (65-byte capability plus 2048-byte token). A native main-frame query checks
origin and result bounds before creating a Rust string; Rust checks URI, capability, lifetime,
document generation and SessionSecret validation again. Resource-load Authorization observation
is not used: a request destination and current top-level URI do not prove its initiating frame.
Queries are at least 100 ms apart, with one cancellable evaluation and one secret slot, whether
or not a bridge wake arrives. Reads are repeatable until native acceptance and view teardown;
an evaluation error or null result can retry without losing the candidate. Navigation rejects
old-document results and clears unconsumed candidates; a fresh committed document can retry.
Only one candidate can be forwarded to the desktop per login window. A child frame can only
request a query of the main frame. The shared
fetch/XHR observer preserves original calls, return values and exceptions; observer failures
cannot escape into Discord's requests, and a failed bridge delivery does not latch capture.

The existing Linux login and verification settings disable HTML media and Web Audio to avoid
initializing an unnecessary GStreamer audio path in the login page. They do not change native
voice or attachment playback. The Linux login user agent remains the existing
`client_core::fingerprint` identity; neither challenge acceptance nor the reporter's failure
cause is established by that setting. TLS, device permissions and challenge handling are unchanged.

The Linux login top frame additionally reports `navigator.vendor` as `Google Inc.` and
provides an empty `window.chrome` namespace when absent, matching two identity hints of
the existing Chrome user agent. These are explicit identity overrides, not a Chromium
engine or proof of detection avoidance. Existing namespaces are preserved; failures do
not interrupt the login bridge. The namespace can select Chromium-specific page code
that WebKit cannot run, so this can also regress compatibility. No Chrome APIs, User-Agent Client Hints, plugins,
WebGL/canvas results or automation flags are fabricated. The separate invite-verification
window and native REST/Gateway identity are unchanged. Live QR success remains unverified.

Close/drop invalidates pending results, clears the secret/scripts/handler, cancels evaluation,
stops loading, terminates the ephemeral web process and destroys the GTK window. GLib pumping
checks a 2-ms deadline between at most 16 callbacks; one native callback may exceed that time.
These are implemented limits, not measured teardown/storage or live login compatibility.

Login exit status distinguishes owner cancellation, the ten-minute lifetime and an unexpectedly
stopped web process. Explicit cancellation discards even a candidate already queued in the same
GTK pump; teardown cannot relabel cancellation as a crash. The failure card offers **Copy login
diagnostics**. Only that click formats/copies a report (under 4 KiB): app/OS/display category,
available GTK/WebKit versions, elapsed seconds, saturating counters, native-candidate acceptance
and the exit reason. It contains no token, capability, account identifier, URL, page text or raw
error. One fixed-size record stays in memory until a new attempt, connection, logout or exit;
there is no automatic upload, log or credential-store/SQLite write.

Issue #173 reports Arch/Wayland build `ee8c246` (`1.0.0-nightly.20260914.16`), which predates
the login-media protection on main. The handoff retry and cancellation defects are separately
reproducible with synthetic tests; this does not establish which failure occurred on that machine.
The ignored Linux `native_login_webkit_retry_and_lifecycle` test uses inline synthetic HTML,
not a Discord login. Run it in an isolated network namespace on a GTK/WebKit desktop:
`cargo test --locked -p platform --features winit/wayland native_login_webkit_retry_and_lifecycle -- --ignored --test-threads=1`.
For the visible failure fixture use `cargo run --locked -p serein --features demo -- --demo --demo-login-failed`.
Packaged Arch/Wayland login with owner-controlled credentials and 2FA remains required before
claiming the report resolved. Do not automate credentials or challenges.

Serein uses Discord’s official login page in a temporary platform webview, not OAuth. The credential handoff is unofficial and live-unverified; see the compatibility matrix. Complete authentication yourself, in the application. Never send passwords, tokens, MFA codes, QR screenshots, or private message contents to the coding agent, issues, logs, or CI.

1. Build `cargo run --locked` on a supported platform. Use a private conversation controlled by the account owner. The owner enables the private-test acknowledgment and presses **Sign in with Discord**.
2. Complete one of the real login methods available in Discord’s page. Do not bypass a challenge or spoof a fingerprint if Discord rejects the engine. Cancel if the page or handoff is unsupported. The webview expires after ten minutes and closes when it supplies a candidate token.
3. Native REST verifies `/users/@me`, rejects a bot account, retrieves the gateway location, and waits for normal-user READY. A socket opening is not authentication success. The token is saved in the OS credential store only after readiness; if saving fails, the UI reports session-only login.
4. Choose the existing private channel/DM. Load one 50-message page. Deliberately compose and send one short test message. Verify it appears in an official Discord client. Reply from that official client and verify the reply appears natively through Gateway. Do not count an offline fixture, matching text, or a bot reply as success.
5. In the same conversation, verify an edit, deletion, HTTP/Gateway confirmation ordering, permission rejection if available, disconnect/resume and non-resumable reload. Keep traffic small; run stress tests only against synthetic transports.
6. Quit after local saves finish. Relaunch and verify credential-store restoration and saved draft recovery without opening the webview. Inspect recovered drafts before sending: an interrupted send can have succeeded remotely.
7. Log out and verify saved-login deletion and local account cache/draft cleanup. The UI must show credential-store or SQLite deletion failures. Local logout does not claim remote session revocation.

Record only date, OS/build, methods tested, pass/fail and redacted failure category in the task PR description. Never record credentials, account/channel IDs, signed URLs, message contents or QR data. **No real owner-controlled session was supplied or exercised during implementation; the live milestone remains blocked.**

Every build's sign-in screen also offers "Sign in with a token": the owner pastes a session token they already hold, for example from another signed-in Serein install. It skips Discord's hosted login page entirely, still requires the owner-authorization checkbox, still validates through `SessionSecret::from_owner_input`, and is saved to the OS credential store the same way a normal login is. It never reads or extracts a credential from another application.

Saved-login startup reports credential lookup separately from Discord connection. A found credential advances the status immediately; absent/invalid/unavailable outcomes remain visible. The UI stops awaiting lookup after 10 seconds and permits manual hosted login. Manual login, preview, logout and timeout discard late lookup results. The synchronous OS call remains on the existing single worker; no background retry workers or plaintext fallback are created.

Gateway login keeps optional voice identity metadata within 4,096 entries / 1 MiB per cache.
Extra referenced users or merged members are omitted from that cache; eligible voice participants
retain their IDs and use the existing fallback when their name/avatar is unavailable. Previously,
crossing either cache budget could stop login before READY, even with no voice participants.
Offline WebSocket regressions cover 4,097 referenced users, UTF-8 names crossing the byte budget,
and 4,097 combined supplemental members. This reproduces one cause of issue #143; the affected
reporter's actual account payload has not been inspected or tested.

Large READY payloads (September 14): a normal account in many servers receives a READY whose
decompressed JSON exceeds the 4 MiB per-event bound because every joined server ships its
channels, roles, emojis and settings inline. The old 4 MiB Gateway limit reported
"Decompressed Gateway payload exceeds 4 MiB; connection stopped" and ended login. The WebSocket
frame, zlib-stream compressed/decompressed accumulation, outer Gateway packet, READY and
READY_SUPPLEMENTAL decoders now share a 64 MiB bound (`MAX_GATEWAY_WIRE`). Individual dispatch
events and REST responses keep their existing limits. Navigation and permission storage now
use the larger, separate account budgets above; voice rosters and presence caches remain bounded
at their existing sizes. An offline WebSocket regression logs in with a READY between
4 and 64 MiB. The reporter's actual payload size has not been inspected.

Hard Gateway frame/compressed/decompressed payload, navigation, actual voice roster, and desktop
synchronization queue limits remain enforced. Their capacity failures identify the limit using
fixed local text and still terminate the connection. An oversized frame stops immediately during
Hello or subsequent startup instead of being retried as a network failure. No payload contents,
account identifiers or credentials are added to the error. Credential saving still requires READY.
