# DuelScript Expressiveness Gap Report

> **Status (v0.8):** 20 of the original 23 gaps have been resolved.
> The grammar, AST, parser, and compiler now cover flip effects,
> scope:self/field, activate_from, damage_step, flag effects,
> custom events, global state, named bindings, change_code/name,
> history queries, confirm/announce, choose actions, delayed effects,
> register_effect, and 15+ grant-style continuous codes.
>
> The 3 remaining gaps (A5 hint_timing, A7 simultaneous triggers,
> and H3 redirect effects like EFFECT_TO_GRAVE_REDIRECT) are tracked
> below.

After auditing 4 complex Lua cards (Galaxy Mirror Sage, Elemental HERO Prisma, Mind Crush, Maiden of Blue Tears) against the current DuelScript grammar, 23 gaps were identified. Most have been resolved — see status markers below.

This document tracks the **remaining path to full Lua parity**.

---

## Gap Categories

### Category A: Effect Type System (7 gaps)

#### A1. Flip effects as a distinct category
Lua uses `EFFECT_TYPE_FLIP` (0x20) for a dedicated flip effect block. Currently we have `when_flipped` as a trigger, but a true flip effect is a different category.

**Proposed:**
```
flip_effect "Gain LP" {
    on_resolve { gain_lp: count((1+, ...)) * 500 }
}
```

#### A2. Dual-type effects (SINGLE + FLIP, SINGLE + CONTINUOUS, etc.)
Many effects combine multiple `EFFECT_TYPE_*` flags. E.g., `EFFECT_TYPE_SINGLE+EFFECT_TYPE_CONTINUOUS` means "a continuous effect that only applies to self". DuelScript's `continuous_effect` block assumes FIELD scope.

**Proposed:** Add `scope: self | field` to continuous effects.

#### A3. Activation from multiple zones (hand + field, gy + banished)
We have `activate_from` in v0.6 but haven't verified it handles zone combinations.

#### A4. Damage-step-only effects
`EFFECT_FLAG_DAMAGE_STEP` allows activation during the damage step. We have `damage_step: true` in v0.6 — needs testing.

#### A5. Hint-timing declarations
Lua's `SetHintTiming(0, TIMING_TOHAND)` tells the UI when this effect prompts the player to activate. We don't have this.

**Proposed:**
```
hint_timing: when_card_sent_to_hand
```

#### A6. Effect category flags for chain-check interaction
`EFFECT_FLAG_DELAY` means "activate at chain end even if event happened mid-chain". Critical for most hand traps. We have `timing: if` which maps to DELAY but this needs verification.

#### A7. Simultaneous trigger flag
`EFFECT_FLAG2_CHECK_SIMULTANEOUS` — chain multiple identical triggers together. Not expressible.

---

### Category B: State Tracking (5 gaps)

#### B1. Persistent flag effects with custom reset masks
Lua: `RegisterFlagEffect(id, (RESET_EVENT|RESETS_STANDARD|RESET_OVERLAY)&~(RESET_LEAVE|RESET_TOGRAVE), 0, 0)`

This sets a flag that resets on most events BUT survives leaving the field and going to GY. DuelScript has `store "label"` but no reset-mask system.

**Proposed:**
```
set_flag "flipped_once" {
    survives: [leave_field, to_gy]
    resets_on: [end_of_duel, chain_end]
}
```

#### B2. Flag effect queries in conditions
```
condition: has_flag "flipped_once" on self
```

#### B3. Global state (shared across card instances)
Maiden of Blue Tears uses `aux.GlobalCheck` + `s.sumgroup` to track state shared between all copies. Needed for tracking "what's been summoned this chain" card-class-wide.

**Proposed:**
```
global_state "summoned_this_chain" {
    type: card_group
    tracks: every (1+, monster) summoned by_special_summon by opponent
    resets_on: chain_end
}
```

#### B4. Card history queries — "was this card previously X?"
Lua: `IsPreviousLocation`, `IsPreviousPosition`, `IsPreviousController`. Needed for "if this card was on the field before being sent to GY".

**Proposed:**
```
condition: previous_location == field
condition: previous_position == face_up
```

#### B5. Reason queries — "why did this happen?"
Lua: `IsReason(REASON_BATTLE|REASON_EFFECT)` — was this card sent to GY by battle? By effect? As cost?

**Proposed:**
```
condition: sent_to_gy_reason includes [battle, card_effect]
trigger: when_destroyed by card_effect
// Already partially supported
```

---

### Category C: Custom Events (3 gaps)

#### C1. Emit custom events
Lua: `Duel.RaiseEvent(group, EVENT_CUSTOM+id, e, 0, tp, tp, 0)` lets a card emit its own event that other effects can listen to.

**Proposed:**
```
emit_event "summoned_this_chain_updated"
```

#### C2. Listen for custom events
```
trigger: on_custom_event "summoned_this_chain_updated"
```

#### C3. Global event handlers
Lua's `aux.GlobalCheck` pattern registers an effect once per duel regardless of how many copies of the card exist. Needed for class-level event handlers.

**Proposed:**
```
global_handler {
    trigger: when_summoned by_special_summon
    on_event {
        add_to_tracked_group "summoned_this_chain"
        emit_event "summoned_this_chain_updated"
    }
}
```

---

### Category D: Targeting & Filters (4 gaps)

#### D1. Named target bindings (cost captures a value for resolution)
Prisma's cost selects a fusion monster and a material — resolution needs to reference what was captured.

**Proposed:**
```
cost {
    reveal (1, fusion monster, extra_deck) as revealed
    send (1, card, deck, where: name in revealed.material_list) to gy as captured
}
on_resolve {
    change_name self to: captured.name until end_phase
}
```

#### D2. Multi-phase cost with dependencies
Currently costs are a flat list. Prisma needs: step 1's result filters step 2's candidates.

#### D3. Selecting from custom groups (not just zones)
Maiden of Blue Tears' effect 1 selects from `s.sumgroup` — a group that's not a standard zone.

**Proposed:**
```
target (1, monster, from: global_state "summoned_this_chain")
```

#### D4. Base stats vs current stats
Lua distinguishes `GetBaseAttack()` (unmodified) from `GetAttack()` (current). Maiden uses `base_attack` for damage calc.

**Proposed:**
```
target.base_atk
target.atk  // current with modifiers
target.original_atk  // printed on card
```

---

### Category E: Announcements & Interaction (2 gaps)

#### E1. Card name announcement with filters
Mind Crush lets the player announce ANY non-Extra-Deck monster. Needs an announcement mini-language or named presets.

**Proposed:**
```
cost {
    announce (1, card) as announced {
        filter: not extra_deck_monster
    }
}
```

Common presets:
- `announce_filter: card_name`
- `announce_filter: attribute`
- `announce_filter: race`
- `announce_filter: level N`

#### E2. Hand reveal / confirm cards
Lua: `Duel.ConfirmCards(1-tp, tc)` — show a card to the opponent.

**Proposed:**
```
confirm target to: opponent
confirm_hand: opponent  // show opponent's entire hand to you
```

---

### Category F: Dynamic Effect Registration (2 gaps)

#### F1. Register effect on another card during resolution
Galaxy Mirror Sage's effect 3 attaches a "banish on leave field" effect TO THE SUMMONED CARD. Prisma attaches a "change code" effect to itself. This is `register_effect on target`.

**Proposed:** Already in v0.6 grammar but untested. Needs verification.
```
on_resolve {
    special_summon target from deck in face_down_defense
    register_effect on target {
        grant: redirect_when_leaving_field to banished
        duration: permanently
    }
}
```

#### F2. Change card code (name/identity change)
Prisma changes its name to match a fusion material. Lua: `EFFECT_CHANGE_CODE`.

**Proposed:**
```
change_name self to: captured.name until end_phase
// or:
change_code self to: captured.code until end_phase
```

---

### Category G: Conditional Resolution (2 gaps)

#### G1. If/else branching based on RUNTIME state
Mind Crush: "If opponent has the card in hand, discard all copies; otherwise, you discard randomly."

We have `if { } else { }` in grammar but need runtime condition evaluation.

**Proposed:**
```
on_resolve {
    if (count((1+, card, opponent hand, where: name == announced.name)) > 0) {
        discard (all, card, opponent hand, where: name == announced.name)
    } else {
        discard (1, card, you controls, hand, random)
    }
}
```

#### G2. Random selection from a group
```
discard (1, card, you controls, hand, random)
```

---

### Category H: Action Primitives Missing (4 gaps)

#### H1. Set card from one zone to S/T zone
Maiden's effect 2 sets a Normal Spell from GY. We have `set` but it assumes hand source.

**Proposed:**
```
set (1, normal spell) from gy
```

#### H2. Change name / change code
Prisma. See F2.

#### H3. Reveal a card without moving it
Prisma's cost reveals a fusion monster without sending it anywhere.

**Proposed:**
```
cost {
    reveal (1, fusion monster, extra_deck)
}
```

#### H4. Confirm cards to opponent
Galaxy Mirror Sage: `Duel.ConfirmCards(1-tp, tc)` shows the face-down summoned monster to the opponent.

**Proposed:**
```
confirm target to: opponent
```

---

## Summary: 23 Gaps Across 8 Categories

| Category | Gaps | Priority |
|----------|------|----------|
| A. Effect type system | 7 | High — affects every card |
| B. State tracking | 5 | High — hand traps, delayed effects |
| C. Custom events | 3 | Medium — advanced interactions |
| D. Targeting & filters | 4 | High — every complex card |
| E. Announcements | 2 | Medium — ~200 cards |
| F. Dynamic effect registration | 2 | High — protection/buff effects |
| G. Conditional resolution | 2 | High — every if/else card |
| H. Action primitives | 4 | Medium — niche actions |

---

## Proposed Implementation Plan

### Phase 1: High-priority core mechanics (Cat A, B, D, F, G)
- `flip_effect` block type
- `scope: self | field` on continuous effects
- Flag effect system with reset masks
- `has_flag`, `previous_location`, `previous_position` conditions
- `target.base_atk`, `target.original_atk`
- Named target bindings (`as name` in cost/target)
- Multi-phase cost with dependencies
- `register_effect on target` verification & tests
- `change_name` / `change_code` actions
- Runtime if/else with full expression evaluation
- Random selection

### Phase 2: Custom events (Cat C)
- `emit_event "name"`
- `trigger: on_custom_event "name"`
- `global_handler { }` for class-level handlers
- `global_state "name" { }` for shared tracked groups

### Phase 3: Announcements & niche (Cat E, H)
- `announce (1, card) as name { filter: ... }`
- `confirm target to: player`
- `reveal (1, ..., zone)` as cost
- `set (1, ...) from zone` with source

### Phase 4: Validation
- Re-write all 20 audit cards in full, verify each parses and compiles
- Each card gets a comment explaining which mechanics it exercises
- Run the comparison harness to see if bit-identical match improves

---

## Estimated Effort

- **Phase 1**: 2-3 days of grammar/AST/parser work. High impact.
- **Phase 2**: 1-2 days. Lower impact but unblocks ~50-100 advanced cards.
- **Phase 3**: 1 day. Small but completes the picture.
- **Phase 4**: Half a day of writing + verification.

Total: **~1 week** to reach full Lua parity expressiveness.

After Phase 1 alone, we should be able to write ~95% of cards cleanly.
After all phases, we should be able to write every card in the game.
