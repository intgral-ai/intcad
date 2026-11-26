# Next-step suggestions

**State, 2026-11-24.** The ghost (decisions 17 to 20) is built on `develop` `f18a9aec2`: its test
list, results and open points are in `autocomplete-tests.md`, and how it works for a user is in
`docs/agent-chat.md`. The sections below are the record of the rounds before it, in order; where
they say chip, card, 5 s, or "Tab sends a chat message", the ghost replaced it.

## First round

Linear INT-1458, spec "规格：预测下一步（最简版）". After the user edits the design and pauses,
the connected agent guesses one next modelling step; a short chip at the end of History shows it;
Tab (or a click) sends it as a chat message whose turn always runs as a preview (PIL-300), and the
preview's card takes Tab / Enter (accept) and Esc (reject). Esc on the chip, a key press or
another edit dismisses it; looking around the model does not. Off by default. The first round is
a demo judged by eye, not a hit-rate measurement.

Base: `tianwei/agent-preview-accept-rebased` (local, not pushed): PIL-300 rounds one and two
(previewed edits computed off the window thread, Stop, Ask Here on Ctrl+K) rebased onto `develop`
`9df336a77`, plus a commit that shows the preview card in the one-page Assistant's web page.

## How it works

```text
 user edits, then pauses 5 s ──► predict::tick (once a frame, before anything reads keys)
                          │  this revision already asked about?        → nothing
                          │  agent busy or running a skill, a preview,
                          │  Ask Here box, dialog, tool or sketch open,
                          │  or the History strip not on screen?        → nothing
                          ▼
              guessing session of the chat's agent ── no MCP bridge, no tools,
              (second ACP process, fresh session      permission requests declined
               per question)                          (cannot change the design)
                          │  {"step": "...", "reason": "..."}   (late answers shown if the
                          ▼                                       revision and selection held)
              chip "Next (Tab): step…" after History's last item; the reason in its tooltip
                          │
                          ├── Tab / click ──► send_with(step, the guess's own selection)
                          │                    with force_preview: the turn runs on a copy;
                          │                    its card: Tab / Enter accept, Esc reject
                          └── Esc (turned down) / key / edit → dismissed
                              (orbit, pan, zoom and clicks in the view are not actions)
```

- `crates/ui-egui/src/agent_chat/predict.rs`: the pause, the question, the answer, the chip, the
  card's keys, the log. Window state: it lives on the chat but is swapped back on tab switches
  like the settings.
- The question carries the selection (`context.rs`), the last 8 History steps with their sizes
  and bodies, the last 6 undo labels, the design's usual values (fillet radii, chamfers, wall
  thicknesses, hole diameters, most used first), the steps the user turned down (Esc on the chip,
  or Reject on their preview), and the user's last 6 chat messages. It asks for exactly one step
  and one reason, as JSON, each at most 12 words, in the app's UI language, naming things as the
  user sees them (no ids such as `F1:-y`). The reason is cut at 120 characters on reading.
- Time and cost: `backend::Usage`, read from ACP's `usage_update` (`cost`) and the prompt reply's
  `usage` (tokens). Each guess is logged with its time, tokens, cost, `billed` (the question reached
  the agent) and outcome under `nextStep` in `ui.agent_chat` and in the app log (`next-step guess
  N: …`), including guesses dropped by an edit after they were sent. Nothing is estimated in the
  app.
- The account-free predictor tests use a recording backend and real design copies. The optional
  desktop recorder uses a supported Claude Code session, not a custom agent ID.

## Decisions taken here (open for review)

1. **The guess runs in a second, tool-less session of the same agent**, not as a hidden turn of
   the chat's own session. It keeps the guess out of the chat's history and context, and it makes
   "the design does not change before the user accepts" structural: that session has no way to
   reach the design. Cost: a second agent process while the setting is on. Recommendation: keep.
2. **Tab's turn always runs as a preview**, whatever "Preview the agent's changes" says (it is off
   by default): `AgentChat::force_preview`, taken by the next turn's start. So no suggestion
   changes the design without the user's Accept. Suggestions are only made while the chat is in
   plain mode, because a skill's turn does not run as a preview.
3. **Tab is taken only for a chip drawn the frame before**: not with History on the left or in
   the Browser, in the Render workspace (which uses Tab itself), while a text field has focus, or
   in a dialog or the Ask Here box. Nothing is asked while the chip has nowhere to go.
4. **The chip's place shows status**: thinking…, connect an agent, paused: sign in, paused: the
   agent stopped. A failed guessing session is tried again after 30 s, doubling to 10 min, once
   the chat's agent is ready.
5. **Looking is not acting.** Pointer moves, clicks, drags and the wheel (orbit, pan, zoom, pick)
   neither dismiss the chip nor cancel a question; key presses, typing and edits do. An edit
   cancels the question out; an answer that arrives after a selection change is logged `stale`
   and not shown.
6. **One question per revision.** Selecting something else restarts the pause but asks nothing
   new; an edit does. Recommendation: keep (cost control).
7. **Tab sends the guess's own selection** (what was selected when it was asked, or none), not
   whatever is selected when Tab is pressed.
8. **Turned-down steps are told to the agent** ("do not suggest them again"), the last 5: Esc on
   the chip, or Reject on the preview Tab started. Their log outcomes are `dismissed` and
   `rejected`; Accept logs `accepted`, a turn with no change `no change`.
9. **The chat panel is left as it is on Tab**: the card's keys are on the chip ("Preview: Tab or
   Enter accepts · Esc rejects") while it waits, so the user need not open the panel.
10. **No guess while a preview exists** (running, summing, or waiting), nor while the agent works.
    Stop only shows while a preview computes, so it never meets a guess. Ctrl+K is a key press: it
    dismisses a chip shown; the box's Enter and Escape are its own.
11. **Only agents make suggestions; no direct model call** (Tianwei, 2026-11-21). A guess always
    goes to an ACP agent, never as a request the app makes to a provider's API itself, so API
    endpoints and Mastra workflows show "connect an agent". The cheaper path to try next is
    develop's built-in Mastra agent (#178, an ACP agent) with the guessing session on a small model
    and a low thinking level; `start_backend` sets no model today.
12. (Replaced by 21.) **A fresh session per question.** The cost of each guess stays the size of one question, and
    the reported session cost is that guess's cost. Recommendation: keep; revisit if start-up
    latency of real agents makes it slow.

## Test cases (`agent_chat/predict_tests.rs`, `acp.rs` tests)

Revised for the usability round (fix numbers from the review). Written and run in the same pass; not yet reviewed by a person (AGENTS.md rule 1 asks for that
before tests are written; this list is for that review). Real: the app state, egui input and
focus, the document and its revision, the preview copy, the prompt text, the reply parser. Faked:
both agents (a backend that records calls and returns scripted events) and the clock (egui input
time).

| Case | Input | Action | Observable result and pass criterion |
|---|---|---|---|
| Off by default | default preferences, agent ready | frames at 0…600 s | no question, no message to the chat, a started session shut down, no chip, `state: off` |
| Turned off | a guess shown | setting off, frames to 600 s | the guess goes, the session shuts down, still exactly one question |
| Pause; looking is not acting | setting on, pause 5 s | frames to 4.9 s; key A at 4.95; a click in the view, an orbit drag, wheel and pinch zoom at 9.91–9.92 | nothing before 5 s after the key; the pointer and wheel do not restart it; one question at 10 s; with a 12 s pause, none at 11 s, one at 12 s |
| Not while busy | agent busy, then a dialog open | frames to 60 s | no question until both clear |
| Nowhere to show it (fix 2, 8) | History in the Browser / on the left / hidden; Render workspace; a sketch; a tool; a skill | frames to 120 s, then cleared | no question while held; one once it clears |
| One per revision (fix 9) | a box; a reply listing two candidates | wait 600 s; Esc; select the box; wait; edit | the first step shown; no second question while shown, after Esc or after the selection change; after the edit, one question in a fresh session |
| Short chip, tooltip (fix 5) | a 110-character step, reported cost $0.12 | answer | chip "Next (Tab): …" cut at a word with "…" (≤ 60 characters of step); tooltip has the whole step, the reason, "opens it as a preview", Esc, "1 suggestions" and "$0.12"; a 400-character reason is cut to 120 |
| Esc dismisses | a guess shown | Esc, then Tab | gone, Esc used up, nothing sent, `dismissed`, the next question lists it as turned down |
| Keys and edits dismiss; looking keeps it (fix 4) | a guess shown | click in the view, orbit, zoom; then key A; separately an edit | still shown after looking; gone after the key (`dismissed`) and after the edit |
| Late answers (fix 4) | a question out | look around and press a key / select something else / edit, then the answer | shown / `stale`, not shown / cancelled, `stale` with its time, `billed`, `billedCalls` 1 |
| Tab: preview, own selection, panel unchanged (fix 1, 6, 7) | previews off (default), panel closed, Alpha selected when asked, Beta when Tab is pressed; the guessing session asks permission | answer, Tab; then a typed message | permission declined; revision held; one message with the step and the context Alpha (not Beta); the turn runs on a copy; `force_preview` taken; panel still closed; `sent`; the typed message afterwards does not run as a preview |
| The card's keys (fix 6, 8) | Tab's turn makes a box on the copy and ends | Tab / Enter / Esc on the waiting card | the chip says "Preview: Tab or Enter accepts · Esc rejects", `state: preview`, no question meanwhile; Tab and Enter accept (revision moves, `accepted`), Esc rejects (revision held, `rejected`, the step listed as turned down) |
| Tab only for a chip on screen; click is Tab (fix 2, 4) | a guess shown | History to the Browser, Tab; separately a click on the chip | Tab sends nothing; the click sends it as a preview |
| Ask Here does not clash | a guess shown; then the Ask Here box open | Ctrl+K; edit; frames to 60 s; Escape; box closed | Ctrl+K dismisses the guess; no question while the box is open; Escape is not taken; after it closes, one question |
| Tab with widgets on screen | a button and a text field | Tab, then a second guess and Tab in the text field | the first Tab moves no focus and sends; Tab in the text field is the field's (regression for a demo bug) |
| Time and cost logged | usage reported (1830 / 42 tokens, $0.0061) | answer 2.25 s after asking | `billed` once sent; `ms` 2250, tokens and cost as reported, `meanMs`, `totalCost`, `billedCalls` |
| Status in the chip's place (fix 3) | pause; question out; no agent; sign-in needed | frames | nothing during the pause; "Next step: thinking…"; "Next step: connect an agent" (`no agent`); "Next step paused: sign in to the agent" (`paused: sign in`) |
| Retry with backoff (fix 3) | the guessing process exits | frames at 6, 35, 36, 95, 96 s | `paused`, wake in 30 s; no start before 30 s; a start at 36 s (fails: no such program), then the next only after 60 s; waits 30, 480, ≤ 600 s |
| Reply parsing | JSON, fenced JSON, plain line, empty step, long text | parse | one step and reason, one line, at most 200 / 120 characters |
| Question contents (fix 8) | four features, a chat message; app in English, then Chinese | build the question | History lines with sizes, usual values with counts, nothing selected, the chat message, "at most 12 words", no ids such as `F1:-y`, "in English" / "in Simplified Chinese" |
| Search (fix 9) | the setting's label | keywords | predict, autocomplete and suggest find it |
| ACP usage | `usage_update`, prompt reply `usage`, bad cost | parse | context tokens and cost; turn tokens; a negative cost is no cost |

## Demo (2026-11-14, zhikun, headless GNOME, scripted agent, previews on, base `1284f4a87`)

Three designs, driven over the control channel (`ui.key` for Tab and Esc, `review` for Accept
and Reject), pause 5 s. Every Tab came back as a preview card; the revision did not move while
the card waited, moved by one on Accept, and stayed on Reject.

- **Case**: box 60 × 40 × 25 → *Fillet the 4 vertical edges of Case with 3 mm* (Tab, preview,
  Accept) → *Shell Case from its top face with 2 mm walls* (Tab, preview, Accept) → *Drill a 5 mm
  hole through the centre of Case* (Tab, preview, Accept). Revision 2 → 3 → 4 → 5.
- **Bracket**: base rounded by hand with 6 mm, then a boss → *Fillet … Boss with 6 mm* ("6 mm is
  the radius this design already uses") (Tab, preview, Accept) → *Shell Boss …* (Esc; revision
  unchanged) → after a hole by hand, nothing more is suggested (the dismissed step is not
  repeated).
- **Plate**: plate rounded by hand → *Drill a 5 mm hole through the centre of Plate* (Tab,
  preview, Reject: revision stays 3) → nothing asked again for the same design.
- **Holes** (420 × 420 × 10 plate with 400 holes): *Fillet the 4 vertical edges of Plate with
  3 mm* (Tab). The preview went through Running, Summing and Review on its worker for about a
  minute; no guess went out meanwhile or in the 6 s the card was left waiting; revision 4 while
  previewed, 5 after Accept.

In every Tab above the driver also left the card waiting 6 s (longer than the pause): no guess
went out.

Measured: the line appeared 5.03–5.05 s after the user's last change. The scripted agent answered
in 6–41 ms (8 guesses); estimated cost $0.0012–0.0015 per guess (254–366 input tokens). A first
recording on `develop` (no previews) and one on `12f39bb` gave the same steps and times. **A real agent was not
measured**: its latency and cost (including its own system prompt, which is far larger than the
1–1.5 k character question) are the open number for the meeting.

## Real agent (2026-11-14, Claude Code through `claude-agent-acp` 0.107.0, previews on)

12 guesses on the same designs (plus one extra case run and one stopped trial run): every one a
sensible next step (shell the case, standoffs for the board, join the boss to the base, the boss's
fillet at the design's 6 mm, corner mounting holes, countersinks, outer edge fillets). From pause
to line: median 10.3 s, 9.1–17.1 s (the 5 s pause plus 4.4–12.5 s of agent time, median 5.2 s).
As the agent reported: 23 k–44 k tokens of context per guess (Claude Code's own system prompt),
80–333 out, $0.009–$0.30 per guess, median $0.12, $1.84 for the 12. Every Tab came back as a
preview card (8 accepted, 4 rejected by the driver's plan); the revision held while each card
waited. Against the scripted agent's estimate ($0.0013, milliseconds), a real guess costs about
90 times more and takes seconds. A guess whose preview was rejected is not followed by another
until the user does something: the design and selection are the state already asked about.

## Usability round (2026-11-14, after the UX review)

The ten points of the review, on the same base. Checked with the review's scripted proxy
(`fake_agent.py`: delay, real-length answer, sign-in error, exit) and with Claude Code for two
guesses, on zhikun in the Black theme of a throwaway profile. The headless GNOME Shell there now
crashes when Xwayland connects (it worked earlier in the day; the GDM greeter restarted at 20:56),
so these runs use the app as a plain Wayland client: no Assistant web view (the panel stays
closed in these paths anyway), screenshots from `ui.screenshot`.

- Scripted proxy, one run (6 guesses, $0.0062 estimated): "connect an agent" before connecting,
  "thinking…" 4.4 s after the edit, the chip shown 7 s later with a 120-character reason cut in
  the tooltip; orbit (middle drag), wheel zoom and a click in the view kept it; Esc dismissed it
  and the next question listed it as turned down; Tab with previews off opened a preview card,
  the panel stayed closed, the revision held, and the chip said "Preview: Tab or Enter accepts ·
  Esc rejects"; Esc rejected (revision held, `rejected`, listed as turned down next time); a click
  on the next chip opened its preview and Enter accepted it (revision +1, `accepted`); with History
  in the Browser an edit and 10 s asked nothing and Tab sent nothing; a sign-in error showed
  "paused: sign in", the retry 30 s later met an exiting agent ("paused: the agent stopped"), and
  the next retry 60 s later recovered and showed a guess.
- Claude Code (`claude-agent-acp` 0.107.0), 2 guesses, $0.43: *Fillet the 4 vertical edges of
  Body1 with 2 mm* ("A plain 20×20 block usually gets rounded corners next.", 4.2 s, $0.30); Tab
  with previews off → preview card after 14 s, panel closed, revision held; Enter accepted. With
  the app in Chinese: *给 Body2 的 4 条竖直边倒圆角 2 mm* ("Body1 已做同样倒圆角，Body2 尺寸相同。",
  4.0 s, $0.13), no internal ids; Esc dismissed it.
- Search finds the setting as predict, autocomplete and suggest (the keyword table now also
  applies to checkbox rows, which have no label of their own). A two-character Chinese query such
  as 建议 does not match inside a label (search's existing rule: three characters for a match
  inside a word); 下一步 does.

## Checks run (zhikun, `ci` profile, base `1284f4a87`)

Usability round (base `ce45a191e`): `cargo clippy -p solidcraft-ui-egui --all-targets -D warnings`,
`cargo xtask lists`, `i18n`, `layers` and `ui-containers` pass; `predict.rs`, `predict_tests.rs`,
`prefs_search.rs` and `settings.rs` pass `rustfmt --check`, and the other touched files have the
base's own diffs only (`mod.rs` 6, `lib.rs` 6, `review.rs` 1). `cargo test --lib -- predict
next_step prefs_search agent_chat::settings`: 65 pass. The full `cargo test -p solidcraft-ui-egui
--lib` ran 4,899 tests in parallel (9 failures: the same 6 `mastra_bundle_*`,
`guarded_control_envelope…`, `variable_fillet_round_trip` and `ui_kit_guard…` as below) and then
stalled with no CPU on GPU tests (`viewport::tests`, earlier also `save_job`, `sketch_side_ui`,
`sketch_attachment_ui`, `surface_errors`) while other sessions used the machine's GPU; the 90 left
pass one at a time (and the stalled ones pass alone on `develop` too).

Earlier round:

`cargo clippy -p solidcraft-ui-egui --all-targets -D warnings`, `cargo xtask lists`, `i18n`,
`layers` and `ui-containers` pass; this change's files pass `rustfmt --check`. `cargo test -p
solidcraft-ui-egui --lib`: 4,887 pass, 9 fail, none in this change: 6 `mastra_bundle_*` and
`guarded_control_envelope_validation_is_integration_owned` (they fail the same way on plain
`develop` on that machine), `variable_fillet_round_trip` (in `tests/KNOWN_FAILING_TESTS`), and
`ui_kit_guard_tests::all_ui_and_apps_use_ui_kit_controls`, which flags the raw `TextEdit` in the
base's `ask_here.rs`. The base itself also has rustfmt diffs (`panel.rs`, `review.rs`,
`tests.rs`) and does not build for wasm (`panel.rs`: `ui_kit::assistant_tool_frame`,
`assistant_actions` and `status_dot` are not in the wasm build). The full `cargo xtask ci` was not
run.

## Demo round (2026-11-20, branch `tianwei/next-step-demo`, local, not pushed)

Recorded demos of the suggestions, without and with the design's project context. New on
screen, composed from the UI kit only (`docs/ui-kit.md`; a first version painted its own glows,
gradients and colours and was rebuilt after review):

- **The card in the view** (`agent_chat/predict_card.rs`) on the anchored HUD host
  (`Container::navigation_card`, `Surface::Navigation`) above the navigation bar. Thinking: the
  Assistant spinner, "Predicting your next step…" and seconds. A guess: the Assistant approval frame
  ("Next step · suggested by …"), the step, its reason, the project memory it follows in the
  Assistant detail box, a **Based on** row (the project as a workspace chip, then History, the
  design's most used values, the selection, the user's chat notes and steps turned down as neutral
  badges), "N of M steps by Tab", and Accept (primary) / Dismiss buttons with their keys beside them
  as menus show shortcuts. Previewing: spinner and the copy's current step. Preview ready: the
  approval frame with the body volume changes and Accept / Reject. Accept and Reject report through
  the app's status toast ("Added to History: …").
- **A ghost chip** at the end of History: the step's command as a History chip, faded as rolled-back
  items are (it is not in the design yet), selected while its preview waits.
- **A key strip** for recordings only (`SOLIDCRAFT_DEMO_KEYS=1`): pressed Tab, Esc, Enter and
  Ctrl chords on a navigation card at the view's bottom left, in the navigation tooltip's keycap
  outline, so a raw recording shows the keys. With the same flag, the app draws the person's
  pointer (control-channel input moves no screen cursor) with a press ripple. It is drawn like the
  agent cursor, but in neutral tokens and without its "Agent" tag, so the person's hand reads apart
  from the agent's.
- **Hand steps** (`examples/demo/hand_steps.py`): the person's modelling through the UI (toolbar
  buttons, plane and face picks, a profile click, values typed a character at a time). The
  bracket project fixture is in `examples/demo/bracket_project/`.
- **The project brief** (`agent_chat/project_brief.rs`): when the saved design is in a project
  (`.intcad/project.toml` at or above it), the question carries the project's name, process and
  material, confirmed `design-intent.toml` keep entries, and for a project the user trusts this
  session (`ui.agent_chat` `trust_project`; PRD F-6) also `design-intent.md`, `AGENTS.md` and the
  confirmed `know-how/<rule>/SKILL.md` rules (drafts are never given, N-4). Notes are numbered; the
  agent answers which ones its step follows (`"memory": [2]`), the card shows them, and Tab's
  message carries their text to the agent doing the step. The app reads the folder and writes the
  brief (R-1, R-2, F-7); the agent never reads `.intcad/`. Trust is kept in memory only, not yet in
  preferences as the PRD asks. The PRD draft still names the folder `.intgral/`; develop's code and
  this reader use `.intcad/`. A demo project is in `examples/demo/next_step_project/`.
- Current recorder: `examples/demo/record_next_step.sh APP NEW_OUTDIR [case|bracket]`
  (headless GNOME Shell, Xwayland and GStreamer MJPEG capture). It seeds one part, waits for a
  rendered ghost, and sends Tab once. The `events.jsonl` evidence checks the changed design,
  matching body measurements and one undo entry, without a chat turn or review. With
  `PROJECT_TEMPLATE` set, it copies that folder under `NEW_OUTDIR/project` and refuses existing
  destinations. Measurements still use `next_step_ab.sh` with `NSM_SCRIPT=autocomplete_measure.py`.

Historical takes from the earlier chat-preview prototype (`demo-takes/README.md` has beats and
timecodes), Claude Code through
`claude-agent-acp` 0.107.0, pause 3 s, previews on. In both, the user notes in chat that it is an
enclosure for a 50 × 30 mm board and makes a 60 × 40 × 25 box.

- **Baseline** (loose design, 3 min 31 s, six Tabs): shell 2 mm, four 5 mm standoffs, 2.5 mm screw
  holes in them, a 2 mm lid, four screw holes through the lid, a 10 × 5 mm cable opening.
- **Project memory** (saved in the trusted demo project, 3 min 57 s): shell 2.4 mm, Ø6.5 × 6 mm
  bosses with 3.6 mm insert holes, 3 mm fillets, a snap lid (dismissed; the user makes the lid by
  hand), 3 mm lid fillets, a 1.6 mm snap lip 0.2 mm inside the walls, a 9.5 × 3.8 mm USB-C opening.
  Each card names the note its step follows. The design intent forbids screws through the lid, and
  none were suggested.

Agent time per guess 3.5–8.9 s; Tab to preview ready 13–35 s. Guesses cost about $0.12 each as
reported, and the brief adds about 1,700 characters to a question. Two baseline takes with the
project take's plan stopped early: the agent made its own lid, the scripted hand-made lid collided
with it, and the next step changed nothing. The kept baseline accepts six in a row. In the first
takes (made with the superseded card), only one of four standoffs showed from the camera: the agent
made one and a 2 × 2 pattern, and the preview's +0.5 cm³ matched four, with the walls hiding three.

## Built-in agent and a small model (2026-11-21, branch `tianwei/next-step-mastra`, local)

The work above, put onto `develop` `9ce777903` (#142 brought the preview base, #178 the built-in
agent; three conflicts, each both sides kept), plus guesses on develop's built-in agent (Mastra
Code over ACP) on a small model, so a guess can run on the user's own plan sign-in.

- **Started as the chat starts it.** When the chat's agent is the built-in one, the guessing
  process gets the same data folder (`MASTRA_APP_DATA_DIR`, where Mastra keeps plan sign-ins) and
  the chat's last `setup` (its keys and custom addresses, `Conn::last_setup`). Its first session is
  asked for once a snapshot says something is connected; with nothing connected the chip says
  "paused: sign in". Without the data folder it is not started, as the chat's agent is not.
- **Tuned before each question** (`next_setting`), one setting at a time, each set once per
  session: the built-in agent's read-only mode (`fast`, Mastra Code's Explore: no tool that
  writes), then the model, then the lowest reasoning effort the model takes (off, none, minimal,
  low). The question waits until the last setting is answered; a refused setting is logged and the
  guess runs on what the session has. Mastra keeps a session's mode, model and effort on its own
  thread, so the chat's sessions are not changed (checked in `@mastra/core`'s `SessionModel.switch`).
- **`agent_next_step_model`** (new preference, no control in the page yet): empty picks the
  first small model the agent lists (a name with mini, haiku, flash, nano, lite or small as a
  word) of the provider the session is on, else of any provider; `same` changes nothing (the
  behaviour before this round, for the comparison); anything else is a model id, used if listed.
  Applies to any agent that lists a model option; the mode is set for the built-in agent only.
- Each guess's log entry and log line now name the model it was asked on.

Decisions taken here (open for review):

13. **A small model is the default for guesses** where the agent lists one; `same` keeps the
    agent's own. Recommendation: keep, once the measurement below says the guesses hold up. On
    Claude Code they did not hold up with Haiku 4.5: see decision 15.
14. **The built-in agent guesses in its read-only mode.** It adds a second wall to "a guess cannot
    change the design" (no MCP bridge, permissions declined, and now no tool that writes).

Known side effect: Mastra counts each model switch in its own settings (`modelUseCounts`), so
guesses raise the small model's use count. The app's model list does not sort by it.

### Test cases for this round (approved by Tianwei 2026-11-21, written in `predict_tests.rs` and `connections.rs`)

Real: the predictor's state machine, preferences, egui input and its clock, the question text.
Faked: the guessing agent (the `Fake` backend: records `new_session`, `connections`,
`set_config_option` and prompts, returns scripted events); for B9 a shell process stands in for
the agent, as in `acp_client`'s built-in tests.

| Case | Input | Action | Observable result and pass criterion |
|---|---|---|---|
| B1 Tuned before the question | built-in; snapshot connected; session offers mode build/{build, plan, fast}, model `anthropic/claude-sonnet-x` of {sonnet-x, `anthropic/claude-haiku-y`, `openai/gpt-5.4-mini`}, effort medium/{off, low, medium, high}; then Ready | frames past the pause; each setting answered | `new_session` once after the snapshot; settings in order mode=fast, model=haiku-y, effort=off, each only after the one before was answered; no prompt before the last answer, one after; the log's `model` is haiku-y |
| B2 `same` | as B1, preference `same` | frames past the pause | no setting sent; the question goes on Ready; `model` is sonnet-x |
| B3 Named model | preference `openai/gpt-5.4-mini`; separately a name not listed | as B1 | model set to gpt-5.4-mini; with the unlisted name no model setting, mode and effort still set |
| B4 Nothing connected | built-in; snapshot with nothing connected | frames to 60 s | no `new_session`; state `paused: sign in`; chip "Next step paused: sign in to the agent" |
| B5 A refused setting | the model setting answered with an error | frames | warning logged; the question goes on the session as it is; that setting not sent again |
| B6 Small-model choice | lists: same provider has haiku; only another provider's mini; `gemini-2.5-pro` only; current already flash; none small | `small_model` | haiku; that mini; none (gemini is not "mini"); none; none |
| B7 Agent news while a setting is out | a `config_option_update` arrives between the mode setting and its answer | frames | no second setting sent while the first is out; the model setting follows the mode's answer |
| B8 Each fresh session tuned again | two questions (two edits) | frames | a `new_session` before the second; its settings sent again in the same order |
| B9 Started as the chat's | built-in with a data folder and a setup with one key; separately no data folder | `start_backend` | the agent process gets `MASTRA_APP_DATA_DIR` = that folder; `setup` with the key goes after `initialize` is answered and before any session; without a folder: the no-folder error, no process |
| B10 The chat's setup is kept | the chat hands its agent a setup | `connections::send` | `Conn::last_setup` equals it |

### Claude goes through Claude Code, not the built-in agent (Tianwei, 2026-11-21)

Guesses on a user's Claude sign-in run in Claude Code (`claude-agent-acp`, signed in with Claude
Code's own login), and the built-in agent is not signed in to Claude. The built-in agent work above
stays for other providers. Why is recorded with INT-1458.

### Measurement on Claude Code (2026-11-21, zhikun)

`claude-agent-acp` 0.107.0. Claude Code's own model is `default`, which the adapter lists as Opus
5.5. Harness: `examples/demo/next_step_ab.sh` with `NSM_AGENT=claude` (this machine's Claude Code
sign-in linked into the test home). Designs case, bracket and plate, two states each, made by hand
steps only, pause 3 s. Each design was run twice per arm, so 12 guesses per arm, and every guess
was shown. Time is the agent's time from question to answer. Cost is what Claude Code reports at
API prices; on a plan, nothing is charged per guess and the tokens come out of the plan's
allowance.

| Arm (`agent_next_step_model`) | Model, effort | Median time (range) | Median cost (range) | 12 guesses | Median context |
|---|---|---|---|---|---|
| `same` (as before) | Opus 5.5, default | 3.46 s (2.93–4.05) | $0.063 (0.007–0.302) | $1.27 | 33.7k tokens |
| empty (auto) | Haiku 4.5, low | 2.29 s (1.22–3.34) | $0.0035 (0.0007–0.0076) | $0.049 | 29.1k |
| `sonnet` | Sonnet, low | 1.33 s (1.09–1.55) | $0.056 (0.003–0.150) | $0.86 | 33.2k |

Against the target (median at most 2.5 s and $0.012, guesses no worse): Haiku meets time and cost.
Sonnet meets time and misses cost by 4.6×.

The guesses still need Tianwei's judgement; this is a first reading.
- **Opus** gave the same step in both runs at all 6 states: shell, mounting holes, join the boss,
  subtract the hole.
- **Haiku** agreed with itself at 3 of 6 states and leaned to fillets. At bracket state 1 both runs
  proposed the 6 mm vertical-edge fillet the user had just made. At plate state 1 one run proposed
  a 5 mm fillet on the top edges of a 6 mm plate.
- **Sonnet** agreed with itself at 5 of 6 states. Once it also proposed the bracket fillet just
  made, and its plate hole pattern gave the wrong size (a 5 mm radius for a 2.5 mm hole).
- At plate state 2, Opus asked in both runs to subtract the cylinder from the plate. The hand
  step's cut had already happened (checked 2026-11-24 on the headless engine: the plate's volume
  falls by 117.8 mm³), so that step does nothing. The question leaves out each feature's operation
  (decision 18).

The costs spread widely because of the prompt cache: with Claude Code's preset, about 19k of the
~30k tokens are written to the cache again on every guess, so a hit saves only part.

### Probe: a lean guessing session (2026-11-21, not in the app)

One question (the case after its fillet, as the app writes it) was sent straight to
`claude-agent-acp` over ACP, still through Claude Code and its own sign-in. Each arm ran two fresh
sessions in one process, so the first is cold and the second warm. Script: the session's
scratchpad, not kept in the repository. The levers, all per session on `session/new`:
- **Override (ov):** `systemPrompt` with `mode: "override"` set to the session note. This is ACP's
  system-prompt field, and the adapter offers `override`.
- **No tools (nt):** `_meta.claudeCode.options.tools: []`.
- **No settings (ns):** `_meta.claudeCode.options.settingSources: []`. The user's own Claude Code
  setup (skills, plugins, CLAUDE.md, settings) is then not loaded.

| Arm | Model | Context | Time | Cost cold / warm | Step (both runs) |
|---|---|---|---|---|---|
| preset (as the app runs it) | Sonnet | 30.3k | 1.0–1.3 s | $0.077 / $0.077 | shell 2 mm, top open |
| ov | Sonnet | 27.6k | 1.0–1.1 s | $0.111 / $0.0033 | shell |
| nt | Sonnet | 460k | 2.8–4.0 s | $1.84 / $0.047 | shell |
| ns | Sonnet | 18.4k | 1.3 s | $0.030 / $0.0025 | shell |
| ov + ns | Sonnet | 16.1k | 1.0–1.2 s | $0.019 / $0.065 | shell; then a 54 × 34 × 20 mm pocket |
| **ov + nt + ns (lean)** | Sonnet | **1.1k** | 1.3 s | **$0.0048 / $0.0007** | shell |
| ov + nt + ns (lean) | Haiku 4.5 | 1.1k | 0.9 s | $0.0002 / $0.0000 | 2 mm fillet on the top edges |

What the probe shows:
- **The user's own setup is the bulk, not the preset prompt.** So today's guess cost depends on
  each user's Claude Code setup.
- **Never use no-tools without no-settings.** With tools off alone, Claude Code sent ~460k tokens.
  The likely cause is the user's MCP servers' tools, put in full instead of deferred (found for
  the lean session, below; not checked for this arm).
- **The lean session meets the target even cold.** A cold guess costs about 1/13 of today's
  median (Opus, $0.063) and a warm one about 1/90, with no tool in the session at all.

Decisions (Tianwei approved building both on 2026-11-21 by approving L1–L5):

15. **On Claude Code, guesses use Sonnet, not the first "small" model.** Haiku 4.5 is the small
    model the rule picks, and its guesses read worse. Built: with `agent_next_step_model` empty
    and the chat's agent Claude Code (agent id `claude`), the guess model is `sonnet` when the
    session lists it, else the small-model rule as before (`CLAUDE_GUESS_MODEL` in `predict.rs`).
    A model named in the preference, and `same`, work as before.
16. **A lean guessing session for Claude Code.** Built: every `session/new` of the guessing
    process is lean when the agent is the Claude Code adapter (`agentInfo.name` ends in
    `claude-agent-acp` or `claude-code-acp`) and offers `systemPrompt` with `override`
    (`acp::lean_session_params`). The session note is then the whole system prompt, with
    `tools: []`, `settingSources: []` and `strictMcpConfig: true` (added 2026-11-24, below);
    `disallowedTools` and `allowDangerouslySkipPermissions: false` stay. This holds for every value of the preference,
    `same` included. The chat's sessions stay as they are. If the agent refuses a lean session
    (any error, a sign-in one too, since the user's own settings may be what signs in), one
    warning is logged and the session is asked for again as before; that process asks for no
    lean session again. It adds a third wall to "a guess cannot change the design" (decision 14).
    Since 2026-11-24 it is used with an API key sign-in only.

### Test cases for the lean session (approved by Tianwei 2026-11-21; L1–L4 in `predict_tests.rs`)

Real: `acp::session_new_params` and the guessing process's `session/new` text, as `acp_client`
sends it; the predictor's state machine. Faked: the agent, a shell stand-in that records its stdin
and answers `initialize` and `session/new` (as B9), and the `Fake` backend for the predictor.

| Case | Input | Action | Observable result and pass criterion |
|---|---|---|---|
| L1 Lean when offered | Claude Code agent; `initialize` reply offers `sessionCapabilities.systemPrompt` | the guessing process opens a session | `session/new` has `systemPrompt` `{mode: "override", content: [the session note]}` and `_meta.claudeCode.options` `tools: []`, `settingSources: []`; no `append` note in `_meta.systemPrompt` |
| L2 The chat is untouched | same agent, the chat's own session | the chat opens a session | its `session/new` is byte-for-byte what it is today (`append`, `disallowedTools`, no `tools` or `settingSources`) |
| L3 Not offered | an agent whose `initialize` reply has no `systemPrompt` offer | the guessing process opens a session | the note goes as today; no override, no `tools` or `settingSources` |
| L4 Refused | `session/new` with the lean fields answered with an error | frames | one warning in the log; a second `session/new` without the lean fields; the guess is asked on it; the lean fields are not sent again by that process |
| L5 Real agent (measurement, not a unit test) | harness, Sonnet, lean, 3 designs × 2 runs | as the table above | context at most 2k tokens per guess; median time at most 2.5 s; median cost at most $0.012; the guesses go to Tianwei |

Proposed while building (2026-11-21, not approved, not written): the behaviour is built, and
these cover what L1–L4 do not.

| Case | Input | Action | Observable result and pass criterion |
|---|---|---|---|
| L6 Sonnet on Claude Code | `Fake` guessing backend; chat's agent id `claude`; preference empty; model option lists `default`, `opus`, `sonnet`, `haiku` | the session is tuned | `config: model=sonnet`, then the lowest effort; with the agent id `codex`, the small-model rule as in B1 |
| L7 Other agents stay as before | stand-in agent named `fake` offering `systemPrompt` with `override` | the guessing process opens a session | the note goes in the standard field with `mode: "append"`; no `tools` or `settingSources` |
| L8 None of the user's MCP servers (proposed 2026-11-24) | as L1, and as L2 | as L1, and as L2 | the guessing session has `_meta.claudeCode.options.strictMcpConfig: true`; the chat's session has no such key |

### L5 result: lean Sonnet in the app (zhikun)

The first run (2026-11-21) **failed** the context and cost limits. The fixed build (2026-11-24)
**passes all three limits**. Same harness as above, 12 guesses per run, every guess shown, model
`sonnet`.

| Run | Median time (range) | Median cost (range) | 12 guesses | Context median (max) |
|---|---|---|---|---|
| 2026-11-21, without `strictMcpConfig` | 1.46 s (1.15–2.70) | $0.0237 (0.0006–1.675) | $1.94 | 209.9k (418.7k) |
| **2026-11-24, with it** | **1.47 s** (1.21–3.55) | **$0.0028** (0.0006–0.0052) | **$0.034** | **1.1k (1.1k)** |
| L5 limit | 2.5 s | $0.012 | | 2k |

Cause: the first guess of every run carried about 418k tokens, the second 1.1k. The first session
is opened when the agent connects and waits out the pause. Most likely Claude Code loads the user's
own MCP servers (claude.ai connectors too) in those seconds: `settingSources: []` does not stop
that, and with no tool search in the session their tools go with the question in full. A later
session is asked at once, before they load. Checked: a lean session that waited 6 s before its
question carried 1,119 tokens with `strictMcpConfig: true`. The same wait without it was not run
again on its own (it costs up to $1.68 a question); the six first guesses of 2026-11-21 stand for it.

Checks on the fixed build: the 35 predictor tests (L1–L4 among them) pass, and `rustfmt --check` on
`acp.rs` is clean. Clippy and the full `cargo xtask ci` were not run on it.

First reading of the 12 guesses (Tianwei judges): shell the case (4 of 4), join the boss (2 of 2),
corner mounting holes (3 of 4; once a 6 mm fillet on the bottom edges). **Plate state 2 is wrong in
both runs** ("Subtract the Hole cylinder from Plate", already cut). It was wrong in both runs of
2026-11-21 too, and for Opus; see decision 18.

## What `develop` brings to suggestions (2026-11-24, `develop` `e8c4b1be2`, read against `9ce777903`)

The frame (Linear INT-1458, INT-1500): shown in the bottom-centre chat, with a ghost in the CAD view
that the user accepts or not.

**Merge cost.** Merged in memory (nothing committed, not compiled). Against `e8c4b1be2`: one
conflicting file, `acp_client.rs`, three hunks, each both sides kept. Against `f18a9aec2` (the same
day, #229): three files, six hunks (`acp.rs` 2, `acp_client.rs` 3, `host_ai/task.rs` 1). `session_new_params` takes a fifth argument
(`project`), which our fallback call must pass.

| On `develop` | What it gives suggestions | Verdict |
|---|---|---|
| The modelling skill's command reference (`skills/solidcraft-modeling/reference.md`; #204 ships the skills with the built-in agent), with the preview on a copy we already have (`review.rs`) | A guess can return the commands that make its step; the app runs them on a copy and draws the ghost at once, with no agent turn on Tab. Probed below. | **Use now** (decision 17) |
| Message box at the bottom centre, on by default (`agent_chat_centre`, #221), with one line above it (`PreviewLine`) | The line is where the step belongs while the chat is closed. Our card does not fit any more: it is anchored at the middle above the navigation bar, where the box now is, and a layer over the box hides the box (`web.rs`, `covered`). With the box focused, Tab goes to the web page, not to the app. By reading, not run. | **Must change** (decision 19) |
| Autocomplete panel in the right rail, empty ("Next steps are not suggested yet. Suggestions will be listed here.") | A place for the log: steps, accepted or turned down, time and cost, the on/off switch. | Use, small (decision 19) |
| Project handed to the built-in agent (#212): brief, fence, reads without a card | More context for a turn Tab starts. `brief.mjs` lists files and leaves `.intcad/` out; our `project_brief.rs` reads `.intcad/`, so the two do not overlap. | Later |
| Plan first (`plan.mjs`) | Costs Tab time: in a project chat the turn's first change is held until `task_write`. With decision 17 Tab starts no turn, so it goes away. Its task list is also a free next step when a turn stops with steps pending. | Goes away with 17; pending steps later |
| DFM findings per run (`dfm::service::findings_for`, with measure, requirement, faces) | A failed rule as the next step. The spec puts DFM fixes later. | Later |
| The note reaches the built-in agent (`session.mjs`) | Until now it never saw `SESSION_NOTE`. After the merge it gets it behind the chat's note ("work through the Intcad tools", "read `skill_reference`") and "Answer in the language the user writes in", which goes against the app-language rule. Every `session/new` also stays open, with a folder of its own, until the app closes its connection: one per guess. By reading, not run. | Fix before guesses run on it. **Fixed 2026-11-26** (decisions 27 and 28) |
| Related verbs in `describe_command` (#215), model picked while busy, Files panel | Nothing for a guess that has no tools. | None |

Not on `develop` yet: release packages carry the built-in agent by default only with PR 214.

### Probe: a guess that carries its commands (2026-11-24, not in the app)

Lean Sonnet through Claude Code, fresh session per question, low effort. Nine states: the six of
L5 and three held out (shelled case, flange with hub, flange with bolt hole). The commands run on
the real engine (headless `intcad-cli` 0.1.2, built 2026-11-22, not this `develop`) on a rebuilt
state. The added text is a short command table and the bodies' faces and edges. Scripts and output
are on zhikun in `~/open-skyfall-eval/ghost-probe/` (the first round's output is not kept).

| Arm | Guesses | Time, median (max) | Cost, median (max) | Context | Commands ran and changed the part |
|---|---|---|---|---|---|
| Words only (round 2) | 18 | 1.32 s (2.0) | $0.0008 (0.0010), warm | 1.1k | – |
| Words and commands in one question (round 2) | 18 | 1.55 s (2.0) | $0.0014 (0.0019), warm | 2.5k | 18 of 18 |
| the same, told to choose the step first (round 3) | 18 | 1.58 s (1.9) | $0.0052 (0.0129) | 2.6k | 18 of 18 |
| **Words first, then commands for that step** (round 4) | 18 | **2.73 s** both (3.5); the words at 1.3 s | **$0.0102** both (0.0125) | 1.1k + 2.5k | **18 of 18** |

Round 1 (nine one-question guesses, cold, while the machine was building: 2.5 s, $0.011) sent the
engine a wrong argument name; its commands, replayed, ran 9 of 9. In all, **63 of 63 ran and changed
the part**.

- The engine took 3–445 ms (median 77 ms in round 4). Today Tab to a ready preview takes 13–35 s.
- For all 63 the change in volume is the one the words ask for (checked by hand). One sentence says
  "top-left corner" for a hole at the bottom left.
- **One question makes smaller steps:** at bracket state 1 and plate state 1 it proposed one hole
  every time (10 of 10) where words only proposed four; telling it to choose the step first did not
  help. Two questions keep the four holes (4 of 4).
- No probe arm asked to subtract the cylinder again at plate state 2, words only included: its
  History lines named the operation and the position, which the app's do not.
- Not tested: sketches, assemblies, many bodies, large topologies (faces and edges were cut at 16
  and 30 a body), imported parts, other agents than Claude Code. The nine states are simple.
- The commands question is often cold ($0.007–0.012), as its text differs per state.

### Decisions (approved by Tianwei 2026-11-24 with the PRD and the test list)

17. **Draw the guess as a ghost from its own commands.** Words first (shown at once), then the
    commands, run on `Session::scratch` off the window thread and drawn by the existing overlay.
    Tab replays them on the design as one undo step, as Accept does today; Esc drops the copy. If
    the commands fail or change nothing, the suggestion is withdrawn (PRD: wrong is worse than
    none; as proposed, Tab would have worked as it did). Commands come from an allow-list
    of modelling commands (no file, export, document or view command), checked by the app.
    Recommendation: yes; it is the ghost the 11-22 meeting asked for. It changes decision 2 (Tab
    accepts what is already drawn) and keeps decision 1: the guessing session still has no tools.
    The line meets 2.5 s; the ghost comes at about 2.8 s.
18. **Name each feature's operation and position in the question** (`feature_values` sends neither).
    Recommendation: yes, with 17's geometry.
19. **Show it where `develop` puts the Assistant:** the step in the line above the message box, Tab
    and Esc working there too, the rail's Autocomplete panel for the log and the switch, and our
    card gone. Needs the Assistant page rebuilt. The look is Pai's. Recommendation: yes, agreed with
    Pai before building.
20. **Merge `develop` first**, before 17–19. Recommendation: yes.
21. **One session for both questions of a suggestion, and the next one's opened ahead** (Tianwei,
    2026-11-24; replaces 12). The session is opened as soon as the last answer is in, and is replaced
    if the sign-in or the model preference changed before it is used. The session before it is
    closed (`session/close`, where the agent offers it): Claude Code keeps two processes and about
    320 MB for each one left open, and the build before this left two behind for every suggestion.
    Claude Code reports tokens per question but cost per session, so the second question's cost is
    the difference.
22. **`strictMcpConfig` on every guessing session** (Tianwei, 2026-11-25; G40 changed with it). A
    full session is the agent's own but for this: the user's MCP servers are not loaded in it.
    `mcp__*` stays blocked as well. It takes 6.5k tokens out of the first question and half the cost
    out of a suggestion at a person's pace; the time is the same.
23. **Words that disagree with the ghost withdraw the suggestion** (Tianwei, 2026-11-25), before any
    command runs; only a hole's size is compared. The questions give a hole's size as its diameter,
    also where a cylinder cut it (G66).
24. **In a design not in mm, a size without a unit is not run** (Tianwei, 2026-11-25). The questions say
    the design's unit and that points are in mm; marks and words compare lengths. In mm nothing asked
    changes. M11: 27 of 27 inch suggestions drawn, against 8 of 27 before.
25. **Rule 9 narrowed** (Tianwei, 2026-11-26): a length in the words counts as a hole's size only when
    radius or diameter is said of it, or when it is within 5% of a drawn hole's size, of twice it or of
    half it, so "a hole 10 mm from each edge" is no longer withdrawn (autocomplete-tests.md, "Rule 9
    narrowed"). R1: none of the 258 recorded, nor of M10's and M11's answers, changes outcome.

Also decided with the PRD: lean sessions only with an API key sign-in, full ones otherwise (this
replaces decision 16's "every guessing session"); all MCP tools blocked in a full guessing session;
Claude Code and Codex only, the built-in agent second; at most 20 suggestions an hour on a plan; the
app, not the agent, says which numbers were guessed.

Test cases for 17–19 are in `autocomplete-tests.md`, with what was measured.

### The built-in agent makes suggestions (2026-11-26)

26. **Suggestions on the built-in agent too** (Tianwei, 2026-11-26). This reverses the PRD's "Claude
    Code and Codex only, the built-in agent second" above. Since #214 a release package carries the
    built-in agent and the chat starts on it, so without this most users of a release get no
    suggestion. A Mastra workflow, an API endpoint and other ACP agents still make none, and Codex
    none until its sessions' isolation is verified (Huaiyuan's change of the same day, M3).

Approved by Tianwei 2026-11-26 with the test list (`autocomplete-tests.md`, "suggestions on the
built-in agent"):

27. **A built-in guessing session is marked `nextStep`** (`_meta.intcad.nextStep`). It carries the
    guessing note alone, not behind the chat's note; the agent adds no language line and no project
    line, starts no MCP server, gives it no design tool even if asked, and opens it in `fast`
    (Explore). This fixes "Fix before guesses run on it" above and keeps decision 14's wall in the agent
    itself. Recommendation: yes.
28. **The built-in agent closes a session on `session/close`.** Mastra Code 1.19.1 does not offer it, so
    each guess would keep a whole Mastra Code runtime until the app quits. Recommendation: yes; it is
    Intcad's own file (`agent.mjs`), and the chat sends no `session/close`, so the chat is as it was.
29. **On the built-in agent the hourly cap follows the guess model's connection:** a plan counts, a key
    or a custom address does not, anything not known counts. Recommendation: yes, as on Claude Code.
30. **The guessing process gets every setup the chat's agent gets**, and the chat's last one again after a
    plan is signed in or out, so a key the user removes stops being used for guesses at once.
    Recommendation: yes.
31. **The guess model on the built-in agent stays the small-model rule** (decision 13: haiku on an
    Anthropic key, gpt-5.4-mini on an OpenAI one), not Sonnet as on Claude Code (decision 15), until
    M12's guesses are read. Recommendation: yes, then decide on M12.

## Known issues and not done

- Sheet metal misses its share (M8): `autocomplete-tests.md`.
- On a plan the second question is written to the prompt cache again instead of read from it: same
  file. It is nearly all of a suggestion's cost.
- In a design not in mm a size without a unit is not run (decision 24); a variable or spline fillet's own
  numbers count as sizes, so such a fillet is not run there: same file, "designs not in millimetres".
- The words check reads holes only, and loses some right suggestions: same file, "words that disagree".
- Building the second question holds the window for 56 ms on a plate with 400 holes (M5). It is
  under the limit of 100 ms, and is the longest thing the feature does on the window's thread.
- Project memory on disk (G49–G53) waits for Pai; the account's answer is a stand-in until the app
  has an account; the use count on Tab and the stop bound (AC-23, AC-22) wait too.
- Claude Code writes a transcript of every guessing session to disk (in the app's own work folder under
  `~/.claude/projects`), as it does for the chat's.
- Codex prediction is disabled (M3). An empty ACP `mcpServers` list means Intcad adds no servers;
  it does not disable the adapter's inherited MCP configuration. The documented Codex ACP 2.2.2
  sandbox modes do not establish no-MCP and no-write-tool isolation across configurations. Normal
  Codex chat is unchanged.
- Suggestions do not follow the user across design tabs: a new tab's chat starts disconnected.
- API endpoints, Mastra workflows and ACP agents other than Claude Code and the built-in agent make no
  suggestions (Codex: above).
- On the built-in agent (M12–M14 and the long run in `autocomplete-tests.md`): measured on one DeepSeek
  key only, not on a plan; no cost is reported; a question carries about 9k input tokens; the guessing
  process's memory rose to about 550 MB over 12 suggestions and held at 550–605 MiB to the 30th, not shown
  to stop; Mastra keeps each closed session's thread (about 60 kB a suggestion in the data folder).
- Where the Assistant is not at the bottom centre, the History chip is the line; History on the
  left or in the Browser shows no chip, so no suggestions are made there (decision 3).
- Pattern spacings, extrusion distances and flange sizes always read as guessed.
- Search needs three characters for a match inside a label, so a two-character Chinese query (建议)
  does not find the setting; 下一步 does.
- The strings are translated into ar, de, he, ja, ko, vi and zh (and en-GB spelling); other locales
  fall back to English, as the pending-strings README describes.
