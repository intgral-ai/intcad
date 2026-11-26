# Autocomplete: test cases for the ghost (INT-1458)

**Approved by Tianwei 2026-11-24**, with the eight rules at the end, before any test was written;
G57–G64, G66–G70, M10 and rules 9 and 10 (under Results) on 2026-11-25, G71–G78, M11 and rules 11–13 the
same day, rule 9 narrowed with R1 on 2026-11-26, and G80–G90 with M12–M14 (suggestions on the built-in agent) the same
day. G1–G48 and G54–G90 are written and pass; G49–G53 wait for Pai. What was measured, and where a test differs
from its case, is under Results. It covers the requirements AC-1 to AC-26 of the PRD on INT-1458
(v0.3). The earlier list (B1–B10, L1–L8) is in `next-step-prediction.md`.

Every case names what is real and what is faked by its group's line. "Withdrawn" means: the line and
any ghost go, nothing is sent to the chat, the design's stamp (`DocumentStamp`), document pointer and
undo depth are what they were before the question, and the log has the outcome named.

- **Fake agent:** the `Fake` backend of `predict_tests.rs` (records prompts, sessions and settings;
  returns scripted answers and usage).
- **Stand-in agent:** the shell script of L1–L4 (records the JSON lines it is sent).
- **Clock:** egui's input time, scripted. No case waits on the wall clock unless it says so.
- **Engine:** the real one, on the open design and on its copy.

## A. Asking

Real: app state, preferences, egui input, the engine, the question text. Faked: the agent, the clock.

| Case | Input | Action | Observable result and pass criterion | AC |
|---|---|---|---|---|
| G1 Off by default, and said | default preferences, English and Chinese | frames to 600 s; read the setting's text | no question, no session; the text names the user's own agent and plan in both languages | 1 |
| G2 Pause | setting on, default pause; separately 4 s; separately 0.2 s and 500 s | edit at 0; frames at 1.4 and 1.5 s; separately a second edit at 1.0 s | nothing at 1.4, one question at 1.5; with the second edit, one at 2.5 and none before; 4 s is kept; 0.2 and 500 fall back to 1.5 | 2 |
| G3 Not while busy | in turn: agent busy, a chat preview running, one waiting, a sketch, a tool, a dialog, a skill, Ask Here, Home | edit; frames to 60 s; then cleared | no question while held, exactly one after | 2 |
| G4 History names the operation and place | plate 100×60×6, then a cylinder r 2.5 with `operation: cut` at (10, 10, −1) on Plate; separately the same cylinder as `new`; a fillet of 4 edges | build the first question | the cut's line has "cut", (10, 10, −1) and Plate; the new body's line has "new" and not "cut"; the fillet's line has its radius and 4 edges | 10 |
| G5 Nothing, or one | answers: empty step; two candidates in one answer; prose; empty text | answer | empty, prose and empty text show nothing and ask no second question; of two candidates only the first is shown; the reason is shown with the step | 3 |
| G6 Two questions, in order | a step as the first answer | answer the first; then the second | after the first answer: the line is shown, one second question in the same session holding the step word for word, the allowed command names, and the faces and edges of the bodies it is about; no second question before the first answer; two questions in all; once the second is answered one fresh session is opened, and the next first question goes in it without waiting for one (changed 2026-11-24, Tianwei: one session, not two) | 3, 6 |
| G7 Large bodies stay small in the question | a body with more than 500 edges | build the second question | at most 8,000 characters; it says how many faces and edges were left out | 6 |
| G8 An edit at each stage | an edit while: the first question is out; the line is shown; the commands compute; the ghost is drawn | edit, then late answers, then frames to 1.5 s | withdrawn (`stale`) at each stage; the late answer shows nothing and starts no second question; one new first question 1.5 s after the edit | 2, 6 |
| G9 Esc and other keys at each stage | the same four stages | Esc; separately the key A | Esc: withdrawn (`dismissed`), the key used up, the step listed as turned down; A: withdrawn, the key not used up; no question until the next edit | 12 |
| G10 Looking is not acting | line shown; ghost drawn | orbit, pan, wheel, a click in the view | both stay | 2 |

## B. The app's checks

Real: the engine on a copy, command resolution (`gate::resolve`), the worker thread where named.
Faked: the agent's answer. A counter in the runner counts commands run on the copy.

| Case | Input | Action | Observable result and pass criterion | AC |
|---|---|---|---|---|
| G11 Only listed commands | one answer each: `file.export`, `file.save`, `file.new`, `doc.open`, `file.send_to_slicer`, `view.pick`, `parameters.change`, `edit.undo`, `sheet.export_dxf`, `sheet.export_bend_table`, `sheet.manage_rules`, an unknown id, a `batch` inside the list, ` FILE.Export ` (case and spaces), and a step with both keys, `id: solid.box` and `command: file.export` (the engine takes `command`) | answer | 0 commands run on the copy each time; no file appears in the test's folder; withdrawn, `not allowed` | 7 |
| G12 One bad command drops all | a good `solid.box`, then `file.export` | answer | 0 commands run; withdrawn, `not allowed` | 7 |
| G13 Every listed command is reachable | one working call for each of the 11 solid commands in PRD §5.2 | answer | each runs on the copy and draws a ghost (so the list has no typing mistake) | 6 |
| G14 A failing command | fillet radius 500; separately a good box then that fillet | answer | withdrawn, `failed`; of the pair nothing remains on the copy | 6 |
| G15 No change | a cut that misses the part (the engine reports success) | answer | withdrawn, `no change` | 6 |
| G16 Not closed | the check function given before and after facts with a body that is no longer closed (pure function; no known way to make one with the listed commands) | check | refused, `not closed` | 6 |
| G17 Repeats | the last History step's own commands; a step turned down, in other case and spacing; the same step with another radius | answer | the first two withdrawn (`repeat`); the third is shown | 6 |
| G18 The stamp holds | the design changes (an undo over the control channel) between question and answer; between answer and ghost; between ghost and Tab | go on | withdrawn each time; Tab lands nothing | 6 |
| G19 Too slow | a copy whose command blocks until the test releases it (real worker thread) | clock to 2.9 s, then 3.0 s | still computing at 2.9; withdrawn, `too slow` at 3.0; the copy's own stop token is set; **the open design's stop token is not**, and a command the user runs then completes; closing the design while it computes also stops the copy | 6, 8 |
| G20 Without threads | `apply_off_thread` unset | answer | the ghost is drawn in the same frame | 6 |
| G21 Too many or too long | 33 commands; an answer over 8,000 characters; not JSON; `commands` missing | answer | withdrawn, `unreadable`; 0 commands run | 6 |

## C. Ghost and Tab

Real: the engine, the overlay's models, undo, preferences. Faked: the agent, the clock.

| Case | Input | Action | Observable result and pass criterion | AC |
|---|---|---|---|---|
| G22 Drawn without touching the design | plate; four `solid.hole` commands | answer | the overlay gives the design's and the copy's models; `agent_chat.preview` is none; stamp, document pointer and undo depth unchanged; no other question while it is drawn | 4 |
| G23 Tab lands what was drawn | as G22 | Tab | four holes in the design; volume, face count and bodies equal the ghost's (volume to 1e-6 of itself); one undo step, and undo gives back the document before; nothing sent to the chat's agent; no review card; `accepted` | 5 |
| G24 The same as running it directly | as G22 | Tab; separately the same commands run directly on a twin design | the two STEP texts are equal but for the header line with the time | 5 |
| G25 Tab before the ghost | line shown, commands not answered | Tab; then the answer; then Tab | the first Tab changes nothing, moves no keyboard focus and is used up; the ghost is drawn; the second lands it | 24 |
| G26 The preview preference | `agent_chat_preview` on; separately off | G23 | the same result both times; no review card | 26 |
| G27 The next at once | G23 | Tab; separately Esc; separately an empty first answer | after Tab the next first question goes the next frame; after Esc or an empty answer none until an edit | 12 |
| G28 Tab that cannot land | the design turned read-only after the ghost was drawn | Tab | nothing applied, a notice, `failed`; the undo depth is unchanged | 5 |
| G29 The chat and a ghost | a ghost drawn; the user sends a chat message (preview preference on, then off) | send | the ghost goes at once; the chat's turn and its preview end as they do without a ghost; at no frame are a chat preview and a ghost both drawn | 2, 4 |

## D. Which numbers have a source

Real: the design, the selection, the chat messages, the project notes. Faked: the agent's answer.

| Case | Input | Action | Observable result and pass criterion | AC |
|---|---|---|---|---|
| G30 Known or guessed | a design with a 2 mm fillet, a parameter `w`, a selected 12 mm face, the chat message "use 3.4 mm holes", a trusted note "inserts 3.6 mm" | commands using 2, "2 mm", 2.0, `"w/2"`, 12, 3.4, 3.6, 2.5, a count of 4, a position equal to an existing hole's x, and one equal to none | known with its source named: 2 (all three spellings), `"w/2"`, 12, 3.4, 3.6, the matching x; guessed: 2.5 and the other position; the count is not marked either way | 9 |
| G31 The agent's word does not count | the answer claims `"guessed": []` | show | the marks are G30's | 9 |
| G32 The line shows the marks | one known and one guessed value | show, English and Chinese | the guessed one is marked in words; the known one's source is in the tooltip | 9 |

## E. Where it shows and who gets the key

G33: real app and egui focus. G34–G36: the bundled page in a browser (`verify-intcad` fixtures);
faked: the desktop's IPC. The look is Pai's, so the page cases check behaviour and not styling.

| Case | Input | Action | Observable result and pass criterion | AC |
|---|---|---|---|---|
| G33 Tab in the app | a ghost drawn; focus in: the view; a text field; a dialog; Ask Here | Tab, then Esc | the view: ours; the other three: theirs, the ghost stays | 11 |
| G34 Tab in the message box | the host says a suggestion is ready; the box empty; with only spaces; with text; during IME composition; with the slash menu open; with an approval waiting | Tab | empty and only spaces: exactly one accept action is sent; the other four: none, and Tab does what it did | 11 |
| G35 Esc in the message box | as G34 | Esc | empty: exactly one dismiss action; the others: none from us | 11 |
| G36 The line over the box | states: none, line, ghost ready, withdrawn; the longest step; English and Chinese; conversation closed and open | show | text as sent, one row, the page as tall as without a suggestion; none and withdrawn show today's line | 11 |
| G37 Why it is not available | no authorisation; agent not signed in; an unsupported agent; hourly cap used | frames to 600 s | one sentence each, once per opening of the app; no question | 14, 15, 17 |

## F. Agents and sessions

Real: `session/new` as the app sends it. Faked: the agent (the stand-in).

| Case | Input | Action | Observable result and pass criterion | AC |
|---|---|---|---|---|
| G38 Six sign-in answers | `_auth/status_update` with kind `account`, `api_key`, `gateway`, `external`, `none`, and none sent | the next guessing session opens | lean (L1's fields) only for `api_key`; the other five full | 13 |
| G39 The report comes late, and changes | first session before any report; then `api_key`; then `account` | three sessions | full, lean, full | 13 |
| G40 What a full session carries | Claude Code, kind `account` | a guessing session and a chat session | guessing: the note as `append`, no override, no `tools`, no `settingSources`, `strictMcpConfig: true` (Tianwei, 2026-11-25; it was "no `strictMcpConfig`"), and `disallowedTools` is today's list plus `mcp__*`; the chat's `session/new` is byte for byte today's | 13, 18 |
| G41 The switch in code | the lean-on-plans constant turned on in the test build | kind `account` | lean; with the constant as shipped, full | 13 |
| G42 Codex | agent id `codex`, recording backend | wait for a suggestion, then send an ordinary chat message | no prediction session or question; the ordinary chat prompt goes out | 14 |
| G43 Not supported yet | the built-in agent, another ACP agent, an API endpoint, a Mastra workflow | edit, frames to 60 s | no session, no question; G37's sentence | 14 |
| G44 Permission requests | the agent asks for permission during either question | poll | declined both times | 18 |

## G. Limits and the log

Real: the predictor, preferences, the app log. Faked: the agent and its usage, the clock.

| Case | Input | Action | Observable result and pass criterion | AC |
|---|---|---|---|---|
| G45 20 an hour | plan sign-in; 20 first questions in 30 min; a 21st edit; then the clock past the first question's hour; separately the limit set to 5; separately kind `api_key` and 60 questions | edits | the 21st is not asked and one notice is shown, once; one is asked again after the hour; at 5 the sixth is not asked; `api_key` asks all 60. A suggestion counts once, however many questions it took; Tab-after-Tab counts | 17 |
| G46 The log | usage on both questions; every outcome of this list | run | each suggestion's entry has both questions' time, tokens and cost (none invented when not reported) and one outcome; at most 200 entries; no request leaves the app but to the agent | 16 |
| G47 Withdrawn lines are counted | 40 lines shown, 3 withdrawn after showing | read the status | shown 40, withdrawn 3 (7.5%); with the line-first constant off, the line appears only with its ghost | 25 |
| G48 The rail panel | three suggestions: accepted, dismissed, withdrawn | open Autocomplete | three rows with step, outcome and time; the switch changes the preference; with none, the existing empty text | 21 |

## H. Project memory (held until Pai says where it lives)

Real: files in a temporary project and a temporary app data folder. Faked: the agent.

| Case | Input | Action | Observable result and pass criterion | AC |
|---|---|---|---|---|
| G49 What is kept | accept one, turn one down, in a trusted project | read the file | two records: step text, command names, outcome, time; no coordinates, no chat text, no parameters | 19 |
| G50 What it does | G49, then the app started again | edit | the step turned down is not shown again; the accepted one is in the first question, as data | 19 |
| G51 Trust is kept, and not by the project | trust a project; restart; separately copy the project (same id) to another folder; separately a project file that claims to be trusted | edit | trusted after restart; the copy and the claim are not; an untrusted project's memory is neither read nor written | 19 |
| G52 Hostile or broken file | line breaks and instructions in a step; 10,000 records; 1 MB; not JSON | edit | one line each, at most 20 records and 2,000 characters in the question; a broken file is ignored with one warning and left as it is | 19 |
| G53 No project, and clearing | an unsaved design; separately two projects and Clear in one | accept; clear | nothing written for the unsaved design; Clear empties only that project's | 19 |

## I. Sheet metal and the account

| Case | Input | Action | Observable result and pass criterion | AC |
|---|---|---|---|---|
| G54 Sheet metal off, then on | a sheet part; `sheet.flange`; the sheet-metal constant off, then on | answer | off: withdrawn, `not allowed`, 0 commands run; on: the flange is drawn and Tab lands it as in G23 | 20 |
| G55 Every listed sheet command | one working call for each of `flange`, `hem`, `fold`, `unfold`, `refold` | answer | each runs on the copy; one that cannot is taken off the list | 20 |
| G56 Authorisation (faked: the account's answer) | allowed; denied; unknown; denied after the ghost was drawn | edit; Tab | asked only when allowed; denied after drawing: Tab lands nothing | 15 |

Held, not listed as cases: the use count on Tab (AC-23) and the measured stop bound (AC-22) wait for
the account interface and Pai's decision.

## J. Measurements (real agents and engine on zhikun; not unit tests)

| Run | How | Pass | AC / PRD §8 |
|---|---|---|---|
| M1 Claude Code, full session | the six L5 states and the three held out, 5 runs each: 45 suggestions | line median ≤ 2.5 s; ghost median ≤ 4 s, 9 in 10 ≤ 6 s; ≥ 95% run and change the part; tokens and cost reported for each question | 8.1, 8.2, 8.4, 8.8 |
| M2 Claude Code, lean | as M1, with an API key sign-in | as M1, and median cost ≤ $0.012 | 8.1, 8.2, 8.4, 8.7 |
| M3 Codex | as M1 | as M1 without cost | 14, 8.10 |
| M4 Nothing reaches MCP | full session; a recording MCP server set up in the user's Claude Code, allowed unasked; 10 questions that ask for its tool | 0 calls recorded | 18 |
| M5 The window keeps moving | the 400-hole plate, a ghost computing | no two frames more than 100 ms apart; dropped at 3 s | 8 |
| M6 Plate state 2 | 10 suggestions in M1's setting | 0 ask to cut the hole again | 10 |
| M7 Tab to landed | every accepted suggestion of M1 | median ≤ 0.5 s; volume, faces and bodies equal the ghost's every time; the stamp did not move before Tab | 4, 5, 8.3, 8.9 |
| M8 Sheet metal | 20 suggestions on three sheet parts | ≥ 90% run and change the part, else it leaves this version | 20, 8.6 |
| M9 Accepted by people | Tianwei and one more modeller, two parts each not used for tuning, ≥ 40 shown, screen recorded | ≥ 50% accepted; withdrawn lines ≤ 5% | 25, 8.5 |

M1 on a plan is about 90 full-session questions. The first question's median was $0.056 at API
prices, so about $5 of allowance if the second costs the same (an estimate).

## Existing tests that change

Each keeps its case; only what is named changes. None is removed without a replacement above.

| Test | Change | Replaced by |
|---|---|---|
| `no_guess_is_asked_before_the_pause_…` | 5 s becomes 1.5 s | G2 |
| `one_guess_per_revision_…` | after Tab the next is asked at once | G27 |
| `the_chip_is_short_…`, `the_chips_place_says_…` | the chip in History becomes the line over the box | G36, G37 |
| `tab_sends_the_guess_as_a_preview_…` | Tab lands the ghost; no chat message | G23, G25 |
| `the_card_tab_started_takes_tab_enter_and_esc` | there is no card | G9, G23 |
| `tab_is_taken_only_for_a_chip_on_screen_…` | a ghost drawn, not a chip | G25, G33 |
| `the_question_carries_history_…` | operation and place added | G4 |
| L1 | lean only for an API key | G38, G40 |
| B1–B10 | the built-in agent makes no suggestions in this version; its tuning code and tests stay as they are | G43 |

## Rules this list adds to the PRD (approved with it)

1. The hourly cap counts suggestions, not questions (G45).
2. The second question carries the bodies the step names, else those of the last History step (G6).
3. At most 32 commands in one suggestion, and 8,000 characters in an answer or a question (G7, G21).
4. Only spaces in the message box counts as empty (G34).
5. Counts (how many holes) are not marked as known or guessed (G30).
6. Trust is kept by project id and folder together (G51).
7. A Tab before the ghost is used up and does nothing (G25).
8. The ghost's copy has a stop token of its own (G19). Today every copy shares the design's token on
   purpose (`Session::stop_token`, `lib.rs:948`; `scratch`, `lib.rs:1949`), so stopping a slow ghost
   with it would stop the user's own work too.

## Results (2026-11-24, on `develop` `f18a9aec2`)

Tests are in `predict_ghost_tests.rs` and `predict_tests.rs`, and the page's in
`.agents/skills/verify-intcad/browser/test_assistant_ui.py`.

After one session replaced two (G6), four older tests count first questions instead of all questions,
since the second one no longer waits for a session; G5 and G27 expect the session opened ahead; G46
takes the second question's cost as a difference. What they prove is the same.

### Where a test differs from its case

| Case | Written | Why |
|---|---|---|
| G9 | a key other than Esc while the first question is out keeps the question; at the other three stages it withdraws, as listed | the PRD and the existing late-answer test say so |
| G28 | a design that refuses the step, not one that is read-only | read-only blocks saving only, so the step lands on it |
| G37 | the sentence shows for 10 s, once per opening of the app | it has no other place to leave from |
| G49–G53 | not written | held until Pai says where the memory lives |
| G56 | the account's answer is faked | `develop` has no account to ask |
| L6, L7 | not written | not approved |

The History chip stays where the message box is not at the bottom centre. The demo's key strip and
pointer stay too.

### Code review (2026-11-24, a reader who had not seen the work)

Found and fixed, all in code written for this list:

| Found | Now |
|---|---|
| A ghost stopped while it computed left its "cancelled" error in the evaluation cache it shares with the design, so the same step then failed for the user too | what stopped work came to is not kept (`doc/src/eval.rs`). G19 runs it on a real worker, and fails without the fix |
| The active component is no part of the stamp, so Tab could land the steps in another component than the one drawn | a ghost is for a design and a place in it (`ghost::Place`) |
| With History rolled back, good steps read as repeats | compared with what stands before the marker |
| In the Render workspace Tab landed a ghost not drawn there | Tab and Esc are Render's own |
| Esc in a text field was taken by the suggestion, and Tab or Esc there, in a dialog or in Ask Here dismissed it | the ghost stays, as G33 says |
| An agent that died on the second question was asked the first again every 30 s | only a first question that came to nothing is asked again |
| A session that could not be opened left the line up and used up the hour | given up and tried again later; the hour counts what was sent |
| A pick while the commands were out withdrew the line | it stays, as G10 says |
| A value written `w = 40` added a parameter | `not allowed` |
| The worker had a 2 MiB stack | the kernel's 8 MiB |
| G10, G15, G19, G23 and G33 tested less than their cases say | they test what the cases say |

Proposed after it, approved by Tianwei 2026-11-25:

| Case | Input | Action | Observable result and pass criterion |
|---|---|---|---|
| G57 Another component | a ghost drawn; another component is activated; separately a sketch is opened | Tab; then 600 s | nothing lands, `stale`; no new question without an edit |
| G58 History rolled back | a marker before the last feature; a good step; separately the step just before the marker again | answer | the first is drawn, the second is `repeat` |
| G59 Render | a ghost drawn; Render | Tab, Esc; back to Model | Render's own; nothing lands; the ghost is still there |
| G60 The agent dies on the second question | first answered, then it exits; separately after Esc | frames to 10 min | the line goes; one first question in all until an edit |
| G61 A session that cannot be opened | `session/new` refused for the first question; separately for the one opened ahead while a ghost is drawn | frames to 10 min; Tab | paused and tried again with the usual wait; the hour is not used; the drawn ghost stays and Tab lands it |
| G62 A new parameter in a value | `length: "w = 40"` in a listed command | answer | `not allowed`; the design has no new parameter |

Proposed with one session for both questions, approved by Tianwei 2026-11-25:

| Case | Input | Action | Observable result and pass criterion |
|---|---|---|---|
| G63 Sessions are closed | stand-in agent that offers `session/close`; separately one that does not; separately the chat's New chat | two suggestions; New chat | the first suggestion's session is closed when the next opens; without the offer, and for the chat, no `session/close` is sent |
| G64 Opened ahead, no longer right | a lean session opened ahead on `api_key`, then `account`; separately `account` between the two questions; separately the model preference changed after it opened | edit, frames to 1.5 s | a new session first, full on `account` and set up with the new model; the question goes only in it |

G57–G64 were written after the code they cover, so each protection was then taken out to see its test
fail. 17 of 18 changes were caught, by the case that names the protection. The one that was not: the
ghost is not drawn while a sketch is open, which no case asks for. G58 redoes the step after the marker as its
good step, since that is the one that read as a repeat. G61 fakes the refusal (the agent's error) and the
restarts fail because there is no such program; G63 and G64 run a real process (a shell script as the
agent), but for the model preference, which is the recording backend.

### Approved by Tianwei 2026-11-25: words that disagree with the ghost

**Why.** Of 265 recorded suggestions 258 have commands, and 116 of those make a hole (`solid.hole`, or a
`solid.cylinder` that cuts). In 14 the words give a hole another size than the ghost draws, all on plate
state 2 (14 of its 32): "5 mm radius" 7 times and "2.5 mm hole" 7 times, for a hole of radius 2.5 mm. The ghost drew the
design's size in all 14. Twice more the words said "5 mm radius" and the ghost followed them (a 10 mm
hole). On the other eight states of the list and the sheet parts: 0. The words are also the undo step's
name, so the wrong size stays after Tab. What was known before it was approved:

- Plate state 2 is the only state whose hole History gives as a radius. But its corners are filleted with
  5 mm, so "5 mm" may be the hole's diameter or the fillet's radius. The recorded runs cannot tell; M10 can.
- Withdrawing alone would fail M1's 95%: of the three M1 runs 41, 40 and 41 of 45 would be left (91%, 89%,
  91%). So the question changes too (G66).
- Sizes other than holes' are not checked: every length in the words of the 258 was a number of the
  commands, but for four pattern steps, whose lengths (6.6, 33) are the design's.

Real: the engine, the question text, the app's checks, the line. Faked: the agent's answers, the clock.

| Case | Input | Action | Observable result and pass criterion |
|---|---|---|---|
| G66 A hole's size is its diameter | plate with a 5 mm fillet; cylinders that cut, one made with `radius: 2.5`, one with `radius: "w/2"` (`w` = 6) and one with `diameter: 7`; a `solid.hole` of 6.6; a cylinder `radius: 4`, `new` | build both questions | History's lines for the three cuts say "diameter 5", "diameter 6", "diameter 7" and not "radius"; the hole "diameter 6.6"; the new body "radius 4"; the fillet "radius 5"; the usual hole diameters count the cuts with the holes (three are listed, as today); the first question says once that a hole's size is its diameter. G4's test expects "diameter 5" where it has "radius 2.5" |
| G67 Words that agree | each step with its commands: "Add a second 5 mm hole", "…5 mm diameter hole", "…2.5 mm radius hole", "…hole of radius 2.5 mm", "…R2.5 hole", "…⌀5 hole", all with a cylinder r 2.5 that cuts and again with a `solid.hole` of 5; "Cut a 6 mm radius through-hole near each corner of Base." with four holes of 12 (recorded); "Drill a 20 mm hole 12 mm deep" with diameter 20, depth 12; "Add a hole 10 mm from each edge" with diameter 6 at (10, 10); "Add four corner holes"; "Add a 0.25 in hole" with 6.35; "Fillet the top edges with 5 mm"; "Pattern the 6.6 mm bolt holes" with `solid.pattern.circular`; "Add a 10 mm radius boss" with a cylinder that joins | answer | the ghost is drawn every time (one that the engine refuses has its own outcome, never `words differ`) |
| G68 Words that disagree | "Add a second 5 mm radius hole at the opposite corner, (90, 50)." and "Add a second 2.5 mm hole at the opposite corner, (90, 50)." (both recorded), with a cylinder r 2.5 that cuts; "Add a 4 mm radius hole" with a hole of 6; "Add a hole of radius 5 mm" with a hole of 5; "Add a 6 mm diameter hole" with a hole of 12 | answer; then frames to 600 s | withdrawn, `words differ`; 0 commands run on the copy; no ghost at any frame; the step is not among those turned down; no new question without an edit |
| G69 In the app's language | Chinese: "在对角打一个半径 5 mm 的孔" and "…直径 5 mm 的孔"; German: "Eine Bohrung mit Radius 2,5 mm" and "…mit Radius 5 mm"; all with a cylinder r 2.5 that cuts | answer | Chinese: withdrawn, drawn; German: drawn, withdrawn. "Radius" is the app's own word (`ui.radius`) |
| G70 Tab before | the line of G68's first step shown, its commands not answered | Tab; then the answer; then Tab | as G25 for the first; withdrawn; the second Tab lands nothing and the undo depth is unchanged |

Rules it adds:

9. Only holes are checked: steps whose commands have `solid.hole` or a `solid.cylinder` that cuts. A
   length in the words (a number with a unit, or after R, ⌀ or Ø) that is another number of the commands
   (a depth, a coordinate) is left out, unless radius or diameter is said of it. The words disagree when
   a length is left and none is the size of a drawn hole.
10. A length is a radius when R, or the app's word for radius in its language, stands next to it. Any
    other is a diameter.

Decided with it: a suggestion that disagrees is withdrawn. (The other choice was to keep the ghost and
have the app write the line from the commands.)

| Run | How | Pass |
|---|---|---|
| M10 Words and ghost | plate state 2 as it is, and one with fillets of 4 mm and a hole of radius 3 mm (so a doubled radius and the fillet's are different numbers); the question as it is and G66's, in turns; 20 suggestions each, 80 in all | with G66's: the words disagree in at most 1 of 20, and the ghost draws the design's hole size in at least 19 of 20, on both plates |
| M1 and M7 again | with the check on | M1's limits, the 95% drawn among them; M7 with one measure (below) |

Run over the 258 recorded, the check in the app catches the 14 and no other.

**Built 2026-11-25** (`words.rs`; the call is in `predict.rs`, `end_commands`). An independent reader of the
code then found six faults, fixed the same day. With the fixes the replay is still 14 of 258.

- Rule 9's order was turned round: a place or a depth that happened to be the hole's size spoke for it
  ("Add a 2.5 mm hole 5 mm from each edge", hole of 5 at (5, 5)). Now left out first, as the rule says.
- A counterbore's width spoke for the hole ("Add a 2.5 mm hole with a 9 mm counterbore").
- "6 mm in diameter" and ".25 in" were not read; a cylinder with `radius: null` was not checked.
- Counts and the hole's own text ("5 mm" beside "5") were read as lengths and as two usual sizes.

Inputs added to the approved cases to test that, and the signs: G67 "Drill a hole 12 mm deep", "2.5 mm in
radius", "5 mm in diameter", two with a counterbore, a boss whose size disagrees (not checked); G68 "0.25 in"
and ".25 in" with 5, R5, ⌀2.5, "6 mm in diameter", the two places above, the counterbore, and a cylinder
written with `null`, an empty text, a text and `subtract`; G69 Russian "радиусом 2,5 мм" and "радиусом 5 мм",
German "2,5 mm Durchmesser".
Four older tests (G15, G31/G32, G47, G58) said 5 mm and drilled 4 or 7 mm; their words or commands now agree.
G67 passed before the code was there, as it must: it holds what is not to be withdrawn. Each part of the
check and of the question was then broken on purpose, 36 changes one at a time: 34 were caught, by the case that
names the part. The other two (a counterbore that speaks for the hole, no decimal comma) showed weak inputs;
with those sharpened both are caught.

What it lets through: a step with several holes agrees when one of its sizes does; a wrong size equal to a
depth or place of the same commands; a hole given by `thread` or `clearance` (the question lists neither)
turns the check off for its answer; the reason's sizes.

What it loses, a right suggestion withdrawn:

- A length that is no size of the hole and no number of the commands, when no hole size is said: "Add a hole
  10 mm from each edge" drilled at (90, 50), "Drill a through hole in the 6 mm plate". Rule 9 as written;
  none in the 258 recorded or in M1's 25 steps with a hole. **Decided 2026-11-26 (Tianwei): rule 9 is
  narrowed**: approved and built under "Rule 9 narrowed", below.
- Rounded words ("6.67 mm" for 6.666667), and by reading the code "4 1/4 in holes" (read as 4.25) and, in an
  inch design, "Hole 2 in the corner" (read as 2 inches) where no other size is said.
- Where a phrase with "radius" is not read as one, from phrases written for 33 languages by the author of the
  code (no speaker checked them): Polish, Finnish, Hungarian, Romanian and Arabic with the word bent, Korean
  with another word (반경); by reading the code, "depth 10 mm radius 2.5 mm" in Japanese, Chinese and Korean.

### Approved by Tianwei 2026-11-25: designs not in millimetres

**Why.** In an inch design the engine reads a bare size of a command in inches and a bare point in
millimetres, for every listed command that takes a size; a picked edge or face is a point of numbers only.
The second question gives the geometry in mm and asks for "numbers from the geometry below". So an agent
that does as asked makes a part 25.4 times too big. What was run (the engine and the app as they are, with
answers written by hand; no agent was asked, so how often an agent does it is not known: M11):

| Inch plate 4 × 2.5 × 0.25 in, the words | The commands' size | Today |
|---|---|---|
| "Add a 1 in boss on Plate" | 25.4 | **drawn, 25.4 inches wide** |
| "Add a second 0.25 in hole" | 3.175 (radius) | withdrawn, `words differ` (G68) |
| "Fillet the top edges with 0.05 in" | 1.27 | withdrawn, `failed` |
| any of them | 1, 0.125, 0.05 (the hole also as "0.125 in" and "3.175 mm") | drawn, the right size |

- **The line says "guessed" wrongly.** For a hole in a design with a 0.25 in hole: 0.25 is said to be guessed
  (the design has it), 6.35 to be the design's (the engine would make it 6.35 inches); "0.3 in" is said to be
  guessed after the user wrote "make it 0.3 in".
- **A design made in mm and switched to inches** is described as "length 100", "radius 5" under "units in": the
  stored bare numbers are mm. Its cut holes read "diameter 0.19685039 in".
- **Words.** "0.266 in" for a hole of 17/64 in is withdrawn, and "0.197 in" for one of 5 mm would be by the same
  rule: rule 9 takes a size to 1e-6 and people round inches.
- The first question's example is "with 2 mm", and History's places ("base (10, 10, -1)") have no unit among
  sizes in inches.

No case of the list has a design that is not in mm. Centimetres, metres and feet go through the same code.

What the cases assume is built (decision below): the questions say the units; in a design not in mm the app
runs no command with a size that has no unit; marks and words compare lengths. **In a design in mm the
questions stay as they are**, byte for byte, and rule 11 does not apply; rules 12 and 13 do.

Real: the engine, the questions, the app's checks, the line. Faked: the agent's answers, the clock. "The inch
plate" is 4 × 2.5 × 0.25 in, fillets of 0.2 in, a hole cut by a cylinder of radius 0.125 in and a Hole of 17/64 in.

| Case | Input | Action | Observable result and pass criterion |
|---|---|---|---|
| G71 The questions say the units | the inch plate; the same in cm; the measured mm plate | build both questions | inch: the second says once that points are in mm and that every size is written with its unit, with an example in inches; its geometry is in mm and says so; in both, each place in History has "mm"; the first's example and request are in inches, and "2 mm" is nowhere. cm: the same in cm. mm: both are what they were before the change, byte for byte |
| G72 A design switched to inches | the mm plate (fillet 5, cut of radius 2.5, Hole 6.6), then `document.units` in | build both questions | every size in History and the usual values has a unit, and read by the engine is the length it was before the switch (1e-6); switched back to mm, History and the usual values are what they were |
| G73 A size without a unit is not run | inch and cm designs; every listed command that takes a size, each of its sizes in turn bare, as a number and as a text ("0.25"), the others with a unit; the boss of 25.4 above | answer; then frames to 600 s | withdrawn, `no unit`; 0 commands run on the copy; no ghost at any frame; not among those turned down; no new question without an edit |
| G74 What needs no unit | inch design: points, picked edges and faces, `dir1`, `normal`, `axis`, counts, `cs_angle: 82` as bare numbers; "w/2" with a parameter `w` of 0.5 in; in a design in mm, a bare size | answer | drawn, every time |
| G75 Sizes with a unit | inch design; each listed command that takes a size, once with sizes in inches and once the same in mm; the same step in a mm design | answer | all three drawn; volumes equal to 1e-6 and faces equal |
| G76 Tab in an inch design | the inch plate; a hole "0.25 in", a boss "1 in" | Tab; undo | lands as drawn (volume to 1e-6, faces, bodies); one undo step with the words; the feature keeps "0.25 in"; undo gives the document back |
| G77 Marks and words in inches | the inch plate. Marks: "0.25 in" and "6.35 mm"; "0.3 in" after the user wrote "0.3 in", "0.3" or "7.62 mm"; "0.4 in". Words for a `solid.hole` of 0.25 in at (20, 40, 6.35): "0.25 in", "1/4 in", `1/4"`, "1/4-inch", "6.35 mm"; "0.266 in" and "17/64 in" for 17/64; "0.197 in" for "5 mm"; "6.67 mm" for 20/3 in a mm design; against them "0.5 in hole", "0.125 in hole", "0.25 in radius hole", "0.27 in" for 17/64, "0.2 in" for "5 mm", "3.2 mm" for 3.175 | answer | marks: the design's, the design's; said, said, said; guessed. Words: drawn down to "20/3"; after "against them" withdrawn, `words differ` |
| G78 Units changed under it | the inch plate: `document.units` mm while the line is shown and its commands are out; separately with the ghost drawn | answer; Tab | withdrawn, `stale`; nothing lands; as G8 and G18 for any other edit |

Rules it adds:

11. In a design whose unit is not mm, a size in a command has a unit. A size is every number (or text that is
    only a number) of a command but a point, a pick, a direction, a count and an angle. One without withdraws
    the suggestion, `no unit`, and nothing runs. A value that names a parameter has the parameter's.
12. Adds to rule 9: a length is the size of a drawn hole when it equals it to 1e-6, or is that size rounded to
    the decimals written and within 0.5% of it.
13. Marks compare lengths: a number with a unit by its length; one without, in the user's words, as the design's
    unit or as mm.

| Run | How | Pass |
|---|---|---|
| M11 Inch parts | M1's four designs (9 states) in inches; the question as it is and G71's with rule 11, in turns; 3 each, 54 in all | with G71's: at least 26 of 27 drawn; 0 ghosts with a wrong scale (a size said in the words is 25.4 times bigger or smaller in the commands, or the ghost's box is more than twice the part's in any direction); the words in inches in at least 26 of 27; M1's times. Both: how many answers had a size without a unit, and how many ghosts a wrong scale |

M1 is not asked again: in mm the questions do not change (G71).

**Decided: a size without a unit is not run.** The choices were:

- **Not run it (the cases as written; chosen).** A wrong scale then cannot come from it, whatever the agent does. It
  loses a suggestion whose agent left out the unit but meant inches; M11 says how many.
- Take the engine's reading (inches) and change only the questions. Fewer are lost; one whose agent kept to
  the mm of the geometry is drawn 25.4 times too big, unless it is a hole's size said in the words. G73 would
  go, and M11's "0 ghosts" is then up to the agent.
- Have the app write "mm" onto it, and ask for sizes in mm. Nothing is lost, but an agent that copies 0.25
  from a History in inches makes a hole of 0.25 mm, and the features the user gets are in mm.

Not covered: "Add 4 1/4 in holes" (read as 4.25 in, withdrawn) and "Hole 2 in the corner" in an inch design
(read as 2 inches, withdrawn when no other size is said); both lose a right suggestion. Sizes other than a
hole's are still not compared with the words.

**Built 2026-11-25** (`ghost::without_unit`, `words.rs`, the questions in `predict.rs` and `ghost.rs`). G72's
"(10, 10, -1) mm" and the hole's "diameter 5 mm" in a switched design keep the unit the part was made in. An
independent reader of the code then found ways round rule 11, fixed the same day:

- Text worked out of bare numbers: "101.6 - 0.5 in", "8/n" with a unit-less parameter `n`, a point
  `["101.6/2", ...]` (an arithmetic point is read in the design's unit, a plain one in mm). A size or a point now
  lacks a unit when what it comes to changes with the unit its bare numbers are read in.
- Metres and feet were not read in words or marks; rounded sixteenths ("0.188 in" for 3/16) failed by float
  noise; the selection's numbers, the project's thickness and ".25" were read wrongly as marks.
- A cut given by its diameter (stored "(7 mm) / 2") was described in inches after the switch.

Inputs added to the approved cases to test that: G73 "101.6 - 0.5 in", "8/n" and the two points; G74 `center`,
`to`, `direction`, a pick by index, `profiles`, a mirror of a body and a point "16*w"; G72 a chamfer, a blind
hole's depth, a shell, a cut by diameter and a fillet of "1/16" in an inch design; G77 feet, metres, ".4",
two ties, the selection and the project's thickness; G78 the new question after the pause. G71's mm case
compares the questions' fixed text byte for byte and the geometry's lines by their form (the kernel's points
may differ in the last digit between machines). "2 1/8" was meant for G73 and G72, but the engine refuses it
as an expression, so it is not a size that can run. G76, G78, G74 and G75 passed before the code was there: they
hold what is not to change. Each part of the change was then broken on purpose, 37 changes one at a time: 30
were caught at first; the seven others (rounding edges, a fraction, the project's thickness, two readings of a
bare number) showed weak inputs, and with those sharpened all 37 are caught. M11 ran on the build before the
last of these test changes, which moved no code but the project's notes into a function of their own.

Left as they are: a variable fillet's `mid` and `knots` and a spline's `radii` need units in an inch design (a
position 0…1 among them reads as a size, so such a fillet is not run); a design whose unit is km, yd or mil
gets an example in mm.

### Approved by Tianwei 2026-11-26: rule 9 narrowed

**Why.** Rule 9 withdraws when the words give any length that is no number of the commands and no hole size
is said: "Add a hole 10 mm from each edge" drilled at (90, 50), "Drill a through hole in the 6 mm plate".
Tianwei decided to narrow it: a length counts as the hole's size only when radius or diameter is said of it,
or when it is twice or half a drawn hole's size. What is known before approval:

- **What it still catches.** The 14 wrong of the 258 recorded are "5 mm radius" (radius said) and "2.5 mm
  hole" (half the hole's 5): both kinds stay withdrawn. In M10 and M11 with the new questions no suggestion
  was withdrawn for its words.
- **What narrowing as worded gives up.** Nine inputs of the approved cases are near the hole's size and
  not twice or half of it, and would be drawn: G68's "0.25 in" and ".25 in" for a hole of 5 mm; G77's
  "0.27 in" for 17/64 in, "0.2 in" for 5 mm, "0.05 ft" for 0.25 in, "0.251 in" for 0.25 in, "17/64 in"
  for 0.2656 in, "3.2 mm" for 3.175 and "6.62 mm" for 6.6.
- **The cases below take a band of 5%** (decided with them, below): a length within 5% of a drawn hole's size, or
  of twice or half of it, counts as the hole's size. Of the nine, six stay withdrawn (all within 2%); "0.25
  in" and ".25 in" for 5 mm (27% off) and "0.05 ft" for 0.25 in (2.4 times) are drawn.

Real: the engine, the check, the app's line. Faked: the agent's answers, the clock. The plate is 100 × 60 × 6 mm;
"the cut" is G67's cylinder of radius 2.5 at (90, 50) that cuts.

| Case | Input | Action | Observable result and pass criterion |
|---|---|---|---|
| G67 adds | "Add a hole 10 mm from each edge" with a hole of 6 at (90, 50); "Drill a through hole in the 6 mm plate" with the cut; "Add a 0.25 in hole" and "Add a .25 in hole" with a hole of 5 (moved from G68); "Add a 5.3 mm hole", "Add a 10.6 mm hole" and "Add a 2.65 mm hole" with a hole of 5 (6% off its size, twice it, half it); the inch plate: "Add a 0.05 ft hole" with a hole of 0.25 in (moved from G77) | answer | drawn, every time |
| G68 adds | with a hole of 5: "Add a 5.2 mm hole", "Add a 10.4 mm hole", "Add a 2.6 mm hole" (4% off its size, twice it, half it); with a hole of 6 at (90, 50): "Add a 3 mm hole 10 mm from each edge", "Add a 12 mm hole 10 mm from each edge", "Add a hole of radius 4 mm 10 mm from each edge" | answer; then frames to 600 s | as G68: withdrawn, `words differ`, 0 commands run on the copy, no ghost at any frame |
| G67–G70, G77 as they are | every other input of the approved cases | answer | the outcome they have now |
| R1 Replay | the 258 recorded, and the answers recorded in M10 (80) and M11 (54), through the app's checks; no agent asked | replay | the same 14 of the 258 withdrawn for their words and no other; M10's and M11's answers get the outcomes in Measurements. Any answer whose outcome changes is listed with its words and its holes |

Rule 9 as it now reads (it replaces rule 9 above):

9. Only holes are checked: steps whose commands have `solid.hole` or a `solid.cylinder` that cuts. A
   length in the words (a number with a unit, or after R, ⌀ or Ø) that is another number of the commands
   (a depth, a coordinate) is left out, unless radius or diameter is said of it. Of the rest, a length is
   a hole's size when radius or diameter is said of it, or when it is within 5% of a drawn hole's size, of
   twice it or of half it; any other is not about the hole. The words disagree when a hole's size is left
   and none is the size of a drawn hole (rule 12).

What it lets through, beyond what is listed above: a length said without radius or diameter that is more
than 5% off a drawn hole's size, twice it and half it ("Add a 5.3 mm hole" for a hole of 5, "0.25 in" for 5 mm).

Decided with it: a band of 5%. The other choice was none (twice or half exactly, to rule 12's rounding),
which draws the nine near sizes above and G68's three new inputs at 4%.

**Built 2026-11-26** (`words.rs`, `differ`). The 103 tests of the predictor pass.

- **R1.** No outcome changes. The 258 recorded: 14 withdrawn for their words before and after. M10's 80,
  whose commands are recorded: the old question on the plate 18 of 20 before and after, the three other
  groups 0 and 0. M11's 35 drawn, whose commands are recorded: 0 and 0 for the words; 5 of the old question's
  8 have a size without a unit, as in Measurements, and none of the new question's 27. M11's 8 withdrawn for
  their words have no commands recorded; each says "diameter" of its size, which the narrowing does not touch.
  The narrowed check withdraws only where the one before did, so nothing drawn can become withdrawn.
- **Broken on purpose**, one change at a time. The narrowing's own 10 (the band, twice, half, the size itself,
  a radius or diameter said, other numbers): 9 caught; the tenth, a diameter said far from the size, had no
  input. Run again, the inch work's 37 are all caught; of the 36 of rule 9's first build, 6 no longer fit the
  code (5 were rewritten to it; the sixth is the narrowing's "other numbers"), and 4 were missed, since the inputs
  that caught them now draw. Inputs added to the approved cases for that: G67 "Drill a hole 10 mm deep" for a
  hole of 20 (a depth half its size); G68 "Add a 9 mm diameter hole", "Add a 0.2 in hole" and "Add a .2 in
  hole" for a hole of 5. With them, the 11 run again (G79's below among them) are all caught.

### Approved by Tianwei 2026-11-26: the built-in agent beside another

**Why.** On `develop` since #214 a release package carries the built-in agent, and the agents found on the
computer stay usable beside it (`agents::is_builtin` now means "the package carries one", not "it is the only
agent"). The guessing session took that to mean the chat runs the built-in agent. With the chat on Claude Code
in such a package, Claude Code was started as the built-in agent is: with its data folder and its connections
setup, and its session waiting for a "connected" that only the built-in agent sends. So no suggestion would
come in a release package. Found by reading the code when `develop` was taken in; not run. The fix, in
`predict.rs`: the session is the built-in one's only when the chat runs that agent (`running_id`).

Real: the app's choice. Faked: the agents (none is started).

| Case | Input | Action | Observable result and pass criterion |
|---|---|---|---|
| G79 The built-in agent beside another | the package carries the built-in agent; the chat runs Claude Code; again with the chat on the built-in agent (since decision 26 through the real `supported()`, no `any_agent`) | start the guessing session | Claude Code: started as an ordinary agent (no data folder or connections setup); the built-in agent: as the chat starts it. Codex is covered separately by G42. |

**Built 2026-11-26** (`predict.rs`, `runs_builtin`; the test is in `predict_tests.rs`). The test reads the
flag that both the start and the wait for "connected" go by; no agent program is there, so none starts. It
passes on the tree with `develop` taken in, and fails with the fix taken out.

### Approved by Tianwei 2026-11-26: suggestions on the built-in agent

**Why.** The review of PR 252: "merged develop, but prediction does not use the built-in Mastra Code".
Since #214 a release package carries the built-in agent and the chat starts on it, and `supported()` in
`predict.rs` lets only Claude Code and Codex through, so most users of a release get no suggestion.
Tianwei reversed the PRD's "Claude Code and Codex only, the built-in agent second" (decision 26 in
`next-step-prediction.md`).

**What was found** (read in Mastra Code 1.19.1, `@agentclientprotocol/sdk` 0.21.1 and `@mastra/core`
as installed on zhikun in `~/open-skyfall-eval/mastra-agent-sdk`, and in this tree; nothing run yet):

- **No `session/close`.** Mastra Code's ACP agent (`dist/acp/agent.js`) has no `closeSession` and offers
  no `sessionCapabilities`; ACP's SDK answers `session/close` with "method not found" then. A session
  is a whole Mastra Code runtime (its storage, workers and timers, and a folder under
  `<data>/sessions/`), cleaned up only when the connection closes. So today the guessing process would
  keep one runtime per suggestion until the app quits. The app already closes guessing sessions where
  the agent offers it (`acp_client.rs`, `new_session`).
- **`fast` is Explore, read-only.** Its tools are `view`, `find_files`, `search_content`, `file_stat`,
  `lsp_inspect`, `ask_user` (Intcad turns it off) and `list-workflows`, `get-workflow`
  (`agents/tool-availability.js`). `@mastra/core` sends only a mode's listed tools to the model
  (`resolveModeActiveTools`), so in `fast` neither MCP tools nor the design tools of the private
  `cadBridge` are offered. Switching the mode leaves the model as it is (`SessionMode.switch`).
- **The note.** The guessing process sends `system_note(SESSION_NOTE)`: the chat's whole note (use the
  `intcad-app` MCP tools, read `skill_reference`) with the guessing note after it as "the user's
  custom instructions". `session.mjs` then adds "Answer in the language the user writes in" and, with
  no project, the line about New chat (`hostOf`).
- **The packaged start.** The guessing process uses `start_builtin`, which strips only
  `SOLIDCRAFT_CONTROL_TOKEN`. The chat's packaged start (`start_packaged`) checks the package's own
  Node and entry (`packaged_agent`) and strips `SOLIDCRAFT_MASTRA_SDK`, `NODE_OPTIONS` and `NODE_PATH`
  too, so a variable left in the user's environment would make the guessing process load another SDK
  or run other code. `start_builtin` never adds `cadBridge` (it is added only where `cad` is set, and
  never to a guessing session), and the guessing process gets no bridge and no project.
- **Sign-in.** `key_sign_in` reads `_auth/status_update`, which only the Claude Code adapter sends. The
  built-in agent's kind of sign-in is in its connections snapshot: a plan (`plans[].state`), or a key
  (`keys[].saved`, not `shadowed`), or a custom address, each for the models under its `prefix`. So on
  the built-in agent every suggestion would count against the hourly cap, a key too.
- **Keys later.** The guessing process gets the chat's setup once, at its start. A key the user removes
  afterwards stays in the guessing process, which goes on using it.
- **The rest is the agent's own.** Words and units checks, the ghost on `Session::scratch` and Tab do
  not depend on the agent, and are kept as they are.

**Plan.** Rust in `crates/ui-egui/src/agent_chat/` (`predict.rs`, `acp_client.rs`, `acp.rs`); JavaScript
in `packages/mastra-runtime/agent/` (`agent.mjs`, `session.mjs`), the built-in agent's own files.

1. `supported()` lets the built-in agent through (`runs_builtin`); a Mastra workflow, an API endpoint
   and other ACP agents stay out. `any_agent` stays a test switch only.
2. The guessing process starts as the chat's agent does: in a package with the package check and the
   package's stripped environment, but with no `cadBridge` and no tool queue
   (`AcpBackend::start_packaged_guesser`); otherwise with `start_builtin` as now.
3. A built-in guessing session's `session/new` carries the guessing note alone and
   `_meta.intcad.nextStep: true`, with `mcpServers: []` and no project. The agent then gives it the note
   only (no language line, no project line), no MCP at all (`disableMcp`), no design tools even if asked,
   and puts it in `fast` before it answers `session/new`.
4. The built-in agent offers `session/close` and cleans the session up, as it does for all of them when
   the connection closes. The app's existing rule then closes each guessing session it is done with.
5. The hourly cap on the built-in agent follows the guess model's connection: a plan counts, a key or a
   custom address does not, anything not known counts.
6. Every setup the chat hands its agent is handed to the guessing process too, and so is the chat's
   last setup when a plan was signed in or out (a setup makes the agent read its sign-ins again). A
   session opened ahead of such a change is replaced before it is used.

Real and faked, by group:
- **Fake agent** (`predict_tests.rs`): the predictor, preferences, egui input, the real engine and
  checks, the ghost and Tab; the agent's answers and snapshots are scripted.
- **Stand-in agent** (`predict_tests.rs`, `acp_client.rs`): the app's real `AcpBackend` and the JSON it
  writes; the agent is a shell script that records what it is sent and answers `initialize` and
  `session/new`, in a package-shaped folder where the case says so (as `account_tests.rs` does).
- **Stand-in Base** (`packages/mastra-runtime/test/`, `node --test`): Intcad's agent files for real;
  Mastra Code's agent class and `createMastraCode` are stand-ins that record their calls, as
  `plan.test.ts` and `reads.test.ts` have them. No model, no network.
- **Real** (zhikun, M12–M14): the app, Mastra Code 1.19.1, a model on a key Tianwei enters himself.

| Case | Input | Action | Observable result and pass criterion |
|---|---|---|---|
| G80 The built-in agent makes suggestions | Fake agent; the chat runs the built-in agent (`running_id` `mastra`, a package carries it, ACP, no workflow); `any_agent` off; a snapshot with a plan signed in; the plate of G67 | edit; frames past the pause; scripted answers to both questions (a hole of 5 mm at (90, 50)) | `new_session` once after the snapshot; mode, model and effort set as in B1; two prompts; the line shows the step; the ghost is drawn; Tab lands it as one undo step whose volume equals the ghost's (G23's measure); the log's model is the small one |
| G81 Still not supported | Fake agent; the chat on the built-in agent with Mastra workflow execution (`chat.mastra`); an API endpoint; another ACP agent (id `gemini`) | edit; frames to 60 s | no session, no question; G37's sentence (G43 without the built-in agent) |
| G82 The hourly cap follows the guess model's connection | Fake agent; limit 20; snapshots: (a) plan `openai-codex` in, model `openai/gpt-5.4-mini`; (b) key `openai` saved, no plan; (c) key saved and the plan in (`shadowed`); (d) the plan expired and the key saved; (e) a custom address, the model under its prefix; (f) the model under no prefix; (g) no snapshot yet | 60 suggestions in 30 min each | (a), (c), (d), (f), (g): 20 asked, the 21st not, one notice; (b), (e): 60 asked; the log's `keySignIn` false and true to match |
| G83 The guessing process follows the chat's setup | Fake agent; guessing process started with setup S1 (key `openai`); a session opened ahead | the chat hands its agent S2 (the key removed); separately a plan signed out in the chat (`builtin_changes` moves, no new setup) | the guessing backend gets `setup` S2, once; after the sign-out it gets S1 again, once; the session opened ahead is replaced (`new_session`) before its question; nothing is sent while nothing changed |
| G84 What a built-in guessing session asks for | stand-in agent named `mastracode`, built-in; custom instructions in the preferences; a project open in the window | the guessing process opens a session; the chat opens one | guessing `session/new`: `mcpServers: []`, `_meta.systemPrompt.append` equal to `SESSION_NOTE` (no word of `SYSTEM_NOTE`, no custom instructions), `_meta.intcad.nextStep: true`, no `_meta.intcad.project`, no `cadBridge`; the chat's `session/new` byte for byte as before this change |
| G85 Started as the package starts it | stand-in `node` in a package-shaped folder (`agent/main.mjs`, `agent.json`, the two SDK `package.json`); the chat on the packaged route; separately the folder without `agent.json`; separately the entry outside `agent/` | start the guessing process | started as `[<root>/node, <root>/agent/main.mjs]`, in its environment `MASTRA_APP_DATA_DIR` = the data folder and `MASTRA_TELEMETRY_DISABLED=1`; the backend's list of removed variables is the package's (`SOLIDCRAFT_CONTROL_TOKEN`, `SOLIDCRAFT_MASTRA_SDK`, `NODE_OPTIONS`, `NODE_PATH`); its `session/new` has no `cadBridge`; the broken folders: nothing started, the state `paused` with the package's error, and the chat's backend untouched. The developer's route (one program named by `SOLIDCRAFT_MASTRA_AGENT`) starts as B9 has it |
| G86 Each guessing session is closed | stand-in built-in agent offering `sessionCapabilities.close`; separately one without it | three suggestions | with the offer: `session/close` for each session the app is done with, so no more than two open at once (one in use, one opened ahead); without it: no `session/close` sent |
| G87 The note alone for a guess | `hostOf` with `_meta.intcad.nextStep: true`, a note N and a project | call | `{project: null, instructions: N}`: no language line, no brief, no project line; without the flag every existing `hostOf` test as it is |
| G88 A guess session has no MCP and no design tools | stand-in `createMastraCode`; `nextStep: true`; the request names an MCP server and a `cadBridge` catalog | `session/new` | `createMastraCode` called with `disableMcp: true`, `mcpServers: {}`, no `extraTools`, `hostInstructions` = N; without the flag: the options as today |
| G89 `session/close` | stand-in Base with sessions s1 (a turn running) and s2 | `initialize`; `closeSession(s1)`; a prompt on s1; `closeSession(s1)` again; `closeSession("nope")` | `initialize` offers `sessionCapabilities.close` and keeps the other capabilities; s1's turn cancelled and aborted, its listener removed, its cleanup run once (its folder gone); the prompt and both later closes answered with an error; s2 untouched |
| G90 A guess session starts read-only | stand-in Base whose session offers modes `build`, `plan`, `fast`; separately one without `fast` | `newSession` with `nextStep: true`; without it | with the flag: mode `fast` before the reply, and the reply's `mode` option says `fast`; without `fast`: the session refused with an error and cleaned up; without the flag: the mode as Mastra Code opened it |
| M12 The built-in agent, packaged (real) | the package layout beside the test build on zhikun, Mastra Code 1.19.1, a key Tianwei enters; case, bracket, plate, flange; 5 runs (Tab in runs 1 and 2): 45 suggestions | harness (`next_step_ab.sh`, `NSM_AGENT=mastra`) | as M1: line median ≤ 2.5 s, ghost median ≤ 4 s and 9 in 10 ≤ 6 s, ≥ 95% run and change the part; Tab lands equal every time (M7's measure); the stamp held before Tab 45 of 45; tokens reported for each question; at most two guessing sessions' folders under `<data>/sessions/` and no growth in the guessing process's memory over the 45 (sampled after each suggestion; reported). Also reported: the data folder's size before and after |
| M13 The developer's route (real) | as M12 with `SOLIDCRAFT_MASTRA_AGENT`, one run: 9 suggestions | harness | at least 8 of 9 drawn |
| M14 Nothing reaches MCP (real) | an MCP server that records each start, set up in the built-in agent's own home; M13's run | harness; then one chat message | no start from the guessing process; the chat's session starts it (the probe works) |

Existing tests that change with it: G43 (the built-in agent is no longer in its list; G81 takes it),
G79 (the chat on the built-in agent without `any_agent`), and `setup_builtin` in `predict_tests.rs`
drops `any_agent`, so B1–B8 run through the real `supported()`.

Decisions approved with the list (in `next-step-prediction.md`): 27 the guessing session marked
`nextStep` for the built-in agent; 28 `session/close` in the built-in agent; 29 the cap by the guess
model's connection; 30 the guessing process follows the chat's setup; 31 the model on the built-in
agent stays the small-model rule (no Sonnet as on Claude Code) until M12's guesses are read.

**Built 2026-11-26.** Rust: `predict.rs` (`supported`, `BuiltinStart`, `paid_by_use`, `follow_setup`),
`acp_client.rs` (`start_packaged_guesser`, the guessing branch of `new_session`), `acp.rs`
(`next_step_session_params`), `connections.rs` (`Conn::setups`). The agent: `agent.mjs` (`closeSession`,
the read-only mode for a guess), `session.mjs` (`isNextStep`, `hostOf`, `sessionBuilder`). G80–G86 are
in `predict_ghost_tests.rs`; G87 in `test/brief.test.ts`; G88–G90 in `test/connections.test.ts`, where
`sessionBuilder` and `intcadAgent`'s session methods were already tested (the list said brief.test.ts for
G88). Two helpers changed for the real runs: `next_step_conn.py key <provider>` asks for a key without
showing it, and `autocomplete_measure.py` no longer waits 90 s for the built-in agent to be "found".

- **Tests** (zhikun, `ci` profile): the predictor's 111 pass (104 before), 1 measurement ignored; the
  agent chat's 864 pass (857 before), 4 ignored; the agent's four test files 59 of 59 on Node 22.23.4.
  `cargo fmt --check` and clippy (`-p solidcraft-ui-egui --all-targets -D warnings`) clean.
- **Broken on purpose**, one change at a time. Rust, 7 of 7 caught: the built-in agent left out of
  `supported` (13 tests fail, G80 and B1–B8 among them); a held plan not put before a key (G82); the
  setup not handed on (G83); a session opened ahead used after a change (G83); the guessing branch of
  `session/new` taken out (G84, G85); `start_builtin` used in a package (G85); the chat's list of
  variables in place of the package's (G85). The agent, 15 of 15 caught: the flag check in `hostOf`
  (G87, G88); `disableMcp` for a guess, ignoring a guess's MCP servers, withholding `extraTools` (G88);
  `close` in `initialize`, the abort, cancelling the turns, answering an unknown id (G89); taking the
  session off the list, the unsubscribe, the cleanup (G89, G90); the switch to `fast`, the reply's
  `modes`, keeping plan-first off a guess, ending a refused guess (G90).

**Real runs (2026-11-26, zhikun).** The test build of `/tmp/m1stub.sh` (the control channel's edits
taken as the window's, not committed), with a package folder made by `agent-package.mjs build` for
`x86_64-unknown-linux-gnu` beside it (Mastra Code 1.19.1, the system's Node 22.23.4, both skill sets).
One DeepSeek key, entered by Tianwei; the guesses ran on `deepseek/deepseek-flash` at the lowest
effort (the session already on it; the small-model rule had nothing to change). Harness:
`next_step_ab.sh` with `NSM_AGENT=mastra` and `autocomplete_measure.py`; the machine had a load of
about 6 from other work.

| Run | Result | |
|---|---|---|
| **M12**, the package's agent, 45 suggestions | line median 1.60 s (90% 2.27 s); **ghost median 3.53 s, 9 in 10 within 5.27 s** (8.9 s at the slowest, four boxes for a lid lip); 44 of 45 ran and changed the part (97.8%); Tab 17 of 17 landed equal to the ghost (24 ms median); the stamp held 45 of 45; input and output tokens reported for both questions of every suggestion (about 9.0k and 10.3k input), no cost reported; at most two session folders of one process at any sample (one in use, one opened ahead) | **passes**, but for the memory (the long run below) |
| **M13**, the developer's route, 9 suggestions | 7 of 9 drawn; line 1.56 s, ghost 3.73 s median; Tab 7 of 7 equal | **misses by one** (8 of 9) |
| **M14**, an MCP server in the agent's own home | started 16 times, every time by the chat's agent process (four times as each run's chat connects); never by the guessing process. The chat's process is told by its pid (started first in each run) and by its memory (the guessing process's grows with its sessions) | **passes** |

- **After Huaiyuan's review fixes** (rebased on `b102c0e69`: Tab only for a ghost on screen, the
  harness's disposable runs): a smoke run on the package's agent, plate and case with Tab, 5 of 5 drawn
  and 5 of 5 landed equal to the ghost (17–40 ms), the stamp held 5 of 5. M12–M14 were not run again.
- **What was not drawn.** Three of 54: twice the agent answered `{"commands": []}` for a fillet of edges it
  could not name ("the remaining vertical corner edges"), once a hole through the case's bottom failed on
  the copy. Each was withdrawn as the rules say; none is particular to the built-in agent.
- **Processes.** In each run the chat's and the guessing process start together. The chat's stays near
  200–280 MB; the guessing process grows from about 250 MB to 330–465 MB over a run's two or three
  suggestions. Each run is a new app, so a process lived through three suggestions at most.
- **The long run** (30 suggestions in one app run on the package's agent, an edit adding a box each time,
  sampled every 2 s): 30 of 30 drawn; at most two session folders of the guessing process at any sample.
  Its memory **rose from about 250 MB to 550 MB over the first 12 suggestions, then held between 550 and
  605 MiB to the 30th** (568–580 MiB over the last eight); the chat's process stayed under 260 MB. So it does
  not stay flat as M12's case asked, and it was not run long enough to show that it stops growing: a
  run of a few hundred suggestions would tell. Claude Code's guessing processes did not grow over M1's 45.
- **The data folder** grew from 0.9 MB to 3.7 MB over M12's 45 suggestions, about 60 kB each: Mastra
  keeps each closed session's thread in its storage.
- **First reading of the guesses** (Tianwei judges): shell the case 10 of 10 (2 or 3 mm, open at the top);
  four corner holes on the plate 5 of 5, the other three corners 4 of 5 at plate state 2 (once one hole),
  and none asked to cut the hole again (M6); the flange's bore 5 of 5 and the bolt-hole pattern 5 of 5
  (four or six copies); at bracket state 1 fillets 4 of 5 and holes once, at state 2 joining the boss 3 of 5.
  As with Haiku on Claude Code, the small model leans to fillets on the bracket.

### Measurements

On zhikun, Claude Code through `claude-agent-acp` 0.107.0, Sonnet at low effort, full sessions on a
Claude plan sign-in. Harness: `examples/demo/next_step_ab.sh` with
`NSM_SCRIPT=autocomplete_measure.py`. Costs are what Claude Code reports at API prices.

| Run | Result | |
|---|---|---|
| M1, 45 suggestions | line median 2.17 s (limit 2.5); **ghost median 4.95 s, 9 in 10 within 6.72 s (limits 4 and 6)**; 43 of 45 ran and changed the part, 95.6% (limit 95%); both questions together $0.114 median, about 23k and 24k tokens of context | misses on the ghost's time |
| **M1 again, one session, 45 suggestions** | line median 1.47 s; **ghost median 3.14 s, 9 in 10 within 4.65 s**; 44 of 45 ran and changed the part, 97.8%; both questions $0.091 median, about 28k and 31k tokens of context | **passes** |
| **M1 with `strictMcpConfig`, 45 suggestions** | line median 1.33 s; **ghost median 2.84 s, 9 in 10 within 3.59 s**; 45 of 45 ran and changed the part; both questions $0.059 median, about 22k and 24k tokens of context | **passes** |
| **M10**, 80 suggestions | the plate: words disagree **18 of 20 with the old question, 0 of 20 with the new**; the second plate: 0 of 20 and 0 of 20; the ghost drew the design's hole size 80 of 80 | **passes** |
| **M1 with G66 and the check, 45 suggestions** | line median 1.32 s; **ghost median 3.11 s, 9 in 10 within 3.82 s**; 45 of 45 ran and changed the part, none withdrawn for words (25 make a hole); both questions $0.060 median | **passes** |
| **M7 with one measure** (`ModelBody::measure`, 1e-6) | 18 of 18 accepted and equal: 20 bodies, the largest difference 0, faces equal; Tab 25 ms median; the stamp held 45 of 45 | **passes** |
| **M11**, inch parts, 54 suggestions | **new question and rule 11: 27 of 27 drawn**, none with a size without a unit, none at a wrong scale, the words in inches every time (3 with no length); Tab 27 of 27 equal; line 1.37 s, ghost 3.31 s median, 9 in 10 within 5.77 s. **Old question: 8 of 27 drawn**: 10 failed (a fillet or shell of mm numbers read in inches), 8 withdrawn for their words (a hole of mm numbers), 1 no change; 5 of the 8 drawn had sizes without a unit, and one of them a pattern spacing of 85.6 (inches) that put its copies off the part | **passes** (the 9 in 10 by 0.23 s) |
| M2, lean | not run: needs an API key sign-in | open |
| M3, Codex | prediction is refused before launching a background session; not run against a real Codex account. Reopen only after verifying MCP and write-tool isolation | deferred |
| **M4 with `strictMcpConfig`**, two questions in each session | 0 of 20 questions reached the recording server, and Claude Code never started it; with both protections taken out it started in 3 of 3 sessions and 6 of 6 questions reached it | passes |
| M4, two questions in each session | 0 of 20 questions reached the recording server; with the block taken out 6 of 6 did | passes |
| M4, first run | 0 of 10 questions reached the recording server; with the block taken out 3 of 3 did, with no permission asked | passes |
| M5 | longest frame 0.3 ms while the ghost computed, dropped at 3 s; but **building the second question holds the window for 56 ms** (headless, no drawing) | passes |
| M6 | 0 of 11 asked to cut the hole again (5 in M1, 1 in a trial, 5 on the later build) | passes |
| M7 | 11 of 11 accepted (9 in M1), faces and bodies equal, volume within 0.1%; Tab 30 ms median; the stamp held every time | passes on time and stamp; equal volume shown by the later run (above, and below) |
| M8, 21 suggestions | 18 ran and changed the part, 85.7% (limit 90%) | **misses: sheet metal leaves this version** |
| M9 | not run: needs two modellers | open |
| After the review, 7 suggestions (case, plate, flange, one run each) | 7 of 7 drawn, accepted with Tab and equal, the stamp held; line 1.69 s, ghost 3.86 s median (8.4 s at the slowest, a fillet the engine took 2.5 s for) | too few to replace M1 |

- **One session (2026-11-24).** M1 rerun on it, 45 suggestions: line median 1.47 s, **ghost median
  3.14 s, 9 in 10 within 4.65 s** (7.7 s at the slowest, a fillet the engine took 2.5 s for), 44 of 45
  ran and changed the part (97.8%), 17 of 17 landed equal with Tab, the stamp held every time. The one
  withdrawn: a fillet that failed on the copy.
- **How much of it is the change.** The service was faster than in the first M1, so the two builds
  were run in turns, 18 suggestions each: two sessions 4.24 s median and 7.1 s for 9 in 10; one
  session 2.89 s and 3.3 s.
- **At a person's pace.** The harness asks again within seconds. With 20 s before each edit (18
  suggestions, all drawn) the ghost took 3.69 s median and 4.45 s for 9 in 10. Other work was on the
  machine in the first minutes of it. It is the number to expect.
- **Processes.** With `session/close`, the app's child processes (the chat's agent included) were at
  most 14 and 1.6 GB resident during M1, and did not grow with the suggestions. A session left open
  keeps two processes and about 320 MB (6 sessions: 1.9 GB), so the build before left about 650 MB
  per suggestion.
- **What a session opened ahead carries.** The user's own MCP servers have time to connect, so their
  instruction text (18,000 characters on this machine) is in the questions: 28k tokens of context
  instead of 23k. Their tools stay blocked (M4 again, with the servers connected and two questions in
  each of 10 sessions: 0 of 20 questions reached the recording server; without the block 6 of 6).
  `strictMcpConfig` keeps the text out (next).
- **`strictMcpConfig` on the full session (2026-11-25).** The user's MCP servers are not loaded in a
  guessing session at all; `mcp__*` stays blocked too. At a person's pace (20 s before each edit), the
  builds without it and with it in turns, 18 suggestions each, all drawn: 28.4k tokens of context
  before and 21.9k after on the first question, both questions $0.124 and $0.059, the first question
  written to the prompt cache again 9 times and 0 times. The time is the same: ghost 2.99 s median and
  4.45 s for 9 in 10 without it, 2.89 s and 4.54 s with it. M1 on it: 18 of 18 landed equal with Tab, the stamp
  held 45 of 45, the app's child processes at most 10 and 1.3 GB. Another session's demo was on the
  machine in part of these runs.
- **The words give the hole the wrong size.** On plate state 2 (a hole cut by a cylinder of radius
  2.5 mm) the ghost drew the right size 7 of 7 times with `strictMcpConfig`, but the words said "5 mm
  radius" 4 times, "2.5 mm hole" once, and were right twice. The build before it said "5 mm radius" 2 of
  11 times, and the probes before the ghost was built had it 2 of 2 times in a full session. Too few to
  say the change made it worse. Since G66 and G68: next.
- **Why the words had it wrong (M10).** On the plate whose hole's diameter and fillets are both 5 mm, the old
  question gave 18 wrong of 20. On the same plate with 4 mm fillets and a hole of 6 it gave 0 of 20, though
  History said "radius 3" there too. So the 5 mm of the fillets was mixed up with the hole's, and a radius in
  History alone does not do it. The question of G66 ends it on both plates (0 of 40).
- **The new question asks for holes more often.** In M1 bracket state 1 gave holes 5 of 5 times (before it
  fillets 4 and a hole 1), case state 3 holes 5 of 5 (a lid, fillets 3, a hole 1). The sentence "A hole's size is
  its diameter" is the one thing new in those questions, 5 suggestions a state; whether it is the cause was not
  tested. All were drawn, but a step the user wants is not measured (M9).
- **The second question misses the cache.** It reads 10k of its 30k tokens from the prompt cache and
  writes the rest again, so it costs about $0.085 (44 of 45), though the first question used the same
  prefix a second before. With `strictMcpConfig` the same, at about $0.056 (44 of 45). In M4 it reads them ($0.007 to $0.010). The cause was not found.
- **M1's two withdrawn.** One fillet failed on the copy, one took over 3 s.
- **What "equal" was in M7 (corrected 2026-11-25).** Here and in every "equal" of the real runs above, the
  harness allowed 0.1% in volume, and compared two measures: the ghost's from a mesh (`volume_at(0.01)`),
  the landed part's from `inspect.measure`. On the 58 landed bodies they differ by up to 0.05%, faces
  equal 58 of 58. On two plates the landed volume is the exact one by formula (35635.619 and 35192.655
  mm³) and the ghost's is off by 0.38 and 0.34, as a mesh is. So the difference looks like the measure,
  but 0.1% would not see a hole of radius 2.6 instead of 2.5. G23 compares one measure to 1e-6 and G24 the
  STEP texts; both pass. Since then the harness uses one measure, and M7 was run again (above): 20 bodies,
  no difference. Plate state 2's landed volume is 35,400.0000 mm³, the formula's number.
- **M8.** Every sheet-metal command given ran (13 of 13 flanges). The three misses were on bodies
  that were already bent: twice no commands for a step (a fillet on the bend corners, a mirrored
  hole), once the same hole again (`repeat`). One set of three parts, three runs each; the flange's
  own height and angle read as guessed even where the design has them.
- **The same question does not always get the same step.** On plate state 1, the later build (same
  question length, 15 minutes on, about 80 tokens less of Claude Code's own context) gave a 5 mm
  fillet on the top edges of a 6 mm plate 5 of 5 times, where M1 gave four corner holes 6 of 6.
  The cause was not found. On plate state 2, where the hole's radius is 2.5 mm, M1 drew the
  right size 4 of 5 times and once (and in the trial) a 5 mm radius; another time its words said a
  5 mm radius and the ghost drew 2.5. The later build gave the right size 5 of 5 times.
  On the one-session build plate state 1 gave four corner holes 5 of 5 times in M1, and state 2 drew the
  right hole size 9 of 9 times, with "5 mm radius" in the words twice.
- **Marks.** A number counts as the design's only for the same kind of thing, so pattern spacings,
  extrusion distances and flange sizes always read as guessed. The error is on the safe side.

### Checks

On zhikun with G57–G64, G66–G78 and `strictMcpConfig` in: `cargo fmt --check`, clippy with warnings as errors (smlib, kernel,
doc, ui-egui, all targets), `cargo xtask lists`, `i18n`, `layers` and `ui-containers`, and the wasm
check pass. Tests: 103 of the predictor's pass (one measurement ignored); 1,472 of ui-egui's (chat,
rail, preferences, settings, strings, timeline, preview) pass. In an earlier run beside M4 two
timing tests of other features failed; both pass alone. In doc (2), engine (11), kernel (13) and smlib (1),
run before this change and not touched by it, every test that fails is in `tests/KNOWN_FAILING_TESTS`.
The page's 17 checks ran through a Node port of the
Python ones, before the review. Not run: the full `cargo xtask ci`, the Python page checks, the page's `tsc`, Windows and
macOS.
