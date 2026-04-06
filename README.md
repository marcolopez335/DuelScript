# DuelScript 🃏

**A domain-specific language for Yu-Gi-Oh card mechanics, built for Rust engines.**

---

## What is DuelScript?

DuelScript (`.ds`) is a clean, readable scripting language designed specifically for defining Yu-Gi-Oh card effects. It replaces verbose, error-prone Lua scripts with a language where the **primitives are the game itself** — zones, chains, timing, costs, and resolution are all first-class concepts.

---

## Project Structure

```
duelscript/
├── Cargo.toml
├── grammar/
│   └── duelscript.pest      ← The full PEG grammar
├── src/
│   ├── lib.rs               ← Public API
│   ├── ast.rs               ← AST node definitions
│   └── parser.rs            ← pest → AST conversion
├── cards/
│   ├── pot_of_greed.ds      ← Simple spell example
│   ├── ash_blossom.ds       ← Hand trap example
│   └── nibiru.ds            ← Multi-effect monster example
└── examples/
    └── parse_card.rs        ← Parse a .ds file and print the AST
```

---

## Language Overview

### Card Header

```ds
card "Card Name" {
  type: Effect Monster | Tuner
  attribute: DARK
  race: Spellcaster
  level: 4
  atk: 1800
  def: 0
}
```

### Effect Block

```ds
effect "Effect Name" {
  speed: spell_speed_2        // spell_speed_1 | 2 | 3
  once_per_turn: true

  condition: in_hand          // where the card must be to activate

  trigger: opponent_activates [search | special_summon | send_to_gy]

  cost {
    discard self
  }

  on_resolve {
    negate effect
  }
}
```

---

## Keyword Reference

### Zones
| Keyword           | Meaning                     |
|-------------------|-----------------------------|
| `hand`            | Hand                        |
| `field`           | Field (any zone)            |
| `graveyard` / `gy`| Graveyard                   |
| `banished`        | Banished zone               |
| `deck`            | Main deck                   |
| `extra_deck`      | Extra deck                  |
| `spell_trap_zone` | Spell/Trap zone             |
| `monster_zone`    | Monster zone                |
| `extra_monster_zone` | Extra monster zone       |

### Trigger Actions
| Keyword          | Meaning                          |
|------------------|----------------------------------|
| `search`         | Add from deck to hand            |
| `special_summon` | Special summon from anywhere     |
| `send_to_gy`     | Send to graveyard                |
| `add_to_hand`    | Add to hand (non-search)         |
| `draw`           | Draw from deck                   |
| `banish`         | Banish a card                    |
| `mill`           | Send top of deck to GY           |
| `token_spawn`    | Spawn a token                    |

### Game Actions
| Syntax                              | Meaning                          |
|-------------------------------------|----------------------------------|
| `controller draws N`                | Draw N cards                     |
| `negate effect`                     | Negate an effect                 |
| `negate activation`                 | Negate an activation             |
| `destroy (N, monster, opponent controls)` | Destroy N opponent monsters|
| `send (N, card, ...) to gy`         | Send cards to GY                 |
| `special_summon self from hand`     | Summon this card                 |
| `search (1, monster) from deck`     | Search deck for a monster        |
| `banish (1, monster) from gy`       | Banish from GY                   |
| `return (1, card) to hand`          | Return card to hand              |
| `discard self`                      | Discard this card                |
| `pay_lp 1000`                       | Pay 1000 LP                      |

### Conditions
| Keyword             | Meaning                         |
|---------------------|---------------------------------|
| `in_hand`           | Card must be in hand            |
| `in_gy`             | Card must be in GY              |
| `on_field`          | Card must be on the field       |

### Phases
| Keyword          | Phase                  |
|------------------|------------------------|
| `draw_phase`     | Draw Phase             |
| `standby_phase`  | Standby Phase          |
| `main_phase_1`   | Main Phase 1           |
| `battle_phase`   | Battle Phase           |
| `main_phase_2`   | Main Phase 2           |
| `end_phase`      | End Phase              |

---

## Integrating into your Rust Engine

```rust
use duelscript::parse;

let source = std::fs::read_to_string("cards/ash_blossom.ds").unwrap();
let file = parse(&source).expect("Failed to parse card");

for card in file.cards {
    // Map card.effects into your engine's effect pipeline
    engine.register_card(card);
}
```

---

## Design Goals

- **Readable** — non-programmers can contribute card scripts
- **Type-safe** — illegal targeting, wrong spell speeds, invalid zones caught at parse time
- **Game-native** — timing, costs, and resolution are structural, not convention
- **Engine-agnostic** — the parsed AST is pure Rust; plug into any engine
- **Moddable** — hot-reload `.ds` files at runtime for rapid iteration

---

## Roadmap

- [ ] Validator — catch illegal card definitions before runtime
- [ ] LSP — syntax highlighting + autocomplete for VS Code
- [ ] Compiler mode — proc macro to compile `.ds` → Rust at build time (zero runtime cost)
- [ ] Card database — community `.ds` files in version control
- [ ] Web editor — browser-based card script editor with live validation

---

## Built With

- [pest](https://pest.rs) — PEG parser for Rust
- Rust 2021 edition

---

*DuelScript is a fan project. Yu-Gi-Oh is owned by Konami.*
