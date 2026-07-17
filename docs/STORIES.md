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
| **S2** | **Qualified player floodgates** | `:no_op` inner `EFFECT_TYPE_FIELD` + player `SetTargetRange` + `RESET_PHASE\|PHASE_END`, `Duel.RegisterEffect(e1,tp)`. CANNOT_SPECIAL_SUMMON 92 (79 with a `splimit` qualifier), CANNOT_ACTIVATE 41, tr=yes reset bucket 236 chains. Unfiltered subset is T36-expressible today; the qualifier extension unlocks the rest. | **~70–90** | Small extension: `restrict_action` gains `("from" ~ zone)? ~ ("except" ~ "(" ~ exempt_expr ~ ")")?` where `exempt_atom = race/attribute/archetype/named/card_type/from-zone`, or/and composition. |
| **S3** | **Snapshot negation pairs** | `:no_op` DISABLE + DISABLE_EFFECT clone on same receiver, same reset (43/61 cards paired), receiver ∈ {GetFirstTarget, GetTargetCards, group loop}, reset ∈ {end_of_turn, while_face_up}. | **~30** | **NONE** — `negate_effects <selector> <duration>` exists (pest); one DSL action lowers to the two-code pair. Field-wide "lingering" variant (18 regs) is a skip class this round. |
| **S4** | **Ritual proc recognition** | `Ritual.AddProcEqual/Greater[Code]` single-call cards; filter reduces to code-list/archetype/race; lv nil or int; no extrafil/extraop/stage2/matfilter/CreateProc. 68-card bucket minus skips. | **~35–40** | Tiny: `summon_level` expr atom ("the summoned monster's own level") in the `where total_level` clause. `ritual_summon_action` exists (pest:519). NOTE: dormant branch `origin/worktree-ritual-addproc-assigned` (2026-06-10) already has assigned-form Ritual.AddProc* skeleton recognition + a RegisterSummonEff pass — inspect/rebase before starting S4/S5 fresh. |
| **S5** | **Fusion proc recognition** | `Fusion.CreateSummonEff` (32) / `SummonEffTG+OP` pair (36); whitelist extraop ∈ {nil, BanishMaterial, ShuffleMaterial}, gc ∈ {nil, ForcedHandler}, unconditional extrafil, no fcheck/chkf/stage2. | **~30** | **✓ grammar half shipped (specced + shipped in same PR, T36/T37 style)**: `fusion_summon_action` gains `fusion_plus_clause? ~ fusion_forced_self? ~ fusion_disposal?` = `("plus" ~ selector)? ~ ("including" ~ "self")? ~ ("sending_materials_to" ~ ("banished"\|"deck"))?` — full seam (pest + AST + parser + validator + compiler + fmt + trait widen + MockRuntime, tests per layer; corpus check + fusion-file fmt byte-identical). `plus` narrowed to `?` (one extrafil per proc; multi-zone pools ride the selector's zone list) and `gy` dropped from the destination set (absent clause = default GY disposal — one canonical spelling). Validator: `plus` pool must be a structured selector (error); non-`all` pool quantity warns (fcheck class, not carried through the seam). **ygobeetle mirror outstanding (engine-dev)** — the adapter does NOT override `fusion_summon`, so the dep bump compiles clean and the knobs are silent no-ops via the trait default; schedule on semantics, no build break will surface it. Translator recognition/apply = PR 2. |
| **S6** | **Card-method ops** | initial-only `card_method_action` (92): single mutating Card method after standard guard — UpdateAttack/Defense/Level → `modify_*`; NegateEffects → `negate_effects`; AddCounter/RemoveCounter → `counter_action`; RemoveOverlayCard → `detach`. Args literal or `count(selector)`. | **~50–60** | **NONE** — all five target actions exist. |
| **S7** | **Simple untranslated primitives** | initial-only `simple_untranslated` subset whose primitive already has a DSL action: ChangePosition 38 (incl. 4-arg toggle → bare `change_position`), SynchroSummon 10, SSet 7, XyzSummon 3, Summon 3, misc. | **~45–55** | **NONE** for this subset. (chain_attack 14, retarget ~21, skip_phase 8, redirect_attack 7, activate_field_spell 6, summon_or_set 4 are each <30 → deferred; retarget/skip_phase are already queued as their own grammar stories.) |
| **S8** | **Delayed-trigger wrapper** | the true `:with_op` class (176 stems): inner FIELD+CONTINUOUS effect on an EVENT_* code with its own SetOperation + reset. Led by EVENT_PHASE+PHASE_END 53, then CHAIN_SOLVING 12, BATTLE_DESTROYING 10, PHASE_STANDBY 10, DAMAGE_STEP_END 9, CHAIN_SOLVED 9, CHAINING 8. MVP: PHASE_END bodies whose inner op is 1-2 already-translatable actions + the `cnt=yes` "once, later this turn" bucket (20). | **~40–55** | **NEW** — the `delayed on <event>` wrapper (below). |

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

// S2: qualifier rides the existing T36 method as an Option, not a new method.
fn restrict_player_filtered(&mut self, player: u8, restriction: PlayerRestriction,
    qualifier: RestrictionQualifier, duration: Duration) {}
// RestrictionQualifier { from_zone: Option<Zone>, except: Option<ExemptExpr> } —
// runtime-surface mirror enum family, same pattern as PlayerRestriction/DamageRule,
// doc-comment table mapping to splimit predicate shapes. Evaluated per would-be-summoned
// card at BOTH seams (action-gen mask + exec pre-check), per the PR #50 rollout pattern.

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

// S8: event-armed recurring listener.
fn register_delayed_trigger(&mut self, event: TriggerEvent, ops: DelayedOps,
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
