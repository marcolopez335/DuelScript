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

**Recommendations.** (1) ~~Fix Pass A slot-tracking~~ — addressed by Phase 20: Pass A now injects via block-bounded `first_empty_resolve_within(txt, block_lo, block_hi)` (`src/bin/lua_translate.rs`), bounds from the signature-based block matcher. (2) Phase 7b: backfill cost on ~15 deferred empty-resolve cards. (3) Phase 8b: backfill target on ~30 remaining deferred cards. (4) ~~Ship T33 grammar~~ — shipped, PR #78. (5) With T33 + block-bounded injection both landed, if-condition extraction is unblocked as a micro-phase (13 cards, below-floor acknowledged).

---

## Translator chores (no translator extension)

Each is a single `corpus-curator` PR.

### ~~Phase 5d — drop redundant Field-type stubs (broaden 5b)~~ ✓ shipped (PR #73)
**Shipped.** −195 errors, −26 warnings, 195 cards cleaned up (1,209 lines removed). Phase 5b's hand-edit (Equip Spell only, commit `9a1edeffc`) generalised into reusable `translate_corpus` cluster `drop_empty_field_type_stubs`. Filter cascade (AST-verified via `parse_v2`): card type ∈ {Effect Monster, Continuous Spell, Continuous Trap}; ≥1 `passive` block; exactly one `effect` block; that effect has empty `resolve` AND no `choose`. Cluster excises whole effect block with whitespace-aware rewrite (single blank between siblings, no orphan blank before card-closing `}`). 11 unit tests cover positive/negative cases for all three card types + Equip Spell exclusion. Initial yield estimate (~80) was low — actual hit was 195 because the proportion of cards with `cannot_attack`/`cannot_be_targeted`/`cannot_be_destroyed` passives plus a single empty-effect leftover was higher than estimated.

---

### ~~Phase 4d — backfill missing `until end_of_turn` (broaden 4b retrofit)~~ ✓ shipped (commit 556ab6a76)
**Shipped.** 2 of the 3 skipped mixed-reset cards backfilled after hand-verifying the lua reset is in fact end-of-turn (`RESET_EVENT|RESETS_STANDARD` without a literal PHASE_END token): c33750856 Code Hack Effect 3, c98431356 Phantom Knights' Wing Effect 1. Third candidate c68507541 Amazoness Pet Liger Effect 2 left unchanged — its reset is `RESET_PHASE|PHASE_DAMAGE_CAL` (battle-step only) and the DSL has no `until damage_calculation` duration; tracked as a future T-series duration item.

---

## Grammar extensions (T-series)

These cross the parse-to-runtime seam. Each is a `grammar-extender` PR.

### ~~T33 — `attach <selector>` action~~ ✓ shipped (PR #78)
**Shipped.** Grammar side landed in prior T-series work: `attach_action = { "attach" ~ selector ~ "to" ~ selector ~ "as_material" }`, `Action::Attach(Selector, Selector)`, compiler wiring, and `DuelScriptRuntime::attach_material(material_id, target_id)` (ygobeetle mirror obligation — engine-dev). PR #78 added the translator: `Duel.Overlay(target_xyz, materials, [send_overlay])` in the `translate_call` action map via `xyz_arg_to_dsl` (`c` → `self`, `tc` → `target`, tracked group binding → captured SelectorSpec, else TODO-skip).

**Actual yield.** 20 effects / 19 cards filled (−20 errors / −6 warnings at the time). Original ~47-card estimate included cards whose Overlay args don't resolve (both-unbound shapes stay TODO-skipped) or whose Overlay sits inside secondary handlers — those remain in the empty-resolve pool for the delayed-trigger story.

---

### ~~T34 — `self.overlay_count` / `self.counter(name)` stats in passive expr~~ ✓ shipped
**Shipped.** Grammar + translator in one branch. Two new expr atoms, closed to the `self` receiver (re-audit found zero `target.`/`equipped.` use): `overlay_count_ref = { "self" ~ "." ~ "overlay_count" }` and `counter_ref = { "self" ~ "." ~ "counter" ~ "(" ~ string ~ ")" }`, ordered before `stat_ref` in `expr_atom`. Wired AST (`Expr::OverlayCount` / `Expr::CounterCount`) → parser → validator (acceptance only, no new invariants) → compiler eval → fmt fixed-point. FF-I trait widen: `fn get_overlay_count(&self, _card_id: u32) -> u32 { 0 }` on `DuelScriptRuntime` (**ygobeetle mirror obligation** — engine-dev); `get_counter_count` already existed.

Translator: `passive_modifier_spec` lowers inline-closure SetValues `function(e,c) return c:GetOverlayCount()*N end` / `c:GetCounter(COUNTER_X|0xN)*N` (factor may lead or trail; counter codes via Phase 13 `counter_arg_to_name`), gated to `EFFECT_TYPE_SINGLE` self shape — in EQUIP/FIELD chains the closure's card param is each *affected* card. `PassiveModifierSpec.value` widened `i64` → `(negative: bool, value: String)`.

**Actual yield.** 10 passive blocks / 9 cards (all canonical closures in corpus): overlay — c94503794, c72971064, c10300821, c16110708, c7020743 (atk+def clone); counter — c31924889, c71413901, c14553285, c21113684 (all Spell Counter). Check delta ±0 (1,847/1,574 unchanged): the gaps were silent fidelity misses, not check errors. Skip classes (tested): one-param closures (5), `e:GetHandler()` receivers (6), `Duel.*` global counts (5), multi-step math (2), in-handler `GetReasonCard` registration (c44161893), Gemini-conditioned chain (c83269557), named fn refs, unknown counter codes.

**Follow-up — ✓ shipped (translator micro-phase, no grammar change).** The count(selector) half re-audited at 30 cards / 39 UPDATE chains with SetValue closures wrapping `Duel.GetMatchingGroupCount`/`GetFieldGroupCount`. `passive_value_expr` now routes count-call bodies to `passive_count_call_to_expr` (passive-aware sibling of Phase 10's `count_call_to_count_expr`): scope players `e:GetHandlerPlayer()` (any chain shape), `c:GetControler()` (SINGLE-self only — in EQUIP/FIELD the card param is each affected card), and literal `0` when the location masks are equal (player-symmetric → `either controls`); exceptions `nil` / own-card → `except self`; negative factors flip the modifier sign. `passive_count_filter` maps parameterized predicates (`Card.IsType`+TYPE_NORMAL/TUNER → kind, `IsSetCard`/`IsRace`/`IsAttribute` + constant via `filter_predicate_to_where`, `IsRitualMonster`, `IsFacedown`) and `aux.FaceupFilter(inner)` compositions (`… and is_face_up`). Setcode map +4 (HORUS, ASSAULT_MODE, AZAMINA, MEKLORD) — which also unlocked one Pass A `fusion_summon` line (c38648860). **Yield: 29 passive blocks / 23 cards + 1 resolve fill; check 1,847→1,846 / 1,574→1,574; all 24 files spot-checked vs lua.** Skips (tested): SetCondition/SetOperation chains (c19733961, c63142001, c99217226, c35057188, c75493362), composite math (`math.min` caps, `+` chains), custom closure filters (c43490025), `Card.IsSpellTrap` (parser lacks nested `(a or b)` predicate — c32588805), asymmetric literal-player scopes, OR'd location masks, foreign exception bindings.

---

### ~~T35 — `choose { ... }` block in resolve~~ ✓ shipped as Phase 16 (PR #120, translator-only)
**Shipped.** Grammar already had `choose { }`; no grammar work needed. Phase 16 recognizes the `Duel.SelectOption` idiom and emits `choose { }` blocks when EVERY option arm translates via the existing Phase 10–15 emitters (skip-not-mis-emit). Label sources: SetLabel-linked (`e:SetLabel(op)` + dispatch on `e:GetLabel()`) and op-side inline. Dispatch shapes: statement arms and chain-slot forks (shared prefix/suffix Set\* writes applied to every variant). Option labels resolve from CDB strs via `register_card_strings` (aux.Stringid index); missing labels skip the card.

**Actual yield.** −3 errors / −1 warning. The ~44-card estimate was dominated by skip classes that remain skipped (below floor, tested): dynamic option lists, `SelectEffect`/`SelectYesNo`, non-contiguous ladders, arm/option-count mismatch, statements outside the dispatch, untranslatable arms, inner `else` inside an arm, value-expression label uses.

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

### T38 — empty-resolve backfill: secondary-handler riders + proc recognition (multi-slice)

**Goal.** Eliminate the last error class in the corpus: 1,618 stems whose `.ds` has an effect block with an empty/missing `resolve` and no `choose` block. Split: 1,065 "secondary handler" stems (the lua op registers an inner `Effect.CreateEffect` instead of acting directly) + 553 "initial-only" stems (the action lives in a library proc closure or an unmapped primitive). The Phase 9 "~95% delayed second handler" framing is **wrong in emphasis**: 889/1,065 secondary stems (83%) hit ONLY `:no_op` buckets — the inner effect is a pure passive value chip (SetCode/SetValue/SetReset, no SetOperation), i.e. an *immediate* action with a *duration*, already expressible with `modify_*`/`set_*`/`negate_effects`/`grant`/`restrict` + the existing `duration` vocabulary. Only 176 stems have a true delayed second handler needing new grammar.

**Yield estimate.** ~450–520 cards across 8 slices (~330 of them with zero new grammar). Prereqs: the reset→duration exact-map fix (`reset_to_duration_kw`, src/lua_ast.rs:6144 — open) and the `set_atk`/`set_def` duration plumb-through (~~open~~ ✓ shipped, PR #134; ygobeetle adapter mirror still pending).

---

#### 1. Ranked slices (≥30-card floor; sub-30 shapes folded or dropped)

| # | Slice | Sub-shape | Safe yield | Grammar |
|---|-------|-----------|-----------|---------|
| **S1** | **Passive stat chips** (translator-only) | `:no_op` inner `EFFECT_TYPE_SINGLE` with `SetCode` in {UPDATE_ATTACK, UPDATE_DEFENSE, UPDATE_LEVEL, CHANGE_LEVEL, SET_ATTACK_FINAL, SET_DEFENSE_FINAL}, receiver ∈ {self, GetFirstTarget, single GetMatchingGroup loop}, value ∈ {int literal, `count(selector)*K`, `ceil(x.base_atk/2)`, `x.atk*2`}, reset in the exact-map table. Buckets: UPDATE_ATTACK 287 (225 pure), SET_ATTACK_FINAL 92, UPDATE_DEFENSE 54, UPDATE_LEVEL 54, CHANGE_LEVEL 40, ATK+DEF clone pairs 37. | **~160–200** | **NONE** — `modify_stat_action` / `set_stat_action` + durations `end_of_turn` / `while_face_up` / `while_on_field` / `next_standby_phase` all exist (pest:534-535, 745). Needs `ceil()` expr fn as a minor addition. **Prereqs:** reset exact-map fix (open) + set-stat duration plumb-through (✓ PR #134). |
| **S2** | **Qualified player floodgates** | `:no_op` inner `EFFECT_TYPE_FIELD` + player `SetTargetRange` + `RESET_PHASE\|PHASE_END`, `Duel.RegisterEffect(e1,tp)`. CANNOT_SPECIAL_SUMMON 92 (79 with a `splimit` qualifier), CANNOT_ACTIVATE 41, tr=yes reset bucket 236 chains. Unfiltered subset is T36-expressible today; the qualifier extension unlocks the rest. | **~70–90** | **✓ grammar half shipped**: `restrict_action` gains `("from" ~ zone)? ~ ("except" ~ "(" ~ exempt_expr ~ ")")?` — full seam (pest + AST + parser + validator + compiler + fmt + trait widen + MockRuntime, tests per layer; corpus check byte-identical). `exempt_expr` is or-of-and-terms over `attribute/race/name/archetype == …`, a from-zone atom, and the ten pred_atom card-type tags (no stat compares — the 2026-07-23 re-audit found none in 152 candidate chains / 83 MVP cards). Validator: qualifiers on the Battle Phase pair error; `from` on the hand-only normal-summon/set family errors. ygobeetle mirror outstanding (engine-dev) — no adapter overrides `restrict_player`, so the widen compiles clean and qualifiers are silent no-ops via the trait default; schedule on semantics, no build break will surface it. **✓ translator half shipped**: the Phase 18 SetTarget skip-gate became `decode_summon_limit` (closed conjunct set: positive IsLocation → from; negated atoms → De Morgan or-terms; `not (A and B)` → and-term; GetRace()~= spelling; aux.TargetBoolFunction shorthand) + the `GetActivateLocation()==L` activate conjunct. Apply: 4 verified fills, −3 errors / −1 warning — the review pass caught and reverted 2 under-stated Union-piece fills (inline `EquipByEffectAndLimitRegister` invisible to the walk; now a CARD_METHOD_POISON fill gate) and a UTF-8 panic in the conjunct splitter. The rest of the 83-card MVP set sits in handlers whose SIBLING statements don't translate yet (multi-step summon loops, fusion stage2 closures); those decode wins land as the surrounding translators do. **✓ follow-up shipped (S2b)**: the general unconsumed-card-method fill gate — every statement-position call outside a curated benign allowlist (chain builds on body-created bindings, counter/helper/`Duel.*` families, group lifetime bookkeeping, `table.*`, client hints) poisons the method-op family; `CARD_METHOD_POISON` grew from the equip pair to the full mutating-method census for expression positions; `body_drops_chains` gained helper-call completeness; the install_watcher path refuses gate-failing sub-handlers. Retrofit via the new `lua_translate audit-fills [--stub]` re-derivation audit: 274 exact machine fills + 16 hand-verified drifted fills stubbed (+273 errors / −30 warnings); 954 handler bodies newly refused; 3,284 report-only refusals remain (generation-era coarse content — the measured backlog). |
| **S3** | **Snapshot negation pairs** | `:no_op` DISABLE + DISABLE_EFFECT clone on same receiver, same reset (43/61 cards paired), receiver ∈ {GetFirstTarget, GetTargetCards, group loop}, reset ∈ {end_of_turn, while_face_up}. | **~30** | **NONE** — `negate_effects <selector> <duration>` exists (pest); one DSL action lowers to the two-code pair. Field-wide "lingering" variant (18 regs) is a skip class this round. |
| **S4** | **Ritual proc recognition** | `Ritual.AddProcEqual/Greater[Code]` single-call cards; filter reduces to code-list/archetype/race; lv nil or int; no extrafil/extraop/stage2/matfilter/CreateProc. 68-card bucket minus skips. | **~35–40** | Tiny: `summon_level` expr atom ("the summoned monster's own level") in the `where total_level` clause. `ritual_summon_action` exists (pest:519). NOTE: dormant branch `origin/worktree-ritual-addproc-assigned` (2026-06-10) already has assigned-form Ritual.AddProc* skeleton recognition + a RegisterSummonEff pass — inspect/rebase before starting S4/S5 fresh. |
| **S5** | **Fusion proc recognition** | `Fusion.CreateSummonEff` (32) / `SummonEffTG+OP` pair (36); whitelist extraop ∈ {nil, BanishMaterial, ShuffleMaterial}, gc ∈ {nil, ForcedHandler}, unconditional extrafil, no fcheck/chkf/stage2. | **~30** | **✓ grammar half shipped (specced + shipped in same PR, T36/T37 style)**: `fusion_summon_action` gains `fusion_plus_clause? ~ fusion_forced_self? ~ fusion_disposal?` = `("plus" ~ selector)? ~ ("including" ~ "self")? ~ ("sending_materials_to" ~ ("banished"\|"deck"))?` — full seam (pest + AST + parser + validator + compiler + fmt + trait widen + MockRuntime, tests per layer; corpus check + fusion-file fmt byte-identical). `plus` narrowed to `?` (one extrafil per proc; multi-zone pools ride the selector's zone list) and `gy` dropped from the destination set (absent clause = default GY disposal — one canonical spelling). Validator: `plus` pool must be a structured selector (error); non-`all` pool quantity warns (fcheck class, not carried through the seam). **ygobeetle mirror outstanding (engine-dev)** — the adapter does NOT override `fusion_summon`, so the dep bump compiles clean and the knobs are silent no-ops via the trait default; schedule on semantics, no build break will surface it. **✓ translator half shipped (PR 2)**: clean-extrafil → `plus` (walk-finalize pool decode; `return_expr` kills fcheck/conditional pools by construction), library `extraop` → disposal, `gc=ForcedHandler` → `including self`, destination-legality predicates (IsAbleToGrave/Deck/Remove) absorbed by a MATCHING disposal on both the pool filter and `matfilter`, Variant-B TG/OP pairing gate (c1498449's custom target wrapper). Real yield well under the ~30 estimate: 13 fills + 1 where-clause upgrade — the audited empty-resolve pool was mostly fcheck/conditional-extrafil/extratg/stage2 (verified per-card); the `including self` class (10 fills) lived in NO-resolve blocks the audit's empty-resolve filter missed. Retrofit: 26 shipped bare lines on knob-bearing calls stubbed back (pre-#145-policy fixed-line emissions; +9 errors / +6 warnings net). Backlog: pool-only `matfilter=aux.FALSE` cards (no "using none" surface), alignment-hazard fusion blocks (c11493868 cost-signature force-assign, c7382007/c29719112 missing-sibling-block), category-stub baseline lines in 23 fusion blocks. **Post-merge reconciliation (2026-09-03):** with the S5 translator and the #150 category-stub emptying both on main, `apply` heals 4 of the 22 emptied fusion blocks (c13234975 Beetrooper Landing, c40110009 Dragonmaid Changeover, c73594093 Metalfoes Fusion, c74335036 Fusion Substitute — plain `CreateSummonEff` race / archetype / OnFieldMat shapes, each hand-verified; −4 errors). The 5th refill apply offers, c31887806 Tobari Effect 5, is the positional-shift mis-fill (skip class 8 below) and stays empty; the three partial refills the pre-S2b translator used to make (c3496543, c6153210, c77124096) are now refused by the S2b general gate. |
| **S6** | **Card-method ops** | initial-only `card_method_action` (92): single mutating Card method after standard guard — UpdateAttack/Defense/Level → `modify_*`; NegateEffects → `negate_effects`; AddCounter/RemoveCounter → `counter_action`; RemoveOverlayCard → `detach`. Args literal or `count(selector)`. | **~50–60** | **NONE** — all five target actions exist. |
| **S7** | **Simple untranslated primitives** | initial-only `simple_untranslated` subset whose primitive already has a DSL action: ChangePosition 38 (incl. 4-arg toggle → bare `change_position`), SynchroSummon 10, SSet 7, XyzSummon 3, Summon 3, misc. | **~45–55** | **NONE** for this subset. (chain_attack 14, retarget ~21, skip_phase 8, redirect_attack 7, activate_field_spell 6, summon_or_set 4 are each <30 → deferred; retarget/skip_phase are already queued as their own grammar stories.) |
| **S8** | **Delayed-trigger wrapper** | the true `:with_op` class (176 stems): inner FIELD+CONTINUOUS effect on an EVENT_* code with its own SetOperation + reset. Led by EVENT_PHASE+PHASE_END 53, then CHAIN_SOLVING 12, BATTLE_DESTROYING 10, PHASE_STANDBY 10, DAMAGE_STEP_END 9, CHAIN_SOLVED 9, CHAINING 8. MVP: PHASE_END bodies whose inner op is 1-2 already-translatable actions + the `cnt=yes` "once, later this turn" bucket (20). 2026-07-24 re-audit: 139 chains / 121 cards; PHASE_END = 46, of which 37 carry SetCondition — but ~11 are the STALENESS-GUARD boilerplate (`GetFlagEffectLabel~=label → e:Reset()`, the DSL's implicit stale-target fizzle) and ~10 are the deferred-search availability check (implicit in `search`) — both decodable, not skips. 45/46 are `SetCountLimit(1)`; for PHASE_END + `until end_of_turn` the reset bounds recurrence to one firing, so no `once` modifier needed for the MVP bucket. | **~40–55** | **✓ grammar half shipped**: `delayed_action` gains the `on <trigger_expr> ("until" <duration>)?` arming (pre-S8 `until <phase>` form unchanged) — full seam (pest + `ast::DelayedArming` + parser + validator + compiler + fmt + `register_delayed_trigger` trait method + MockRuntime, tests per layer; corpus check byte-identical). Validator (hardened by the pre-PR review — 24 findings, 3 blocker clusters): zero-event-code triggers error via the shared `trigger_to_event_code` predicate (an enumerated ignition/custom list missed the SentTo/ReturnedTo unmapped-zone arms — silent wrapper drop); bare `summoned` errors (header multi-emit can't ride a single-code seam); qualified triggers error (who/category/method/reason filters aren't carried — `by flip` exempt, it selects its own event); `choose` and chain-context `negate` in the body error; nesting banned in BOTH directions; standby/draw arming bounded by end-of-turn warns (inert-spelling rule). `damage_step_end` trigger added (spec example (b) was inexpressible — new Trigger variant + EVENT_DAMAGE_STEP_END=1141). Body `target` references stay legal (outer chain target via label plumbing, stale-target fizzle; fire-time re-entry is body-ONLY — whole-resolve re-entry would re-arm geometrically). ygobeetle mirror outstanding (engine-dev) — default no-op, schedule on semantics. **✓ translator half shipped**: `translate_delayed_on_chain` routes the PHASE_END with_op class ahead of the watcher fallback. Class B (the 24-chain majority — deferred searches) decodes when the op body translates to exactly one action and any SetCondition is the bare availability check (filter AND location must match the op's own search; conjoined returns are semantic gates). Apply: 8 fills + 2 shipped `install_watcher event: end_phase` lines converted to their faithful delayed-on form (c58092907/c64161630 — each re-derives byte-identical; the review caught and reverted a 4th hand-flip, c77675029, whose compound-location set the zone-fidelity gate itself refuses; a 3rd, c41044418, was converted on this branch but is REFUSED by the S2b general gate once rebased onto it — its `s.operation` registers two sibling FIELD chains the walk drops (the Monster Reborn allowance + the flag listener), so the block stays the S2b stub), −8 errors / −1 warning at the pre-S2b base. Drift ledger correction: 5 of the 12 remaining end_phase watcher lines do NOT re-derive under today's translator (c37534148/c71071546 silent + c24920410/c25067275/c26120084 whose shipped `target` spellings are right but the current watcher body renderer says `self`) — all five PRE-DATE this branch; the card-registered TRIGGER_F class stays outside the flip. The labelobj class (destroy/banish/send "it" — the summon-then-kill family) ships as tested-but-dormant infrastructure (`RegisterEffectChain::label_object`, `FunctionBody::staleness_guard`): the label-source audit found ZERO chain-target stores — every corpus instance stores the just-summoned card, which needs a **`special_summon … as <binding>` surface (S8b follow-up)** before it can fill. Zone-fidelity gate refuses compound-LOCATION searches (deck\|gy renders zoneless → would default on-field). **✓ S8b shipped (stacked)** — the summoned-card labelobj surface: `special_summon … as sp` binding + `return … to field` (full v2 seam, `return_to_field` trait default no-op → ygobeetle mirror outstanding), the SELF-RESETTING lifetime gate (no SetReset + bare `SetCountLimit(1)` + REQUIRED staleness guard whose flag id the SAME body registered on the stored label object — a foreign-flag guard is a semantic gate, review fix; labelobj decode only, no Class-B), label decode widened to `e:GetHandler()` → `self` and quantity-1 summoned `g:GetFirst()` → `sp`, plus the SendtoHand→`return … to hand` op mapping. The pairing is routing-faithful on every emitter (flat body, `body_drops_chains` replay, AND the choose-arm path — the pre-review arm loop emitted `{ destroy sp }` beside an unbound summon line) and stamps no binding when the wrapper emits Class-B or the chain drops. ReturnToField decode was WITHDRAWN by the review: its family banishes with REASON_TEMPORARY, which the generic path renders as a plain (permanent) `banish`, contradicting `return_to_field`'s no-op-unless-temporarily-banished contract — the `to field` DSL surface ships reader-side only (LinkedCard precedent) until a temporary-banish action surface exists. Apply: ZERO fills — the 2026-07-24 cross-reference found every emittable family card's block already occupied by a generator category-stub (the dominant `special_summon (all, card, either controls)` spelling, 5,374 corpus-wide; the return-to-field flagship c36261276 sits behind the `banish (all, card, either controls)` sibling, a ~748-line population), which apply never rewrites; the empty-resolve stragglers (c40939228, c55285840) carry TODO riders and correctly skip. The harvest is gated behind a **stub-refinement pass** (own audited story: replace a LONE category-stub line when the aligned handler now translates completely and states the same category). |
| **S9** | **Continuous-listener block alignment** (translator-only) | `EFFECT_TYPE_*+EFFECT_TYPE_CONTINUOUS` chains registered with a SetOperation in `initial_effect` (flag registration on flip/summon, counter bookkeeping, maintenance costs) consume a positional index in the apply passes, but the corpus generator never emitted an `effect` block for them: 1,339 cards carry one, 951 have exactly consumers-minus-listeners blocks, 146 got a block anyway. On the hazard-free fast path every consumer at or after a listener sat one block late — Tobari the Sky Ninja (c31887806) re-created e3's hand special summon inside e4's fusion block on every `apply` (the #150 non-idempotent card). | — (retrofit) | **✓ shipped**: the listener and every chain after it carry `block_alignment_hazard` (the inclusive inverse of the bare-activate gate), so those cards leave the positional fast path for the Phase 20 matcher; a listener claims its own block only when the generator emitted one AND its signature is distinctive (once-per-turn — a bare listener tops out below MIN_CLAIM). Census: 1,112 cards re-assign, 408 consumers move, 27 lose a real block, 1,394 listener assignments were phantom indices. Retrofit by the #147 differential (base-SHA census vs fixed census, per-block old/new owner + rendered resolve/condition/cost/target, then an any-effect sweep over the re-assigned cards for older-era residue), every hit hand-verified against lua/card text: 38 honest stubs (19 one-late machine fills incl. the DESTROY_REPLACE-body class, 18 Phase-2/3a-era misplacements, 1 S2b-gate exact match under the corrected owner) + 12 misplaced header lines dropped (4 conditions / 5 costs / 3 targets); 10 stubs re-filled by `apply` with the correct owner's line. Apply on the re-anchored assignment: 13 new fills / 11 conditions / 12 costs / 15 targets / 1 duration retrofit, plus the 4 `fusion_summon` heals S5 stubbed "for a future apply pass". 2,178 → 2,192 errors (+14 honest) / 1,471 → 1,465 warnings; `apply` and `audit-fills --stub` byte-idempotent on re-run; Tobari stops re-filling. **✓ S9b shipped (`EFFECT_SPSUMMON_PROC` orphan-consumer gate):** proc chains with a SetOperation are the same orphan-consumer class — the generator renders the procedure into `summon { }`, never into an effect block (c41424250, c46412900, c73218989, c74631897). Census: 628 cards / 634 proc chains, 332 with a SetOperation (all `EFFECT_TYPE_FIELD`, no `_G`, no helper wrappers); of those cards 323 have more consumers than effect blocks (266 exactly consumers−procs), 270 carry the proc AHEAD of a real consumer, 322 had the proc holding a block index. The proc and every chain after it carry `block_alignment_hazard`; `entity_sig` marks the proc `EntityKind::Orphan` (gap 0, scores `NEG_INF` against every block — unlike an S9 listener a proc never owns an effect block, so there is no own-block rescue; the 4 cards where the generator did emit a block for an alternate proc keep it unmatched). Differential retrofit (base-SHA census vs fixed: 322 cards re-assign, 333 non-proc consumers move, 315 owner-changed blocks = 48 empty / 210 generator category stubs / 4 already the new owner's rendering / 53 hand-verified): 7 one-block-late machine fills stubbed (Mighty Neptune, Crimson Nova, Zushin, BLS Envoy, Swap Frog, Solar Wind Jammer, Thunder Dragonlord — the last three carried the proc's OWN `spop` body), 6 misplaced header lines dropped (5 `target` — Tellusion, Nebthet, H.A.M.P., Berserkion, Loud Cloud; 1 `condition:` — Indiora), 46 review blocks verified as generator-authored content in the right block and kept. Apply on the re-anchored assignment: 11 fills (Silent Swordsman +500, Regenesis Lord / Fire Fist Swallow / Diabellstar / Star Ryzeal set-from-deck, Cyber Pharos `fusion_summon`, Cyberdark End equip, Thunder Dragonduo, Alien Overlord, Zushin with its co-emitted Level-1-Normal target decl, Solar Wind Jammer `modify_level` refilling its own stub) + 1 choose (Akz, the Pumer), 20 conditions, 3 costs, 17 targets. 2,192 → 2,187 errors / 1,465 → 1,464 warnings; `apply` and `audit-fills --stub` byte-idempotent on re-run. **Backlog:** (b) ✓ **shipped** — the any-effect sweep is now the standing `lua_translate audit-owners [--stub]` mode (`block_match::foreign_fills`): every block whose shipped resolve byte-equals a NON-owner consumer's current rendering, classified mechanically — *coincidental* (owner renders the same: clone pairs / sibling bodies, never touched), *owner-unassigned* (hazard-gated, report-only), *alignment-suspect* (the header vetoes or scores negative against the owner, or the card's effect-block count ≠ its consumer count — right-content-wrong-owner shapes: the `EFFECT_SPSUMMON_PROC` orphans S9b has since gated [Blue-Eyes Shining Dragon, All-Eyes Phantom Dragon, Ursarctic Grand Chariot], block-less ritual helpers [Prayers of the Voiceless Voice, Rise of the Salamangreat], the generator's missing-block shells [Firewall Guardian]; report-only), *misplaced* (owner renders a different or empty body — `--stub` empties). Re-derived census on the S9 tree (pre-S9b): 855 blocks / 733 cards (the 718/686 figure was the throwaway census' narrower join) = 173 coincidental + 427 owner-unassigned + 53 alignment-suspect + 202 misplaced (121 owner-differs + 81 owner-empty). All 202 hand-verified against lua + card text (the block's cost/trigger/OPT header anchors the owner's chain; in every one the owner's chain never performs the shipped verb at resolution and the body is a sibling effect's sentence — the flip-monster `destroy target`/`add_to_hand target` in the face-down ignition, the GY-quick `special_summon target` in the Set-self block, the pendulum-scale ignition carrying the monster effect, the SPYRAL Drone / Jioh / Crystal Girl clone trios all carrying a 4th effect's line). Stubbed 202; `apply` refilled 56 with the owner's own line (+11 co-emitted target decls: Full Armor Photon / Nova Kaiser attach, Booster Dragon ATK+DEF, Scarabs / Medusa Worm / Golem Sentry face-down, Fire Fist Set-from-deck family, Crystal Girl delayed add, Quandax / Star Guardian synchro …); 146 stay honest stubs (owner undecodable — `Duel.Equip` non-canonical, delayed/flag ops, SynchroSummon side channels — or refused by the S2b / cost-header gates). Rebased onto S9b (PR #161) before merge: 2,187 → 2,327 errors (+140) / 1,464 → 1,357 warnings (−107); the sweep on the rebased tree is already at its fixed point (0 misplaced / 0 stubs / 0 fills — none of the 202 sit on S9b's 55 re-anchored cards), and `audit-owners --stub`, `audit-fills --stub` and `apply` are byte-idempotent on re-run. **Remaining backlog (report-only, counts on the S9b-rebased tree):** 428 owner-unassigned blocks / 417 cards (297 on cards whose block census ≠ consumer census — 248 with MORE blocks than consumers, the generator's text-split surplus; 131 on census-even cards where the matcher declined) — needs the block-identity story (text-ordinal names ↔ chains) before any of them can be adjudicated; 51 alignment-suspect / 50 cards (5 header-conflict, 49 census-mismatch, 3 both — S9b's gating resolved 2 of the 53 pre-rebase hits; the remaining census-mismatch shapes are the block-less ritual helpers and the generator's missing activation shells, which need their own hazard flags); 180 coincidental / 93 cards are correct as shipped; (c) generator `mandatory` noise on ignition blocks keeps listener-adjacent ignitions without OPT below MIN_CLAIM (Skilled Red/Blue/White Magician) — signature-model tuning, measured against the existing rescues. |

| **S10** | **Stub refinement** (translator-only, apply "Pass S") | The corpus generator wrote one placeholder line per lua `SetCategory` bit it could spell (`special_summon (all, card, either controls)`, `add_to_hand (…)`, `destroy (…)`, `send (…) to gy`, `banish (…)`, `return (…) to hand\|deck`, `take_control (…)`, `discard (…)`, `send (…) to extra_deck`); they validate, so `check` never flags them and `apply` (empty resolves only) never revisits them — 8,161 resolve bodies that are exactly ONE such line across 6,414 cards (the S8b harvest sat behind them). | — (retrofit) | **✓ shipped**: `src/stub_refine.rs` (lone-stub matcher over the closed vocabulary; spelling→category table with the two generator-vs-translator synonyms `return … to hand` ≈ `add_to_hand` [CATEGORY_TOHAND] and `return … to deck` ≈ `send … to deck` [CATEGORY_TODECK], each evidenced on real cards; rider table with card-text keywords; `category_match`) + Pass S in `lua_translate apply`, which reuses Pass A's decision path verbatim (`plan_fill` — one function now serves A, A2 and S) and rewrites a lone stub only when the owner's fill is complete under every existing gate AND states the stub's category. Hazard-gated effects have no assignment and never reach it; blocks the matcher left unassigned are counted, never rewritten; choose-bearing blocks and multi-line stub bodies are out of scope. Hand verification of the first run found four shapes the CURRENT translator renders lossily (Link/Fusion/Ritual generic mapping → plain `special_summon … from extra_deck`; `Duel.SpecialSummon(…,1-tp,…)` → `special_summon self`; `SEQ_DECKTOP/BOTTOM` → plain `send … to deck`; non-literal `LOCATION` args — compound `LOCATION_A\|LOCATION_B` or a label-dispatched `loc` — → ZONELESS selector = on-field; non-literal exclusion arg → `except self` dropped) — Pass S refuses them (`stub_refine::lossy_call`, 377 skips, 326 of them the zoneless class) while Pass A's empty-resolve policy is left untouched as a flagged backlog (policy flip + retrofit of the shipped instances). Apply: 8,161 lone stubs → **2,622 refined** (1,026 special_summon / 891 add_to_hand incl. 181 `return … to hand` / 428 destroy / 194 send-to-gy / 46 return-to-deck / 28 banish / 8 discard / 1 take_control), 66 category mismatches (positional residue the check catches — e.g. a `special_summon` stub whose owner renders `add_to_hand`), 4,919 gate refusals, 175 unassigned, 2 already exact; errors/warnings unchanged (2,327 / 1,357, post-#162 baseline); byte-idempotent on re-run (0 refined, diff md5 identical). **Backlog:** the four lossy shapes above as Pass A policy flips; `DiscardHand(…,1,1)` renders `1+` (over-permissive quantity); trigger-header parity is NOT a Pass A gate (c8129306's `sent_to gy` header vs EVENT_RELEASE pre-dates this slice); the 4,919 gate-refused stubs are the measured translator backlog by category. |

**Folded/dropped (<30 or out of scope):** equip riders (EQUIP_LIMIT 48, EQUIP-type UPDATE_ATTACK) → T33 equip/attach surface; union_proc 26, delegated_op 20, other_complex 48 (branching), CHANGE_CODE 26 / CHANGE_ATTRIBUTE 14 (need `change_name`/`change_attribute` recognition — fold into a later S1b if counts hold), label-valued `e:GetLabel()` cards (~70 across buckets) → deferred behind a future cost-tag plumbing story, INDESTRUCTABLE_BATTLE 26 → fold into S2b grant-aura follow-up.

---

#### 2. Grammar proposal — delayed-trigger wrapper (S8)

Extend the existing `delayed_action` (pest:647, currently phase-only) rather than adding a new keyword:

```pest
delayed_action = { "delayed" ~ (
      "on" ~ trigger_expr ~ ("until" ~ duration)?   // NEW: event-armed, recurring until reset
    | "until" ~ phase_name                          // existing phase-deferred form, unchanged
  ) ~ "{" ~ action+ ~ "}" }
```

Semantics: registers `EFFECT_TYPE_FIELD+EFFECT_TYPE_CONTINUOUS` on the event, SetOperation = body, SetReset from the duration clause (default `end_of_turn`); fires on EVERY event occurrence until reset (NOT one-shot). A one-shot variant rides an optional `once` modifier later (the SetCountLimit bucket, cnt=yes n=20).

```ds
// (a) "destroy it during the End Phase" — EVENT_PHASE|PHASE_END, cnt=yes bucket
resolve {
    special_summon (1, monster, from gy)
    delayed on end_phase until end_of_turn {
        destroy those
    }
}

// (b) c11755663 Dinowrestler Martial Anga — EVENT_DAMAGE_STEP_END, recurring
resolve {
    delayed on damage_step_end until end_of_turn {
        skip_phase battle for turn_player    // blocked on the skip_phase story
    }
}

// (c) "if this destroys a monster by battle, inflict 1000" — EVENT_BATTLE_DESTROYING
resolve {
    delayed on destroys_by_battle until end_of_turn {
        damage opponent 1000
    }
}
```

`trigger_expr` is reused from the effect-header trigger grammar; the validator restricts the body to actions with no target-decl dependence (the wrapper body binds no chain target).

---

#### 3. Runtime seam (ygobeetle mirror obligations)

```rust
// ✓ shipped (PR #134): set_atk/set_def/change_level carry Duration.
// ygobeetle adapter mirror of the widened signatures still outstanding.
fn set_atk(&mut self, card_id: u32, value: i32, duration: Duration);
fn set_def(&mut self, card_id: u32, value: i32, duration: Duration);

// ✓ shipped (T38 S2 grammar PR): qualifier rides the existing T36 method
// as an Option — the widen landed on restrict_player itself, not a new
// method (the sketch's restrict_player_filtered name is superseded).
fn restrict_player(&mut self, player: u8, restriction: PlayerRestriction,
    qualifier: Option<&RestrictionQualifier>, duration: Duration) {}
// RestrictionQualifier { from_zone: Option<u32>, except: Option<ExemptFilter> } —
// runtime-surface mirror family, same pattern as PlayerRestriction/DamageRule;
// zones as LOCATION_* masks, attribute/race as engine codes (TokenSpec
// convention), archetype/name as strings (CardFilter convention); doc-comment
// tables map every variant to its splimit predicate shape. Evaluated per
// would-be-summoned card at BOTH seams (action-gen mask + exec pre-check),
// per the PR #50 rollout pattern. ygobeetle mirror outstanding — no adapter
// override exists, so the dep bump compiles clean (silent no-op via trait
// default); schedule on semantics like the S5 fusion knobs below.

// ✓ shipped (T38 S5 grammar PR): fusion proc knobs on the fusion surface.
// ygobeetle mirror outstanding — NOTE the adapter does NOT override
// fusion_summon, so the next dep bump compiles CLEAN and the knobs are
// silent no-ops via the trait default: schedule the mirror on semantics
// (pool/forced-material/disposal correctness); unlike the #134 set-stat
// widen, no build break will ever surface this one.
fn fusion_summon(&mut self, card_id: u32, player: u8, material_ids: &[u32],
    extra_pool: Option<&ExtraMaterialPool>, must_include_self: bool,
    material_destination: Option<MaterialDestination>) -> bool;
// ExtraMaterialPool { filter: CardFilter, my_location: u32, opponent_location: u32 }
// — the lua extrafil filter+location pair; masks RELATIVE to `player`, the
// Duel.GetMatchingGroup(f,tp,my,opp) / SetTargetRange convention. `where`
// predicates on the plus selector are not yet mirrored (register_redirect
// filter_flags precedent — summary now, extension later without breaking the
// seam). MaterialDestination { Banished, Deck } — extraop ∈
// {Fusion.BanishMaterial, Fusion.ShuffleMaterial}; None = default GY disposal;
// engines gate material legality on the destination (IsAbleToRemove/IsAbleToDeck)
// and dispose BEFORE the summon completes (BreakEffect seam).

// ✓ shipped (T38 S8 grammar PR): event-armed recurring listener. The
// sketch's TriggerEvent/DelayedOps params landed as the register_delayed
// house convention instead — u32 EVENT_* code + card_id, body delivery
// via re-entering the compiled effect (same open evolution point as
// register_delayed); duration defaulted to EndOfTurn by the compiler.
fn register_delayed_trigger(&mut self, event: u32, card_id: u32,
    duration: Duration) {}
```

Cross-cutting contracts (documented on the trait, per decisions-2.md house style):
- **Stacking, never idempotent:** each `modify_*` resolution registers an independent chip (Silent Swordsman +500/turn accumulates).
- **One-shot value snapshot:** exprs (`count(...)`, `x.overlay_count*K`) evaluate once at resolution; never re-evaluated on continuous refresh.
- **Exact reset decoding:** `WhileOnField` = RESETS_STANDARD event set on the *receiver*; `WhileFaceUp` = +RESET_DISABLE; `EndOfTurn` = phase-end AND the standard event set (a bare phase timer that survives zone moves diverges from ocgcore). Reuses ygobeetle's `lua_reset_to_query_flags` seam (PR #51).
- **Set-final layering:** SET_ATTACK_FINAL is a distinct pipeline slot after base/update layers, timestamp-ordered.
- **Stale-target fizzle:** targeted applications silently no-op when the target is gone/face-down at resolution.
- **`negate_effects`** registers the DISABLE + DISABLE_EFFECT(RESET_TURN_SET) pair — one action, two engine facets (lands on the known ygobeetle EFFECT_DISABLE gap).
- **`ritual_summon`/`fusion_summon`** stubs exist (runtime.rs:1264); real obligations: material legality with EFFECT_RITUAL_LEVEL / EFFECT_EXTRA_FUSION_MATERIAL overrides, disposal-before-summon with BreakEffect seam, SUMMON_TYPE typing + CompleteProcedure, two-stage player selection surfaced through the choose mechanism.

---

#### 4. Skip classes (hard, apply-pass rejects)

1. **Block-to-handler binding failures:** dimension tags are card-level unions; the empty block's OWN SetOperation handler must contain the bucket shape (c21249921, c11493868, c10204849, c21522601, c27143874 all proved wrong-block fills otherwise).
2. **Impure handlers:** any Duel mutator, modal Select/Announce, RegisterFlagEffect payload, tiered ct-branching, or interactive flow alongside the registration.
3. **Label plumbing:** `SetValue(e:GetLabel())` / SetLabelObject cross-effect protocols (~70 cards) — deferred behind cost-tag plumbing, not dropped.
4. **Non-mapped resets:** RESET_OPPO_TURN combos, reset counts `,2`, PHASE_DAMAGE_CAL/PHASE_BATTLE/PHASE_STANDBY variants, masked exprs (`&~RESET_TOFIELD`) — exact-map misses skip, never approximate.
5. **Function-valued SetValue** (dynamic recompute — snapshot semantics would be *wrong*, not lossy) and any inner SetCondition/SetTarget/SetCountLimit outside S8's whitelist.
6. **Receivers without selectors:** battle-derived (GetAttacker/GetBattleTarget), equip targets, label objects, eg-derived — until battle-context selectors exist.
7. **Equip-rider shapes** (EQUIP-type inner effects, EquipByEffectAndLimitRegister, aux.AddEREquipLimit) → T33.
8. **Positional shift from block-less CONTINUOUS consumer chains (✓ GATED — T38 S9, PR #159):** the corpus generator emitted no `effect` block for `EFFECT_TYPE_*+EFFECT_TYPE_CONTINUOUS` listener chains (flag registration on flip/summon, etc.), but `block_match::compute_assignments`' positional fast path counts them as index consumers, so when such a chain precedes the real effects every later handler lands one block late. Census: 1,339 cards carry a CONTINUOUS consumer (951 fit consumers−continuous == blocks, 146 do have a block, 242 other); 503 have one AHEAD of a real effect with more consumers than blocks (the shift-risk population). Live instance: c31887806 Tobari the Sky Ninja — `apply` writes e3's hand-summon body into Effect 5 (e4's QUICK_O fusion block; its `IsTurnPlayer(1-tp)` window also has no condition atom) on every run, re-creating the line #150 removed — the corpus' one documented apply-non-idempotent card. Fix: route CONTINUOUS-consumer cards through the signature matcher instead of the positional path (e3's speed/OPT/cost signature matches Effect 4, e0's does not), then the #147-style old-vs-new differential retrofit over the 503-card population. **Shipped 2026-09-03 (PR #159):** the listener and every chain after it now carry `block_alignment_hazard` and route through the Phase 20 matcher; the 503-card retrofit (38 hand-verified stubs, 12 dropped header lines, 10 machine refills) landed with it — see the S9 row. Tobari re-derives byte-identical.
8. **Oath cost-site registrations** (19 Floowandereeze-pattern) — until a `cost { oath { ... } }` placement lands.
9. **Proc knobs:** ritual CreateProc/extrafil/extraop/stage2/matfilter; fusion chkf/fcheck/conditional-extrafil/custom-extraop/custom-gc.
10. **Wrong-header blocks:** never fill a resolve onto a block whose trigger/cost header already mismatches the lua (route to a header-fix pass); never overwrite non-empty resolves; every pass byte-idempotent on re-run.

---

#### Acceptance
- Prereq PRs: reset exact-map fix (lua_ast.rs:6144) with corpus correction for the confirmed-wrong emissions (c61151074, c11493868); ~~`set_atk`/`set_def` duration plumb-through~~ ✓ PR #134 (adapter mirror outstanding).
- Each slice = its own PR pair (translator, then corpus apply) per the translator-phase pattern; `duelscript check` error count strictly decreases per apply, zero warnings, oracle byte-identical on untouched blocks, structural idempotency (second run no-op).
- S2/S5/S8 grammar PRs ship the full seam (pest + AST + parser + validator + compiler + fmt + trait + MockRuntime, tests per layer) before their apply passes, T36/T37 style. (S5 ✓.)

**Agent.** `lua-translator` for S1/S3/S4-recognition/S6/S7; `grammar-extender` for S2/S5-extensions/S8; `engine-dev` for the adapter mirrors (set-stat duration, restrict qualifier, delayed trigger, ritual/fusion procedures).

---

### T39 candidates — empty-resolve triage (2026-10-20)

**Why.** `duelscript check cards/official/` reports 2,327 errors on main (post-#162) and every one is `Effect 'Effect N' must have a resolve or choose block`. The T38 slices were sized from shape audits and under-delivered badly (S5 13 fills vs ~30, S2 4 vs 70–90, S8 8 vs 40–55, S8b 0) because the audits counted the *shape* while the apply pass refuses on a *different, earlier* gate. Nobody could say per block WHY it stays empty: `translate` prints no drop reason and `apply` only aggregates (`todo|incomplete|no map|align`). This section records the diagnostic that fixes that and the ranked list it produces.

**Tool (shipped, this PR).** `lua_translate explain <corpus_dir> <lua_dir> [cards.cdb]` prints ONE tab-separated line per effect block the validator flags — no `choose`, and either no `resolve` or an empty one; permanent-passive activation shells (Continuous/Field/Equip Spell, Continuous Trap; no trigger line, no cost) are exempted exactly as the validator exempts them:

```
card  block  owner-binding|unassigned  reason-code  detail  SetType  stmts
```

`reason-code` is the FIRST gate that refuses along Pass A's real decision path — not a parallel heuristic: the one decision path #163 introduced (`plan_fill`, shared by Pass A, A2 and the stub-refinement Pass S) now returns a `FillPlan` whose refusal arm carries the closed `FillReason` enum (src/bin/lua_translate.rs; each variant documented, each variant documented and mapped to the `ApplyReport` counter it was always charged to), and the completeness gate became `lua_ast::body_drop_reasons` (`BodyDrop` enum, every drop in check order; `body_drops_chains` is now `body_drop_reason(..).is_some()`). `EffectSkeleton::summon_helper_verdict` names the helper gate; `FunctionBody::method_poison_source` records the first statement the S2b general gate refused. `detail` is the gate's own datum: the first unmapped `Duel.*` call, the chain's `code/type/reset/reg/value` + slot markers (`with-cond/with-op/with-target/with-range/forced/duel-reg/choice-arm/multi`, `conflicting-sets`), the poisoning statement, the helper param set, the header text — plus a remaining-blocker census on drop verdicts (`+N drops, M todo`; `+0 drops, 0 todo` = this ONE gate is the whole blocker). `stmts` is the walk-collected statement count (Duel calls + chains + method/counter ops + helper calls). Choose-shaped effects (SelectOption / label dispatch) that Phase 16 refused print `choose-undecodable` with the plain-path reason appended.

**Parity proofs (rebased onto main @ 19af08eee, i.e. after #161/#162/#163; the #163 stub refinement did not move a single bucket).** 2,327 lines = the 2,327 validator errors exactly (2,061 cards; 830 shells exempt); 0 `ok` verdicts (the corpus is at the apply fixed point). `apply` output byte-identical before/after the refactor: origin/main binary vs this branch's binary on two scratch copies of `cards/official/`, `diff -rq` = 0 lines over 13,298 files, apply report identical; `audit-owners --stub`, `audit-fills --stub` and `apply` (incl. Pass S, `stubs refined: 0`) are all no-ops against the committed corpus. 755 lib tests (+4) / 22 bin tests, zero warnings.

#### Reason-code table (2,327 blocks / 2,061 cards)

| reason | blocks | cards | first gate that refused |
|---|---|---|---|
| `chain-unlowered` | 1,049 | 966 | a `RegisterEffect` chain in the op body has no lowering (T38 S1 completeness gate) |
| `method-poison` | 311 | 295 | S2b general statement gate: an unmodeled statement-position call (first one named) |
| `todo-only` | 264 | 239 | every `Duel.*` statement is unmapped |
| `unassigned` | 208 | 201 | no block owner: 138 "no consumer for this block index", 69 hazard unrescued (Phase 20 / S9 / S9b), 1 no lua effects |
| `summon-helper-undecodable` | 141 | 140 | fusion/ritual helper params or frame refuse the line |
| `helper-unlowered` | 73 | 51 | a script-local `s.*` helper statement with no lowering |
| `no-body` | 58 | 47 | `SetOperation` names a library fn / inline closure the walk has no body for |
| `target-decl-undecodable` | 50 | 48 | fill references `target`, block has no decl, SetTarget filter not exactly mapped (#162 stubbed 29 more of these) |
| `counter-op-unlowered` | 43 | 37 | counter op with computed count / unmapped receiver |
| `method-op-unlowered` | 32 | 30 | `NegateEffects` / `UpdateAttack` / … with non-decodable args or receiver |
| `cost-header-mismatch` | 24 | 23 | shipped `cost { … }` does not re-derive from the lua cost (skip class 10) |
| `incomplete` | 14 | 14 | action + TODO mix |
| `counter-poison` | 14 | 13 | expression-position / either-or counter placement |
| `no-lines` | 13 | 12 | every collected call is a UI/query skip (`GetFirstTarget` + `ConfirmCards`, …) |
| `mixed-method-ops` | 13 | 13 | method op beside a Duel action (S6 mixed class) |
| `choose-undecodable` | 8 | 8 | choose-shaped but Phase 16 refused; 5 dynamic option lists |
| `helper-frame-mismatch` | 8 | 8 | helper fill onto a header with no condition/cost surface (S5 review) |
| `interference` | 4 | 4 | Phase 10 stat-write guard would drop a line |

Owner assigned: 2,119 blocks; unassigned: 208. Reason codes that never surface from the block side but exist in the enum (Pass A counters): `no-handler` (pure passives own no block), `replacement-chain` (its block is the `replacement` surface), `no-block`.

`chain-unlowered` by inner sub-gate (derived from the detail's markers): **decode-refused 484** (a mapped single-type family refused the value / receiver / reset), **field-family-refused 315** (`EFFECT_TYPE_FIELD` → Phase 15 / T36 / T37 / S8 translators all declined), **code-unmapped 190** (no family for the `SetCode`), slot-gated 39 (`forced` 15, `choice-arm` 12, `with-cond` 8, …), conflicting-sets 14, event-watcher-refused 10, equip-rider 5. Sole-blocker (`+0 drops, 0 todo`) chains: 675 / 616 cards.

By owner `SetType`: IGNITION 481 / ACTIVATE 444 / QUICK_O 326 / SINGLE+TRIGGER_O 306 / FIELD+TRIGGER_O 189 / SINGLE+TRIGGER_F 154 / FIELD+TRIGGER_F 64 / FIELD 48 / (unassigned or helper, no SetType) 311. By walk statement count: 1 → 306, 2 → 620, 3 → 371, 4 → 288, 5 → 119, 6+ → 188, 0 → 35 (and 415 with no walked body: unassigned + helpers).

#### Top-20 first-unmapped shapes (numbers normalized; chain = `SetCode` of the first unlowered chain)

| first unmapped shape | blocks | cards | reason bucket |
|---|---|---|---|
| chain `EFFECT_UPDATE_ATTACK` | 218 | 201 | chain-unlowered (199 decode-refused, 7 field, 4 choice-arm, …) |
| `Duel.ChangePosition` | 90 | 74 | todo-only / incomplete (~50 "non-toggle position matrix", ~32 "target provenance unknown", 9 unrecognized shape) |
| helper `s.*` | 73 | 51 | helper-unlowered (`s.arcanareg` 32 / 12 cards, `s.equipop` 21 / 20, long tail 20) |
| chain `EFFECT_SET_ATTACK_FINAL` | 68 | 63 | decode-refused 66 |
| `Card:RegisterFlagEffect` | 64 | 62 | method-poison (58 with no other drop/todo) |
| chain `EFFECT_UPDATE_LEVEL` | 54 | 51 | decode-refused 47 |
| target-decl-undecodable | 50 | 48 | `s.distg` 14, `s.atktg` 7, `s.target` 7, `s.postg` 5, `s.mattg` 4 |
| chain `EFFECT_EQUIP_LIMIT` | 47 | 46 | code-unmapped (equip rider → T33 surface) |
| chain `EFFECT_CHANGE_LEVEL` | 40 | 37 | decode-refused 37 |
| chain `EFFECT_DISABLE` | 38 | 37 | 21 decode-refused (SINGLE), 15 field-family (with-target) |
| `Card:GetHandler` (`e:GetHandler():X(…)` statements: SetHint, EquipByEffect…, …) | 38 | 33 | method-poison / counter-op-unlowered |
| `Card:CancelToGrave` | 35 | 35 | method-poison (activation shells of Fusion/Equip-like spells) |
| chain `EFFECT_CANNOT_ACTIVATE` | 32 | 31 | field-family: `value=s.aclimit` closures (T38 S2 qualifier decode doesn't reach them) |
| chain `EFFECT_LEAVE_FIELD_REDIRECT` | 27 | 25 | 18 code-unmapped + 9 forced — no resolve-time redirect surface |
| `Card:SetCardTarget` | 25 | 22 | method-poison (card-target link protocol) |
| cost-header-mismatch | 24 | 23 | `Cost.SelfDiscard` / `s.discost` / `Cost.AND(…)` vs shipped header |
| `Duel.Equip` | 22 | 20 | todo "non-canonical shape" (equip target ≠ self/target) |
| chain `EFFECT_CHANGE_ATTRIBUTE` | 22 | 20 | decode-refused (12 of them `value=e:GetLabel()`) |
| `Card:NegateEffects` | 21 | 20 | 15 method-op-unlowered + 6 mixed |
| `Card:UpdateAttack` | 21 | 20 | 12 method-op-unlowered + 4 mixed + 5 poison |

Also in the top 30: chain `EFFECT_INDESTRUCTABLE_BATTLE` 20, `Card:EquipByEffectAndLimitRegister` 20, chain `EFFECT_ADD_TYPE` 20, helper `params: fusfilter,matfilter,extrafil,extraop,extratg` 19 (the `extratg` family, ~36 total), chain `EFFECT_CANNOT_TRIGGER` 19, chain `EVENT_PHASE+PHASE_END` 19 (S8 Class-B refusals), `Card:AddCounter` 19 (computed `ct` / `e:GetLabel()` counts), `Duel.ChangeTargetCard` 17 (retarget — queued grammar story), chain `EFFECT_CANNOT_ATTACK` 16, `Duel.ChainAttack` 16, `Duel.MoveSequence` 13, `Duel.Overlay` 12, `Duel.ChangeAttackTarget` 10.

Decode-refused stat chips (`UPDATE_ATTACK/DEFENSE/LEVEL` + `SET_ATTACK_FINAL` + `CHANGE_LEVEL`, SINGLE type, no slot marker — 381 blocks) by `SetValue` shape: local binding (`atk` / `val` / `-ev` — `Duel.GetMatchingGroupCount(…)*K`, `g:GetClassCount(…)*K`, event values) 112, literal with a non-chain receiver (`Duel.GetAttacker()`, `e:GetLabelObject()`, `g:GetFirst()` of an unmapped group, loop vars over unmapped groups) 83, `e:GetLabel()` 55 (label plumbing, skip class 3), arithmetic (`tc:GetOverlayCount()*300`, `ct*500`, `tc:GetMaterialCount()*500`) 49, stat reads (`tc:GetAttack()/2`, `SET_ATTACK_FINAL` from another card's stat) 47, other 9.

#### Ranked candidates (error-triage decision rule: reuses grammar > chore > needs grammar)

"Est." = blocks whose ONLY blocker is that shape — `+0 drops, 0 todo` for drop verdicts, exactly one distinct unmapped method for todo verdicts. **This over-counts**: a block that clears the named gate still has to pass every LATER gate (cost-header parity, helper-frame parity, bare-target co-emission, the Phase 10 interference replay), and walk-invisible content is only partly visible — for `method-poison` rows the census is blind (the poison hides everything behind it), and for every row the walk does not see `if`-arm structure, local computations, or loops it did not classify. Treat Est. as an upper bound; the T38 record says real yield lands at 20–50% of it.

| # | Candidate | Est. (blocks / cards) | DSL surface | Grammar? | Hard sub-classes visible now |
|---|---|---|---|---|---|
| **1** | **Stat-chip decode widening** — decode-refused `EFFECT_UPDATE_*` / `SET_ATTACK_FINAL` / `CHANGE_LEVEL`: (a) `value_bindings` whose RHS is `Duel.GetMatchingGroupCount(f,…)*K` → `count(sel) * K`; (b) arithmetic over `tc:GetOverlayCount()` / `GetMaterialCount()` / a loop counter `ct`; (c) multi-target `aux.Next(g)` loops whose `g` has a mapped selector → group selector line; (d) `tc`/`g:GetFirst()` receivers from an already-mapped `SelectMatchingCard` | 314 sole / 285 cards in the class; ~100 / 90 for (a)–(d) | `modify_atk/def/level <sel> ± expr until <duration>`, `set_atk/def/level` (pest:552-553); needs `target.overlay_count` expr atom (only `self.overlay_count` exists, pest:36) | NONE for (a)/(c)/(d); tiny expr atom for (b) | `e:GetLabel()` values (55, skip class 3), `GetAttacker`/`GetLabelObject` receivers (skip class 6), `g:GetClassCount(...)` ("different names", no atom), `RESET_PHASE\|PHASE_DAMAGE_CAL` resets (unmapped lifetime), stat reads from a second card |
| **2** | **`Duel.ChangePosition` 5-arg matrix** — `(au, ad, du, dd)` quads beyond the exact toggle `(FACEUP_DEFENSE,0,FACEUP_ATTACK,0)`: `(FACEUP_DEFENSE,FACEUP_ATTACK,FACEUP_ATTACK,FACEUP_ATTACK)` / `(FACEUP_DEFENSE,FACEDOWN_DEFENSE,FACEUP_ATTACK,FACEUP_ATTACK)` are the toggle-with-flip spellings | 89 / 73 sole; ~50 / 42 for the matrix sub-shape | `change_position <sel> [to <pos>]` (pest:544) | NONE — but needs a RULING on what bare `change_position` means for the face-down arms before any matrix maps | ~32 "target provenance unknown" (candidate 4), 9 unrecognized arg shapes |
| **3** | **Code-table extension chore** — `EFFECT_CANNOT_DIRECT_ATTACK` → `grant … cannot_attack_directly`; `EFFECT_CANNOT_TRIGGER` → `grant … cannot_activate effects`; `EFFECT_CHANGE_LSCALE` / `UPDATE_LSCALE` → `set_scale`; `EFFECT_CHANGE_LEVEL_FINAL` → `set_level`; `EFFECT_UPDATE_RANK` → `change_rank`; `EFFECT_SWAP_BASE_AD` → `swap_stats` | 9 + 18 + 12 + 5 + 2 + 4 = 50 / 47 before the value/receiver/reset decode (candidate 1's refusals apply) → ~25–35 | `grant_ability` (pest:600), `change_property_action` (pest:555), `swap_action` (pest:591) | NONE | `EFFECT_ADD_TYPE` 20, `EFFECT_TRAP_ACT_IN_SET_TURN` 11, `INDESTRUCTABLE_COUNT` 8, `EFFECT_EXTRA_ATTACK_MONSTER` 7 have no surface |
| **4** | **Selector provenance for named filters** — "target provenance unknown" on `ChangePosition` ~32 / `Release` 13 / `Remove` 11 / `SendtoGrave` 10 / `SSet` 4 / … plus the 50 `target-decl-undecodable` blocks (`s.distg`, `s.atktg`, …): the S1 named-filter refinement maps `s.filter` bodies with exactly-mapped atoms; these carry an atom outside the mapped set | ~70 (todo side) + 50 (target-decl side) / ~110 cards | existing selector `where` clauses + `target <selector>` co-emission | NONE, but needs an atom census of the refused filters (the detail names `g` / the handler, not the atom) before an estimate is trustworthy | `REASON_COST` ops in op handlers (cost-tagged resolve ops) need a `tribute`/`banish … as cost` decision |
| **5** | **Field-family decode widening** — `EFFECT_TYPE_FIELD` chains on codes with an existing surface: `DISABLE` + SetTarget filter → `negate_effects (all, monster, …)`; `INDESTRUCTABLE_EFFECT` 13 / `CANNOT_SPECIAL_SUMMON` 15 / `CANNOT_ATTACK` 9 / `CANNOT_ATTACK_ANNOUNCE` 9 with-target → `grant <group sel>`; `CHANGE_DAMAGE` 16 + `NO_EFFECT_DAMAGE` 8 + `REFLECT_DAMAGE` 5 → `damage_rule` (T37) when `value=s.damval` is a constant closure; `CANNOT_ACTIVATE` 30 → `restrict … except (…)` (T38 S2) when `s.aclimit` decodes | 315 in class; ~60–80 / 55–75 for the named families | `negate_effects`, `grant`, `damage_rule`, `restrict` | NONE for these; `EXTRA_SUMMON_COUNT` 16 / `SKIP_DP` 5 / `BP_TWICE` 3 / `SKIP_BP` 3 / `SET_SUMMON_COUNT_LIMIT` 3 need grammar | field chains are `duel-reg` + `with-range` by construction (not slot refusals here); the SetTarget filter must map to a selector `where` exactly |
| **6** | **Summon-helper factory-frame relaxation** — `frame: SetCondition` 16 / `SetCost` 11 / `SetType` 7 on `Fusion.CreateSummonEff` / `Ritual.AddProc*` / `CreateProc`: emit the line when the .ds header already models the frame (the Variant-B `helper_frame_matches` rule, extended to factory forms) | 36 / 35 upper bound | `fusion_summon` / `ritual_summon` | NONE | `extratg` param family ~36 (custom target wrapper, fcheck class), `stage2` ~10, ritual own-level EQUAL 5 (`summon_level` atom, S4 backlog) |
| **7** | **Orphan generator blocks chore** — 138 blocks with NO lua consumer for their index: generator-era extra blocks beside library procs (ritual 50, union 31, pendulum 5, equip/ER 5, link 2, xyz 1) and 44 cards whose chain registers no `SetOperation` at all (SetTarget-only activations, c14604710). Audit-then-act per Phase 5b: either recognise the proc (union → the T38 "union_proc 26" fold) or drop the block when it carries no header content | 138 / 137 | — | chore (recognisers may need `union` surfaces) | never auto-delete: the block may encode card-text structure the lua folds into a library call |
| **8** | **Computed counter counts** — `AddCounter(COUNTER_X, ct)` / `e:GetLabel()` counts | ~28 / 25 | `place_counter <name> <unsigned> on <sel>` (pest:579) | small widen: `unsigned` → `expr` | `e:GetLabel()` counts are label plumbing |
| **9** | **Archetype helper `s.arcanareg`** (Arcana Force coin flip) | 32 / 12 | `flip_coin { heads {…} tails {…} }` (pest:596) | NONE, archetype-bound | per-archetype rule: defer |
| **10** | **Flag registration** — `c:/tc:RegisterFlagEffect(id, RESETS_STANDARD_PHASE_END, 0, 1)` poisons 64 blocks (58 census-clean) | ≤58 / 56 (census blind) | none — flags are the cross-effect protocol a sibling `SetCondition` reads (the S8b foreign-flag finding) | needs a `flag` surface or a proof the flag is read only by the same card's implicit stale-target fizzle | treat as skip class 3 until designed |
| — | **Unassigned hazards** 69 (S9/S9b signature-model tuning), `EFFECT_EQUIP_LIMIT` 47 + `s.equipop` 21 + `EquipByEffectAndLimitRegister` 20 (T33 equip surface), `LEAVE_FIELD_REDIRECT` 27 (resolve-time redirect), `Duel.ChangeTargetCard` 17 + `ChangeAttackTarget` 10 (retarget story), `ChainAttack` 16, `SkipPhase` 7 (skip_phase story), `Card:CancelToGrave` 35, `Card:SetCardTarget` 25, `Card:CopyEffect` 16, `cost-header-mismatch` 24 (header-fix pass, skip class 10) | — | — | grammar stories already queued or new | — |

**Recommended next three.** (1) Stat-chip decode widening (translator-only, biggest sole-blocker pool, and every later candidate's chains flow through the same decoders); (2) ChangePosition matrix (one decoder, 89 blocks behind a single `Duel.*` method — settle the bare-`change_position` semantics first); (3) the code-table chore (an afternoon, ~30 blocks, and it reuses candidate 1's decoders so it should ship right after it). Re-run `explain` after each slice; the buckets shift as gates move.

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
