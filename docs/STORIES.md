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

### T39 — post-T38 translator-only candidates

Per-block triage of every empty `resolve { }` left after T38 (2,327 blocks / 2,061 cards): `chain-unlowered` 1,049 blocks is the largest reason, and within it the **decode-refused stat chip** (inner `EFFECT_TYPE_SINGLE` with `SetCode` ∈ {UPDATE_ATTACK 218, SET_ATTACK_FINAL 68, UPDATE_LEVEL 54, CHANGE_LEVEL 40, UPDATE_DEFENSE 6, …}) is the largest translator-only class. Slices below are ranked by safe yield; each is a translator PR + corpus PR per the translator-phase pattern.

| # | Slice | Sub-shape | Yield | Status |
|---|-------|-----------|-------|--------|
| **S1** | **Stat-chip decode widening** | see below | 33 blocks / 30 cards | ✓ shipped (PR #166) |
| **S2** | **`Duel.ChangePosition` 5-arg matrix** | see below | 24 blocks (18 new + 6 reshape refills) | ✓ shipped (PR #167) |
| **S6** | **Code-table chore** (triage candidate 3) | see below | 7 blocks / 6 cards | ✓ shipped |

#### T39 S1 — stat-chip decode widening (translator-only) ✓

**Audit.** 382 stat-chip `chain-unlowered` blocks; 254 are "sole-blocker" (nothing else in the handler drops). By `SetValue` shape (sole): local-binding 71, literal 62, arith 54, `e:GetLabel()` 47, stat-read 17, other 3. By receiver: `tc` 143 (GetFirstTarget 48, `g:GetFirst()` of a `GetMatchingGroup` 26, select-pick / loop-var / event-group the rest), `c` 58, battle-context (`a`/`at`/`d`/`bc`) 26, label-object / `rc` / `eg` 27. By reset: `RESETS_STANDARD_PHASE_END` 108, `RESET_EVENT|RESETS_STANDARD[_DISABLE]` 86, mapped `PHASE_DAMAGE[_CAL]` 22, unmapped (`,2` counts, `RESET_OPPO_TURN`, `PHASE_BATTLE`, `&~RESET_TOFIELD`) 38. Reading ≥5 cards per bucket showed the census over-counts: most "literal / tc" refusals are if/else or `SelectYesNo`-conditional values, reset counts of 2, or label-object receivers — all hard skips. The real decode gaps were in the **value atoms** and the **filter predicate table**, not the receivers.

**MVP (emitted).** All in `src/lua_ast.rs`, no grammar change:
1. **Named-filter counts** — `parse_lua_value_ctx` threads the walked function table into `count_call_to_count_expr`, so `Duel.GetMatchingGroupCount(s.filter, …)` resolves through `named_filter_to_where` (same exactness gate as the group-modifier refinement; a filter taking a trailing param skips). Stock predicates with ONE trailing filter arg (`Card.IsSetCard,…,nil,SET_LAVAL`) map through `map_group_filter_with_extras`.
2. **`#g` group length** → `count(<selector>)` for a mapped, `all`-quantity, non-select, extras-free group binding (The Big March of Animals).
3. **Predicate atoms** `Card.HasLevel` → `is_monster and not is_xyz and not is_link` (utility.lua:744 — its EFFECT_RANK_LEVEL carve-out names a code the DSL has no model for, so inside the DSL's model the form is exact), `HasNonZeroAttack`/`HasNonZeroDefense` → `is_face_up and atk/def > 0`; bare stock use (`Card.HasNonZeroAttack` as the group filter) and named-filter atom use both covered. New setcodes: Blackwing, Constellar, Deskbot, Gagaga, Ghostrick, Performage, War Rock.
4. **Life-point reads** `Duel.GetLP(tp)` / `Duel.GetLP(1-tp)` → `your_lp` / `opponent_lp` (Endless Decay, Exodia Incarnate).
5. **Lua `//`** → DSL `/` (floor == truncate on non-negative stats); **`tc:GetLink()`** → `target.link`.
6. **`math.ceil(x/N)`** → `(x + N-1) / N` — the exact integer identity under the compiler's integer division; the grammar's `( expr )` atom carries it (Gouki Bearhug, Eternal Sunshine, Lionhearted Locomotive, Dark Anthelion).
7. **Value-side target provenance gate (policy flip, retrofitted).** `method_call_to_stat` lowered every `tc:Get<Stat>()` to `target.<stat>` on spelling alone; the receiver path has always required `tc` to be `Duel.GetFirstTarget()`-bound. The stat-chip families now apply the same gate to values (`value_tc_is_declared_target`). `audit-fills` diff against the base binary: 15 handlers lose a `target.`-valued rendering, every one hand-checked as a select-pick / attacker / battle-target / operated-group `tc` (Boreastorm, Jurrac Ptera, Magistus Chorozo, Desavewurm, Wrecker Panda, Deskbot 004, …); ONE shipped line was affected — c41209827 Starving Venom Fusion Dragon `modify_atk self + target.atk until end_of_turn` (tc = `SelectMatchingCard` pick, the block declares no target) — stubbed by hand in the corpus commit (audit-fills' `--stub` only auto-stubs fills the body still renders).

**Skip classes (documented, counts = sole-blocker blocks).** `e:GetLabel()` label plumbing 47 (T38 class 3); battle-context / label-object / reason-card / event-group receivers 53 (class 6); non-mapped resets 38 (class 4); branch-conditional values (if/else `SetValue`, `SelectYesNo`, `and/or` ternaries) and reassigned locals ~20 (class 2/5); `Duel.AnnounceLevel` / `GetChainInfo` values 10; `GetSum` / `GetClassCount` / `tc:GetOverlayCount()` / `GetMaterialCount` / `GetOriginalLevel` / `GetTextAttack` reads 14 (**grammar backlog**: `target.overlay_count`, `original_level`, a `sum(selector, stat)` expr); `c:IsCode(id)` named filters 7 (needs the card's own name — **backlog**: `name == self.name` or a cdb lookup at emit); `Card.IsPosition` filters 3 (**backlog**: a `SelectorSpec` position slot); or-under-conjunction filters (`aux.FaceupFilter(Card.IsRace,A|B)`, `not (A and B)`) 3 — the v2 paren-predicate parser now has a passing test, so the "broken paren branch" gates in `map_group_filter` / `lua_filter_expr_to_where` can be lifted in a follow-up (shared-mapper flip → audit-fills sweep); named filters with local aliases (`local lv=c:GetLevel()` then `return … lv>0`) 1; loop values reading the declared target while the loop var is not `tc` (Sylvan Dancepione, Snowdrop) 4 — every instance also carries a group extra arg, so no yield.

**Yield.** 33 blocks / 30 cards filled (+1 stale partial fill stubbed, +1 policy-flip stub): 2,327 → 2,294 errors, 1,357 → 1,352 warnings. Against the census' 314-block "sole-blocker" estimate the true decodable share was ~13% — the brief's "20–50% is real" was itself optimistic for this class because the census counts a handler as sole-blocked whenever the chip is its only *drop*, which includes every conditional-value and foreign-receiver chip. Side yield from the shared mapper widenings: Constellar Omega (immune grant), Mischief of the Wolves (S6 method ops), Uni-Zombie (Pass S refined a lone `discard` stub — note `Duel.DiscardHand` renders `(1+, …)` regardless of its min/max args, a pre-existing lossy renderer worth its own fix).

#### T39 S2 — `Duel.ChangePosition` 5-arg position matrix (shipped)

**Ruling (the DSL contract for the two `change_position` forms).**

- `change_position <sel> to <pos>` puts EVERY selected monster in `<pos>` whatever its current position (`defense_position` = face-up defense, so a face-down card is flipped). Lua: the 1-arg form, and any matrix whose admitted arms agree.
- Bare `change_position <sel>` is the game-rule "change its battle position": face-up attack ↔ face-up defense, and a face-down monster is flipped to **face-up attack** (Card Rotator's own parenthetical; the Burei/Dream Cicada Q&A). Lua spells it `(POS_FACEUP_DEFENSE, *, POS_FACEUP_ATTACK, POS_FACEUP_ATTACK)` — the `ad` arm varies (`FD_DEF` / `FU_ATK` / `FU_DEF`) across cards with identical text. The S7-era reading ("bare = toggle, face-down untouched") was the `(FU_DEF, 0, FU_ATK, 0)` spelling, which the corpus uses only under a face-up guard/filter.
- Face-down ATTACK is unreachable in play (Darkness Approaches aside); the decoder ignores the `ad` arm and never admits that position. `0` arms = unchanged (ocgcore); a missing arm defaults to `au`; the 6th/7th args (`noflip` — "Flip effects are not activated" — / `setavail`) have no DSL surface.
- ygobeetle mirror (backlog, adapter): `ds_runtime_adapter.rs::change_position` is a 3-cycle (FU_ATK → FU_DEF → **FD_DEF** → FU_ATK); the FU_DEF → FD_DEF step matches no lua form. It must become FU_ATK ↔ FU_DEF, FD_* → FU_ATK (a flip). `change_position_to` is already exact (it also emits `PositionChanged` for an identity move — harmless, note only).

**Decoder (src/lua_ast.rs `action_change_position`, new `FunctionBody::position_guards`).** The walk records, per `Duel.ChangePosition(<var>, …)` first argument, the current positions the enclosing guards admit (`<v>:IsFaceup()` / `IsFacedown()` / `IsAttackPos()` / `IsDefensePos()`, with `not`, over top-level `and` conjuncts of enclosing `if`/`elseif` conditions, plus `if A or B then return end` early-outs narrowing the rest of the block; several sites on one variable take the UNION). The rendered selector's `where is_face_up` / `is_face_down` conjunct narrows further. Then: all admitted arms one literal position → `to <pos>`; all admitted arms the ruling's toggle → bare form; all admitted arms identity → TODO (no-op); else TODO. `c:GetEquipTarget()`-style derived receivers are refused (they rendered as `self` before — 3 shipped mis-aims).

| bucket (90 blocks before, by matrix shape) | blocks / cards | verdict |
|---|---|---|
| A `(FU_DEF, *, FU_ATK, FU_ATK)` game-rule toggle, provenance ok | 36 / 29 | **14 filled** (bare form; 10 cards incl. the 2-block Superheavy Samurai quartet); **20 `target-decl-undecodable`** — the fill is `change_position target`, the block has no decl and `s.postg` carries `Card.IsCanChangePosition` (unmapped atom → candidate 4); 1 cost-header-mismatch, 1 incomplete |
| A, provenance unknown (`d=Duel.GetAttackTarget()`, chain-info filters) | 2 / 2 | skip (candidate 4) |
| B `(FU_DEF, 0, FU_ATK, 0)`, provenance unknown | 2 / 2 | skip (candidate 4) |
| C uniform 4-arm `(FU_DEF ×4)` Hyades | 1 / 1 | **filled** `to defense_position` |
| D `(0,0,FU_ATK,0)` Naga (face-up defense guard) | 1 / 1 | **filled** `self to attack_position` |
| D `(FU_ATK,FU_ATK,FU_DEF,FU_DEF)` on `Card.IsFacedown` groups (Swords of Revealing Light, Dark-Piercing Light) | 2 / 2 | **filled** `… where is_face_down) to defense_position` |
| D `(0,0,FU_ATK,FU_ATK)` Stop Defense / Barses / Reptilianne Poison, `(FD_DEF,0,FD_DEF,0)` Erokin, `(FU_DEF,FD_DEF,0,0)` Water Dancer, `(FU_DEF,FD_DEF,FU_DEF,FD_DEF)` Fane | 8 / 7 | decoder renders `to <pos>` under the body guard (unit-tested) but every block stops at `target-decl-undecodable` (same `IsCanChangePosition` atom); Lose 1 Turn (no guard, `GetTargetCards`) stays TODO |
| E 3-arg `(FU_DEF, FD_DEF)` Soldier Gaia / World of Spirits / Borrelsword | 3 / 3 | renders `to defense_position`; `target-decl-undecodable` |
| F uniform 2-arg, provenance unknown (`eg:Filter`, `Duel.GetAttacker()`, chain-info filters, column groups) | 25 / 19 | skip (candidate 4) |
| noflip 6-arg (Amazoness Archers, VW-Tiger Catapult, Dark Gaia, Tin Archduke, …) | 9 / 7 | skip — grammar backlog: a `without_flip` modifier (or `flip <sel> face_up`) |
| Staplen `(FU_ATK,0,FU_ATK,0)` on `e:GetHandler()` under a guard on the OTHER variable | 1 / 1 | skip (correct: admitted set unrestricted) |

**Policy flip / retrofit.** `audit-fills` could not see this flip (a TODO-only body is not a `BodyDrop`), so it grew `--shape <action>`: a RESHAPE class for shipped resolves the gate still accepts whose `<action>` lines no longer byte-match the current rendering; `--stub` empties them and `apply` refills under the corrected decoder. 11 hits, all hand-verified: 3 `c:GetEquipTarget()` mis-aims (Ailin, Second Goblin, c50418970 → stub), Cloudian Eye of the Typhoon + Windstorm of Etaqua (`(FU_DEF,0,FU_ATK,0)` on a lossy selector that lost its `IsFaceup` → the bare form would flip face-down cards → stub), Junk Sleep ×3 + Tsuchinoko + Ultramafus (pre-existing selector drift `where is_face_down` / `except self`, refilled exactly; Junk Sleep block 3 was a positional-residue `to attack_position` where the lua says `to face_down_defense`), WAKE CUP! Mocha (positional residue `target to attack_position` → `self to face_down_defense`). The other 16 cards with shipped bare lines re-derive byte-identically (all carry `IsFaceup` guards or `Card.IsFaceup` filters).

**Yield.** 24 effects filled (18 new + 6 reshape refills) + 1 Pass S refinement (Berry Magician Girl: lone `special_summon` stub → `change_position self` + `special_summon (1, card, you control, from deck)`), 5 resolves stubbed. Check 2,327 → 2,313 errors (−14), 1,357 → 1,355 warnings (−2). Against the ~50 estimate: the decoder now renders 50 of the 52 matrix-form blocks with known provenance (Staplen and Lose 1 Turn are the two honest refusals), but 30 of them sit behind `target-decl-undecodable` on one atom — mapping `Card.IsCanChangePosition` in the named-filter refinement (candidate 4) releases them for free. 761 lib / 23 bin tests, zero warnings; second fixed-point run a no-op.

**Backlog.** (1) `Card.IsCanChangePosition` / `IsCanTurnSet` atoms (candidate 4; ~27 blocks here + the lossy `where`-less selectors of Hyades / Curse of Fiend / Registrider / Etaqua); (2) `noflip` grammar (9 blocks); (3) ygobeetle adapter bare-form cycle fix (above); (4) `Duel.SelectPosition` player-chosen positions (8 blocks: `change_position <sel> to (attack_position | face_down_defense)` choice form) and `Duel.TossCoin` arms (2).


#### T39 S3 — selector filter fidelity ✓ shipped (policy flip + retrofit)

**Defect.** `spec_from_matching` / `spec_from_get_matching` built action selectors from `Duel.GetMatchingGroup` / `SelectMatchingCard` / `SelectTarget` args and DROPPED every lua filter outside a six-entry literal table (`map_group_filter`): the `filter_mapped: false` flag only protected group-applied stat modifiers, while `destroy` / `special_summon` / `add_to_hand` / … rendered the unfiltered superset and the engine executed it (Mirror Force c44095762: `destroy (all, card, opponent controls, from monster_zone)` for a `c:IsAttackPos()` filter). Only exactly-mapped named filters on GROUP-provenance specs got a where-clause (T38 S1 `refine_group_bindings`); select-provenance specs never carried their filter at all.

**Census (main @ aaec3f9df, 6,095 action selectors over 13,298 cards).** 5,761 (94.5%) had `filter_mapped=false`; 1,770 shipped resolve lines byte-equalled such a selector's rendering.

| Raw filter class | selectors | shipped lines |
|---|---|---|
| named `s.*`, pure single-return body | 4,404 | 1,646 |
| `aux.*` combinator (`NecroValleyFilter` 620, `FaceupFilter` 50, `AND` 25) | 741 | 42 |
| inline `Card.X` | 369 | 62 |
| named `s.*`, multi-statement body | 162 | 16 |
| unexpressible exception arg (`tc`, `exc`, a group) | 79 | 2 |
| other (`Card.IsCode` + extras, closures, …) | 6 | 2 |

Residual atoms (each selector counted once per atom; shipped-line column = lines whose selector carried it): `IsSetCard` 1,897 / 677 (452 distinct `SET_*`, the curated table had 57) · `IsAbleToHand` 1,564 / 772 · `IsCanBeSpecialSummoned` 1,484 / 425 · `IsCode` 1,234 / 468 (`id` 635, literal 264, param 74, `CARD_*` 56) · `aux.NecroValleyFilter` 662 / 24 · non-`c:` leaves (`Duel.GetLocationCountFromEx(…)>0`, `mc:IsCanBeXyzMaterial`, …) 488 / 65 · `IsAbleToGrave` 389 / 148 · `IsSpellTrap` 325 / 161 · `IsSSetable` 158 / 89 · `IsType(TYPE_EQUIP|FIELD|UNION|…)` 126 / 44 · `IsAbleToRemove` 105 / 16 · `ListsCode` 97 / 61 · `IsRace`/`IsAttribute` with a param or alias 159 / 37 · `IsSummonable` 64 / 42 · `IsAbleToDeck` 63 / 6 · `IsAttackPos`/`IsDefensePos`/`IsPosition` 62 / 27 · `GetCode()` compares 38 / 18 · `IsCanChangePosition` 33 / 13 · `IsAttack`/`IsDefense` 52 / 25 · `IsSpecialSummoned` 18 / 9 · `IsPublic` 21 / 0.

**Policy flip (src/lua_ast.rs, selector/filter region only).** ONE shared mapper — `resolve_filter` → `lua_filter_expr_to_map` → `atom_map` → `card_pred_to_where` — resolves inline `Card.X` (call extras = its args), `aux.FilterBoolFunction` / `FaceupFilter` (bound args replace the extras, as utility.lua does) / `NecroValleyFilter` (varargs pass through) / `AND` / `NOT`, pure named `s.*` filters (later params bound positionally to the call's extra args, the script `id` to the current card) and inline closures into a `FilterMap { atoms, position, implied, residual }`; `lua_filter_expr_to_where` is its exact projection (so target declarations and stat-chip groups ride the same widening). New atoms, all on existing grammar: `IsAttackPos`/`IsDefensePos`/`IsPosition(POS_*)` → the `position_filter` slot (`in attack_position`; shares the slot with `except self` — the pair refuses), `IsCode(N | id | CARD_X | a,b)` and `GetCode() ==/~= N` → `name == "…"` via the cdb name table + `card_counter_constants.lua`, `IsAttack`/`IsDefense` exact, `IsAttributeExcept` → `not attribute ==`, `IsSpellTrap` → `is_spell or is_trap`, `IsPendulumMonster`, `IsTuner`, the `IsXSpell`/`IsXTrap` helpers and `TYPE_EQUIP/FIELD/QUICKPLAY/COUNTER/CONTINUOUS/GEMINI/UNION/SPIRIT/TOON` → `type == "…"`, the `RACES_BEAST_BWARRIOR_WINGB` alias, and `IsSetCard(SET_X)` for every constant of `archetype_setcode_constants.lua` whose words match ≥85% of its cdb members' names (`src/setcode_names.rs`, 20 pinned overrides — `tellarknight`, `F.A.`, `rokket`, …). Mixed `and`/`or` renders lua's precedence through the v2 parser's `Nested` branch (`(is_spell or is_trap) and archetype == "X"`; the "paren branch is broken" note was stale — task_bb225d5f landed). **Transparent legality predicates** (`implied_by_action`, each verified on ≥3 cards whose text names only the action): `IsAbleToHand` ← SendtoHand, `IsAbleToGrave` ← SendtoGrave/Release, `IsReleasable[ByEffect]` ← Release, `IsAbleToRemove` ← Remove, `IsAbleToDeck`/`IsAbleToExtra` ← SendtoDeck, `IsCanBeSpecialSummoned` + the free-zone leaf `Duel.GetLocationCountFromEx(…)>0` ← SpecialSummon[Step], `IsDestructable` ← Destroy, `IsSSetable` ← SSet, `IsSummonable` ← Summon, `IsCanChangePosition`/`IsCanTurnSet` ← ChangePosition, `IsControlerCanBeChanged`/`IsAbleToChangeControler` ← GetControl, `aux.NecroValleyFilter` ← any action; `…AsCost` variants ride along. They never block their own action and ALWAYS block any other consumer (chain receivers, equip helpers, a different action). **The gate:** every action selector (`group_arg` → `SelectorSpec::action_dsl`, incl. Overlay/Equip/GetControl/ChangePosition and the Phase 17 one-card-selection chain receivers) refuses on any residual — unmapped atom, foreign binding, impure filter, unexpressible exception arg, `g:Filter(…)` suffix, position+except-self slot clash — as a TODO line plus `BodyDrop::SelectorFilterUnmapped` (`FillReason::Drop`, code `selector-filter-unmapped`, detail `Duel.X(arg): <residual>`), so Pass A/A2/S skip and `explain` names it. `spec_from_matching` now also keeps kind-only filters (`Card.IsSpell` → `spell`, not `card`). Nothing is deliberately left over-approximated.

**Retrofit (standing tool).** `audit-fills` renders every shipped block under the current policy AND the pre-gate legacy policy (`with_legacy_selector_policy`: exact named-filter refinement on group-provenance specs only, superset rendering, no gate); a block equal to the legacy rendering is a machine fill the flip invalidated and `--stub` empties it. Round 1: 1,748 stubs (1,348 `selector-policy` = tighter rendering now, 383 `selector-filter-unmapped`, 17 pre-existing helper/chain drops) → `audit-owners --stub` 0 misplaced (288 coincidental / 268 owner-unassigned / 48 alignment-suspect, report-only) → `apply` 1,348 (+70 co-emitted target declarations, 62 Pass S stub refinements) fills; round 2: 0 stubs / 0 fills / `stubs refined: 0`, `git status` clean. `duelscript check` (main @ 70158a135, post-S1/S2): 2,280 → 2,650 errors (+395 refused, −25 refilled), 1,350 → 1,409 warnings (+81 `cost but no resolve` on refused blocks, −22: 17 bare-`target` warnings cleared by the co-emitted declarations). Tests 772 → 791 lib + 23 → 24 bin, zero warnings.

#### T39 S4 — `Duel.DiscardHand` exact min/max + shared filter mapper ✓ shipped (policy flip + retrofit)

**Defect.** The resolve-side `Duel.DiscardHand(player, filter, min, max, reason[, ex, …])` arm rendered a constant `discard (1+, card, you control, from hand)` — player, filter and count all dropped (Uni-Zombie c49959355 "discard 1 card" shipped as `1+`; Forced Requisition's `1-tp, ct, ct` the same line). The cost-side arm (`extract_cost_block`) already decoded min/max; an older translator had also left `discard (all, card, either controls)` placeholders for the same call.

**Census (main @ 0a0faa0ae).** 559 shipped `discard (` lines: 194 cost `1` / 8 cost `2` / 2 cost `5` / 1 cost `1+` / 175 cost `all` (`Cost.SelfDiscard`-era placeholders — a different, unowned renderer) and, in resolve blocks, 33 `1+` + 146 `all` — every resolve line derives from `Duel.DiscardHand`. Lua count shapes over the touched cards: `tp,1,1` ×107, `1-tp,1,1` ×22, `tp,2,2` ×3, `p,…` ×5, computed (`ct`, `lv`, `ac`) ×5, `tp,1,60` ×1, `tp,1,3` ×1.

**Renderer rule (`discard_hand_spec` / `action_discard_hand`, src/lua_ast.rs).** Literal `min == max` → exact `N`; the open-ended `(N, 60)` "any number" idiom → the grammar's at-least `N+` (the same reading `quantity_from` gives a `SelectMatchingCard` range; `N..M` does not exist); a bounded literal range (`1, 3`, The Cheerful Coffin) or a computed count refuses as a TODO. `tp` → `you control`, `1-tp` → `opponent controls`, any other player refuses. The filter rides the shared predicate mapper (`refine_spec` → `resolve_filter`): `IsDiscardable` / `IsAbleToGrave` are transparent legality atoms for `Duel.DiscardHand` (c11449436, c29599813, c47778083), any other residual refuses through `SelectorSpec::action_dsl` and the fill gate reports `selector-filter-unmapped`. The `ex` arg follows `classify_exception` (`c` → `except self`, other non-nil refuses). Cost placement is unchanged: the call renders where it appears (cost handler → `cost { }`, operation handler → `resolve { }`); the cost arm shares `discard_quantity` (corpus output byte-identical). At runtime `Quantity::Exact(n)` and `AtLeast(n)` both pick exactly `n` (`select_cards(n, n)`) and `CostAction::Discard` ignores its selector entirely (always 1 card) — compiler-side gaps, noted for the compiler session.

**Retrofit.** `audit-fills --shape discard` now fires the RESHAPE class regardless of the body's drop verdict (a shipped `<action>` line the decoder would not write is wrong whether or not the rest lowers; hits print `drop=<code>`). Round 1: 156 blocks stubbed (113 drop-free, 43 with a sibling drop: 24 chain-unlowered, 10 selector-filter-unmapped, 7 method-poison, 1 counter-op, 1 helper) → `audit-owners --stub` 0 misplaced → `apply` 56 refilled (+2 co-emitted target declarations; 44 `discard (1, card, you control, from hand)`, 10 `opponent controls`, 5 with a where-clause); round 2: 0 reshape / 0 stub / 0 fills / `stubs refined: 0`, `git status` clean. 165 wrong lines removed (144 `all`, 21 `1+`), 59 exact lines written. `duelscript check`: 2,650 → 2,740 errors (+90 empty resolves the body gate refuses), 1,409 → 1,422 warnings. Tests 794 → 802 lib + 24 bin, zero warnings. **Left behind (report-only):** 12 `1+` + 2 `all` resolve lines (c1197847, c1218214, c22501005, c26162470, c30336082, c36849933, c38105306, c39568067, c4841383, c49328340, c74923978, c75884822, c97803170) in blocks owned by bare `EFFECT_TYPE_ACTIVATE` chains with no operation handler — the `audit-owners` owner-unassigned class, out of every re-derivation tool's reach; 175 cost-side `discard (all, card, either controls)` placeholders (no cost-block audit exists); the cost arm still ignores the `ex` arg (the ~20 `REASON_COST…, e:GetHandler()` shapes render without `except self`) — reading it there needs a cost-block audit before the flip can be retrofitted.

#### T39 S5 — modal-branch fill gate + resolve-time `Duel.Release` refusal ✓ shipped (policy flip + retrofit)

**Defect.** Pass A's body walk (`collect_duel_calls` and every sibling extractor) descends into every `if` / `elseif` / `else` arm and emits the arms' statements in sequence. A handler whose arms are selected at resolution — by a `Duel.SelectEffect` / `SelectOption` / `SelectYesNo` / `SelectEffectYesNo` result, directly or through a local, or by the stored `e:GetLabel()` the target handler wrote — therefore filled the UNION of mutually exclusive arms. Flagship c29876299 Megalith Anastasis effect 3 shipped `draw 2 / shuffle_hand / discard 1 / send (1, …, opponent controls, from monster_zone) to gy`: both arms of a "draw 2 then discard 1 OR tribute 1 opponent monster" choice, and the tribute spelled as a send-to-GY. The Phase 16 choose path decodes only the static two-plus-option `Duel.SelectOption` + contiguous `==k` ladder; the S1-era `in_choice_arm` marker and the #117 either/or guard cover chain slots and counter / method ops, not action lines.

**Gate (`FunctionBody::modal_fork`, `BodyDrop::ModalBranch`, code `modal-branch`, detail = the discriminant).** `extract_modal_fork` collects every local / assignment whose RHS carries a modal discriminant, then finds the first `if` whose condition (or an `elseif` condition) reads one and whose arms carry effectful content (a `Duel.*` call that is not a cosmetic / query skip, a register chain or `RegisterEffect` commit, a counter / method op or poisoned family, a helper call). Excluded by construction: a `GetLabel`-guarded bare early-out (`if e:GetLabel()==0 then return end` — a recorded condition like any `IsRelateToEffect` guard), the T38 S8 `GetFlagEffectLabel(…) ==/~= e:GetLabel()` staleness idiom, `re:GetLabel()` (another effect's label), and forks whose arms are UI-only. A prompt inside a bare early-out (`if #g==0 or not Duel.SelectYesNo(…) then return end`) IS a fork — the player declines the whole tail. `body_drop_reasons` lists the fork first and keeps listing the arms' own blockers, so `explain` shows `modal-branch | +N drops, M todo`. The gate sits AFTER the Phase 16 choose attempt in `plan_fill`, so every choose shape that decoded before still decodes (unit test `modal_branch_choose_path_still_decodes_static_select_option`; corpus: no choose block lost, one gained — Card Shuffle, below).

**Census (audit-fills differential, base binary vs gated binary on main @ 247f85f6e).** 499 blocks newly flagged: 495 `modal-branch` + 4 Release. By fork shape (total / exact machine fills stubbed): SelectYesNo / SelectEffectYesNo 303 / 65; GetLabel ← SelectEffect in the target handler 75 / 1 (Megalith); op-side dynamic SelectOption 36 / 1 (Card Shuffle — the else-arm shape the choose path handles once the block is empty); op-side SelectEffect 28 / 1 (The Duke of Demise: pay 500 or destroy self); GetLabel ← a non-Select verdict stored by the target (flag / announced attribute / which card survived) 16 / 3 (Revenge Rally, DNA Checkup, Doomstar Ulka) + 23 / 0 with no Select anywhere; GetLabel ← dynamic SelectOption in the target 14 / 1 (Ghostrick-Go-Round). 62 of the 72 modal stubs were fully-flattened (every arm's statement rendered); 10 had a dropped statement besides (pre-S1-era partial fills). The other 423 are REVIEW / report-only: generation-era placeholders (`special_summon (all, card, either controls)`, `return … to deck`) in modal bodies — the same backlog S2b / S3 / S4 left report-only, now at least named by the gate. **Release:** 4 shipped `send … to gy` lines derived from `Duel.Release(…, REASON_EFFECT)` (Megalith, Teardrop the Rikka Queen ×2 `send target to gy`, Chef's Special Recipe `send (all, …, opponent controls, from monster_zone) to gy`) — all stubbed; `audit-fills --shape send` finds one more Release handler (c39468724) whose shipped `send (all, card, either controls) to gy` is the generator's SendtoGrave placeholder, not a Release line (report-only, like c78986941's `destroy (all, …)`).

**Release decision.** `grammar/duelscript.pest` has `tribute <selector>` ONLY as a `cost_action`; the resolve `action` rule has no tribute. A release is not a send-to-GY (it skips cards that cannot be tributed, fires "when tributed" triggers, is stopped by tribute floodgates), so `Duel.Release(…, REASON_EFFECT)` in an operation handler is now a TODO line (`no resolve-time tribute action (grammar \`tribute\` is cost-only …)`) — refused, not re-spelled. Cost-side `Duel.Release(c, REASON_COST)` keeps its cost-block `tribute` lowering. A resolve-time `tribute` action is a grammar story (needs a runtime seam + ygobeetle mirror) before these 4 + the ~60 Release handlers currently refused for other reasons can fill.

**SelectEffect widening — refused (below the ≥10 floor).** The option table is regular (`{b1,aux.Stringid(id,n)}` pairs, 311 / 310 / 48 sites for b1 / b2 / b3) and the dispatch is a 1-based `op==k` ladder, so a decoder is mechanical; but the `b_k` availability conditions are NOT transparent — Megalith's carry `not Duel.HasFlagEffect(tp,id)` per-mode once-per-turn flags the choose grammar cannot state — and the yield is ~0: `explain` on the empty blocks finds 4 op-side SelectEffect bodies + the GetLabel-via-SelectEffect subset, none a sole blocker (`+0 drops, 0 todo` = 1 block corpus-wide, a SelectYesNo); the 29 SelectEffect bodies whose arms DO all render carry generator placeholders that `apply` never rewrites. Backlog: a SelectEffect choose decoder gated on legality-only `b_k` conjuncts (IsPlayerCanDraw / IsExistingMatchingCard / IsExistingTarget / `#g>0` / `true`), paired with a stub pass over those 29 blocks.

**Retrofit (standing tool, extended).** The legacy policy switch became a bitset (`LEGACY_SELECTOR` = S3 widening + S4 discard line, `LEGACY_RELEASE` = S5): `with_legacy_selector_policy` reverts every flip, the new `with_legacy_release_policy` only the Release one — a block refilled under #171's exact discard line but before S5 (Megalith) matches neither the current nor the all-flips rendering. `audit-fills` compares shipped against all three, prints `legacy-release:` lines when they differ, and its STUB / REVIEW lines now carry `todo=N detail=<gate datum>` (the census instrument); the no-gate mismatch reason is `legacy-policy` (was `selector-policy`). Round 1: 75 stubs (72 modal-branch, 3 Release) → `audit-owners --stub` 0 misplaced (583 coincidental / 264 owner-unassigned / 47 alignment-suspect, report-only) → `apply` 1 fill: Card Shuffle re-emerges as a `choose { option "Shuffle my Deck" … option "Shuffle opponen't Deck" … }` block (CDB strs verbatim); round 2: 0 stubs / 0 fills / `stubs refined: 0`, `git status` clean. All 75 hand-verified against lua + card text (the PR table). `duelscript check`: 2,740 → 2,764 errors (+24 on 22 cards; the remaining 51 stubs sit on Field / Continuous Spell activation shells the validator exempts), 1,422 → 1,422 warnings (+2 `cost but no resolve`, −2 cleared). Tests 808 → 817 lib + 24 bin, zero warnings. **Left behind (report-only):** 423 modal bodies with generation-era content; 1 Release handler with a generator `send (all, …)` placeholder; the SelectYesNo class as a whole (303 bodies) — an `optional { }` / "you can" surface does not exist in the grammar, so "then you can draw 1" has no faithful spelling either way.

#### T39 S6 — code-table chore ✓ shipped (translator-only)

**Scope.** Triage candidate 3: inner `EFFECT_TYPE_SINGLE` chains whose `SetCode` had no decoder although the DSL "already has the surface" — 50 blocks / 47 cards on main @ e3bbffd8d (CANNOT_DIRECT_ATTACK 9, CANNOT_TRIGGER 18, CHANGE_LSCALE 8, UPDATE_LSCALE 4, CHANGE_LEVEL_FINAL 5, UPDATE_RANK 2, SWAP_BASE_AD 4; RSCALE 0). No grammar change.

**Grammar verification (the step the estimate skipped).** Only three of the seven codes have a surface that carries the chain's lifetime. `grant <sel> cannot_attack_directly | cannot_activate effects until <dur>` (pest `grant_action` / `grant_ability`) and `set_level <sel> <expr> until <dur>` (`set_stat_action`, `stat_name` = atk|def|level) are real. `set_scale <sel> to <expr>` (`change_property_action`) has NO `until` clause — `Action::SetScale` compiles to `rt.set_scale(card, v)` permanently — and every corpus CHANGE_LSCALE chain is `RESETS_STANDARD[_DISABLE]_PHASE_END`; `stat_name` has no `scale`, so UPDATE_LSCALE has no modify form at all. `change_rank <sel> to <expr>` is grammar-only: no `Action::ChangeRank` variant, no parser/compiler arm; `stat_name` has no `rank`. `swap_stats <sel>` has no duration and `rt.swap_stats` is permanent; all 4 SWAP_BASE_AD chains carry a reset. Emitting any of those would silently drop a lifetime — the exact failure the CHANGE_LEVEL→`set_level` note in `set_stat_action` refuses — so they are recorded as **grammar backlog** (`set_scale … until`, `modify_scale`, `rank` in `stat_name` + a `ChangeRank` AST arm, `swap_stats … until`), not emitted. Compiler fidelity notes, not blockers: `GrantAbility::CannotAttackDirectly` lowers to code 86 (`EFFECT_CANNOT_ATTACK_ANNOUNCE`) where lua's code is 73 (`EFFECT_CANNOT_DIRECT_ATTACK`), and `CannotActivate(_)` lowers to 2 (`EFFECT_DISABLE`) where lua's is 7 (`EFFECT_CANNOT_TRIGGER`) — the DSL phrases state the card text; the engine mapping is `grant_to_code`'s to fix.

**Emitted (src/lua_ast.rs).** `grant_ability_for` gains `EFFECT_CANNOT_DIRECT_ATTACK → cannot_attack_directly` (the field-family table already had it) and `EFFECT_CANNOT_TRIGGER → cannot_activate effects`, both through the existing Phase 4e in-resolve grant path (exact reset map, receiver provenance gates, shared group mapper for `aux.Next` loops) and the Phase 15 equip-rider fallthrough; `set_stat_action` gains `EFFECT_CHANGE_LEVEL_FINAL → set_level` (same `_FINAL` priority note as SET_ATTACK_FINAL). New gate, scoped to the two S6 codes: a `SetValue` must be absent or an integer literal (`chain_value_is_flag`) — `1` is the flag idiom and Gold Pride's `500` is `e1:Clone()` residue ocgcore never reads; a closure value is a per-activation condition the phrase cannot carry. The pre-S6 grant families keep their value-blind policy (a corpus-wide flag gate is a flip owed its own retrofit).

**Audit (explain TSV, per bucket, ≥5 cards read each).** CANNOT_DIRECT_ATTACK: 5 `tc` (GetFirstTarget) receivers, 2 `c`, 2 `ec` (battle-context); resets 4 PHASE_END, 1 event-only, 4 PHASE_BATTLE / DAMAGE_CAL compounds (refused). CANNOT_TRIGGER: 13 `tc`, 1 `c`, 1 `sc`, 1 `sg:GetFirst()` forced, 1 loop; values none/`1`/clone-residue `500`; 4 `with-cond` / `forced` slot refusals, 3 `RESETS_CANNOT_ACT` (= `RESETS_STANDARD&~RESET_LEAVE`, unmapped), 2 `_EXC_GRAVE`, 1 `RESET_SELF_TURN`. CHANGE_LEVEL_FINAL: value is always `lv = tc:GetLevel()` (→ `target.level`); 3 receivers are `aux.Next(g)` loops whose `g = GetMatchingGroup(…, tc)` EXCLUDES the target (Star Light Star Bright adds filter extras) — the selector grammar has only `except self`, so the shared mapper refuses (**backlog: `except target`**); Tannhauser Gate (`GetChainInfo` pair) and Six Samurai (`GetLabelObject`) are receiver/value skips. All FIELD-type chains stay on the Phase 5 / field-family path (CANNOT_TRIGGER is not added to `field_grant_ability_for` — out of scope).

**Yield.** 7 blocks / 6 cards, every fill hand-verified against lua + card text: Heroic Chance (`set_atk target target.atk * 2` + `cannot_attack_directly`, end_of_turn), Massivemorph (ATK/DEF doubled + `cannot_attack_directly`, `while_face_up` — the text has no "this turn"), Gold Pride - Pedal to the Metal! (+500 / indestructible ×2 / `cannot_activate effects`, with its co-emitted "Gold Pride" target decl), Performapal Seal Eel ×2 (Set S/T `cannot_activate effects`, face-down S/T-zone decl), Gorgonic Golem, Water Dragon Cluster (opponent Effect-monster group: `set_atk … 0` + `cannot_activate effects`). 2,740 → 2,733 errors / 1,422 → 1,421 warnings; `audit-owners --stub` / `audit-fills --stub` / `apply` all no-ops on re-run. Against the 25–35 estimate the shortfall is structural: 18 of the 50 blocks sit on codes whose surface lacks a duration, and the rest are the S1 skip classes (resets, slots, receivers, target decls). Post-apply the explain buckets read CANNOT_DIRECT_ATTACK 4 / CANNOT_TRIGGER 10 (all decode-refused now, none code-unmapped); Megingjord, Leo Arms, Vaylantz Duke, Chow Len, Druid Wid moved to `target-decl-undecodable` (set-list filters, `GetSequence`, label-object targets — Chow Len's chain is also label-guarded, so that gate is the right one). Policy-flip check (base vs S6 `audit-fills` on the pristine corpus): 0 stubs either side, no shipped S6-shape line re-renders; 9 pre-existing REVIEW blocks merely gain an S6 line while still refused elsewhere; 19 blocks LEAVE the review set because S6 completes their handler — their shipped bodies are generator category stubs that Pass S declined at `plan_fill` (bare-`target` renderings without a decodable decl), a measured harvest for the target-decl work.

**Bonus census (ChangePosition behind `target-decl-undecodable`, post-apply).** 86 blocks carry the reason; 41 of them are handlers that call `Duel.ChangePosition`, and in 34 the target fn (`s.postg` 27, `s.target` 7, …) reaches `IsCanChangePosition` (30) or `IsCanTurnSet` (4) — directly in the `SelectTarget` filter arg or via a named `s.filter`/`s.posfilter`. PR #169's transparent allowlist cannot help them: it lives in `SelectorSpec::action_dsl(method)` (`implied_by_action`), consulted by the ACTION consumer, whereas `extract_target_decl_refined` resolves the filter through `map_group_filter` → `named_filter_to_where` → `lua_filter_expr_to_where` — the mapper's own atom table with no `method` context (and not `resolve_filter`). A target decl is a legality surface, so the fix is not to mark the predicate transparent but to let the decl path accept a predicate the block's own resolve action implies (thread the owning handler's `Duel.*` method into the decl extraction, or record `filter_implied` on the decl spec and check it at co-emission).

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
