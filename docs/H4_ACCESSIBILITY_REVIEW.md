# H4 runtime accessibility — issue #231

Base: `8df0d7c6a03582d61e6046e2a77cb27bf605e4f4`. Owner: the #231
implementation change. Scope: the existing built React critical journey in the
private deployment envelope. No API, persistent state, dependency, or backend
architecture change is needed.

## Plan and current-truth review

1. Repair chat and character-drawer focus entry, keyboard exit, and restoration.
   Reuse the installed Radix dialog for the drawer. Chat remains a non-modal
   labelled region; returning focus to the chapter dismisses it.
2. Give committed chat, branch, and world outcomes named announcements; keep
   partial streaming text out of the committed live log. Preserve retry and
   unknown-outcome semantics and never announce an uncommitted turn as success.
3. Add Reader/Characters main landmarks and reading-version grouping. Apply
   reduced motion at app composition and both imperative scroll callers; size
   the chat panel against the dynamic viewport with reachable controls.
4. Strengthen the existing browser helpers and specs: real forward/backward Tab
   reachability, element-identity cycle detection, bounded-walk failure,
   visible focus, announcements, open-overlay reflow, and reduced motion.
5. Run frontend dependency audit, type, lint, FSD, unit, build, and built-app
   Chromium gates. Capture opened-overlay screenshots and re-review the diff.
   Required CI and independent review remain merge evidence.

Source review confirms missing focus handling in both overlays, a fixed 560px
chat height, smooth scrolling in both callers, unnamed outcome containers,
generic Reader/Characters main containers, and a tab helper that silently
accepts its iteration ceiling and compares labels rather than DOM identity.

## Contract/design review before implementation

Disposition: proceed with the bounded implementation; this is an implementer
review, not independent approval. No non-author review is currently recorded.

- Preserve all FSD public APIs and downward dependencies. No backend change is
  justified by these frontend defects; cloud-native/DDD/service boundaries stay
  governed by the existing architecture gate.
- Do not add a focus-manager abstraction or a second browser harness. Native
  controls, CSS and the existing Radix dependency cover the drawer contract.
  Chat must not trap keyboard focus or hide the chapter from assistive tools.
- Selecting a drawer character removes that trigger: restore chat focus to the
  persistent Characters toolbar button. A character-card trigger remains valid
  on the Characters page. Closing a drawer must not steal focus from newly
  opened chat. Disabled/loading input must not prevent focus entry or closing.
- Committed messages and world journals are durable display inputs. Stream
  deltas and pending requests are provisional. Separate log/status/error
  semantics prevent token-by-token or duplicate result announcements.
- Negative checks must detect exhausted walks, identical-label controls,
  hidden/offscreen focus and left as well as right overflow. Modal cycles need
  explicit containment checks; a cycle alone never proves page reachability.
- Rollout: ordinary frontend artifact replacement. Abort on unreachable
  controls, lost focus, misleading announcements, or browser failures; revert
  the frontend commits. No migration, data rollback, or new telemetry applies.
- Human screen-reader/mobile/non-author evidence in #169 and provider/live
  qualification in #222/#229/#230 remain separate requirements. Automation
  cannot close those requirements.

## Implementation and adversarial review

### Follow-up plan review: focus behind non-modal chat

A fresh complete Tab walk on `b35a120` at 320px fails at the Reader translation
button: the button receives focus behind the floating chat panel. The previous
named-target checks passed because they did not inspect intermediate stops.

Reviewed decision: dismiss the non-modal panel when focus enters the background,
without redirecting that focus. Keep its local draft mounted in each owning
page so dismissal cannot discard unsent input. Reopening the same character
restores that draft; a different character, identity, chapter, or novel gets a
fresh keyed panel. Existing spoiler/ownership invalidation still unmounts it.
Use one document focus listener inside the existing widget and one visibility
state per caller. No focus trap, responsive modal mode, placement algorithm,
new global store or persistence is needed. Add complete Tab/Shift+Tab walks and
draft-reopen evidence before considering the finding closed.

The strengthened walk also exposes the import form's visually clipped file
input and a textarea hidden by its nested scrolling form/footer. Reviewed
correction: use a visible native button to open the existing file input, and
let the entire bounded dialog scroll with a viewport-limited textarea. The
input resets after selection so repeat selections work; its own empty-file
label must not contradict the authoritative selected-file list. The button
keeps the file chooser keyboard accessible without a second selection state.

Disposition: implementer plan review approves this bounded correction; an
independent approval is still not represented by this record.

Reviewer: Codex, the implementer (2026-09-05). This pass is not independent
human approval. The follow-up uses immediate background-focus dismissal with a
mounted local draft, explicit focus restoration before DOM removal, a native file-picker
button, and one scrolling import dialog. Independent design/final-evidence
review and required CI remain merge gates.

The initial browser pass found a real focus race: mounting chat while Radix was
still releasing the drawer redirected focus into the disappearing drawer.
Selection now waits for `onCloseAutoFocus` before mounting chat. Reopening the
drawer closes the old panel; chat restores focus only when focus was still
inside it. Reader tests wait for opening but retain synchronous assertions for
spoiler/progress/persona invalidation.

Chat's store publishes an optimistic user message before its SSE commit.
Unresolved messages and deltas now remain outside the saved live log until
commit. Browser evidence covers rejection, visible retry, unchanged idempotency
key and saved results; the focused widget test covers a visible partial stream.
World requests no longer announce both ordinary pending status and an
unknown-result alert while the request is still running.

The installed Motion hook captures only the initial preference. A small shared
`useSyncExternalStore` subscription uses native media-query change events for
both app animation policy and chat scrolling; Reader's explicit scroll reads
the same preference. Browser checks cover reduced motion at initial load and a
live operating-system preference change. No new dependency is needed.

The browser gate deliberately injects identical-label controls, a walk ceiling,
left-clipped content and obscured focus. It also submits an actual world action
and checks a newly committed turn, avoiding the former broad button selector
and pre-existing narrative text. Tab walkers distinguish actual DOM elements
and fail at their bound; named-target checks establish reachability separately
from modal containment. Named-target checks now inspect every intermediate
stop, with an injected covered-stop test proving that a visible destination
cannot hide an inaccessible path. Focus checks allow native scrolling to settle
and then require a visible, unobscured control. Native scroll padding keeps Reader's
fixed bars clear of keyboard targets.

## Executed evidence and remaining boundaries

Local environment: Linux, Node `24.18.0`, pnpm `11.24.0`, Chromium
`151.0.7922.34`. CI uses the repository's Node 26 profile and remains
authoritative. API responses are synthetic fixtures; widgets, routing,
browser focus, styles and stream parsing are real.

| Check | Result |
|---|---|
| Frozen dependency installation and production dependency audit | Pass; no known production vulnerability reported |
| `pnpm lint`, `pnpm type-check` | Pass |
| `pnpm lint:fsd` | 84 files, 339 edges, zero violations; negative self-tests pass |
| `pnpm test --maxWorkers=2` | 26 files, 220 tests pass |
| `VITE_API_URL=/api pnpm build` | Pass |
| Opened overlays in final built-app suite | Pass at 320×720, 568×320 and 320×256 |
| `pnpm exec playwright test` (final built app, no retries) | 48 pass |
| CI workflow | YAML parses; browser failure artifacts retained for 7 days |

An unrestricted local unit run timed out in the existing one-second lazy-home
route assertion (219 passed, one failed); the bounded-worker full run above
passed without changing that test or its deadline. Exact-commit CI results
belong to the linked pull request.

One follow-up browser attempt had blank-page startup timeouts in the advanced
rules and batch-import screens plus one keyboard visibility failure, then ended
with SIGTERM before completing the suite. Its orphaned local preview process
was identified and stopped before starting the final clean preview. That attempt
is not passing evidence and its startup cause is unconfirmed.
The harness now retains failure traces/screenshots, and the browser CI job uploads
them on failure. This diagnostic limitation remains visible to independent review;
no timeout, retry count, or assertion was relaxed. The final clean-preview
run passed all 48 tests, including each previously failing path, without retries.

Visual review of the final built app checked the visible focus ring and
reachable header/input/send controls in the opened overlays:
[320px character drawer](./evidence/h4-character-drawer-320.png),
[320px chat](./evidence/h4-chat-320.png),
[short-landscape chat](./evidence/h4-chat-landscape.png), and the updated
[keyboard-opened file picker result](./evidence/batch-novel-import.png).

No backend, API, SQL, migration, auth/session, provider, persistent data,
dependency version, or lifecycle contract changes are included. Backend
runtime/cloud-readiness and real provider/persistence drills are outside this
frontend change's evidence. #169 still requires a human with a named real
screen reader and independent journey completion on merged main. #222, #229,
#230 and #236 retain their separate live-qualification requirements.
