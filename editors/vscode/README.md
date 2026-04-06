# DuelScript for VS Code

Syntax highlighting for DuelScript (`.ds`) files.

## Installation

### From source (development)

1. Copy or symlink this folder into your VS Code extensions:
   ```bash
   ln -s /path/to/duelscript/editors/vscode ~/.vscode/extensions/duelscript
   ```
2. Reload VS Code (`Cmd+Shift+P` → "Reload Window")
3. Open any `.ds` file — syntax highlighting is automatic

### Manual VSIX (coming soon)

```bash
cd editors/vscode
npx vsce package
code --install-extension duelscript-0.5.0.vsix
```

## Features

- Keyword highlighting for all DuelScript constructs
- Card types, attributes, races, zones in distinct colors
- Game actions highlighted as function calls
- Trigger and condition keywords
- Comment support (`//` and `/* */`)
- Bracket matching and auto-closing
- Code folding on `{ }` blocks

## Screenshot

```ds
card "Ash Blossom & Joyous Spring" {
    type: Effect Monster | Tuner      // storage.type
    attribute: FIRE                    // constant.language
    race: Zombie                       // constant.language
    level: 3                           // constant.numeric
    atk: 0
    def: 1800

    effect "Negate" {                  // keyword.control
        speed: spell_speed_2           // keyword.other
        once_per_turn: hard
        optional: true                 // constant.language
        condition: chain_link_includes [add_to_hand, special_summon]
        trigger: opponent_activates [search | draw]
        cost {
            discard self               // entity.name.function
        }
        on_resolve {
            negate effect              // entity.name.function
        }
    }
}
```
