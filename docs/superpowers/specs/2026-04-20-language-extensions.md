# DuelScript Language Extensions — Corpus-Driven Pre-Specs

**Date:** 2026-04-20
**Format:** `decisions-2.md`-compatible pre-specs (admin can lift each
entry verbatim into the decisions log when scheduling the phase).

---

## Motivation

A full Lua-corpus audit of `CardScripts/official/*.lua` (13,298 files,
815,838 LOC) enumerated every distinct primitive used by the engine's
card scripts. Ranked by call frequency and mapped against the v2 DSL
grammar, the audit surfaced that:

- **Core actions, selectors, stat queries, and passive/restriction/
  replacement blocks cover ~85% of the corpus by call weight.**
- **The remaining 15% is disproportionately present in competitive
  post-2014 cards** — hand traps, reason-based reactive effects, and
  "was this card previously X" history queries.
- **The three gaps tracked in `docs/EXPRESSIVENESS_GAPS.md` (A5 / A7 /
  H3) are under-sampled** — that document audited 4 cards. The
  corpus-weighted top gaps are materially different.

### Top 3 gaps by call-frequency impact

| # | Lua primitive(s) | Calls | Cards unblocked (est.) | Gap ID |
|---|---|---:|---:|---|
| 1 | `EVENT_CHAINING` + `EFFECT_FLAG_DELAY` w/ category filter | 961 | 400–700 | new |
| 2 | `IsReason(REASON_*)` as a generic predicate | 2,063 | 300–500 | B5 (partial) |
| 3 | `IsPreviousLocation` / `IsPreviousController` / `IsPreviousPosition` | 1,441 | 200–400 | B4 |

Total expected semantic-body closures across these three: **~1,000–1,600
cards** of the 5,387 currently-empty effects. This would drive the
"must have a resolve or choose block" error count down by ~18–30% in a
single focused trio of T-phases, without touching the tail-of-long-tail
cards that legitimately need LLM or `raw_effect` fallback.

---

## Ordering rationale

1. **T26 / EVENT_CHAINING extension first** — highest per-card impact
   (hand traps are omnipresent in modern play). DSL already has the
   `opponent_activates [...]` primitive at grammar line 233, so this is
   a widening of an existing mechanism rather than a greenfield
   addition. Lowest structural risk.
2. **T27 / `reason` predicate** — unblocks the most call sites (2,063)
   and is a pure additive grammar change with a small runtime shim.
   Independent of T26.
3. **T28 / `previous_*` predicates** — requires three new runtime
   trait methods on `DuelScriptRuntime` (snapshot reads). Widens the
   engine seam, so it should come after T26/T27 to avoid piling
   trait-surface changes.

All three are independent — the admin can parallelize them via the
existing backend-dev / integrator split.

---

## S-II (2026-04-20, T26 pre-spec) — EVENT_CHAINING extension: `you_activates`, `any_activates`, expanded category filter

**Rule ID:** S-II. Compact form (pattern established by M-II / N-II /
O-II). First entry under S-II; next in sequence after R-II (T25 close).

**Purpose:** Close the largest single corpus gap: 961 Lua effects use
`EVENT_CHAINING` — the "when any card/effect is being activated"
trigger that fires during chain-link formation (before resolution).
This is the trigger under every hand trap (Ash Blossom, Effect Veiler,
Infinite Impermanence, Called by the Grave, Droll & Lock Bird, Ghost
Belle, Ghost Ogre, PSY-Framegear Gamma, etc.) and under reactive
negates (Solemn Strike, Stardust Dragon on-chain, etc.).

The DSL already has `opponent_activates [category_list]` at grammar
line 233 with an 8-category filter. The gap is:

1. No `you_activates` (own-chain interrupters: *Trap Dustshoot* when
   self-activated, *Solemn Warning* self-chain-blocks).
2. No `any_activates` (chain-any interrupters: *Imperial Order*,
   *Skill Drain*, *Jinzo*-style).
3. The `category_list` is under-populated vs the corpus's needs —
   missing `pay_lp`, `discard`, `return_to_deck`, `summon_from_deck`,
   `equip`, `pendulum`, plus type-dispatch (`monster_effect` vs
   `spell_effect` vs `trap_effect`).

**Grep verification (plan-time):**

- `grep -c "EVENT_CHAINING" /Users/marco/git/CardScripts/official/*.lua`
  → **961 call sites**
- `grep -n "opponent_activates" /Users/marco/git/duelscript/grammar/duelscript.pest`
  → line **233** (the rule to widen)
- `grep -n "category = " /Users/marco/git/duelscript/grammar/duelscript.pest`
  → line **255** (the category alternation)
- Existing tests referencing the trigger: grep `cards/goat/*.ds` for
  `opponent_activates` → ash_blossom.ds uses it.

**Design (locked at spec time):**

1. **Grammar** (`grammar/duelscript.pest`):
   ```
   // Replace line 233
   | ("opponent_activates" | "you_activates" | "any_activates") ~ category_list?
   ```
2. **Grammar** — extend the `category` alternation at line 255:
   ```
   category = {
       "search" | "special_summon" | "send_to_gy" | "add_to_hand"
     | "draw" | "banish" | "destroy" | "negate" | "mill"
     | "pay_lp" | "discard" | "return_to_deck" | "equip"
     | "activate_spell" | "activate_trap" | "activate_monster_effect"
     | "monster_effect" | "spell_effect" | "trap_effect"
     | "normal_summon" | "fusion_summon" | "synchro_summon"
     | "xyz_summon" | "link_summon" | "ritual_summon"
     | "pendulum_summon"
   }
   ```
3. **AST** (`src/v2/ast.rs`): extend `Trigger::Activates` variant with
   an enum `ActivatesSubject { Opponent, You, Any }`. Extend
   `ActivationCategory` with the 6 new cases.
4. **Compiler** (`src/v2/compiler.rs`): extend the EVENT_CHAINING
   emission to write an `ep`-comparison closure per subject:
   - `Opponent` → `ep != controller`
   - `You` → `ep == controller`
   - `Any` → no ep filter
5. **Runtime trait** (`src/v2/runtime.rs`): **no new method.** Existing
   `effect_player()` and chain-info queries suffice.
6. **Validator** (`src/v2/validator.rs`): new rule — `you_activates` on
   speed 1 without `mandatory` triggers the existing timing-decl
   warning (no behavioural change, validator re-uses the rule it
   already has for `opponent_activates`).
7. **Tests** — 5 inline tests:
   - `t26_you_activates_parses_and_matches`
   - `t26_any_activates_ignores_ep`
   - `t26_new_categories_parse`
   - `t26_backcompat_opponent_activates_unchanged`
   - `t26_you_activates_backed_by_effect_player`

**Migration surface:**
- duelscript only: grammar (2 lines), ast (~10 lines), compiler
  (~30 lines), validator (minor), 5 tests.
- ygobeetle: no trait change; adapter already routes EVENT_CHAINING.
- duelfield: none.

**Expected deltas:**
- duelscript lib: **138 → 143** (+5 tests).
- `corpus_compiles_official`: 0 parse regressions (additive syntax).
- Validation warnings: **unchanged** by T26 alone — the payoff comes
  when a follow-up migration pass uses the new grammar to fill empty
  hand-trap bodies (that's a separate M-phase, not part of T26).
- Trait method count on `DuelScriptRuntime`: **unchanged**.

**Scope discipline:**
- **No** `chain_link_index` condition (that's a separate follow-up).
- **No** EVENT_CHAIN_SOLVED / EVENT_CHAIN_SOLVING (separate triggers,
  separate T-phase).
- **No** rewrite of hand-trap cards in the corpus — the grammar widens,
  the corpus is updated in a later migration pass.

**Risks & alternatives considered:**
- *Alt A (rejected):* expose `ep` as a first-class expression so
  users write `condition: chain_ep != controller`. Rejected: exposes
  engine internals that the DSL deliberately abstracts over
  (`controller`/`opponent` keywords already do this transparently).
- *Alt B (rejected):* keep `opponent_activates` and add two parallel
  rules (`you_activates`, `any_activates`). Rejected: DRY violation,
  harder to keep filter lists in sync as they grow.

**Dispatch path:** admin → backend-dev (grammar + ast + compiler +
tests). No integrator hop needed (no trait widen). Close with **T-II**.

**Letter sequencing:** **S-II** (plan, this entry) + **T-II** (close).

---

## U-II (2026-04-20, T27 pre-spec) — `reason` predicate primitive

**Rule ID:** U-II. Compact. Closes partial B5.

**Purpose:** Lua's `IsReason(REASON_BATTLE|REASON_EFFECT)` pattern is
called 2,063 times across the corpus. DSL has partial coverage
(`destroyed_by_battle`, `destroyed_by_effect`, `destroys_by_battle`)
bundled into specific triggers, but no *generic* `reason` predicate
usable:

- **In conditions**: `condition: reason includes [battle, effect]` on
  an effect already triggered by something else (e.g. "when sent to
  GY, but only if it was by battle or effect, not as a cost").
- **In where clauses**: `target (1, monster, where reason == effect)`
  — selecting cards by why they were placed in their current state.
- **Composed with existing triggers**: a generic "when sent to GY"
  trigger plus a reason filter.

The current pattern forces authors to invent a new named trigger for
every `(event, reason)` combination, which scales poorly: EVENT_REMOVE
+ reason filters alone wants "banished_by_battle", "banished_by_cost",
"banished_by_effect", etc.

**Grep verification (plan-time):**

- `grep -c "IsReason" /Users/marco/git/CardScripts/official/*.lua`
  → **2,063 call sites**
- `grep -nE "by_battle|by_effect|REASON_" /Users/marco/git/duelscript/grammar/duelscript.pest`
  → lines **208–211, 250, 616–617** — current reason support is
  welded into trigger variants.
- `grep -n "condition_decl" /Users/marco/git/duelscript/grammar/duelscript.pest`
  → the alternation to extend.

**Design (locked):**

1. **Grammar** — new condition variant:
   ```
   reason_filter = {
       "battle" | "effect" | "cost" | "material" | "release"
     | "rule"   | "discard" | "return" | "summon" | "battle_or_effect"
   }
   reason_condition = {
       "reason" ~ ("==" | "!=" | "includes") ~
       ( reason_filter | "[" ~ reason_filter ~ ("," ~ reason_filter)* ~ "]" )
   }
   ```
   Add `reason_condition` as an alternative in `condition_decl`.
2. **Grammar** — same `reason_filter` usable in `where` clauses via
   existing predicate_atom alternation (one-line add).
3. **AST** (`src/v2/ast.rs`): new `Condition::Reason { op, filters:
   Vec<ReasonFilter> }` variant.
4. **Compiler** (`src/v2/compiler.rs`): emit to `rt.current_reason()
   & BITMASK != 0` / `== BITMASK` per op. Bitmask constants from
   EDOPro `REASON_*` values.
5. **Runtime trait** (`src/v2/runtime.rs`): **new method**
   `fn current_reason(&self) -> u32 { 0 }` — default returns 0 (no
   reason known). MockRuntime extends for tests. Adapter exposes the
   current chain reason already computed by the engine.
6. **Validator**: reason predicate on an effect with no trigger is a
   warning (reason is meaningful only inside an event context).
7. **Tests** — 4 inline tests:
   - `t27_reason_equality_matches_bitmask`
   - `t27_reason_includes_is_any`
   - `t27_reason_in_where_clause`
   - `t27_reason_without_trigger_warns`

**Migration surface:**
- duelscript: grammar, ast, compiler, validator, mock_runtime, tests.
- ygobeetle: **1 new adapter method** exposing the current reason
  (already tracked internally — surface it).
- duelfield: none.

**Expected deltas:**
- duelscript lib: **138 → 142** (+4 tests, net T26 if already landed:
  143 → 147).
- Trait method count: **98 → 99** (1 new method, FF-I crossed
  intentionally).
- `corpus_compiles_official`: 0 regressions.
- ygobeetle lib: **191 → 191** unless a corpus-level integration test
  is added (**+1 ds_* file** if chosen).

**Scope discipline:**
- **Top 10 most-used reason filters only** (see `reason_filter` above).
  Rarer flags (REASON_FUSION, REASON_RITUAL, REASON_ACTION, etc.) in a
  later extension.
- **No** reason-based *triggers* (those already exist as named trigger
  variants like `destroyed_by_battle`). T27 is conditions-only.
- **No** removal of existing bundled triggers (back-compat preserved).

**Risks & alternatives considered:**
- *Alt A (rejected):* expand the bundled-trigger approach
  (`sent_to_gy_by_battle`, `banished_by_cost`, …). Rejected: O(events ×
  reasons) combinatorial explosion in grammar; already flagged in
  EXPRESSIVENESS_GAPS.md B5.
- *Alt B (rejected):* expose `rt.last_reason()` as a bare expression in
  the DSL. Rejected: raw runtime access violates the DSL-declarative
  contract.

**Dispatch path:** admin → backend-dev (duelscript grammar + ast +
compiler) || integrator (ygobeetle `current_reason` adapter). Close
with **V-II**.

**Letter sequencing:** **U-II** (plan) + **V-II** (close).

---

## W-II (2026-04-20, T28 pre-spec) — `previous_*` predicates: location, controller, position

**Rule ID:** W-II. Compact. Closes B4.

**Purpose:** 1,441 Lua call sites use `IsPreviousLocation` /
`IsPreviousController` / `IsPreviousPosition`. Canonical patterns:

- "If this card was on the field before being sent to GY" — needed by
  any card that distinguishes field-destruction from hand/deck moves
  (Trap Hole, Bottomless, Mirror Force follow-ups).
- "If this card's controller changed this chain" — needed by *Creature
  Swap* / *Snatch Steal* cascade effects.
- "If this card was face-up before being flipped" — needed by flip
  effects that distinguish flip-summon from set-then-activate.

DSL has `trigger: flipped` and similar, but no way to ask "what state
was this card in *before* the event I'm reacting to?" from inside a
resolve/condition.

**Grep verification (plan-time):**

- `grep -c "IsPreviousLocation" CardScripts/official/*.lua` → **1,441**
- `grep -c "IsPreviousController" CardScripts/official/*.lua` →
  substantial subset.
- `grep -c "IsPreviousPosition" CardScripts/official/*.lua` → subset.
- DSL AST `predicate_atom` in `src/v2/ast.rs` → the enum to extend.
- DSL grammar predicate lines in `grammar/duelscript.pest` → confirm
  at plan time.

**Design (locked):**

1. **Grammar** — extend predicate atoms:
   ```
   previous_predicate = {
       "previous_location"   ~ compare_op ~ zone_value
     | "previous_controller" ~ compare_op ~ ("you" | "opponent" | "controller")
     | "previous_position"   ~ compare_op ~ position_value
   }
   ```
   Add as an alternative in the where-clause predicate_atom alternation
   and in `condition_decl`.
2. **AST** (`src/v2/ast.rs`): three new `PredicateAtom` variants —
   `PreviousLocation`, `PreviousController`, `PreviousPosition`.
3. **Compiler** (`src/v2/compiler.rs`): emit to runtime-trait calls.
4. **Runtime trait** (`src/v2/runtime.rs`): **three new methods**
   (Required, no default — must be implemented by real engines):
   ```
   fn previous_location(&self, card_id: u32) -> u32;
   fn previous_controller(&self, card_id: u32) -> u8;
   fn previous_position(&self, card_id: u32) -> u32;
   ```
   Defaults return `0` (unknown / current) so engines that haven't
   wired the snapshot history don't crash.
5. **MockRuntime** (`src/v2/mock_runtime.rs`): add `prev_snapshots:
   HashMap<u32, CardSnapshot>` plus setter helpers for tests.
6. **Validator**: `previous_*` in a resolve/condition where the card
   being queried is `self` is the canonical use. Other selectors are
   allowed but warned (accuracy of engine snapshot unknown).
7. **Tests** — 6 inline tests (2 per method: positive + negative).

**Migration surface:**
- duelscript: grammar, ast, compiler, validator, mock_runtime,
  `TRAIT_REFERENCE.md` updated, 6 tests.
- ygobeetle: **3 new trait methods implemented in adapter** — the
  engine already tracks `previous_location` on each `CardState` (or via
  a history snapshot); surface the three reads.
- duelfield: none.

**Expected deltas:**
- duelscript lib: **138 → 144** (+6 tests; +10 if after T26+T27).
- Trait method count: **98 → 101** (3 new methods, FF-I crossed
  intentionally for all three — this is the explicit seam widen).
- ygobeetle lib: **191 → 191** unless a corpus-level integration test
  is added.
- `corpus_compiles_official`: 0 regressions.

**Scope discipline:**
- **Three predicates only.** `previous_attack`, `previous_defense`,
  `previous_code`, `previous_type` deferred to a later extension.
- **Read-only** — no mechanism to *force* a previous state; the engine
  is the source of truth.
- **Comparison operators limited to** `==` and `!=`. No `<` / `>` on
  location bitmasks (makes no sense) or controllers (only two players).

**Risks & alternatives considered:**
- *Alt A (rejected):* instead of three methods, one method
  `previous_snapshot(card_id) -> CardSnapshot` that returns everything.
  Rejected: forces every adapter to maintain full historical snapshots
  even if it only supports one field; per-method reads let adapters
  opt into what they support.
- *Alt B (rejected):* express as events (`trigger: moved_from_field`).
  Rejected: doesn't address the condition/where-clause use case which is
  2/3 of the call sites.

**Dispatch path:** admin → integrator (runtime trait + ygobeetle
adapter) + backend-dev (duelscript grammar/ast/compiler). Runs in
parallel. Close with **X-II**.

**Letter sequencing:** **W-II** (plan) + **X-II** (close).

---

## After T26 + T27 + T28

Expected aggregate state:
- DSL call-weighted coverage: **~85% → ~92%** of the Lua corpus.
- `duelscript check cards/official/`: empty-effect errors drop
  from 5,387 to an estimated 3,800–4,400 (once a mini-migrator pass
  uses the new grammar to fill hand-trap / reason / history effect
  bodies — the language extensions themselves don't translate cards,
  they *enable* translation).
- `DuelScriptRuntime` method count: **98 → 101**.
- Trait-seam FF-I crossed: intentional for T27 + T28; T26 is
  additive-only (grammar widen, no trait change).

The T29 / T30 / T31 pre-specs below continue the language-extension
sequence, each scoped narrowly enough to land in a single phase.

---

## Y-II (2026-04-20, T29 pre-spec) — chain-end triggers: `chain_solved`, `chain_solving`

**Rule ID:** Y-II. Compact.

**Purpose:** 296 Lua call sites use `EVENT_CHAIN_SOLVED` (155) or
`EVENT_CHAIN_SOLVING` (141). These are the triggers that fire "when
a chain ends" and "while a chain is resolving" respectively —
essential for delayed effects like *Battle Fader* (return-to-hand
after damage step), *Infinite Impermanence* (cleanup on chain end),
*Fire Formation - Tenki* (stat re-check after chain resolves), and
*Artifact* cards (return-to-hand cleanup).

DSL currently has `trigger: end_phase` and event-specific triggers
but no general "chain ended" or "chain is mid-resolve" primitive.

**Grep verification (plan-time):**

- `grep -c "EVENT_CHAIN_SOLVED" CardScripts/official/*.lua` → **155**
- `grep -c "EVENT_CHAIN_SOLVING" CardScripts/official/*.lua` → **141**
- Current trigger alternation at grammar/duelscript.pest lines
  230–234 (Chain triggers section) — add two new tokens.
- No existing DSL equivalent — both triggers are net-new.

**Design (locked):**

1. **Grammar** (`grammar/duelscript.pest`, in the `trigger` rule):
   ```
   // Chain triggers
   | "chain_solved"
   | "chain_solving"
   ```
2. **AST** (`src/v2/ast.rs`): two new `Trigger` variants:
   `ChainSolved`, `ChainSolving`.
3. **Parser** (`src/v2/parser.rs`): two new match arms in the
   bare-trigger keyword matcher.
4. **Compiler** (`src/v2/compiler.rs`):
   - `Trigger::ChainSolved`  → `tm::EVENT_CHAIN_SOLVED`
   - `Trigger::ChainSolving` → `tm::EVENT_CHAIN_SOLVING`
   - Confirm both constants exist in `src/v2/constants.rs`; add if
     missing (they're standard EDOPro event codes 1121 / 1122).
5. **Runtime trait**: **no new method.** These are pure trigger
   codes; the existing `event_categories` / `event_player` / etc.
   already cover the data the condition closure needs.
6. **fmt** (`src/v2/fmt.rs`): two new match arms.
7. **Validator**: no new rules — same speed/timing rules as other
   chain triggers.
8. **Tests** — 3 inline tests:
   - `t29_chain_solved_parses_and_compiles_to_correct_event`
   - `t29_chain_solving_parses_and_compiles_to_correct_event`
   - `t29_fmt_roundtrip_both`

**Migration surface:**
- duelscript only. No trait widen. No adapter changes.
- ygobeetle: the engine already fires these events via the chain
  infrastructure; the adapter simply routes them (no code changes
  needed if the engine already wires EVENT_CHAIN_SOLVED/SOLVING
  through the same channel as other events — verify at close).

**Expected deltas:**
- duelscript lib: **+3 tests**.
- Trait method count: **unchanged**.
- FF-I: **not crossed** (additive grammar only).
- Corpus: unchanged by T29 alone; payoff comes when a follow-up
  migration pass uses the new triggers. Estimated ~100–200 cards
  could get semantic bodies once a pattern-based pass combines
  these with T26's chain-activation primitives.

**Scope discipline:**
- **No** chain-link-count condition (that's a separate extension).
- **No** new event_player routing — reuse what T26 added.
- **No** adapter method additions.

**Risks & alternatives considered:**
- *Alt A (rejected):* fold both into a single `chain_end` trigger
  with an optional qualifier. Rejected: `EVENT_CHAIN_SOLVED` and
  `EVENT_CHAIN_SOLVING` are semantically distinct (solved = after
  resolution, solving = mid-resolution), and the Lua corpus uses
  both separately. Keeping them separate mirrors the engine.
- *Alt B (rejected):* a single `chain_ended` with a phase discriminator.
  Rejected for the same semantic-distinctness reason.

**Dispatch path:** admin → backend-dev (duelscript grammar + ast +
compiler + tests). No integrator hop. Close with **Z-II**.

**Letter sequencing:** **Y-II** (plan) + **Z-II** (close).

---

## AA-II (2026-04-20, T30 pre-spec) — `used_as_material` trigger refinement

**Rule ID:** AA-II. Compact. Starts the double-letter cycle of the
`-II` series (Z-II is T29's close).

**Purpose:** 240 Lua call sites use `EVENT_BE_MATERIAL` — the
"when this card is used as Xyz/Synchro/Link/Fusion/Ritual material"
trigger. Covers *Instant Fusion* fusion-material-side-effects,
*Tuner Warrior* synchro-material cascades, *Xyz Material* GY
cascades (Evilswarm Nightmare, Number 101: Silent Honor ARK), and
most "if this card was used as material" archetypal mechanics.

DSL has `trigger: used_as_material ("for" summon_method)?` at
grammar line 242 — **partial coverage.** The gap:

1. The existing trigger fires regardless of how the card was used
   (no distinction between "used as Xyz material" vs "used as
   Synchro material" even though Lua distinguishes them).
2. No data passthrough on *which* summoned card consumed this one
   (needed for cascades like "if I was material for X, add X to
   hand").
3. No filter by *material role* (Xyz-attached vs tributed vs fused).

**Grep verification (plan-time):**

- `grep -c "EVENT_BE_MATERIAL" CardScripts/official/*.lua` → **240**
- `grep -n "used_as_material" grammar/duelscript.pest` → line **242**
- Existing AST variant: `Trigger::UsedAsMaterial(Option<SummonMethod>)`.

**Design (locked):**

1. **Grammar** — widen the existing `used_as_material` trigger to
   optionally accept a `role` qualifier and a `by` selector for the
   summoned-card binding:
   ```
   | "used_as_material" ~ ("as" ~ material_role)?
                       ~ ("for" ~ summon_method)?
                       ~ ("by" ~ "as" ~ ident)?
   material_role = { "xyz_attached" | "tributed" | "fused"
                   | "synchro" | "link" | "ritual" }
   ```
2. **AST** — upgrade variant to struct form:
   ```rust
   Trigger::UsedAsMaterial {
       role: Option<MaterialRole>,
       method: Option<SummonMethod>,
       summoned_by_binding: Option<String>,  // e.g. "as summoner"
   }
   ```
3. **Parser**: widen existing `starts_with("used_as_material")`
   branch to parse all three optional clauses.
4. **Compiler**:
   - All variants map to `tm::EVENT_BE_MATERIAL` (existing).
   - Role filter in the condition closure via a new runtime method
     (see #5).
   - Binding write: when the trigger fires, the compiler emits a
     binding write `rt.set_binding("<name>", material_summoner_id)`
     so `resolve { ... target }` can reference the summoning card.
5. **Runtime trait**: **two new methods** (FF-I crossed
   intentionally):
   - `fn material_role(&self) -> u32 { 0 }` — returns a bitmask for
     the role (Xyz-attached / tributed / fused / …).
   - `fn material_summoner_id(&self) -> u32 { 0 }` — the card that
     was summoned using this material.
   Both default to 0 so engines that haven't wired material metadata
   don't crash.
6. **MockRuntime**: two new fields + builder methods.
7. **fmt**: updated formatter.
8. **Tests** — 4 inline tests covering role filter, method filter,
   binding passthrough, and format round-trip.

**Migration surface:**
- duelscript: grammar, ast, parser, compiler, runtime trait (+2),
  mock_runtime, fmt, 4 tests.
- ygobeetle: **2 new adapter methods** — both straight reads from
  the engine's existing `EventMaterial` payload on `EVENT_BE_MATERIAL`.
- duelfield: none.

**Expected deltas:**
- duelscript lib: **+4 tests**.
- Trait method count: **+2** (FF-I crossed).
- Corpus: unchanged by T30 alone; enables ~100–150 cards once a
  migration pass uses the role filter.

**Scope discipline:**
- **No** per-material-type cascades for equip-swap-style patterns
  (those are a separate concern — the current binding passthrough
  only covers "who summoned me").
- **No** counter-based material tracking (e.g., "this card gets a
  counter each time it's been used as material") — that's a
  continuous-effect concern, not a trigger concern.

**Risks & alternatives considered:**
- *Alt A (rejected):* three separate triggers —
  `used_as_xyz_material`, `used_as_tribute`, `used_as_fusion_material`.
  Rejected: O(n) trigger explosion; the role discriminator is
  cleaner.
- *Alt B (rejected):* move the binding into `resolve` via an
  implicit `material_summoner` expression. Rejected: surprises
  authors who expect bindings to be explicit.

**Dispatch path:** admin → backend-dev (duelscript grammar + ast +
compiler + mock + fmt + tests) || integrator (ygobeetle adapter: 2
methods). Close with **BB-II**.

**Letter sequencing:** **AA-II** (plan) + **BB-II** (close).

---

## CC-II (2026-04-20, T31 pre-spec) — leave-field redirects (H3 gap)

**Rule ID:** CC-II. Compact. Closes EXPRESSIVENESS_GAPS.md gap
**H3** (the third and last of the audited-at-v0.8 gaps).

**Purpose:** 274 Lua call sites use `EFFECT_LEAVE_FIELD_REDIRECT`.
This is the primitive behind *Macro Cosmos*, *Dimensional Fissure*,
*D.D. Crow* targets, *Banisher of the Radiance*, *Skill Drain*-style
redirect effects, and any card that says "instead of being sent to
the Graveyard, banish it." Also covers destination redirects to
hand (bounced instead of destroyed) and to deck (shuffled instead).

DSL has `replacement { instead_of: destroyed; do { banish self } }`
as a partial model — but only for *destruction* replacement, not
general *destination redirection*, and only on the affected card
itself (not as a global floodgate from a different card).

**Grep verification (plan-time):**

- `grep -c "EFFECT_LEAVE_FIELD_REDIRECT" CardScripts/official/*.lua`
  → **274**
- Existing replacement block grammar: grep for `replacement_block`
  in `grammar/duelscript.pest` — extend to support redirects.

**Design (locked):**

1. **Grammar** — new passive-effect block:
   ```
   redirect_block = { "redirect" ~ "{"
                     ~ redirect_scope
                     ~ redirect_source
                     ~ redirect_destination
                     ~ redirect_filter?
                  ~ "}" }
   redirect_scope       = { "scope:" ~ ("self" | "field" | "opponent_field" | "both_fields") }
   redirect_source      = { "from:" ~ zone }
   redirect_destination = { "to:" ~ zone }
   redirect_filter      = { "when:" ~ selector }
   ```
2. **AST**: new `Redirect` block type on `Card`, parallel to
   `Replacement`. Fields: scope, source, destination, filter.
3. **Parser**: parse the new block under `card_item` alternation.
4. **Compiler**: emit as a passive continuous effect with
   `EFFECT_LEAVE_FIELD_REDIRECT` effect code.
5. **Runtime trait**: **one new method:**
   ```rust
   fn register_redirect(&mut self, source_card: u32,
                        from_zone: u32, to_zone: u32,
                        scope_mask: u32) { /* default no-op */ }
   ```
   Defaults to no-op so engines that don't support redirects skip
   silently.
6. **MockRuntime**: record redirects; expose via `dump_calls()` for
   test verification.
7. **fmt**: pretty-print the new block.
8. **Validator**: check (a) `from` and `to` zones are distinct, (b)
   scope matches the source (`scope: self` + `from: field` is
   nonsensical if the card isn't on the field — warn).
9. **Tests** — 5 inline tests:
   - `t31_macro_cosmos_pattern_redirect_to_banished`
   - `t31_field_scope_affects_both_players`
   - `t31_self_scope_only_affects_own_card`
   - `t31_bad_zone_combo_fails_validator`
   - `t31_fmt_roundtrip`

**Migration surface:**
- duelscript: grammar, ast, parser, compiler, validator, mock,
  fmt, 5 tests, TRAIT_REFERENCE.md.
- ygobeetle: **1 new adapter method**. The engine already has
  `ContinuousEffect::Redirect { source_zone, dest_zone, filter }`
  via its continuous-effect manager (used for Macro Cosmos
  internally — verify at plan time). Adapter wires
  `register_redirect` to that machinery.
- duelfield: none.

**Expected deltas:**
- duelscript lib: **+5 tests**.
- Trait method count: **+1** (FF-I crossed).
- Corpus: unchanged by T31 alone; enables ~50–100 cards in a
  follow-up migration pass.

**Scope discipline:**
- **One destination per redirect block** — no "either GY or banished
  depending on a filter" (that's a replacement block's job).
- **No** redirect activation triggers (triggered-only redirects
  use the existing replacement block). T31 is for *continuous*
  floodgate-style redirects.
- **No** conditional destinations beyond a simple `when:` selector.

**Risks & alternatives considered:**
- *Alt A (rejected):* model as an extension to the existing
  `replacement` block. Rejected: replacement is event-triggered
  (at the moment of an attempted move) and fires per-event;
  redirect is continuous (active while the source card is on the
  field). Conflating them loses the continuous semantics the engine
  needs.
- *Alt B (rejected):* use passive/grant system (`grant:
  redirect_to banished`). Rejected: grant is self-scoped, but
  Macro Cosmos redirects *all* cards' GY moves to banished —
  needs field scope that the grant system doesn't model.

**Dispatch path:** admin → backend-dev (duelscript grammar + ast +
parser + compiler + mock + fmt + tests) || integrator (ygobeetle
adapter: 1 method, route to existing ContinuousEffect manager).
Close with **DD-II**.

**Letter sequencing:** **CC-II** (plan) + **DD-II** (close).

---

## Aggregate projection after T26 + T27 + T28 + T29 + T30 + T31

If all six phases land:

- Lua-call-weighted coverage: **~85% → ~95%**.
- Empty-effect validation errors: **5,387 → ~3,000** (after a
  mini-migrator pass uses the new grammar to fill hand-trap /
  reason / history / chain-end / material / redirect effect
  bodies — the language extensions themselves don't translate
  cards, they *enable* translation).
- `DuelScriptRuntime` method count: **98 → 104** (+1 T27, +3 T28,
  +2 T30, +1 T31; T26 and T29 don't add methods).
- Trait-seam FF-I crossed in T27, T28, T30, T31. T26 and T29 are
  additive-only.

The ~5% long tail after these six phases is dominated by:

- `Duel.BreakEffect` (1,889 calls) — continuous-effect chain break
  patterns.
- `EFFECT_FLAG_CANNOT_DISABLE` (3,143 calls) — a metadata-only flag;
  requires a DSL effect-metadata mechanism.
- `EFFECT_COUNT_CODE_OATH` (1,103 calls) — once-per-name-oath
  semantics; currently partial via `once_per_turn: hard`.
- `aux.GlobalCheck` + `EVENT_CUSTOM` (424 calls) — B3/C1 gaps
  (shared state across card instances).

These four represent the genuine end-of-roadmap — each is a deep
design question, not an additive grammar tweak. Beyond that point,
`raw_effect` Lua fallback is the pragmatic answer for the remaining
<1% of cards with uniquely weird mechanics.

---

## Migration M-phase (follow-up to language extensions)

After T26+T27+T28 (minimum) land, a migrator pass can re-run over
the 13,298-card corpus using the richer grammar to fill empty effect
bodies. Three migration-pass strategies, any or all of which can
be used:

1. **Pattern-based** — detect common Lua skeletons (recruit-on-destroy,
   search-on-summon, destroy-on-tribute) and emit canonical DSL.
2. **CDB-bitmask decode** — deterministically extract materials for
   Xyz/Synchro/Fusion/Link/Ritual monsters and link arrows for Link
   monsters from the BabelCdb `def` field (the data is already on
   disk, just not extracted).
3. **LLM-assisted** — for the long tail of one-off cards, feed the
   Lua + card text + current .ds + language reference to an LLM
   and accept the diff.

Strategy #2 closes all 452 "Link monster must declare link_arrows"
errors and a large chunk of the 2,106 "Extra deck monster should
declare materials" warnings in a single deterministic pass without
needing any language-extension prerequisites — it's the cheapest
win and can land independently of the T-series.
