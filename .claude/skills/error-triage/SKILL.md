---
name: error-triage
description: Use when picking the next translator phase or chore in duelscript — analyses `duelscript check` output, buckets errors by shape, and surfaces the highest-yield fix. Cuts the audit step from ~10 min to ~30 sec.
---

# Error Triage

The duelscript corpus check produces ~3,500 errors right now. Most are the same shape; the long tail is small. This skill turns the raw output into a ranked yield list.

## Run the check

```bash
cargo run --release --features cdb --bin duelscript -- check cards/official/ 2>&1 | tail -3
```

Last line gives the totals: `── Summary: 13298 file(s) checked, N error(s), M warning(s) ──`.

## Bucket by shape

```bash
cargo run --release --features cdb --bin duelscript -- check cards/official/ 2>&1 \
  | grep -E "ERROR\]" \
  | sed 's/.*\[ERROR\] [^:]*: //' \
  | sort | uniq -c | sort -rn | head -20
```

The dominant error class today is `Effect 'Effect N' must have a resolve or choose block`. Sub-bucket by `Effect N` index — Effect 1 stubs are the most common (active spells/traps; trigger effects on monsters), Effect 2/3 are continuation chains.

## Find what shape the empty-resolve cards have in lua

Use the `corpus-audit-pattern` skill to scan `/Users/marco/git/CardScripts/official/cXXXX.lua` for cards with empty resolves and bucket by:

- `EFFECT_TYPE_*` distribution.
- `SetCode(EFFECT_*)` distribution.
- Most-frequent unmapped `Duel.*` methods (compare against the known-mapped list in `lua-corpus-context`).
- `RegisterEffect` receiver distribution (`tc`, `c`, `e:GetHandler()`, group iterators, …).

Highest yield = highest count × MVP-feasibility. A shape that hits 200 cards but needs new DSL grammar is lower priority than one that hits 100 cards and reuses existing grammar.

## Existing yield estimates (as of 2026-04-30)

Per `project_session_handoff.md`, the open phases ranked by estimated yield:

| Phase | Description | Est. cards | Reuses grammar? |
|---|---|---|---|
| 4c | non-literal SetValue (function-refs, expressions) | ~150 | yes |
| 5c | non-stat passive codes (EFFECT_IMMUNE_EFFECT, EFFECT_CANNOT_*) | ~100-200 | yes (grant_decl) |
| 6 | s.condition body extraction | ~300-500 | yes (condition_expr) |
| 7 | s.cost body extraction | ~200 | yes (cost block) |
| 8 | s.target body extraction | ~400 | yes (target_decl) |
| per-archetype | Salamangreat / Madolche / Trickstar / Drytron templates | varies | mostly |
| 5d | drop redundant Field-type stubs (Phase 5b broaden) | ~80 | yes (chore) |

These numbers are observed counts from the audit step at the time of the previous session. Re-audit before starting — the corpus shifts as phases land.

## Decision rule

1. Translator extension that reuses existing grammar → highest priority. (Phases 4c, 5c, 6, 7, 8.)
2. Chore PRs that drop redundant stubs after a translator phase → ship as follow-up.
3. New grammar required → use `grammar-extender` agent first; that's a longer T-series-style PR.
4. Per-archetype templates → only after no-grammar phases are exhausted; high effort, archetype-bound yield.

## Hazards

- **Don't fixate on the headline error count.** Adding 165 passive blocks (Phase 5) only dropped 4 errors but materially improved the corpus. Some shapes need a follow-up chore PR (Phase 5b) to convert their structural improvement into an error reduction.
- **Don't pick a phase that needs grammar without a plan.** Grammar additions cross the FF-I seam (parser + AST + validator + compiler + tests). They're full T-series PRs.
- **The "errors" denominator is misleading.** Many cards have multiple effects; one card can be 3 errors. Per-card yield is a better metric for prioritising phase MVPs.

## Where this skill applies vs related skills

- After triage picks a phase → run `translator-phase-pattern` skill end-to-end.
- For lua-side decoder → `lua-corpus-context` skill.
- For the audit script template → `corpus-audit-pattern` skill.
