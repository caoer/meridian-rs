---
tags: [type/rule, rules/check]
id: status-move
paths:
  - tasks/**
---

# status-move (U18 — the board's status-move guard)

A board card's `status:` moves along the board, never sideways. The board
vocabulary this repo already uses is `todo → in-progress → blocked → review →
done` — declared verbatim by `effects/tests/fixtures/rules/task_conventions.star`
("`status` changed — valid values: todo, in-progress, blocked, review, done") and
asserted by two goldens (`golden__all_rules_status_change.snap`,
`golden__task_conventions_notice_refresh.snap`); the forward spine also shows up
in the hook-tier cards `tests/hook-tier/corpus/tree/tasks/*.md`, the reaction
feeder's `todo`→`in-progress`→`review` cases in `wire-serve/src/reaction.rs`, and
the run plane's `todo`/`done` board pages.

Two non-forward moves are admitted, each for a reason the tree states:

- The BOUNCE `review → in-progress`: the `close-verdict` floor rules a
  re-decision must LAND rather than be refused, and a reviewer who rejects must
  be able to send the card back for rework.
- The ESCALATION pair `in-progress ↔ blocked`. `blocked` is where in-flight work
  parks while someone else clears the way — the corpus card
  `mrd/tests/corpus/tree/tasks/r3a-impl-plan.md` states the entry rule ("DD
  conflict with repo reality → card blocked + message leader; never silently
  deviate"), `policy/src/reaction.rs:237` exercises `in-progress → blocked` as an
  ordinary transition, and `reaction.rs:393` hangs `blocked_by:` on an
  `in-progress` card. Every one of those sources attaches blocking to work
  already in flight, so `in-progress` is the only status that enters `blocked`,
  and the escalation resolving returns the card to the work it interrupted —
  `blocked → in-progress`, and nowhere else. A `todo` card has no one working it
  to block; a `blocked` card is not finished, so it cannot reach `review` or
  `done` without passing back through the work.

This rule is the mechanism of verdict row **9.12 (gate-as-rev), SUPERSEDED**:
a status move is guarded by an armed starlark CHECK, not by a new engine surface
tying a written fingerprint to the move (requirements decision 10, R1.4).

What it judges, and what it deliberately does not:

- Only a MOVE — both states carry a `status:` and the two differ. A card's birth
  (no `status:` before) is not a move and is never judged; neither is a write
  that leaves `status:` alone while editing other fields.
- Only pages the `paths:` scope covers (`tasks/**`) — the run plane's board
  pages are not task cards.

```starlark
def check_change(change):
    # Judge the status MOVE only — nothing else on the card.
    if "status" not in change.fields_changed:
        return
    before = change.before.frontmatter.get("status")
    after = change.doc.frontmatter.get("status")
    if before == None or after == None or before == after:
        return

    legal = {
        "todo": ["in-progress"],
        "in-progress": ["review", "blocked"],
        "blocked": ["in-progress"],
        "review": ["done", "in-progress"],
        "done": [],
    }

    allowed = legal.get(before)
    if allowed == None:
        refuse(
            message = "status-move: " + change.doc.path + " sits at `" + before +
                      "`, which is not a board status. The board is `todo` -> `in-progress` -> `review` -> `done`, with `in-progress` <-> `blocked` while the card is escalated. Set the card to the board status it really occupies, then move it one step.",
            passing = "advance-to-next-status",
        )
        return
    if after in allowed:
        return
    if len(allowed) == 0:
        refuse(
            message = "status-move: " + change.doc.path + " moved `" + before +
                      "` -> `" + after + "`. The board has no move out of `done` — a finished card stays finished. Open a new card for the follow-on work.",
            passing = "advance-to-next-status",
        )
        return
    refuse(
        message = "status-move: " + change.doc.path + " moved `" + before +
                  "` -> `" + after + "`, which is not a legal board move. From `" +
                  before + "` the legal moves are: `" + "`, `".join(allowed) +
                  "`. Move the card one step along the board (`todo` -> `in-progress` -> `review` -> `done`); `review` bounces back to `in-progress` for rework, and `in-progress` parks at `blocked` while the card is escalated.",
        passing = "advance-to-next-status",
    )
```
