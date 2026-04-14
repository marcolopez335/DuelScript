# V2 Promotion: Remove V1 and Make V2 Primary

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove all v1 code, promote v2 to be the only DuelScript system, fix the 37 remaining parse failures, and update the CLI to use v2.

**Architecture:** The v2 system (`src/v2/`) already has its own parser, AST, validator, and compiler. It reuses engine constants and the `DuelScriptRuntime` trait from v1's compiler module. This plan extracts those shared pieces into v2, deletes everything v1, updates the CLI, and renames v2 paths to be the primary paths.

**Tech Stack:** Rust, pest (PEG parser), cargo

---

### Task 1: Fix the `cannot_attack_directly` PEG ordering bug

**Files:**
- Modify: `grammar/duelscript_v2.pest:516-517`

The PEG parser matches `"cannot_attack"` before `"cannot_attack_directly"` due to ordered choice. All 37 parse failures are caused by this.

- [ ] **Step 1: Fix the grammar ordering**

In `grammar/duelscript_v2.pest`, find the grant_ability rule around line 516:

```pest
    "cannot_attack"
  | "cannot_attack_directly"
```

Swap the order so the longer alternative comes first:

```pest
    "cannot_attack_directly"
  | "cannot_attack"
```

- [ ] **Step 2: Run the v2 official error report test**

Run: `cargo test test_v2_official_error_report -- --nocapture 2>&1 | grep "v2_official parse"`

Expected: `v2_official parse: 13298 ok, 0 fail`

- [ ] **Step 3: Run full test suite**

Run: `cargo test 2>&1 | grep "test result"` — all suites should pass.

- [ ] **Step 4: Commit**

```bash
git add grammar/duelscript_v2.pest
git commit -m "fix: PEG ordering for cannot_attack_directly — fixes 37 parse failures"
```

---

### Task 2: Move shared infrastructure into v2

**Files:**
- Create: `src/v2/constants.rs` (engine constants extracted from type_mapper)
- Create: `src/v2/runtime.rs` (DuelScriptRuntime trait + CardFilter + Stat)
- Create: `src/v2/mock_runtime.rs` (MockRuntime for testing)
- Modify: `src/v2/mod.rs`
- Modify: `src/v2/compiler.rs` (update imports)

The v2 compiler currently imports:
- `crate::compiler::type_mapper as tm` — only uses constants (LOCATION_*, CATEGORY_*, etc.)
- `crate::compiler::callback_gen::DuelScriptRuntime` — the trait definition
- `crate::ast::Stat` — 4 references in `eval_v2_stat_field`
- `crate::test_harness::mock_runtime::MockRuntime` — in tests only

- [ ] **Step 1: Create `src/v2/constants.rs`**

Copy all `pub const` values from `src/compiler/type_mapper.rs` (lines 17-106) into a new file `src/v2/constants.rs`. These are engine bitfield constants (EFFECT_TYPE_*, CATEGORY_*, EVENT_*, LOCATION_*, PHASE_*, EFFECT_FLAG_*). No functions, no v1 AST imports. Also copy the `CountLimit` struct from `src/compiler/mod.rs` (it's just `pub struct CountLimit { pub count: u32, pub code: u32 }`).

```rust
// src/v2/constants.rs
// Engine constants matching EDOPro/YGOPro constant.lua

// Effect types
pub const EFFECT_TYPE_SINGLE:     u32 = 0x1;
pub const EFFECT_TYPE_FIELD:      u32 = 0x2;
// ... (copy all constants from type_mapper.rs lines 17-106)

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountLimit {
    pub count: u32,
    pub code: u32,
}
```

To get the exact content, read `src/compiler/type_mapper.rs` lines 17-106 and copy verbatim.

- [ ] **Step 2: Create `src/v2/runtime.rs`**

Extract the `DuelScriptRuntime` trait from `src/compiler/callback_gen.rs` (lines 27-162) into `src/v2/runtime.rs`. Also move the two v1 types it depends on:

```rust
// src/v2/runtime.rs

/// Card filter for runtime queries
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardFilter {
    Monster, Spell, Trap, Card, Token,
    NonTokenMonster, TunerMonster, NonTunerMonster,
    NormalMonster, EffectMonster, FusionMonster,
    SynchroMonster, XyzMonster, LinkMonster, RitualMonster,
    ArchetypeMonster(String), ArchetypeCard(String), NamedCard(String),
}

/// Stat identifier for card queries
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stat {
    Atk, Def, Level, Rank, BaseAtk, BaseDef, OriginalAtk, OriginalDef,
}

/// Trait that engines implement to expose game state and operations
/// to compiled DuelScript closures.
pub trait DuelScriptRuntime {
    // ... (copy the full trait from callback_gen.rs lines 27-162,
    //      replacing crate::ast::CardFilter with CardFilter
    //      and crate::ast::Stat with Stat)
}
```

- [ ] **Step 3: Create `src/v2/mock_runtime.rs`**

Copy `src/test_harness/mock_runtime.rs` into `src/v2/mock_runtime.rs`. Update its imports:
- Replace `use crate::ast::*;` with `use super::runtime::{CardFilter, Stat};`
- Replace `use crate::compiler::callback_gen::DuelScriptRuntime;` with `use super::runtime::DuelScriptRuntime;`
- Replace `use super::super::compiler::type_mapper;` (if any) with `use super::constants;`

Also copy `src/test_harness/scenario.rs` content into mock_runtime.rs or a separate file — the DuelScenario builder. Update its imports similarly: replace `crate::compiler::{compile_card, CompiledCard}` with the v2 equivalents.

- [ ] **Step 4: Update `src/v2/mod.rs`**

```rust
pub mod ast;
pub mod parser;
pub mod validator;
pub mod compiler;
pub mod constants;
pub mod runtime;
#[cfg(test)]
pub mod mock_runtime;
```

- [ ] **Step 5: Update `src/v2/compiler.rs` imports**

Replace:
```rust
use crate::compiler::type_mapper as tm;
use crate::compiler::callback_gen::DuelScriptRuntime;
```
With:
```rust
use super::constants as tm;
use super::runtime::{DuelScriptRuntime, Stat, CardFilter};
```

Replace the 4 occurrences of `crate::ast::Stat::*` with just `Stat::*`:
- Line 555: `&crate::ast::Stat::Atk` → `&Stat::Atk`
- Line 557: `&crate::ast::Stat::Def` → `&Stat::Def`
- Line 558: `crate::ast::Stat::Level` → `&Stat::Level`
- Line 559: `crate::ast::Stat::Rank` → `&Stat::Rank`

Replace test imports:
```rust
// old:
use crate::test_harness::mock_runtime::MockRuntime;
// new:
use super::mock_runtime::MockRuntime;
```

- [ ] **Step 6: Verify v2 tests still pass**

Run: `cargo test v2:: 2>&1 | grep "test result"`

Expected: all v2 tests pass (62 lib tests include the v2 tests).

- [ ] **Step 7: Commit**

```bash
git add src/v2/constants.rs src/v2/runtime.rs src/v2/mock_runtime.rs src/v2/mod.rs src/v2/compiler.rs
git commit -m "refactor: extract shared infrastructure into v2 module"
```

---

### Task 3: Delete all v1 code

**Files to delete:**
- `grammar/duelscript.pest` (v1 grammar, 1248 lines)
- `src/ast.rs` (v1 AST, 1467 lines)
- `src/parser.rs` (v1 parser, 3350 lines)
- `src/validator.rs` (v1 validator, 811 lines)
- `src/compiler/` (entire directory — callback_gen.rs, expr_eval.rs, type_mapper.rs, mod.rs)
- `src/engine/` (entire directory — bridge.rs, mod.rs — uses v1 AST)
- `src/test_harness/` (entire directory — uses v1 compiler)
- `src/migrate.rs` (Lua→v1 migration)
- `src/fmt.rs` (v1 formatter)
- `src/lua_transpiler.rs` (v1 Lua transpiler)
- `src/database.rs` (coupled to v1 AST)
- `src/llm.rs` (calls v1 parse)
- `src/python.rs` (exposes v1 types)

**Binaries to delete:**
- `src/bin/migrate_batch.rs` (Lua→v1)
- `src/bin/migrate_v2.rs` (v1→v2, migration done)
- `src/bin/llm_generate.rs` (uses v1 llm module)
- `src/bin/duelscript_lsp.rs` (uses v1 parser — will rebuild later for v2)

**Tests to delete:**
- `tests/sprint3_cards.rs`
- `tests/sprint4_cards.rs`
- `tests/sprint5_cards.rs`
- `tests/batch_migrate.rs`
- `tests/cards_snapshot.rs`
- `tests/cards_runtime.rs`
- `tests/corpus_smoke.rs`
- `tests/differential.rs`
- `tests/lsp_smoke.rs`
- `tests/v06_features.rs`
- `tests/snapshots/` (entire directory)

**Card directories to delete:**
- `cards/official/` (13,298 v1 format cards)
- `cards/test/` (80 v1 test cards)
- `cards/audit/` (5 v1 audit cards)

**Other:**
- `examples/parse_card.rs` (uses v1)

- [ ] **Step 1: Delete v1 source files**

```bash
rm src/ast.rs src/parser.rs src/validator.rs src/migrate.rs src/fmt.rs src/lua_transpiler.rs src/database.rs src/llm.rs src/python.rs
rm -rf src/compiler/ src/engine/ src/test_harness/
rm grammar/duelscript.pest
```

- [ ] **Step 2: Delete v1 binaries**

```bash
rm src/bin/migrate_batch.rs src/bin/migrate_v2.rs src/bin/llm_generate.rs src/bin/duelscript_lsp.rs
```

- [ ] **Step 3: Delete v1 tests and snapshots**

```bash
rm tests/sprint3_cards.rs tests/sprint4_cards.rs tests/sprint5_cards.rs
rm tests/batch_migrate.rs tests/cards_snapshot.rs tests/cards_runtime.rs
rm tests/corpus_smoke.rs tests/differential.rs tests/lsp_smoke.rs tests/v06_features.rs
rm -rf tests/snapshots/
```

- [ ] **Step 4: Delete v1 card directories and example**

```bash
rm -rf cards/official/ cards/test/ cards/audit/
rm examples/parse_card.rs
rmdir examples/ 2>/dev/null
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "remove: delete all v1 code, tests, and card directories"
```

---

### Task 4: Update lib.rs and Cargo.toml

**Files:**
- Modify: `src/lib.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Rewrite `src/lib.rs`**

Replace the entire file with:

```rust
// DuelScript — a standalone scripting language for Yu-Gi-Oh card mechanics.

pub mod v2;
pub mod cdb;

// Re-export the v2 public API at crate root for convenience
pub use v2::parser::{parse_v2, V2ParseError};
pub use v2::validator::{validate_v2, ValidationReport};
pub use v2::compiler::compile_card_v2;
pub use v2::ast;
pub use v2::constants;
pub use v2::runtime::{DuelScriptRuntime, CardFilter, Stat};
```

- [ ] **Step 2: Clean up Cargo.toml**

Remove deleted binary entries and unused features:

```toml
[features]
default = []
cdb = ["rusqlite"]
lsp = ["tower-lsp", "tokio", "serde_json", "serde"]
```

Remove these `[[bin]]` entries:
- `migrate_batch`
- `migrate_v2`
- `llm_generate`
- `duelscript_lsp` (temporarily — will add back when v2 LSP is built)

Remove these features: `python`, `lua_transpiler`, `llm`

Remove unused dependencies: `pyo3`, `reqwest`

Remove `cdylib` from `crate-type` (that was for Python bindings):
```toml
[lib]
name = "duelscript"
crate-type = ["rlib"]
```

Remove the `[[example]]` entry.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check 2>&1 | tail -5`

Expected: no errors (warnings OK at this stage).

- [ ] **Step 4: Commit**

```bash
git add src/lib.rs Cargo.toml
git commit -m "refactor: clean lib.rs and Cargo.toml for v2-only"
```

---

### Task 5: Update CLI to use v2

**Files:**
- Modify: `src/bin/duelscript.rs`

The CLI has 4 commands: `check`, `inspect`, `fmt`, `test`. Update each to use v2 parser/validator/compiler.

- [ ] **Step 1: Rewrite the CLI imports and check command**

Replace the imports:
```rust
use duelscript::v2::parser::parse_v2;
use duelscript::v2::validator::{validate_v2, Severity};
use duelscript::v2::compiler::compile_card_v2;
use duelscript::v2::runtime::DuelScriptRuntime;
```

Update `cmd_check` to use `parse_v2` and `validate_v2`. The v2 validator returns a `ValidationReport` directly — check `src/v2/validator.rs` for the exact API (the `validate_v2` function takes a `&File` and returns `ValidationReport`).

Update `cmd_inspect` to use `parse_v2` and print v2 AST fields (card.fields.card_types, card.effects, card.passives, card.restrictions, card.replacements, card.summon).

For `cmd_fmt` — since we deleted the v1 formatter and don't have a v2 formatter yet, either remove the command or stub it with a "not yet implemented" message.

For `cmd_test` — use `compile_card_v2` instead of v1's `compile_file`. The v2 compiler returns `CompiledCardV2` which has `effects: Vec<CompiledEffectV2>` with `callbacks` containing condition/cost/target/operation closures. Wire up MockRuntime from `duelscript::v2::mock_runtime::MockRuntime`.

- [ ] **Step 2: Verify CLI works**

```bash
cargo run -- check cards/v2_test/pot_of_greed.ds
cargo run -- check cards/v2_test/
cargo run -- inspect cards/v2_test/pot_of_greed.ds
cargo run -- test cards/v2_test/pot_of_greed.ds
```

Expected: check shows OK for all 151 cards, inspect prints v2 AST, test executes effects.

- [ ] **Step 3: Commit**

```bash
git add src/bin/duelscript.rs
git commit -m "feat: CLI now uses v2 parser/validator/compiler"
```

---

### Task 6: Rename v2 paths to primary

**Files:**
- Rename: `grammar/duelscript_v2.pest` -> `grammar/duelscript.pest`
- Rename: `cards/v2_test/` -> `cards/goat/`
- Rename: `cards/v2_official/` -> `cards/official/`
- Modify: `src/v2/parser.rs` (update grammar path in `#[grammar = "..."]`)
- Modify: all test `include_str!` paths that reference `v2_test`

- [ ] **Step 1: Rename grammar file**

```bash
mv grammar/duelscript_v2.pest grammar/duelscript.pest
```

Update the `#[grammar = "..."]` attribute in `src/v2/parser.rs`:
```rust
// Find: #[grammar = "grammar/duelscript_v2.pest"]
// Replace: #[grammar = "grammar/duelscript.pest"]
```

This is typically in the `#[derive(Parser)]` block near the top of the file.

- [ ] **Step 2: Rename card directories**

```bash
mv cards/v2_test cards/goat
mv cards/v2_official cards/official
```

- [ ] **Step 3: Update all test paths**

In `src/v2/parser.rs`, replace all `cards/v2_test/` with `cards/goat/` and `cards/v2_official` with `cards/official`.

In `src/v2/validator.rs`, replace `cards/v2_test` with `cards/goat`.

In `src/v2/compiler.rs`, replace `cards/v2_test/` with `cards/goat/`.

- [ ] **Step 4: Verify tests pass**

Run: `cargo test 2>&1 | grep "test result"` — all should pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "rename: v2 paths become primary (grammar, cards)"
```

---

### Task 7: Final cleanup and push

- [ ] **Step 1: Run full test suite**

```bash
cargo test 2>&1 | tail -20
```

Verify: 0 failures, 0 warnings.

- [ ] **Step 2: Check for stale references**

```bash
grep -r "v1\|duelscript\.pest\b" src/ --include="*.rs" | grep -v "duelscript.pest" | head -20
grep -r "cards/test\b\|cards/official\b" src/ --include="*.rs" | head -10
```

Fix any remaining stale path references.

- [ ] **Step 3: Update Cargo.toml version**

Bump version to `1.0.0` — this is the v2 release:

```toml
version = "1.0.0"
```

- [ ] **Step 4: Final commit and push**

```bash
git add -A
git commit -m "DuelScript v1.0: v2 promoted to primary, v1 removed"
git push
```
