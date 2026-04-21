# DuelScript Cookbook

Recipes for writing common card archetypes by hand. Each recipe shows
the canonical `.ds` source, what it compiles to, and which runtime
hooks the engine needs to honor.

For the language reference, see `V2_LANGUAGE_REFERENCE.md`.
For engine integration, see `ENGINE_INTEGRATION.md`.

---

## Index

| # | Card | Mechanic |
|---|------|----------|
| 1 | Pot of Greed | Simplest spell — draw N |
| 2 | Raigeki | Filtered destroy (opponent only) |
| 3 | Mystical Space Typhoon | Targeted destroy |
| 4 | Lightning Vortex | Discard cost + filtered destroy |
| 5 | Karma Cut | Discard cost + banish |
| 6 | Compulsory Evacuation Device | Bounce target to hand |
| 7 | Forbidden Chalice | ATK modifier with duration |
| 8 | Mirror Force | Normal Trap mass-destroy |
| 9 | Solemn Strike | Counter Trap, pay LP |
| 10 | Sangan-style trigger monster | When-summoned trigger |
| 11 | Multi-effect monster | Two effects on one card |
| 12 | Galaxy Mirror Sage | Flip effect + persistent flag |
| 13 | Ash Blossom-style hand trap | Chain category check |
| 14 | Beastworld Aegis | Continuous trap with stacked grants |
| 15 | Aegis of the Selected | register_effect on a selected target |
| 16 | Pendulum Recovery | send X to extra_deck |

---

## 1. Pot of Greed — the simplest spell

> "Draw 2 cards."

```duelscript
card "Pot of Greed" {
    type: Normal Spell
    password: 55144522

    effect "Draw 2" {
        speed: spell_speed_1

        on_resolve {
            draw 2
        }
    }
}
```

**What's happening:**
- `type: Normal Spell` — categorizes as `EFFECT_TYPE_ACTIVATE`
- `speed: spell_speed_1` — only activatable during your own Main Phase
- `on_resolve` — actions that fire after the chain link resolves
- `draw 2` — calls `DuelScriptRuntime::draw(player, 2)`

**Engine hooks needed:** `draw`. Already implemented in `DuelApi`.

---

## 2. Raigeki — filtered destroy

> "Destroy all monsters your opponent controls."

```duelscript
card "Raigeki" {
    type: Normal Spell
    password: 12580477

    effect "Destroy Opponent's Monsters" {
        speed: spell_speed_1

        on_resolve {
            destroy (1+, monster, opponent controls)
        }
    }
}
```

**Target expression breakdown:**
- `(1+, monster, opponent controls)`
- `1+` — at least one card; `0` if none on field
- `monster` — filter by `CardFilter::Monster`
- `opponent controls` — `ControllerRef::Opponent`

**Engine hooks needed:** `get_field_cards`, `card_matches_filter`, `destroy`.

**Common variations:**
- `destroy (1+, monster, you controls)` — destroy your own monsters
- `destroy (1+, monster, either_player controls)` — destroy ALL monsters
- `destroy (1, spell, opponent controls)` — destroy 1 opponent S/T

---

## 3. Mystical Space Typhoon — targeted destroy

> "Target 1 Spell/Trap on the field; destroy that target."

```duelscript
card "Mystical Space Typhoon" {
    type: Quick-Play Spell
    password: 5318639

    effect "Destroy Spell or Trap" {
        speed: spell_speed_2

        on_activate {
            search (1, spell, either_player controls) from spell_trap_zone
        }

        on_resolve {
            destroy (1, spell, either_player controls)
        }
    }
}
```

**What's new:**
- `Quick-Play Spell` — `EFFECT_TYPE_ACTIVATE` with quick speed
- `speed: spell_speed_2` — required for Quick-Plays and Normal Traps
- `on_activate` — runs at activation time to pick targets
- `from spell_trap_zone` — the source zone for the search

**Validator note:** Quick-Plays must use `spell_speed_2`. Normal Spells
must use `spell_speed_1`. The validator catches mismatches.

---

## 4. Lightning Vortex — cost + filtered destroy

> "Discard 1 card; destroy all face-up monsters your opponent controls."

```duelscript
card "Lightning Vortex" {
    type: Normal Spell
    password: 69162969

    effect "Discard and Destroy" {
        speed: spell_speed_1

        cost {
            discard (1, card, you controls)
        }

        on_resolve {
            destroy (1+, monster, opponent controls)
        }
    }
}
```

**Cost block:** runs in two phases.
- `cost(check_only=true)` — verify the cost is payable (the engine
  doesn't activate the card if this returns false)
- `cost(check_only=false)` — actually pay it

**Engine hooks needed:** `discard` (already implemented).

---

## 5. Karma Cut — banish from GY

```duelscript
card "Karma Cut" {
    type: Normal Trap
    password: 71283180

    effect "Discard and Banish" {
        speed: spell_speed_2

        cost {
            discard (1, card, you controls)
        }

        on_resolve {
            banish (1, monster, opponent controls)
        }
    }
}
```

The interesting bit: `banish` is a separate `DuelApi` method from
`destroy`. The engine puts the card in the banished pile, not the GY.

---

## 6. Compulsory Evacuation Device — bounce to hand

```duelscript
card "Compulsory Evacuation Device" {
    type: Normal Trap
    password: 94192409

    effect "Bounce Target" {
        speed: spell_speed_2

        on_resolve {
            return (1, monster, either_player controls) to hand
        }
    }
}
```

**Two flavors of "to hand":**
- `return X to hand` — bounce from field
- `add_to_hand X from gy` — search from GY/deck/banished

The migrator distinguishes them based on the source zone in the Lua
`Duel.SendtoHand` call. Authoring by hand: pick `return` for field,
`add_to_hand` for non-field sources.

---

## 7. Forbidden Chalice — ATK buff with duration

```duelscript
card "Forbidden Chalice" {
    type: Normal Spell
    password: 24286941

    effect "Boost ATK" {
        speed: spell_speed_1

        on_resolve {
            modifier: atk + 400 on (1, monster, either_player controls) until_end_of_turn
        }
    }
}
```

**ATK modifier syntax:**
- `modifier: atk +/- expr on TARGET duration`
- `on TARGET` — required when targeting other than self
- `duration` — `until_end_of_turn`, `until_end_phase`, `permanently`,
  `until_next_turn`

**Engine hook:** `modify_atk`. Note that direct mutation of `atk_mod`
is futile — the engine's continuous-effect manager recomputes it. The
DS adapter queues mods to `pending_ds_atk_def_mods`, and `run_ds_effect`
drains them into `ContinuousEffectManager` so they survive recompute.

---

## 8. Mirror Force — Normal Trap

```duelscript
card "Mirror Force" {
    type: Normal Trap
    password: 44095762

    effect "Destroy" {
        speed: spell_speed_2

        on_resolve {
            destroy (1+, monster, opponent controls)
        }
    }
}
```

Traps look identical to spells except for `type:` and the chain
dispatch path. The yugaioh engine's `exec_activate_trap` adds the
trap to the chain queue, then `pass_chain` resolves it through the
same path as spells — including the DS short-circuit.

---

## 9. Solemn Strike — Counter Trap with LP cost

```duelscript
card "Solemn Strike" {
    type: Counter Trap
    password: 40605147

    effect "Negate" {
        speed: spell_speed_3

        cost {
            pay_lp 1500
        }

        on_resolve {
            negate activation
        }
    }
}
```

**Counter Traps** must use `spell_speed_3`. The validator enforces this.

`negate activation` calls `DuelScriptRuntime::negate_activation()`,
which the engine implements by marking the chain link as negated and
sending the offending card to the GY without resolving its effect.

---

## 10. Sangan-style trigger monster

> "When this card is Normal Summoned, draw 1 card."

```duelscript
card "Test Trigger Monster" {
    type: Effect Monster
    password: 40640057
    attribute: DARK
    race: Fiend
    level: 3
    atk: 1000
    def: 600

    effect "Draw on Summon" {
        speed: spell_speed_1
        trigger: when_summoned

        on_resolve {
            draw 1
        }
    }
}
```

**Trigger effects** are dispatched by the engine's event system. When
the engine raises `EVENT_SUMMON_SUCCESS`, it iterates `card_effects`
looking for entries with matching `code` and a trigger flag in
`effect_type`. DS effects get registered into this map by
`ensure_script_loaded`, so they're indistinguishable from Lua effects
to the trigger collector.

**Common triggers:**
- `when_summoned`, `when_summoned by_special_summon`
- `when_destroyed`, `when_destroyed by battle`
- `when_sent_to gy`, `when_banished`
- `when_attacked`, `when_flipped`

---

## 11. Multi-effect monster

```duelscript
card "Test Multi Effect Monster" {
    type: Effect Monster
    password: 40640058
    attribute: DARK
    race: Fiend
    level: 4
    atk: 1500
    def: 1000

    effect "Draw on Summon" {
        speed: spell_speed_1
        trigger: when_summoned
        on_resolve {
            draw 1
        }
    }

    effect "Draw on Destroy" {
        speed: spell_speed_1
        trigger: when_destroyed by battle
        on_resolve {
            draw 2
        }
    }
}
```

Each `effect` block compiles to its own `EffectDefinition` with its
own `effect_type`, `code`, and callbacks. The engine's chain link
carries an `effect_index` so the right effect fires for the right
event — `effect[0]` runs on summon, `effect[1]` runs on destroy.

If the engine ever calls `run_ds_effect_indexed(card_id, player, None)`
(no index), the dispatcher runs the *first* matching effect. Triggers
always pass an index, so they pick the right one.

---

## 12. Galaxy Mirror Sage — flip effect + persistent flag

A flip effect that gives LP, plus a flag that survives the card
leaving the field, so a later "if previously flipped" trigger can
fire.

```duelscript
card "Galaxy Mirror Sage" {
    type: Effect Monster | Flip
    password: 98263709
    attribute: LIGHT
    race: Spellcaster
    level: 3
    atk: 800
    def: 500
    archetype: ["Galaxy"]

    flip_effect "Gain LP" {
        on_resolve {
            gain_lp: count((1+, "Galaxy" monster, you controls, gy)) * 500
        }
    }

    effect "Remember Flip" {
        speed: spell_speed_1
        trigger: when_flipped
        on_resolve {
            set_flag "flipped_once" on self {
                survives: [leave_field, to_gy]
                resets_on: [end_of_duel]
            }
        }
    }

    effect "Revive Galaxy" {
        speed: spell_speed_1
        optional: true
        trigger: when_sent_to gy by card_effect
        condition: has_flag "flipped_once" on self

        on_resolve {
            special_summon (1, "Galaxy" monster) from deck in face_down_defense
        }
    }
}
```

**Flag effects** are persistent state stored on the card. The
`survives` list overrides the default reset events — by including
`leave_field` and `to_gy`, the flag stays set even after the card
goes to the graveyard. The `resets_on` list adds extra reset events
on top of survives.

**Engine hooks needed:** `register_flag`, `clear_flag`, `has_flag`.

---

## 13. Ash Blossom — hand trap with chain check

```duelscript
card "Ash Blossom & Joyous Spring" {
    type: Effect Monster | Tuner
    password: 14558127
    attribute: FIRE
    race: Plant
    level: 3
    atk: 0
    def: 1800

    effect "Negate" {
        speed: spell_speed_2
        once_per_turn: hard
        optional: true
        timing: if
        activate_from: [hand]

        condition: chain_link_includes [search, special_summon, send_to_gy]

        cost {
            discard self
        }

        on_resolve {
            negate activation
            and_if_you_do {
                destroy (1, card, opponent controls)
            }
        }
    }
}
```

**What's new:**
- `activate_from: [hand]` — this card activates from the hand, not
  the field. Required for hand traps.
- `once_per_turn: hard` — hard once-per-turn restriction (the card_id
  itself is the OPT key).
- `optional: true` + `timing: if` — "you can" rather than "you must",
  no miss-timing.
- `chain_link_includes [...]` — only fires if the current chain link
  has one of the listed effect categories.
- `and_if_you_do { ... }` — sequential resolution; only runs if the
  preceding action succeeded.

---

## 14. Beastworld Aegis — multi-grant continuous trap

```
card "Beastworld Aegis" {
    type: Continuous Trap
    password: 40640083

    continuous_effect "Empower the Pack" {
        scope: field
        apply_to: (1+, monster, you controls)
        grant: piercing
        grant: double_attack
        grant: cannot_be_destroyed_by_battle
    }
}
```

`continuous_effect` blocks can stack any number of `grant:` clauses
on the same `apply_to` selector. Use `scope: field` for effects that
affect *other* monsters (vs `scope: self` for the host card alone).

---

## 15. Aegis of the Selected — register_effect on a target

```
card "Aegis of the Selected" {
    type: Equip Spell
    password: 40640084

    effect "Untouchable Aegis" {
        speed: spell_speed_1
        on_resolve {
            select (1, monster, you controls) as protected
            register_effect on (1, monster, you controls) {
                grant: cannot_be_targeted_by_card_effects
                grant: unaffected_by_spell_effects
                duration: until_end_of_turn
            }
        }
    }
}
```

`register_effect on <target>` attaches a sub-effect to a card you've
already selected. It accepts multiple `grant:` clauses plus an
optional `duration:` (`until_end_of_turn`, `until_end_phase`,
`permanently`, etc.).

---

## 16. Pendulum Recovery — send to extra deck

```
card "Pendulum Recovery" {
    type: Normal Spell
    password: 40640085

    effect "Reclaim Pendulum" {
        speed: spell_speed_1
        once_per_turn: hard
        on_resolve {
            send (1, "Pendulum" monster, you controls, gy) to extra_deck
        }
    }
}
```

The `send X to <zone>` action handles every cross-zone movement
that isn't a draw / banish / destroy. The DSL recognizes `gy`,
`hand`, `deck`, `extra_deck`, `extra_deck_face_up`, `banished`,
`monster_zone`, `spell_trap_zone`, `pendulum_zone`, `field_zone`.

---

## Authoring workflow

1. **Find or create the .ds file.** Card files use the `c<password>.ds`
   naming convention so the loader can index by passcode.

2. **Validate as you go.**
   ```sh
   duelscript check cards/test/c12580477.ds
   ```

3. **Auto-format before committing.**
   ```sh
   duelscript fmt cards/test/
   ```

4. **Snapshot the compile output** so future regressions get caught:
   ```sh
   UPDATE_SNAPSHOTS=1 cargo test --test cards_snapshot
   ```

5. **Smoke test the runtime behavior** with the mock harness:
   ```sh
   cargo run --bin cards_runner -- cards/test/c12580477.ds
   ```

6. **Run an end-to-end yugaioh integration test** if you have the
   engine wired up. See `engine/tests/duelscript_integration.rs` in
   the yugaioh repo for the test pattern.

---

## When the language can't express something

If you're stuck and the existing actions don't cover your card,
check `EXPRESSIVENESS_GAPS.md` — that file tracks known gaps and
roadmap items. For real custom logic that doesn't fit the DSL, you
can fall back to a `raw_effect { ... }` block which preserves Lua
metadata bit-identically and lets you inline DSL actions inside.
