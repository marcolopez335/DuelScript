---
name: translator-phase-pattern
description: Use when working on a Phase N / Nb / Nc translator step in duelscript — extending lua_ast.rs to recognise a new lua chain shape and applying it to cards/official/. Codifies the audit → MVP → emit → apply → measure → 2-commit-PR workflow used across Phases 4, 4b, 5, 5b.
---

# Translator Phase Pattern

Repeatable methodology for adding a new lua-chain shape to the duelscript translator. Used in PRs #56 (Phase 4), #57 (4b), #58 (5), #59 (5b).

## When to use

- A class of empty `resolve { }` blocks all share a common lua shape.
- Or: a class of cards is missing a passive / restriction / replacement block that lua already encodes.
- Or: an existing translator pass left a regression worth retrofitting.

If only one or two cards match the shape, hand-write them. The phase pattern earns its overhead at ~30+ cards.

## The 9 steps

```
1. Audit prevalence  →  2. Narrow MVP  →  3. Extend lua_ast.rs
                        ↓
4. Unit tests  →  5. Translator emit fn  →  6. Apply mode integration
                        ↓
7. Run apply, check delta  →  8. 2-commit PR  →  9. Rebase-merge
```

### 1. Audit prevalence

Use the `corpus-audit-pattern` skill. A python regex over `cards/official/*.ds` cross-referenced with `/Users/marco/git/CardScripts/official/cXXXX.lua` is the standard tool. Output: candidate count, sample card IDs, breakdown by sub-shape.

Decision: if candidates < 30, skip — not worth a phase.

### 2. Narrow MVP

For each candidate sub-shape, decide: emit, skip, or defer. Bias toward **skip-not-mis-emit** — silently producing a wrong DSL line is worse than leaving the resolve empty.

Common reasons to skip in MVP:
- Multi-target loops without a translatable group source.
- Non-literal `SetValue` (function refs, expressions).
- Unknown receiver paths in `<X>:RegisterEffect(eN)`.
- `EFFECT_TYPE_FIELD` chains without `SetTargetRange`.

Document the skips — they become the next phase's backlog.

### 3. Extend `src/lua_ast.rs`

The two extension points:
- **`EffectSkeleton`** — top-level chains in `s.initial_effect`. Used by Phase 5.
- **`FunctionBody::register_chains: Vec<RegisterEffectChain>`** — chains inside operation handlers. Used by Phase 4 / 4b.

Add new struct fields conservatively. Each new field should answer one question (`code`, `value`, `reset`, `register_target`, `multi_target`, `loop_source_group`, …). Keep raw arg strings until emit time — easier to add new lookups later.

### 4. Unit tests

`cargo test --lib --features lua_ast,cdb lua_ast`. Each test sources a real-shape lua snippet, calls `walk()`, asserts the extracted struct, and asserts the emitted DSL line. Patterns:

- One test per primary shape (single-target, multi-target, clone, etc.).
- One test per skip case (non-literal value, unknown receiver, missing TargetRange).
- One test for negative-value sign handling.

### 5. Translator emit fn

Either:
- `translate_register_chain(chain) -> Option<DslLine>` for in-resolve emission.
- `EffectSkeleton::passive_modifier_spec() -> Option<PassiveModifierSpec>` + `to_dsl_block()` for passive/restriction emission.

Always return `Option` so unrecognised shapes drop out cleanly.

### 6. Apply mode integration

`src/bin/lua_translate.rs apply` has two passes:

- **Pass A** — fill empty `resolve { }` blocks. Uses `walk.functions[handler]`.
- **Pass B** — inject card-level blocks (passive / restriction). Uses `walk.effects[i]`.

If your phase emits per-effect lines, extend Pass A. If it emits card-level blocks, extend Pass B with a dedup check (modifier-line scan, name-clash scan, or block-text scan) so reruns are idempotent.

### 7. Run apply, check delta

```bash
cargo run --features lua_ast,cdb --release --bin lua_translate -- \
    apply cards/official /Users/marco/git/CardScripts/official
cargo run --release --features cdb --bin duelscript -- check cards/official/ 2>&1 | tail -3
cargo test --lib                        # baseline
cargo test --lib --features lua_ast,cdb # extended
```

Record before/after error and warning counts.

### 8. 2-commit PR

Always split into two commits — keeps the diff reviewable:

1. `feat(lua_ast): Phase N — <shape description>` — touches `src/lua_ast.rs` + `src/bin/lua_translate.rs` + tests only.
2. `feat(corpus): apply lua-ast Phase N (-X errors)` — touches `cards/official/*.ds` only.

Co-author trailer:
```
Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
```

### 9. Rebase-merge

```bash
git push -u origin <branch>
gh pr create --base main --head <branch> --title "..." --body "..."
gh pr merge <N> --rebase --delete-branch
git fetch origin && git checkout main && git reset --hard origin/main
```

Harness blocks direct `git push origin main` — this PR loop is the workaround.

## Hazards

- **Don't reformat the corpus mid-phase.** `duelscript fmt cards/official/` reformats ~2,700 files. Scope your PR to only the cards your phase touches.
- **Don't ship retrofits silently.** If your phase fixes a regression in earlier-shipped corpus content (Phase 4b retrofitted Phase 4's missing `until end_of_turn`), call it out in the PR description and bound the change to cards whose lua reset matches the new detection.
- **Don't auto-remove existing data.** If an empty `resolve { }` is "redundant" because your new passive captures it, write a separate audit-then-apply pass with safety filters (no other content in the effect, single-effect-card, etc.). See Phase 5b for the template.

## Where this skill applies vs related skills

- **Audit step** → see `corpus-audit-pattern` skill for the python regex template.
- **Lua idioms** → see `lua-corpus-context` skill for `Effect.CreateEffect` / `RESETS_STANDARD` / `aux.Next` decoder.
- **DSL grammar** → see `dsl-grammar-context` skill before adding emit forms; new DSL syntax requires a `grammar-extender` agent first.
- **Picking the next phase** → see `error-triage` skill.
