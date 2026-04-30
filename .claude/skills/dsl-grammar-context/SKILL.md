---
name: dsl-grammar-context
description: Use when adding or extending DSL syntax in duelscript — touching grammar/duelscript.pest, src/v2/parser.rs, src/v2/ast.rs, src/v2/validator.rs, or src/v2/compiler.rs. Maps the parse → AST → validate → compile seam.
---

# DSL Grammar Context

duelscript's parse pipeline is five files. Touching one almost always means touching the rest. Skill is the seam map.

## Pipeline overview

```
.ds source
   │
   ▼
grammar/duelscript.pest        — PEG grammar (the spec)
   │  (pest_derive)
   ▼
src/v2/parser.rs               — Pair<Rule> → AST nodes
   │
   ▼
src/v2/ast.rs                  — typed AST
   │
   ▼
src/v2/validator.rs            — semantic checks (must-have-resolve, etc.)
   │
   ▼
src/v2/compiler.rs             — AST → CompiledEffectV2 (closures + bitfields)
   │
   ▼
src/v2/runtime.rs              — DuelScriptRuntime trait (engine seam)
```

`src/v2/fmt.rs` is the inverse pretty-printer. `src/v2/lsp.rs` consumes the same AST for diagnostics. Round-trip tests (`test_all_goat_cards_roundtrip`, `test_m_phase_translated_cards_roundtrip`) verify the parser/fmt fixed-point.

## Top-level grammar shape

`grammar/duelscript.pest` lines to know:

- `card_block` (~line 50) — top-level `card "<name>" { … }`.
- `effect_block` (~line 200) — activated effect (resolve / choose).
- `passive_block` (~line 612) — continuous modifier (`passive "<name>" { … }`).
- `restriction_block` — grant/deny abilities.
- `replacement_block` — "instead of X, do Y".
- `redirect_block` — leave-field redirects (T31).
- `selector` (~line 344) — `(qty, kind, controller, zone, position, where)` core matcher.
- `predicate` — boolean expressions inside `where … and …`.
- `action` (~line 470) — verbs in `resolve { … }`.
- `duration` (~line 695) — `until` clauses (this_turn / end_of_turn / end_phase / …).

## Adding a new action

Touch in this order:

1. **Grammar** — add the rule to `grammar/duelscript.pest` under `action = { … | new_rule | … }`.
2. **AST** — add a new `Action::*` variant to `src/v2/ast.rs`.
3. **Parser** — add a `Rule::new_rule => Action::*` arm to `parse_action` in `src/v2/parser.rs`.
4. **Compiler** — add a match arm to the action-execution dispatch in `src/v2/compiler.rs` (search for `Action::Damage` to find the dispatch).
5. **Runtime trait** — if the action needs a new engine call, add a method to `DuelScriptRuntime` in `src/v2/runtime.rs` and a stub in `MockRuntime` (`src/v2/mock_runtime.rs`).
6. **Fmt** — add the inverse renderer to `format_action` in `src/v2/fmt.rs`.
7. **Tests** — at minimum a parse test in `parser.rs`, a compile-and-execute test against MockRuntime in `compiler.rs`, and a round-trip test in `fmt.rs`.

Adding a runtime trait method crosses the **FF-I seam** — see `~/git/.claude/skills/duelscript-context/SKILL.md` and the T-series notes in `~/git/.claude/state/decisions-2.md`. The integrator agent is responsible for matching trait additions on the ygobeetle adapter side.

## Selector and predicate cheat sheet

`selector` accepts:
- shorthand keywords: `self`, `target`, `equipped_card`, `negated_card`, `searched`, `linked_card`
- named binding: `<ident>` (referenced from a binding-aware action)
- structured: `(<quantity>, <card_filter>, <controller>?, <zone_filter>?, <position_filter>?, <where_clause>?)`

`predicate` atoms:
- stat compare: `atk > 1500`, `level == 4`
- enum compare: `attribute == DARK`, `race == Spellcaster`, `type == Effect Monster`, `archetype == "Gusto"`
- name match: `name == "Dark Magician"`
- type tags: `is_face_up`, `is_token`, `is_xyz`, `is_link`, `is_pendulum`, …

## Validator invariants

`src/v2/validator.rs` enforces card-level invariants:

- Every `effect` block must have `resolve { … }` OR `choose { … }`. ← **the dominant remaining error class** (3,539 cards).
- Material declarations match card type (Synchro needs tuner/non-tuner, Link needs link materials, etc.).
- Triggers and timings are valid for the effect's speed.
- Trait-method coverage — actions that require a trait method only validate when ygobeetle exposes it.

When adding a new action, decide whether validator should require any companion clause. (E.g. `equip <eq> to <target>` only validates when both selectors are present.)

## Compiler bitfield mapping

`compile_card_v2` produces `CompiledEffectV2` with edopro-compatible bitfields:

- `effect_type` — `EFFECT_TYPE_*` OR-mask. `compile_passive` derives from `Scope`. `compile_effect` derives from speed + trigger + activation kind.
- `category` — `CATEGORY_*` OR-mask. Derived from action verbs in resolve body.
- `code` — `EVENT_*` for triggers, `EFFECT_*` for continuous. Modifier passives use `EFFECT_UPDATE_ATTACK` / `_DEFENSE`.
- `range`, `target_range` — location bitmasks for continuous-effect dispatch.

`grant_to_code` (in `compile_passive`) maps `GrantAbility` enum to `EFFECT_*` codes — the table was audited during T29 and matches edopro's `constant.lua`.

## Tests to run when touching the seam

```bash
cargo test --lib                           # default features (195 tests)
cargo test --lib --features cdb            # with BabelCdb reader
cargo test --lib --features lua_ast,cdb    # with full feature set
cargo test --lib roundtrip                 # parser/fmt fixed-point (19 tests)
cargo run --release --features cdb --bin duelscript -- check cards/official/  # full corpus
```

Zero warnings invariant — never ship a change that emits `unused_variable` or `dead_code`. Delete the dead code instead.

## Where this skill applies vs related skills

- **Lua-side translation** → see `lua-corpus-context` and `translator-phase-pattern` skills.
- **Picking what error class to attack** → see `error-triage`.
- **Cross-repo trait coordination** → see `~/git/.claude/skills/duelscript-context/SKILL.md` and the admin agent.
