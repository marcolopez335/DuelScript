---
name: corpus-audit-pattern
description: Use when scanning the duelscript corpus and the lua source to characterise a shape — counting prevalence, bucketing by sub-shape, identifying safety filters for an apply pass. Provides reusable python-regex templates for the common audit shapes.
---

# Corpus Audit Pattern

Every phase / chore PR starts with an audit. This skill is the script template.

## Why python over rust

Audits are throwaway. Python's `re` module + glob is faster to iterate than building a small rust bin. When a regex pattern stabilises, promote it into the translator (`src/lua_ast.rs`).

## Template — count cards matching a lua shape

```python
import re, glob, os

LUA_DIR = '/Users/marco/git/CardScripts/official'
DS_DIR  = 'cards/official'

empty_re = re.compile(r'resolve \{\s*\}')

candidates = []
for ds in glob.glob(os.path.join(DS_DIR, '*.ds')):
    txt = open(ds).read()
    if not empty_re.search(txt): continue        # filter ds-side first (cheap)

    stem = os.path.basename(ds).replace('.ds', '')
    lua = os.path.join(LUA_DIR, stem + '.lua')
    if not os.path.exists(lua): continue
    src = open(lua, errors='ignore').read()

    # YOUR LUA-SHAPE CHECK HERE
    if 'EFFECT_UPDATE_ATTACK' not in src: continue

    candidates.append(stem)

print(f'candidates: {len(candidates)}')
print('sample:', candidates[:5])
```

## Template — bucket by sub-shape

```python
buckets = {}
for stem in candidates:
    src = open(os.path.join(LUA_DIR, stem + '.lua'), errors='ignore').read()
    m = re.search(r'SetType\(([^)]+)\)', src)
    key = m.group(1).strip() if m else 'NO_SETTYPE'
    buckets.setdefault(key, 0)
    buckets[key] += 1

for k, v in sorted(buckets.items(), key=lambda x: -x[1])[:10]:
    print(f'  {v:5d}  {k}')
```

## Template — extract the chain following an `Effect.CreateEffect`

```python
chains = re.findall(
    r'local\s+(\w+)\s*=\s*Effect\.CreateEffect\([^)]*\)(.*?)(?=local\s+\w+\s*=\s*Effect\.CreateEffect|\bend\b)',
    body, re.DOTALL,
)
for binding, chain_body in chains:
    code = re.search(rf'{binding}:SetCode\(([^)]+)\)', chain_body)
    val  = re.search(rf'{binding}:SetValue\(\s*(-?\d+)\s*\)', chain_body)
    has_op = re.search(rf'{binding}:SetOperation\(', chain_body)
    if not code or not val or has_op: continue
    # candidate
```

## Template — apply with safety filters (chore phase)

```python
import re, glob, os

DS_DIR = 'cards/official'
empty_re   = re.compile(r'\n\s*effect\s+"[^"]*"\s*\{[^{}]*?resolve\s*\{\s*\}\s*\}\s*\n', re.DOTALL)
passive_re = re.compile(r'\bpassive\s+"', re.MULTILINE)
type_re    = re.compile(r'^\s*type:\s*Equip\s+Spell\s*$', re.MULTILINE)

dropped = 0
for f in sorted(glob.glob(os.path.join(DS_DIR, '*.ds'))):
    txt = open(f).read()
    # FILTER STACK — every condition must hold for safe apply
    if not type_re.search(txt): continue
    if not passive_re.search(txt): continue
    blocks = re.findall(r'effect\s+"[^"]*"\s*\{((?:[^{}]|\{[^{}]*\})*)\}', txt, re.DOTALL)
    if len(blocks) != 1: continue
    body = blocks[0]
    if any(k in body for k in ('cost {', 'choose {', 'condition', 'target ', 'target:', 'trigger:', 'restriction')):
        continue
    if not re.search(r'resolve \{\s*\}', body): continue

    # SAFE — apply
    new_txt = empty_re.sub('\n', txt, count=1)
    if new_txt == txt: continue
    new_txt = re.sub(r'\n{3,}', '\n\n', new_txt)
    open(f, 'w').write(new_txt)
    dropped += 1

print(f'dropped: {dropped}')
```

## Patterns library

### Find lua chains in `s.initial_effect`

```python
m = re.search(r'function s\.initial_effect\(c\)\s*(.*?)\n\s*end\s*$',
              src, re.DOTALL | re.MULTILINE)
if not m: return
body = m.group(1)
```

### Find handler bodies (s.activate, s.operation, s.condition, etc.)

```python
m = re.search(rf'function s\.{handler_name}\([^)]*\)\s*(.*?)\n\s*end\s*$',
              src, re.DOTALL | re.MULTILINE)
```

### Distinguish effect on self vs on target

```python
re.search(rf'\bc:RegisterEffect\({binding}\)',  chain_body)   # on self
re.search(rf'\btc:RegisterEffect\({binding}\)', chain_body)   # on target
```

### Detect a for-tc-in-aux.Next loop

```python
m = re.search(r'for\s+(\w+)\s+in\s+aux\.Next\((\w+)\)\s+do', chain_body)
loop_var, source_group = m.group(1), m.group(2) if m else (None, None)
```

## Hazards

- **Bias toward over-restriction.** Filter cascades are easier to relax later than to backfill bugs from a too-loose filter.
- **Never skip the count step.** The pattern of "regex looks right → apply to corpus → discover edge case" wastes a PR cycle. Always print the count and a sample first.
- **Pyhton regex is line-anchored by default for `^$`** — use `re.MULTILINE` for line-anchors over multi-line strings.
- **Watch for `\n{3,}` collapse.** When deleting a block, leftover blank lines are common. Always end with `re.sub(r'\n{3,}', '\n\n', txt)`.
- **Errors='ignore' on open.** Some lua files have non-UTF8 bytes. Ignore on read; never write back to the lua source — the lua corpus is read-only.

## Where this skill applies vs related skills

- Audit step of `translator-phase-pattern` skill.
- Lua-shape detection ↔ `lua-corpus-context` for which constants matter.
- Yield ranking ↔ `error-triage` skill.
