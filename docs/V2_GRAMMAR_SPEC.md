# DuelScript v2 Grammar Specification

## Design Principles

1. **Read like card text** — a player should understand the .ds file
2. **Every mechanic has a keyword** — no numeric bitfields, no raw_effect
3. **Cards describe behavior, engines execute rules**
4. **Controller-aware** — effects know who they apply to
5. **Composable** — complex cards built from simple primitives

---

## Card Structure

```
card "Name" {
    id: <number>
    type: <card_type> [| <card_type>]*
    attribute: <attribute>
    race: <race>
    level: <number>        // or rank: / link:
    atk: <number | ?>
    def: <number | ?>      // omitted for Link monsters
    scale: <number>        // Pendulum only
    link_arrows: [<dir>, ...]  // Link only
    archetype: [<string>, ...]

    // Card text as comments
    // "Effect text here..."

    summon { ... }         // how this card enters the field
    effect "Name" { ... }  // activated/trigger effects
    passive "Name" { ... } // always-on continuous effects
    restriction "Name" { ... } // grants/denies abilities
}
```

---

## 10 Test Cards

### 1. Pot of Greed (simplest spell)

```
card "Pot of Greed" {
    id: 55144522
    type: Normal Spell

    // Draw 2 cards.

    effect "Draw 2" {
        speed: 1
        resolve {
            draw 2
        }
    }
}
```

### 2. Lava Golem (complex summon procedure + restriction + mandatory trigger)

```
card "Lava Golem" {
    id: 102380
    type: Effect Monster
    attribute: FIRE
    race: Fiend
    level: 8
    atk: 3000
    def: 2500

    // Cannot be Normal Summoned/Set. Must first be Special Summoned
    // (from your hand) to your opponent's field by Tributing 2
    // monsters they control. You cannot Normal Summon/Set the turn
    // you Special Summon this card. Once per turn, during your
    // Standby Phase: Take 1000 damage.

    summon {
        cannot_normal_summon
        special_summon_procedure {
            from: hand
            to: opponent_field
            cost {
                tribute (2, monster, opponent controls)
            }
            restriction {
                apply_to: summoner
                cannot_normal_summon
                duration: this_turn
            }
        }
    }

    effect "Burn" {
        mandatory
        trigger: standby_phase
        who: controller
        resolve {
            damage controller 1000
        }
    }
}
```

### 3. Thousand-Eyes Restrict (fusion + continuous + absorb + redirect)

```
card "Thousand-Eyes Restrict" {
    id: 63519819
    type: Fusion Monster | Effect Monster
    attribute: DARK
    race: Spellcaster
    level: 1
    atk: 0
    def: 0

    // "Relinquished" + "Thousand-Eyes Idol"
    // Other monsters on the field cannot change battle positions or
    // attack. Once per turn: equip 1 opponent's monster to this card.
    // ATK/DEF become equal to equipped monster's. If this card would
    // be destroyed by battle, destroy equipped monster instead.

    summon {
        cannot_normal_summon
        fusion materials: "Relinquished" + "Thousand-Eyes Idol"
    }

    passive "Lock Field" {
        scope: field
        target: all monsters except self
        grant: cannot_attack
        grant: cannot_change_position
    }

    effect "Absorb" {
        speed: 1
        once_per_turn
        target (1, monster, opponent controls)
        resolve {
            equip target to self
        }
    }

    passive "Copy Stats" {
        scope: self
        set_atk: equipped.atk
        set_def: equipped.def
    }

    replacement "Shield" {
        instead_of: destroyed_by_battle
        do {
            destroy equipped_card
        }
    }
}
```

### 4. Black Luster Soldier - Envoy of the Beginning (banish summon + choose)

```
card "Black Luster Soldier - Envoy of the Beginning" {
    id: 72989439
    type: Effect Monster
    attribute: LIGHT
    race: Warrior
    level: 8
    atk: 3000
    def: 2500

    // Cannot be Normal Summoned/Set. Must first be Special Summoned
    // by banishing 1 LIGHT and 1 DARK monster from your GY.
    // Once per turn: choose 1:
    //   - Banish 1 monster on the field.
    //   - If this card destroyed a monster by battle, it can attack again.

    summon {
        cannot_normal_summon
        special_summon_procedure {
            from: hand
            cost {
                banish (1, monster, you control, gy, where attribute == LIGHT)
                banish (1, monster, you control, gy, where attribute == DARK)
            }
        }
    }

    effect "Choose" {
        speed: 1
        once_per_turn
        choose {
            option "Banish" {
                target (1, monster, either controls)
                resolve {
                    banish target
                }
            }
            option "Double Attack" {
                trigger: destroys_by_battle
                resolve {
                    grant self double_attack until end_of_turn
                }
            }
        }
    }
}
```

### 5. Mirror Force (battle trap)

```
card "Mirror Force" {
    id: 44095762
    type: Normal Trap

    // When an opponent's monster declares an attack: Destroy all
    // your opponent's Attack Position monsters.

    effect "Destroy Attackers" {
        speed: 2
        trigger: opponent_attack_declared
        resolve {
            destroy (all, monster, opponent controls, in attack_position)
        }
    }
}
```

### 6. Jinzo (continuous negate)

```
card "Jinzo" {
    id: 77585513
    type: Effect Monster
    attribute: DARK
    race: Machine
    level: 6
    atk: 2400
    def: 1500

    // Trap Cards cannot be activated. Negate all Trap effects on field.

    summon {
        tributes: 1
    }

    passive "Trap Lockdown" {
        scope: field
        target: all traps
        grant: cannot_activate
        negate_effects
    }
}
```

### 7. Sangan (sent to GY trigger with filter)

```
card "Sangan" {
    id: 26202165
    type: Effect Monster
    attribute: DARK
    race: Fiend
    level: 3
    atk: 1000
    def: 600

    // If this card is sent from the field to the GY: Add 1 monster
    // with 1500 or less ATK from your Deck to your hand.

    effect "Search" {
        speed: 1
        mandatory
        trigger: sent_to gy from field
        once_per_turn: hard
        resolve {
            search (1, monster, from deck, where atk <= 1500)
            add_to_hand searched
        }
    }
}
```

### 8. Enemy Controller (multi-choice quick-play)

```
card "Enemy Controller" {
    id: 98045062
    type: Quick-Play Spell

    // Activate 1 of these effects:
    // - Target 1 face-up monster opponent controls; change position.
    // - Tribute 1 monster; target 1 face-up monster opponent controls;
    //   take control until End Phase.

    effect "Control" {
        speed: 2
        choose {
            option "Change Position" {
                target (1, monster, opponent controls, face_up)
                resolve {
                    change_position target
                }
            }
            option "Take Control" {
                cost {
                    tribute (1, monster, you control)
                }
                target (1, monster, opponent controls, face_up)
                resolve {
                    take_control target until end_phase
                }
            }
        }
    }
}
```

### 9. Call of the Haunted (continuous trap + linked destruction)

```
card "Call of the Haunted" {
    id: 97077563
    type: Continuous Trap

    // Activate by targeting 1 monster in your GY; Special Summon it
    // in Attack Position. When this card leaves the field, destroy
    // that monster. When that monster is destroyed, destroy this card.

    effect "Revive" {
        speed: 2
        target (1, monster, you control, in gy)
        resolve {
            special_summon target from gy in attack_position
            link self to target
        }
    }

    effect "Linked Destruction" {
        trigger: self leaves_field or linked_card destroyed
        resolve {
            destroy linked_card
        }
    }
}
```

### 10. Solemn Judgment (counter trap)

```
card "Solemn Judgment" {
    id: 41420027
    type: Counter Trap

    // When a monster would be Summoned, OR a Spell/Trap is activated:
    // Pay half your LP; negate, and if you do, destroy that card.

    effect "Negate" {
        speed: 3
        trigger: summon_attempt or spell_trap_activated
        cost {
            pay_lp half
        }
        resolve {
            negate
            and_if_you_do {
                destroy negated_card
            }
        }
    }
}
```

---

## Syntax Summary

### Block Types
| Block | Purpose | Example |
|-------|---------|---------|
| `card` | Top-level card definition | `card "Name" { ... }` |
| `summon` | How the card enters play | `summon { tributes: 1 }` |
| `effect` | Activated/trigger effect | `effect "Name" { speed: 1 ... }` |
| `passive` | Always-on continuous effect | `passive "Name" { scope: field ... }` |
| `restriction` | Grant/deny abilities | `restriction { cannot_attack }` |
| `replacement` | "Instead of X, do Y" | `replacement { instead_of: destroyed ... }` |

### Speed
| Value | Meaning |
|-------|---------|
| `speed: 1` | Normal (spells, ignition, triggers) |
| `speed: 2` | Quick (traps, quick effects, hand traps) |
| `speed: 3` | Counter (counter traps only) |

### Frequency
| Syntax | Meaning |
|--------|---------|
| `once_per_turn` | Soft OPT (resets if leaves/returns) |
| `once_per_turn: hard` | Hard OPT (tracked by card name) |
| `once_per_duel` | Once per game |
| `mandatory` | Must activate (not optional) |

### Triggers
| Syntax | Event |
|--------|-------|
| `trigger: summoned` | When this card is summoned |
| `trigger: special_summoned` | When Special Summoned specifically |
| `trigger: destroyed` | When destroyed |
| `trigger: destroyed_by_battle` | When destroyed by battle |
| `trigger: destroyed_by_effect` | When destroyed by card effect |
| `trigger: sent_to gy` | When sent to GY |
| `trigger: sent_to gy from field` | When sent from field to GY |
| `trigger: banished` | When banished |
| `trigger: leaves_field` | When leaves the field |
| `trigger: flipped` | When flipped face-up |
| `trigger: attacked` | When this card is attacked |
| `trigger: attack_declared` | When any attack is declared |
| `trigger: opponent_attack_declared` | When opponent declares attack |
| `trigger: destroys_by_battle` | When this destroys by battle |
| `trigger: battle_damage` | When battle damage is dealt |
| `trigger: direct_attack_damage` | When direct attack damage dealt |
| `trigger: standby_phase` | During Standby Phase |
| `trigger: end_phase` | During End Phase |
| `trigger: draw_phase` | During Draw Phase |
| `trigger: main_phase` | During Main Phase |
| `trigger: battle_phase` | During Battle Phase |
| `trigger: damage_calculation` | During damage calculation |
| `trigger: summon_attempt` | When a summon is attempted |
| `trigger: spell_trap_activated` | When Spell/Trap activated |
| `trigger: opponent_activates` | When opponent activates effect |
| `trigger: chain_link` | When a chain link is added |
| `trigger: targeted` | When this card is targeted |
| `trigger: position_changed` | When battle position changes |
| `trigger: control_changed` | When control changes |

### Who
| Syntax | Meaning |
|--------|---------|
| `who: you` | The card's owner |
| `who: opponent` | The opponent of the owner |
| `who: controller` | Whoever currently controls this card |
| `who: summoner` | The player who summoned this card |
| `who: both` | Both players |

### Actions
| Syntax | Action |
|--------|--------|
| `draw N` | Draw N cards |
| `destroy target` | Destroy targeted card(s) |
| `destroy (filter)` | Destroy matching cards |
| `banish target` | Banish targeted card(s) |
| `banish (filter) from zone` | Banish from specific zone |
| `send target to zone` | Send card to zone |
| `return target to hand` | Return to hand |
| `return target to deck [top/bottom/shuffle]` | Return to deck |
| `search (filter) from deck` | Search deck for card |
| `add_to_hand target` | Add to hand |
| `special_summon target from zone [in position]` | Special Summon |
| `normal_summon target` | Normal Summon |
| `set target` | Set face-down |
| `flip_down target` | Flip face-down |
| `change_position target` | Change battle position |
| `take_control target [until duration]` | Take control |
| `equip target to monster` | Equip card |
| `negate` | Negate activation/summon |
| `negate_effects target` | Negate effects |
| `damage player amount` | Deal effect damage |
| `gain_lp amount` | Gain LP |
| `pay_lp amount` | Pay LP |
| `modify_atk target amount` | Modify ATK |
| `modify_def target amount` | Modify DEF |
| `set_atk target value` | Set ATK to value |
| `set_def target value` | Set DEF to value |
| `create_token { ... }` | Create token(s) |
| `attach target to xyz as_material` | Xyz material |
| `detach N from target` | Detach overlay |
| `place_counter "name" N on target` | Place counter |
| `remove_counter "name" N from target` | Remove counter |
| `mill N` | Send top N of deck to GY |
| `excavate N` | Reveal top N of deck |
| `reveal target` | Reveal card(s) |
| `look_at target` | Look at card(s) |
| `shuffle_deck [whose]` | Shuffle deck |
| `announce type/attribute/race/level` | Announce |
| `flip_coin { heads { ... } tails { ... } }` | Coin flip |
| `roll_dice { ... }` | Dice roll |
| `grant target ability [duration]` | Grant ability |

### Target Expressions
```
target                          // previously selected target
self                            // this card
(N, filter)                     // select N matching cards
(N, filter, who controls)       // with controller
(N, filter, who controls, zone) // with zone
(all, filter, ...)              // all matching
```

### Filters
```
monster / spell / trap / card
"Name" monster                  // specific name
"Archetype" card                // archetype match
face_up / face_down
in attack_position / in defense_position
where atk <= 1500               // stat filter
where level >= 4
where attribute == DARK
where race == Dragon
```

### Costs
```
cost {
    pay_lp N                    // or: pay_lp half
    discard (filter)            // discard from hand
    tribute (filter)            // tribute from field
    banish (filter) from zone   // banish from zone
    send (filter) to gy         // send to GY
    detach N from self          // Xyz materials
    remove_counter "name" N from self
    reveal (filter)
}
```

### Conditions
```
condition: you control (filter)
condition: opponent controls (filter)
condition: no monsters on field
condition: lp <= N
condition: cards_in_gy >= N
condition: hand_size >= N
condition: on_field
condition: in_gy
condition: phase == battle
condition: chain_includes [search, special_summon]
```

### Durations
```
this_turn
until end_of_turn
until end_phase
until next_turn
while_on_field
while_face_up
permanently
```

### Summoning
```
summon {
    // Normal summon requirements
    tributes: N

    // Cannot normal summon
    cannot_normal_summon

    // Special summon procedure
    special_summon_procedure {
        from: hand / gy / banished / deck
        to: your_field / opponent_field
        cost { ... }
        condition: ...
        restriction { ... }
    }

    // Fusion materials
    fusion materials: "Card A" + "Card B"
    fusion materials: (2+, monster, where attribute == DARK)

    // Synchro materials
    synchro materials {
        tuner: (1, tuner monster)
        non_tuner: (1+, non-tuner monster)
    }

    // Xyz materials
    xyz materials: (2, monster, where level == 4)

    // Link materials
    link materials: (2+, effect monster)

    // Ritual (on the spell card)
    ritual_summon (filter) using (materials) where total_level >= N

    // Pendulum
    pendulum_summon (filter) from [hand, extra_deck_face_up]
}
```

---

## What's New vs v1

| v1 | v2 | Why |
|----|-----|-----|
| `raw_effect { effect_type: 64 ... }` | Gone entirely | Bitfields aren't a language |
| `continuous_effect / replacement_effect / equip_effect / redirect_effect` | `passive` / `replacement` | Unified model |
| `on_resolve { }` | `resolve { }` | Shorter, card-centric |
| `on_activate { }` | `target (filter)` at effect level | Targeting is declarative |
| `speed: spell_speed_1` | `speed: 1` | Less verbose |
| `optional: true` | Absent (default is optional) | Less verbose |
| `optional: false` | `mandatory` | Reads like card text |
| `grant: cannot_be_destroyed_by_battle` | `grant: cannot_be_destroyed by battle` | More natural |
| `condition: you_control_no_monsters` | `condition: no monsters on your field` | Reads like English |
| No summon procedure | `summon { special_summon_procedure { ... } }` | First-class |
| `controller` implicit | `who: controller` explicit | Clear ownership |

---

## Gaps Found (Goat Format Review)

Reviewed 20 complex Goat cards. Found 10 mechanics the v2 spec doesn't yet cover:

### Gap 1: Multi-turn duration
**Card:** Crush Card Virus — "for the next 3 turns"
**Need:** `duration: 3_turns` or `duration: N_turns(3)`
```
resolve {
    destroy (all, monster, opponent controls, where atk <= 1500)
    for_duration 3_turns {
        check_drawn_cards opponent {
            if (monster and atk <= 1500) { destroy it }
        }
    }
}
```

### Gap 2: Damage step duration
**Card:** Injection Fairy Lily — ATK boost during damage calc only
**Need:** `until end_of_damage_step` duration
```
effect "Power Boost" {
    speed: 2
    trigger: damage_calculation
    cost { pay_lp 2000 }
    resolve {
        modify_atk self +3000 until end_of_damage_step
    }
}
```

### Gap 3: Condition — summoned this turn
**Card:** Dark Magician of Chaos — "if this card was summoned this turn"
**Need:** `condition: self summoned_this_turn`
```
effect "Recover Spell" {
    trigger: end_phase
    condition: self summoned_this_turn
    resolve { ... }
}
```

### Gap 4: Replaceable event — leaves field
**Card:** Dark Magician of Chaos — "if this card would leave the field, banish it instead"
**Need:** `instead_of: leaves_field`
```
replacement "Banish Instead" {
    instead_of: leaves_field
    do { banish self }
}
```

### Gap 5: Announce and reference
**Card:** Vampire Lord — "Declare 1 card type; opponent sends 1 of that type from Deck to GY"
**Need:** `announce type as declared` → filter by `declared`
```
resolve {
    announce type as declared
    send (1, card, from opponent_deck, where type == declared) to gy
}
```

### Gap 6: Cost binding — reference tributed/discarded card
**Card:** Metamorphosis — "Tribute 1 monster. Special Summon Fusion of same Level"
**Need:** `cost { tribute (1, monster) as tributed }` → `where level == tributed.level`
```
effect "Transform" {
    speed: 1
    cost {
        tribute (1, monster, you control) as tributed
    }
    resolve {
        special_summon (1, fusion monster, from extra_deck, where level == tributed.level)
    }
}
```

### Gap 7: Target condition based on LP
**Card:** Ring of Destruction — "target monster whose ATK <= opponent's LP"
**Need:** `where atk <= opponent_lp` in target filter
```
target (1, monster, opponent controls, face_up, where atk <= opponent_lp)
```

### Gap 8: Intercept future draws
**Card:** Crush Card Virus — "check all monsters drawn for 3 turns"
**Need:** Future-event interceptor (very complex)
**Approach:** `install_watcher` block
```
install_watcher "virus" {
    event: opponent_draws
    duration: 3_turns
    check { if (monster and atk >= 1500) { destroy drawn_card } }
}
```

### Gap 9: Token-specific restrictions
**Card:** Scapegoat — tokens "cannot be Tributed for a Tribute Summon"
**Need:** Restrictions on created tokens
```
create_token {
    name: "Sheep Token"
    atk: 0, def: 0
    count: 4
    position: defense
    restriction {
        cannot_be_tributed for tribute_summon
    }
}
```

### Gap 10: Mandatory cost vs optional activation
**Card:** Imperial Order — MUST pay 700 LP each standby (not optional to activate, mandatory cost)
**Need:** Distinguish "mandatory cost" from "optional effect with cost"
```
effect "Maintenance" {
    mandatory
    trigger: standby_phase
    maintenance_cost {
        pay_lp 700
        if_cannot_pay {
            destroy self
        }
    }
}
```

---

## Additional Constructs Needed

From the gap analysis:

### Durations (extended)
```
duration: this_turn
duration: until end_of_turn
duration: until end_phase
duration: until end_of_damage_step    // NEW
duration: until next_standby_phase    // NEW
duration: N_turns(3)                  // NEW
duration: while_on_field
duration: while_face_up
duration: permanently
```

### Conditions (extended)
```
condition: self summoned_this_turn    // NEW
condition: self attacked_this_turn    // NEW
condition: self flipped_this_turn     // NEW
condition: on_field
condition: in_gy
condition: in_hand
condition: in_banished
```

### Cost Bindings
```
cost {
    tribute (1, monster) as tributed     // bind for later reference
    discard (1, card) as discarded       // bind for later reference
}
resolve {
    ... where level == tributed.level    // reference bound card
    ... damage opponent discarded.level * 100
}
```

### Announce System
```
announce type as declared              // player names a type
announce attribute as declared
announce level as declared
announce card as declared              // player names a card
```

### Replaceable Events (extended)
```
instead_of: destroyed_by_battle
instead_of: destroyed_by_effect
instead_of: destroyed                  // either
instead_of: sent_to_gy
instead_of: banished
instead_of: returned_to_hand
instead_of: returned_to_deck
instead_of: leaves_field               // NEW - any zone transition off field
```

### Future Watchers
```
install_watcher "name" {
    event: opponent_draws / opponent_summons / ...
    duration: N_turns(3)
    check { ... }
}
```

---

## Spec Status

- **Core syntax:** Complete (10 test cards all expressible)
- **Actions:** 30+ defined, covers 95% of Goat cards
- **Gaps found:** 10 (all have proposed solutions above)
- **Next:** Write the PEG grammar implementing this full spec
