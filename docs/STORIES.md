# DuelScript Backlog — Stories

Active backlog of work items, ordered by yield. Each story states the goal, the agent, the acceptance criteria, and the dependency chain.

State as of 2026-05-01 — `cards/official/` parses 13,298 / 13,298, **3,470 errors** / **585 warnings** remain (post-Phase 6).

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

### Phase 7 — `s.cost` body extraction
**Goal.** Translate `s.cost` handler bodies into DSL `cost { ... }` blocks.

**Yield estimate.** ~200 cards.

**Approach.**
- Recognise common cost shapes: `Duel.PayLPCost(tp, N)`, `Duel.DiscardHand(tp, ...)`, `Duel.Release(c, REASON_COST)`, `Duel.Remove(c, POS_FACEUP, REASON_COST)`.
- Map to DSL cost actions: `pay_lp N`, `discard <selector>`, `tribute self`, `banish self`.

**Acceptance.**
- ≥ 4 cost shapes covered.
- Apply emits ≥ 150 cost blocks.
- Roundtrip + corpus check passes.

**Agent.** `lua-translator`.

---

### Phase 8 — `s.target` body extraction
**Goal.** Translate `s.target` handler bodies into DSL `target <selector>` declarations on the effect.

**Yield estimate.** ~400 cards.

**Approach.**
- Reuse existing `SelectorSpec` extraction (Phase 3a/b already extracts `Duel.SelectTarget(...)` calls).
- Promote it from "in-resolve binding" to "effect-level target" when the call appears in `s.target`.
- Skip targets with custom Lua filter closures (deferred).

**Acceptance.**
- Apply emits ≥ 200 target lines.
- Roundtrip + corpus check passes.

**Agent.** `lua-translator`.

---

## Translator chores (no translator extension)

Each is a single `corpus-curator` PR.

### Phase 5d — drop redundant Field-type stubs (broaden 5b)
**Goal.** Phase 5b dropped redundant `effect "Effect N" { resolve { } }` stubs on Equip Spell cards once a passive captured the chain. Extend the same filter to other type cards (Effect Monster, Continuous Spell, Continuous Trap) when they have a Phase 5 passive AND only one effect block AND it's empty.

**Yield estimate.** ~80 cards.

**Approach.** Extend the filter cascade in the Phase 5b apply script.

**Acceptance.**
- Filter cascade documented in commit message.
- ≥ 50 cards cleaned up.
- Roundtrip + corpus check passes.

**Agent.** `corpus-curator`.

**Depends on.** Phase 5 already shipped (✓).

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
