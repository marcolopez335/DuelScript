# DuelScript Backlog — Stories

Active backlog of work items, ordered by yield. Each story states the goal, the agent, the acceptance criteria, and the dependency chain.

State as of 2026-05-03 — `cards/official/` parses 13,298 / 13,298, **3,274 errors** / **559 warnings** remain (post-Phase 8 / Phase 9 audit-failed-stopped).

---

## Translator phases (no new grammar)

These reuse existing DSL syntax. Each is a single `lua-translator` PR.

### ~~Phase 4c — non-literal SetValue~~ ✓ shipped (PR #61)
**Shipped.** -13 errors, -6 warnings. Direct method calls (\`tc:GetAttack()\` etc.), one-step math, unary minus, and local-var resolution covered. Function-refs and inline closures deferred to Phase 4d.

### ~~Phase 4d — function-ref + closure SetValue~~ ✗ dropped (2026-04-30 audit)
**Audit finding.** Re-audit at Phase 4d kickoff (2026-04-30) revealed the original 77-card estimate counted predicate filter function-refs (target/disable/replacement filters returning booleans like `c == e:GetLabelObject()`) as if they were numeric stat-value functions. Filtering to actual `EFFECT_UPDATE_ATTACK`/`EFFECT_UPDATE_DEFENSE` chains:

- Handler-body path (where `parse_lua_value` runs): **0 cards** with empty resolve and translatable function-ref / closure SetValue.
- Passive path (`s.initial_effect` → `passive_modifier_spec`, different code path): ~3 cards (`c:GetBaseAttack()*2`, `c:GetLevel()*100` shapes) — below floor.
- Passive path with stat extensions: ~10 cards using `c:GetCounter(...)*N` / `c:GetOverlayCount()*N` — needs DSL `self.counter` / `self.overlay_count` grammar additions first.
- Passive path with `Duel.GetMatchingGroupCount(...)*N`: ~7 cards — needs DSL `count(<selector>)` integration in passive emit.

**Decision.** Drop Phase 4d. The ~20 cards in the passive path can be revisited as a future "Phase 5e — non-literal passive modifier value" once stat-extension grammar (overlay_count / counter / count(selector)) is added. Tracked below in the grammar T-series.

---

### ~~Phase 5c — non-stat passive codes (grants)~~ ✗ shipped-by-history (2026-04-30 audit)
**Audit finding.** Re-audit at Phase 5c kickoff revealed legacy v1-era sprints (Sprint 41 "grant-style continuous codes" + Sprint 68b "grant conversion") already populated the corpus with grant blocks before the v2 rewrite:

| ability | existing `grant:` lines in corpus |
|---|---|
| `cannot_be_destroyed by battle` | 619 |
| `cannot_be_destroyed by effect` | 446 |
| `cannot_attack` | 642 |
| `cannot_be_targeted` | 544 |
| **total** | **2,251** |

Lua-side: 951 candidate chains across 882 cards in `s.initial_effect`. After `is_purely_passive` gate + safe FIELD-shape whitelist: 285 chains / 269 cards. Per-card deficit check (lua-chain count > existing-grant count for same ability): **0 cards**. Yield = 0.

**Decision.** Mark shipped-by-history. STORIES estimate did not account for legacy translation passes; the only viable bucket is the in-resolve register-chain population (Phase 4e below).

---

### ~~Phase 4e — in-resolve grant chains~~ ✓ shipped (PR #63)
**Shipped.** -50 errors, -18 warnings, 61 grant lines added across 50 cards. Four ability codes covered (EFFECT_INDESTRUCTABLE_BATTLE / EFFECT_INDESTRUCTABLE_EFFECT / EFFECT_CANNOT_ATTACK / EFFECT_CANNOT_BE_EFFECT_TARGET). `translate_register_chain` split into stat-modifier vs grant paths sharing a `resolve_chain_selector` helper; reset gate mandatory for grants.

---

### ~~Phase 6 — `s.condition` body extraction~~ ✓ shipped (PR #66)
**Shipped.** −6 errors, −2 warnings, 375 condition lines added across 360 cards. Thirteen Lua predicate shapes mapped to 9 DSL grammar atoms (`phase ==`, `in_gy`, `on_field`, `in_hand`, `in_banished`, `previous_location ==`, `reason ==`, `reason includes`, `lp/opponent_lp <op> N`). Compound `A and B` / `A or B` conditions supported. Pass C added to `lua_translate apply`. 12 unit tests added. Shapes without grammar atoms (`IsTurnPlayer`, `IsExistingMatchingCard`, `ep~=tp`, …) deferred.

---

### ~~Phase 7 — `s.cost` body extraction~~ ✓ shipped (PR #68)
**Shipped.** −1 error, 0 warnings, 182 cost blocks added across 182 cards. Five cost shapes covered: `pay_lp N` (via `Duel.PayLPCost` + inline `Cost.PayLP(N)`), `discard (N, card, you control, from hand)` (via `Duel.DiscardHand` with generic filters), `tribute self` (`Duel.Release(c, …)`), `banish self` (`Duel.Remove(c, …)`), `send self to gy` (`Duel.SendtoGrave(c, …)`). Pass D added to `lua_translate apply`. 12 unit tests added. Effects with empty resolve blocks skipped to avoid checker warnings. Phase 7b candidate: fill cost-on-empty-resolve cards once their resolve bodies are translated.

---

### ~~Phase 8 — `s.target` body extraction~~ ✓ shipped (PR #70)
**Shipped.** 0 errors delta, 0 warnings delta, 301 target declarations added across 301 cards. Two generic-filter shapes covered: `nil` filter (161 cards) and `aux.TRUE` filter (140 cards). `extract_target_decl` added to `lua_ast.rs`, reusing `spec_from_matching` / `SelectorSpec` from Phase 3a/b. Pass E added to `lua_translate apply`. 7 unit tests added (253 total lua_ast,cdb). `LOCATION_PZONE` → `pendulum_zone` and `LOCATION_MMZONE` → `extra_monster_zone` added to `zone_from_locations`; `LOCATION_FZONE` / `field_zone` omitted (PEG grammar ordered-choice conflict — "field" matches before "field_zone"). Custom named filters (~3,254 cards), variable quantities (28), and empty-resolve cards (33) deferred to Phase 8b.

---

### Phase 9 — `s.operation` body extraction ✗ audit-failed-stopped (2026-05-03, issue #75)
**Audit finding.** 1,042 files with empty `resolve { }` blocks audited against CardScripts Lua mirror. No sub-shape clears the ≥30 safe-card floor.

**Root causes:**

1. **Secondary handler pattern (~95%+ of empties).** The primary operation handler (`s.activate`) builds a delayed/conditional field effect and registers it via `Duel.RegisterEffect(e, tp)`. The actual action (`Destroy`, `SpecialSummon`) lives in a secondary handler (`s.desop`, `s.spop`) that is never linked to `walk.effects`. The DSL has no syntax for a delayed-trigger wrapper. Grammar blocked.

2. **If-condition action pattern (small subset).** ~13 cards execute their action as the boolean test of an `if` statement (`if Duel.SSet(tp,tc)>0 then ...`). `collect_duel_calls` descends into if-bodies but not if-condition expressions. Safe yield after slot-alignment: 13 cards — below floor.

3. **Pass A slot-tracking bug.** `first_empty_resolve()` does a global file scan with no effect-index awareness. Passes C/D/E use `condition/cost/target_inject_pos(txt, effect_idx)`; Pass A does not. This creates mis-injection risk for cards where earlier effects are already filled. Fix is a correctness chore (no yield by itself).

**Shapes evaluated:**

| Sub-shape | Safe count | Blocker |
|---|---|---|
| Secondary handler (field effects) | 0 | No DSL wrapper grammar |
| If-condition action | 13 | Below floor + slot-tracking bug |
| `Duel.Overlay` (attach) | 0 | Needs T33 grammar |

**Side effects.** Apply run produced 3 residual Pass E target injections (`c76524506.ds`, `c7241272.ds`, `c78783557.ds`) — no error/warning delta. 253 tests passing, 0 regressions.

**Recommendations.** (1) Fix Pass A slot-tracking (correctness, no yield). (2) Phase 7b: backfill cost on ~15 deferred empty-resolve cards. (3) Phase 8b: backfill target on ~30 remaining deferred cards. (4) Ship T33 grammar (`attach`) to unlock ~47 `Duel.Overlay` cards. (5) Once T33 + slot-tracking fix land, revisit if-condition extraction as a micro-phase (13 cards, below-floor acknowledged).

---

## Translator chores (no translator extension)

Each is a single `corpus-curator` PR.

### ~~Phase 5d — drop redundant Field-type stubs (broaden 5b)~~ ✓ shipped (PR #73)
**Shipped.** −195 errors, −26 warnings, 195 cards cleaned up (1,209 lines removed). Phase 5b's hand-edit (Equip Spell only, commit `9a1edeffc`) generalised into reusable `translate_corpus` cluster `drop_empty_field_type_stubs`. Filter cascade (AST-verified via `parse_v2`): card type ∈ {Effect Monster, Continuous Spell, Continuous Trap}; ≥1 `passive` block; exactly one `effect` block; that effect has empty `resolve` AND no `choose`. Cluster excises whole effect block with whitespace-aware rewrite (single blank between siblings, no orphan blank before card-closing `}`). 11 unit tests cover positive/negative cases for all three card types + Equip Spell exclusion. Initial yield estimate (~80) was low — actual hit was 195 because the proportion of cards with `cannot_attack`/`cannot_be_targeted`/`cannot_be_destroyed` passives plus a single empty-effect leftover was higher than estimated.

---

### Phase 4d — backfill missing `until end_of_turn` (broaden 4b retrofit)
**Goal.** Phase 4b retrofitted 38 cards / 49 lines. The script skipped 3 cards with mixed-reset Lua (DAMAGE_CAL, REDIRECT). Investigate each individually and either ship a hand-edit or document why the missing `until` is correct.

**Yield estimate.** 3 cards (manual review).

**Agent.** `corpus-curator` for the apply, with manual inspection.

---

## Grammar extensions (T-series)

These cross the parse-to-runtime seam. Each is a `grammar-extender` PR.

### T33 — `attach <selector>` action
**Goal.** New DSL action mapping to `Duel.Overlay(...)` — attach a card as an Xyz Material.

**Pre-spec letter.** `HHH-II` (next free in decisions-2.md).

**Yield estimate.** ~47 cards in the empty-resolve bucket reference `Duel.Overlay`.

**Approach.**
1. Add `attach_action = { "attach" ~ selector ~ "to" ~ selector }` to `grammar/duelscript.pest`.
2. Add `Action::Attach { what: Selector, to: Selector }` to `src/v2/ast.rs`.
3. Wire parser, compiler, fmt, MockRuntime stub.
4. Add `DuelScriptRuntime::attach_overlay` trait method.

**Acceptance.**
- Parse, compile, MockRuntime, roundtrip, corpus checks pass.
- New trait method documented for `engine-dev` to mirror in ygobeetle.

**Agent.** `grammar-extender`. Then route trait impl to `engine-dev`.

---

### T34 — `self.overlay_count` / `self.counter(name)` stats in passive expr
**Goal.** Extend DSL `expr` grammar with stat-refs for overlay materials and named counters so the dropped Phase 4d's passive-path cards (~17) become translatable. Currently `passive_modifier_spec` bails on non-literal SetValue.

**Yield (post-grammar).** ~10 overlay/counter cards + ~7 count(selector) cards = ~17 cards if combined with a follow-up translator extension to `passive_modifier_spec`.

**Approach.**
1. Add `self.overlay_count` and `self.counter(<name>)` to grammar `expr` rule.
2. Wire AST + compiler (mock + DuelScriptRuntime trait method).
3. Follow-up translator pass: extend `passive_modifier_spec` to handle non-literal SetValue with `c:GetOverlayCount()*N` / `c:GetCounter(0xN)*N` shapes.

**Agent.** `grammar-extender` first; then `lua-translator` for the passive-side translator extension.

---

### T35 — `choose { ... }` block in resolve
**Goal.** Translate `Duel.SelectOption(tp, ...)` UI choices into a `choose { ... }` block (already exists in grammar but is rarely used).

**Pre-spec letter.** TBD.

**Yield estimate.** ~44 cards.

**Approach.** Translator-side change rather than grammar extension if `choose { }` already exists. Confirm by reading `parse_choose_block`. If grammar work is needed, this becomes a grammar-extender story.

**Agent.** `lua-translator` if grammar exists; otherwise `grammar-extender` first.

---

### T36 — `restrict` player-scoped restriction action (specced + shipped in same PR)
**Goal.** New resolve action for lua resolve-time `EFFECT_TYPE_FIELD` effects that restrict a PLAYER rather than cards — `Effect.CreateEffect` + `EFFECT_FLAG_PLAYER_TARGET` + `SetTargetRange(p1,p2)` player flags + `Duel.RegisterEffect(e1,tp)`. The existing `grant <selector> <ability>` form is card-scoped and cannot express these.

**Pre-spec letter.** n/a (ledger retired).

**Yield estimate.** ~123 chains (Phase 15 audit). Corpus-wide lua survey backing the keyword set: `EFFECT_CANNOT_SPECIAL_SUMMON` 469 (418 of them `SetTargetRange(1,0)`, 466 unfiltered), `EFFECT_CANNOT_ACTIVATE` 119, `EFFECT_CANNOT_SUMMON` 26, `EFFECT_SKIP_BP` 10, `EFFECT_CANNOT_BP` 9, `EFFECT_CANNOT_MSET` 4, `EFFECT_CANNOT_SSET` 4.

**Syntax.**
```
restrict_action = { "restrict" ~ player_scope ~ player_restriction ~ duration? }
player_scope = { "both_players" | "you" | "opponent" }   // (1,1) / (1,0) / (0,1)
```
`player_restriction` is a closed keyword set (no free strings), 11 keywords: `cannot_special_summon`, `cannot_normal_summon`, `cannot_set_monsters`, `cannot_set_spells_traps`, `cannot_activate_spells_traps`, `cannot_activate_monster_effects`, `cannot_activate_spells`, `cannot_activate_traps`, `cannot_activate`, `cannot_conduct_battle_phase`, `skip_battle_phase`. Pest prefix rule: the `cannot_activate_*` variants are ordered before `cannot_activate`.

**Filtered-activation decision (option a, partial).** Most `EFFECT_CANNOT_ACTIVATE` chains carry a `SetValue` filter. The four high-count filter shapes got keywords: `re:IsHasType(EFFECT_TYPE_ACTIVATE)` (27) + `re:IsSpellTrapEffect()` (7) → `cannot_activate_spells_traps`; `re:IsMonsterEffect()` (15) → `cannot_activate_monster_effects`; trap-card activations (6) → `cannot_activate_traps`; spell-card activations (5) → `cannot_activate_spells`; `SetValue(1)` (15) → bare `cannot_activate`. Exotic filters (attribute/location/code-specific, ~30 chains) stay out — the follow-up translator skips them.

**Runtime seam (ygobeetle mirror obligation).** New `DuelScriptRuntime` trait method:
```rust
fn restrict_player(&mut self, player: u8, restriction: PlayerRestriction, duration: Duration) {}
```
`PlayerRestriction` is a runtime-surface mirror enum (same pattern as `Duration`/`TokenSpec`) with a doc-comment table mapping each variant to its `EFFECT_*` code + activation filter. The compiler resolves the relative scope to absolute player indices (`both_players` → two calls), matching the `take_control`/`player_who_to_idx` house style. `engine-dev` must mirror this on `YgobeetleRuntimeAdapter`.

**Acceptance (all shipped in this PR).**
- Grammar + AST (`Action::Restrict`, `PlayerScope`, `PlayerRestriction`) + parser + validator + compiler + fmt + runtime trait + MockRuntime, tests at each layer.
- Parse, validate, compile→mock, fmt-roundtrip tests green; zero warnings; corpus counts unchanged.
- Follow-up translator phase (emit `restrict` lines into `cards/official/`) ships separately.

**Agent.** `grammar-extender` (this PR). Then `lua-translator` for the corpus apply pass; `engine-dev` for the adapter mirror.

---

### T37 — `damage_rule` player-scoped damage-shaping action (specced + shipped in same PR)
**Goal.** New resolve action for lua resolve-time `EFFECT_TYPE_FIELD` + `EFFECT_FLAG_PLAYER_TARGET` effects that shape the damage a PLAYER takes — "you take no (battle/effect) damage", "effect damage is halved", "battle damage is doubled", "damage becomes LP gain", "damage is inflicted to the opponent instead". T36's sibling: `restrict` forbids a player ACTION; `damage_rule` shapes incoming DAMAGE.

**Pre-spec letter.** n/a (ledger retired).

**Yield estimate.** ~53 chains (Phase 15 audit; re-survey against today's failing set found 80 chain-sites). Failing-card FIELD chain-sites by code: `EFFECT_CHANGE_DAMAGE` 31, `EFFECT_AVOID_BATTLE_DAMAGE` 26, `EFFECT_CHANGE_BATTLE_DAMAGE` 11, `EFFECT_REFLECT_DAMAGE` 6, `EFFECT_REVERSE_DAMAGE` 4, `EFFECT_REFLECT_BATTLE_DAMAGE` 2.

**Syntax.**
```
damage_rule_action = { "damage_rule" ~ player_scope ~ damage_rule ~ duration? }
```
Reuses T36's `player_scope` (`you` (1,0) / `opponent` (0,1) / `both_players` (1,1)). `damage_rule` is a closed keyword set (no free values), 10 keywords: `no_damage`, `no_effect_damage`, `halve_effect_damage`, `no_battle_damage`, `halve_battle_damage`, `double_battle_damage`, `reverse_damage`, `reverse_effect_damage`, `reflect_effect_damage`, `reflect_battle_damage`. Pest prefix rule: `damage_rule_action` is ordered before `damage_action` in the `action` alternatives (shared `damage` prefix); within the keyword set the `no_*`/`reverse_*` families are ordered longest-first defensively. Naming: a new action keyword rather than `restrict` reuse — `double_battle_damage` and `reverse_damage` are not restrictions, and the trait method registers different effect codes.

**SetValue decision (corpus survey, `EFFECT_CHANGE_DAMAGE` n=88).** Plain `SetValue(0)` (25) = "takes no damage" at all (scripts commonly pair a clone with `EFFECT_NO_EFFECT_DAMAGE`) → `no_damage`, distinct from the clean `r&REASON_EFFECT→0` guard (13) → `no_effect_damage`. Halving guards (16) → `halve_effect_damage`. Excluded as translator-skip classes: chain-id-specific "that damage becomes 0/doubles" shapes (24, `CHAININFO_CHAIN_ID` label matching), effect-damage doubling (6, all chain-bound), arbitrary multipliers/fixed replacement values (`400`, `GetLP()/2`), card-scoped `SetTargetRange(LOCATION_MZONE,…)` forms of the battle codes, and `aux.ChangeBattleDamage` continuous singles (passive-side, not resolve chains).

**Runtime seam (ygobeetle mirror obligation).** New `DuelScriptRuntime` trait method:
```rust
fn set_damage_rule(&mut self, player: u8, rule: DamageRule, duration: Duration) {}
```
`DamageRule` is a runtime-surface mirror enum (same pattern as `PlayerRestriction` T36) with a doc-comment table mapping each variant to its `EFFECT_*` code + `SetValue` shape. The compiler resolves the relative scope to absolute player indices (`both_players` → two calls), matching the `restrict_player`/`take_control` house style. `engine-dev` must mirror this on `YgobeetleRuntimeAdapter`.

**Acceptance (all shipped in this PR).**
- Grammar + AST (`Action::DamageRule`, `DamageRule`; `PlayerScope` reused) + parser + validator + compiler + fmt + runtime trait + MockRuntime, tests at each layer.
- Parse, validate, compile→mock, fmt-roundtrip tests green; zero warnings; corpus check output byte-identical (1922 errors / 1614 warnings).
- Follow-up translator phase (emit `damage_rule` lines into `cards/official/`) ships separately.

**Agent.** `grammar-extender` (this PR). Then `lua-translator` for the corpus apply pass; `engine-dev` for the adapter mirror.

---

## Per-archetype templates

Long-tail. Each archetype = one PR. Defer until grammar-free phases are exhausted.

### Salamangreat
~50 cards. Common shape: link summon + effect chain.

### Madolche
~40 cards. Common shape: shuffle from gy + monster effects.

### Trickstar
~35 cards. Common shape: damage to opponent on add-to-hand.

### Drytron
~30 cards. Common shape: ritual procedure variants.

**Agent.** `lua-translator` (per-archetype emit table).

---

## Project hygiene

These do not reduce errors but unblock public contribution.

### License split
**Goal.** Resolve the AGPL (corpus, derived from Project Ignis) vs MIT (Cargo.toml) conflict.

**Options.**
1. Split `cards/official/` into a sibling AGPL repo, add as a git submodule.
2. Relicense duelscript to AGPL-3.
3. Strip the corpus, keep grammar + CLI + LSP MIT.

**Acceptance.** A LICENSE file exists at the root of every published repo and matches its declared crate metadata.

**Agent.** Human decision; integrator implements once decided.

---

### CI on PRs
**Goal.** GitHub Actions workflow that runs:
- `cargo test --lib`
- `cargo test --lib --features lua_ast,cdb`
- `cargo test --lib roundtrip`
- `cargo build` zero-warnings check
- `duelscript check cards/official/` fails the PR if error count rises

**Acceptance.** Green badge on README. PRs that regress error count auto-block.

**Agent.** `integrator`.

---

### CONTRIBUTING.md
**Goal.** Walkthrough of the translator-phase-pattern + corpus-curator-pattern + T-series workflow so external contributors can pick up a story without admin handholding.

**Acceptance.** New contributor can follow the doc to ship a Phase Nx PR end-to-end.

**Agent.** Human-authored, reviewer-checked.

---

### Good-first-issue labels
**Goal.** Bucket the remaining 3,539 empty-resolve errors by lua shape and label representative samples on GitHub as `good first issue`. Each label maps to one of the open phases above.

**Acceptance.** ≥ 20 labelled issues, each with a one-line shape description and a link to the relevant phase doc.

**Agent.** `corpus-curator` (audit) + human (issue creation).

---

## Cross-repo

### ygobeetle T39 — fusion EVENT_BE_MATERIAL bracket
**Goal.** Track `~/git/.claude/state/plan.md` last-touched 2026-04-23 — the fusion EVENT_BE_MATERIAL bracket needs implementation.

**Agent.** `engine-dev` for the impl, `integrator` for the duelscript-side trait stub if signature changed.

---

### ygobeetle — `get_card_archetypes` lookup table (setcode → name)
**Goal.** Phase 5c may surface a need for setcode-to-archetype lookups; ygobeetle should expose them via a trait method.

**Agent.** `engine-dev`.

---

### ygobeetle — instance-id disambiguation in `remove_card`
**Goal.** Documented in prior session — no change yet.

**Agent.** `engine-dev`.

---

## How this doc evolves

After a story ships, mark it done with the PR number. New stories from triage land at the bottom of the relevant section. The yield estimates are stale within ~3 PRs — re-run `error-triage` before starting a new translator phase.
