# DuelScript

**A domain-specific language for Yu-Gi-Oh card mechanics.**

DuelScript replaces Lua card scripting with a readable, type-safe, declarative format that compiles to engine-compatible bitfields (EDOPro/YGOPro compatible).

## Status

- **13,298 / 13,298** cards in `cards/official/` parse, compile, and validate (100%)
- **151 / 151** Goat Format cards hand-verified in v2 syntax
- **135 lib tests** + full-corpus regression suite passing
- **LSP server** + **VS Code extension** (diagnostics, hover, completion)
- **CLI** with `check`, `inspect`, `fmt`, and `test` subcommands

## Quick Example

```
// cards/goat/pot_of_greed.ds
card "Pot of Greed" {
    id: 55144522
    type: Normal Spell

    effect "Draw 2" {
        speed: 1
        resolve {
            draw 2
        }
    }
}
```

```
// cards/goat/solemn_judgment.ds
card "Solemn Judgment" {
    id: 41420027
    type: Counter Trap

    effect "Negate Everything" {
        speed: 3
        trigger: summon_attempt
        cost {
            pay_lp half
        }
        resolve {
            negate and destroy
        }
    }
}
```

## Project Structure

```
duelscript/
├── grammar/duelscript.pest   # PEG grammar (the spec)
├── src/
│   ├── lib.rs                # Crate root — re-exports the v2 public API
│   ├── cdb.rs                # BabelCdb SQLite reader (feature: `cdb`)
│   ├── bin/
│   │   ├── duelscript.rs     # CLI: check / inspect / fmt / test
│   │   ├── duelscript_lsp.rs # LSP server (feature: `lsp`)
│   │   └── lua_inventory.rs  # Lua-corpus inspection utility
│   └── v2/
│       ├── ast.rs            # AST types
│       ├── parser.rs         # PEG → AST
│       ├── validator.rs      # Semantic validation
│       ├── compiler.rs       # AST → closures bound to DuelScriptRuntime
│       ├── fmt.rs            # Pretty-printer (round-trips through parser)
│       ├── runtime.rs        # DuelScriptRuntime trait — engine seam
│       ├── mock_runtime.rs   # Test runtime with call-log capture
│       ├── segoc.rs          # Simultaneous-effect-go-on-chain ordering
│       ├── constants.rs      # EDOPro-compatible bitmasks
│       └── lsp.rs            # Language Server (feature: `lsp`)
├── cards/
│   ├── official/             # 13,298 migrated c<ID>.ds files
│   └── goat/                 # 151 hand-verified Goat Format cards
├── editors/                  # VS Code extension
├── tests/corpus_compile.rs   # Full-corpus parse+compile regression
└── docs/                     # Language reference, trait reference, cookbook
```

## Usage

### Rust

```rust
use duelscript::{parse_v2, compile_card_v2, validate_v2};

let file = parse_v2(source)?;
validate_v2(&file);
let compiled = compile_card_v2(&file.cards[0]);
// compiled.effects[0].effect_type == 0x10 (ACTIVATE)
// compiled.effects[0].category   == 0x10000 (DRAW)
```

Each compiled effect exposes `condition`, `cost`, `target`, and `operation` closures bound to the `DuelScriptRuntime` trait (`src/v2/runtime.rs`). A host engine implements the trait; the compiled closures call back into it.

### CLI

```bash
cargo run --bin duelscript -- check   cards/goat/                     # Validate
cargo run --bin duelscript -- inspect cards/goat/pot_of_greed.ds      # Show AST
cargo run --bin duelscript -- fmt     cards/goat/ --check             # Lint formatting
cargo run --bin duelscript -- test    cards/goat/pot_of_greed.ds      # Run effect vs MockRuntime
```

### LSP / VS Code

Build the language server and install the extension:

```bash
cargo build --features lsp --bin duelscript_lsp
cd editors/vscode && npm install && vsce package
code --install-extension duelscript-*.vsix
```

## Documentation

- [V2 Language Reference](docs/V2_LANGUAGE_REFERENCE.md) — syntax specification
- [V2 Grammar Spec](docs/V2_GRAMMAR_SPEC.md) — PEG grammar design
- [Trait Reference](docs/TRAIT_REFERENCE.md) — `DuelScriptRuntime` engine seam
- [Engine Integration](docs/ENGINE_INTEGRATION.md) — integration guide
- [Cookbook](docs/COOKBOOK.md) — patterns for common card types
- [Expressiveness Gaps](docs/EXPRESSIVENESS_GAPS.md) — remaining Lua-parity work

## File Naming

Card scripts use `c<ID>.ds` naming under `cards/official/` (e.g., `c55144522.ds`), matching the ProjectIgnis Lua convention. Hand-written cards under `cards/goat/` use readable names (e.g., `pot_of_greed.ds`).

## License

MIT
