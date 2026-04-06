# DuelScript

**A domain-specific language for Yu-Gi-Oh card mechanics.**

DuelScript replaces Lua card scripting with a readable, type-safe, declarative format that compiles to engine-compatible bitfields (EDOPro/YGOPro compatible).

## Status

- **13,219 of 13,298** Lua card scripts migrate, parse, and compile (99.4%)
- **10/10** hand-verified cards produce bit-identical output to Lua
- **47 tests** passing across 5 test suites
- **Python module** available via PyO3

## Quick Example

```
// c55144522.ds — Pot of Greed
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

```
// c14558127.ds — Ash Blossom & Joyous Spring
card "Ash Blossom & Joyous Spring" {
    type: Effect Monster | Tuner
    attribute: FIRE
    race: Zombie
    level: 3
    atk: 0
    def: 1800
    password: 14558127

    effect "Negate" {
        speed: spell_speed_2
        once_per_turn: hard
        optional: true
        condition: chain_link_includes [add_to_hand, special_summon, send_to_gy, draw]
        trigger: opponent_activates [search | special_summon | send_to_gy | draw]
        cost {
            discard self
        }
        on_resolve {
            negate effect
        }
    }
}
```

## Project Structure

```
duelscript/
├── grammar/duelscript.pest     # PEG grammar (the spec)
├── src/
│   ├── ast.rs                  # AST types with Expr system
│   ├── parser.rs               # PEG → AST (full coverage)
│   ├── compiler/               # AST → engine bitfields
│   │   ├── mod.rs              # compile_card() entry point
│   │   ├── type_mapper.rs      # DSL → EDOPro constants
│   │   ├── callback_gen.rs     # condition/cost/target/operation closures
│   │   └── expr_eval.rs        # Runtime expression evaluator
│   ├── validator.rs            # Semantic validation
│   ├── database.rs             # Card registry with O(1) lookup
│   ├── migrate.rs              # Lua → DuelScript converter
│   ├── python.rs               # PyO3 Python bindings
│   ├── engine/bridge.rs        # Engine-agnostic trait
│   └── cdb.rs                  # BabelCdb SQLite reader
├── cards/official/             # Verified c<ID>.ds files
├── tests/                      # Integration tests
└── docs/
    ├── LANGUAGE_REFERENCE.md   # Full language spec
    └── ENGINE_INTEGRATION.md   # SDK / integration guide
```

## Usage

### Rust

```rust
use duelscript::{parse, compile_card};

let file = parse(source)?;
let compiled = compile_card(&file.cards[0]);
// compiled.effects[0].effect_type == 0x10 (ACTIVATE)
// compiled.effects[0].category == 0x10000 (DRAW)
```

### Python

```bash
pip install maturin
maturin develop --features python
```

```python
import duelscript

db = duelscript.CardDB("cards/official")
card = db.get_by_id(55144522)
compiled = duelscript.compile(open("cards/official/c55144522.ds").read())
```

### CLI

```bash
cargo run -- check cards/official/       # Validate all cards
cargo run -- inspect cards/official/c55144522.ds  # Show AST
```

### Migration

```bash
# In your code:
use duelscript::migrate::migrate_directory;
let results = migrate_directory(Path::new("CardScripts/official"));
// 99.4% success rate on 13,298 scripts
```

## Documentation

- [Language Reference](docs/LANGUAGE_REFERENCE.md) — full syntax specification
- [Engine Integration Guide](docs/ENGINE_INTEGRATION.md) — how to integrate with your engine

## File Naming

Card scripts use `c<ID>.ds` naming (e.g., `c55144522.ds`), matching the ProjectIgnis Lua convention.

## License

MIT
