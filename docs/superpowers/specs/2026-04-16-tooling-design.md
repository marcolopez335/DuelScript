# DuelScript Tooling Design

## Goal

Give DuelScript authors the tools to write cards efficiently: auto-formatting, effect testing, real-time editor diagnostics.

## Two Plans

Split into two independent implementation plans:

1. **CLI Commands** (`fmt` + `test`) — standalone binary subcommands
2. **LSP + VS Code** — editor integration

## Plan 1: CLI Commands

### `duelscript fmt <file|dir>`

Auto-formats `.ds` files to canonical style.

**Behavior:**
- Parses input with v2 parser
- Pretty-prints AST back to source with canonical formatting:
  - 4-space indent
  - One statement per line inside blocks
  - Field declarations with aligned colons
  - Blank line between top-level blocks (summon, effect, passive, etc.)
- Verifies the formatted output parses back to an equivalent AST
- Writes file in-place if changed
- `--check` flag: dry-run, prints changed files, exits 1 if any differ

**Success criteria:**
- `cargo run --bin duelscript -- fmt cards/goat/` formats 151 cards without breaking any of them
- Re-running `fmt` on already-formatted files is a no-op
- `--check` mode exits 0 on clean files, 1 on dirty files

### `duelscript test <file.ds>`

Compiles a card and executes each effect against MockRuntime.

**Behavior:**
- Parses + compiles the card
- For each effect, runs: condition → cost (check + pay) → targets (check + select) → operation
- Prints compact summary:
  ```
  Pot of Greed (55144522)
    Effect 1 "Draw 2":
      condition:  PASS
      cost:       none
      targets:    none
      operation:  draw(0, 2)
    State: P0 hand=2 deck=38 LP=8000
  ```
- Flags:
  - `--lp N` — set both players' starting LP
  - `--hand "id,id,..."` — seed player 0's hand
  - `--deck "id,id,..."` — seed player 0's deck
  - `--verbose` — dump full call log

**Success criteria:**
- `duelscript test cards/goat/pot_of_greed.ds` shows `draw` was called
- `duelscript test cards/goat/lava_golem.ds --lp 4000` respects the LP override

## Plan 2: LSP + VS Code

### LSP Server (`duelscript_lsp` binary)

Implements Language Server Protocol using `tower-lsp`.

**Capabilities:**
- **Diagnostics**: parse + validate on every document change, send errors/warnings with line ranges
- **Hover**: at cursor position, show:
  - Card name + type if hovering a card name
  - Effect metadata (speed, trigger, frequency) if hovering effect name
  - Grammar context (which block you're in) if hovering a keyword
- **Completion**: trigger on keywords:
  - Top-level: `card`, `id:`, `type:`, `attribute:`, `race:`, `level:`, `atk:`, `def:`, `summon {`, `effect {`, `passive {`, `restriction {`, `replacement {`
  - Inside effect: `speed:`, `trigger:`, `condition:`, `cost {`, `resolve {`, `target`, `mandatory`, `once_per_turn`
  - Inside resolve/cost: action names (`draw`, `destroy`, `banish`, `special_summon`, etc.)
- **Go-to-definition**: for `target` references, jump to the `target (...)` declaration above

**Success criteria:**
- Typing invalid syntax shows red squigglies within 200ms
- Hovering `speed: 2` in an effect shows "Spell Speed 2 — Quick effect"
- Typing `dr` in a resolve block offers `draw` completion

### VS Code Extension

Extends the existing syntax-only extension with LSP.

**Changes:**
- Add `src/extension.ts` — launches `duelscript_lsp` and connects via stdio
- Add `snippets/duelscript.json` — `card`, `effect`, `passive`, `restriction` stubs
- Update `package.json`: add activation events, language client dependencies

**Packaging:**
- `npm install` + `vsce package` produces a `.vsix`
- Install locally via `code --install-extension duelscript-*.vsix`

**Success criteria:**
- Opening a `.ds` file in VS Code activates the LSP and shows diagnostics
- Typing `card` and pressing Tab expands to a full card skeleton
- Hovering over an effect name shows compiled metadata in a tooltip

## Non-Goals

- Formatter preserving comments perfectly (v1 had this — for v2 we can round-trip via AST and accept comment loss as v1.0 behavior)
- LSP code actions / refactorings
- Debug Adapter Protocol (DAP) — effect execution through the IDE

## Execution Order

1. Plan 1 first (CLI commands) — grounds the work, no new dependencies
2. Plan 2 second (LSP + VS Code) — depends on the validator/parser being stable, which it is
