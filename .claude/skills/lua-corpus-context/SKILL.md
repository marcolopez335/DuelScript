---
name: lua-corpus-context
description: Use when reading or analysing the Project Ignis lua corpus — the source of truth that duelscript translates from. Covers paths, license, common idioms, and the Lua-side conventions a translator needs to recognise.
---

# Lua Corpus Context

The duelscript translator's input is the Project Ignis card-script corpus — Lua 5.3 scripts targeting edopro's ocgcore engine. This skill is the decoder.

## Path

```
/Users/marco/git/CardScripts/official/cXXXX.lua
```

One file per card, named by passcode. Mirrors the `cards/official/cXXXX.ds` layout. Subfolders (`pre-release/`, `unofficial/`, `mod/`) exist but the duelscript corpus translates only `official/`.

## License — a real blocker

The corpus is **AGPL-3** (per `/Users/marco/git/CardScripts/README.md`):

> Copyright (C) 2020 Project Ignis contributors … AGPL v3.

`cards/official/*.ds` is a machine-translated derivative of those Lua scripts. Inherits AGPL. duelscript's `Cargo.toml` declares **MIT** for the engine — the corpus and engine cannot ship under one license without a split.

If anyone asks about open-sourcing or accepting outside contributions: this is the first thing to fix. Options are documented in `project_session_handoff.md`.

## Effect-creation idiom

The canonical chain edopro effects are built with:

```lua
function s.initial_effect(c)
    local e1 = Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_*)        -- one of ACTIVATE / SINGLE / FIELD / EQUIP / TRIGGER_*  / IGNITION
    e1:SetCode(EVENT_* | EFFECT_*)   -- trigger event OR continuous-effect code
    e1:SetCategory(CATEGORY_*)       -- visible category bits (DESTROY, DRAW, …)
    e1:SetCondition(s.condition)     -- optional gating function
    e1:SetCost(s.cost)               -- optional cost function
    e1:SetTarget(s.target)           -- optional targeting function
    e1:SetOperation(s.operation)     -- the actual effect body
    c:RegisterEffect(e1)             -- commit
end
```

Within an operation handler, the same chain is used to register *further* effects — typically continuous modifiers applied to the resolved target:

```lua
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc = Duel.GetFirstTarget()
    local e1 = Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(800)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
```

Distinguish:

- `c:RegisterEffect(e1)` → effect on the card itself.
- `tc:RegisterEffect(e1)` → effect on a resolved target.
- `e:GetHandler():RegisterEffect(e1)` → equivalent to `c` from inside an operation handler.

## SetValue

Most cards use literal ints. The translator also encounters:

- `SetValue(s.atkval)` — function-ref. Body usually `return some_expr`. Resolved at runtime.
- `SetValue(atk/2)` — local-variable reference (defined earlier in the handler).
- `SetValue(tc:GetAttack())` — method call, runtime.
- `SetValue(function(e,c) return … end)` — inline closure.

Phase 4 / 5 handle only literals. Function-ref and expression handling is Phase 4c / 5c territory.

## SetReset

Reset args determine the effect's lifetime. The macro definitions live in
`/Users/marco/git/CardScripts/constant.lua` (lines ~295-301) — read them, don't
guess. The key fact: **`RESETS_STANDARD` contains NO phase reset.**

```lua
RESETS_STANDARD           = RESET_TOFIELD|RESET_LEAVE|RESET_TODECK|RESET_TOHAND
                           |RESET_TEMP_REMOVE|RESET_REMOVE|RESET_TOGRAVE|RESET_TURN_SET
RESETS_STANDARD_DISABLE   = RESETS_STANDARD|RESET_DISABLE
RESETS_STANDARD_PHASE_END = RESET_EVENT|RESETS_STANDARD|RESET_PHASE|PHASE_END
```

Translation mapping (see `reset_to_duration_kw()` in `lua_ast.rs` — exact
token-set matching, NOT substrings):

- `RESET_PHASE | PHASE_END` (incl. the `RESETS_STANDARD_PHASE_END` /
  `RESETS_STANDARD_DISABLE_PHASE_END` macros) → `end_of_turn`.
- `RESET_EVENT | RESETS_STANDARD` (bare, no phase pair) → lasts while the card
  stays on the field (resets on leave-field, relocation, or being turned
  face-down). → `while_on_field`. **Not** end-of-turn.
- `RESET_EVENT | RESETS_STANDARD_DISABLE` → same, plus reset-on-negation
  (RESET_DISABLE — inexpressible in DSL durations). → `while_on_field`.
- `RESET_PHASE | PHASE_DAMAGE[_CAL]` (± standard events) → ends after the
  damage step / damage calculation. → `end_of_damage_step`. **Not** end-of-turn.
- standard events + `RESET_PHASE | PHASE_STANDBY [| RESET_SELF_TURN]` →
  "until your next Standby Phase" card text. → `next_standby_phase`.
- `RESET_SELF_TURN` / `RESET_OPPO_TURN` qualified phase ends → end of a
  SPECIFIC player's turn — no DSL keyword; skip.
- `RESET_EVENT | RESETS_REDIRECT`, `RESET_CHAIN`, `&~` bit arithmetic,
  `RESETS_STANDARD_EXC_GRAVE`, `RESETS_CANNOT_ACT`, `PHASE_BATTLE` combos → skip.

## SetType bitmask (relevant constants)

| Constant | Meaning | DSL emit |
|---|---|---|
| `EFFECT_TYPE_SINGLE` | applies to the card itself | passive default |
| `EFFECT_TYPE_FIELD` | continuous field-wide | `scope: field` + selector from SetTargetRange |
| `EFFECT_TYPE_EQUIP` | applies to equipped card | `target: equipped_card` |
| `EFFECT_TYPE_ACTIVATE` | spell/trap activation | `effect { resolve { … } }` |
| `EFFECT_TYPE_TRIGGER_F` | mandatory trigger | `effect` with `mandatory` + `trigger:` |
| `EFFECT_TYPE_TRIGGER_O` | optional trigger | `effect` with `trigger:` |
| `EFFECT_TYPE_IGNITION` | ignition (main-phase manual) | `effect` with `trigger: ignition` |

These OR together — `EFFECT_TYPE_SINGLE | EFFECT_TYPE_TRIGGER_F` is a mandatory-trigger single-card effect.

## SetTargetRange

`SetTargetRange(my_locs, opp_locs)` — for FIELD-type effects, names whose monsters/zones are affected:

| `(my, opp)` | Reach |
|---|---|
| `(LOCATION_MZONE, 0)` | your monsters |
| `(0, LOCATION_MZONE)` | opponent monsters |
| `(LOCATION_MZONE, LOCATION_MZONE)` | both sides |
| `(LOCATION_HAND, LOCATION_HAND)` | both hands |

## Iteration idioms

- `for tc in aux.Next(g) do … end` — iterate over a Group `g`. Used to apply per-target effects. Multi-target.
- `local tc = g:GetFirst()` — first card of a group.
- `local tc = Duel.GetFirstTarget()` — first card of the activated effect's target list.

## Common Duel.* methods

Already mapped (Phase 2/3): SpecialSummon, SendtoHand, SendtoGrave, SendtoDeck, Destroy, Remove (banish), Release (tribute), DiscardHand, Damage, Recover, Draw, ChangePosition, Equip, SSet, ShuffleDeck, NegateAttack/Activation/Effect, SynchroSummon/XyzSummon/LinkSummon/FusionSummon/RitualSummon, Summon.

Skipped (meta): Hint, HintSelection, ConfirmCards, BreakEffect, SetOperationInfo, SetTargetPlayer, SetTargetParam, SetTargetCard, SetChainLimit.

Skipped (queries used in cond/target): Is*, Get*, Select*, GetMatchingGroup.

Top unmapped methods (per error-triage on remaining 3,539 errors): RegisterFlagEffect (meta), Overlay (xyz attach), SelectOption (UI choice), ShuffleHand (translatable).

## Where to look first

Before adding a new translator phase:
1. Read 3-5 sample lua files for the shape in question.
2. Note which of the constants above appear with what frequency.
3. Decide MVP scope: which sub-shapes emit, which skip.
