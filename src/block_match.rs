// ============================================================
// block_match — Phase 20: signature-based handler→block matcher.
//
// The apply passes of `lua_translate` map lua-walk effects onto the
// `effect "Effect N"` blocks of a .ds file. The historical mapping was
// purely positional (i-th index-consuming walk effect → i-th block),
// which lies whenever a chain owns a block without consuming an index
// (bare EFFECT_TYPE_ACTIVATE shells, clones of summon-helper chains,
// unregistered clones) or when the .ds generator skipped a text effect.
// The `block_alignment_hazard` gates introduced in Phase 12/14 prevent
// the resulting mis-fills by skipping EVERYTHING past a hazard ordinal
// — stranding ~1,650 blocks corpus-wide.
//
// This module replaces the blind zip with a conservative signature
// alignment:
//
//   - Every lua chain (and every phantom clone ordinal) becomes an
//     ordered ENTITY with a signature derived from its Set* calls.
//   - Every .ds effect block becomes an ordered BLOCK with a signature
//     parsed from its header lines.
//   - A Needleman–Wunsch-style alignment (monotonic by construction —
//     crossings are impossible) scores entity/block pairs; a pair is
//     CLAIMED only when it appears in EVERY optimal alignment (counted
//     exactly via path counting), its own score clears MIN_CLAIM, and
//     no mutable-feature veto contradicts it.
//
// Correctness bar:
//   - Hazard-free cards take a positional fast path IDENTICAL to the
//     historical behavior — the matcher only changes outcomes where the
//     hazard gates skipped fills.
//   - Signature scores use only IMMUTABLE .ds features (speed / timing /
//     trigger / mandatory / once_per_turn) — features that no apply pass
//     ever injects — so assignments are a fixed point of apply: a rerun
//     sees the same signatures and computes the same claims.
//   - Mutable features (cost{} / target / condition:) act only as a
//     VETO: a block demanding a feature the chain lacks is never
//     claimed. Passes only ever inject features their matched chain
//     has, so a veto can never be created for a pair that was claimed.
// ============================================================

use crate::lua_ast::{EffectSkeleton, LuaReport};

// ── Block ranges ─────────────────────────────────────────────

/// Byte ranges of every `effect "..." { ... }` block in `txt`, in order.
/// Each range is `(start_of_effect_keyword, position_after_closing_brace)`.
/// Brace-balanced — handles nested `resolve { ... }` / `cost { ... }` etc.
pub fn effect_block_ranges(txt: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut search = 0usize;
    let bytes = txt.as_bytes();
    while let Some(rel) = txt[search..].find("effect \"") {
        let abs_eff = search + rel;
        let Some(open_rel) = txt[abs_eff..].find('{') else { break };
        let abs_open = abs_eff + open_rel;
        let mut depth = 1usize;
        let mut i = abs_open + 1;
        while i < bytes.len() {
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 { break; }
                }
                _ => {}
            }
            i += 1;
        }
        if depth != 0 { break; }
        let close_after = i + 1;
        out.push((abs_eff, close_after));
        search = close_after;
    }
    out
}

// ── Signatures ───────────────────────────────────────────────

/// What kind of block-ownership a walk-side entity has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    /// Owns a block and consumes a positional index today (operation
    /// handler or summon helper) — the only kind whose claim is used to
    /// fill.
    Consumer,
    /// Owns a block but consumes no index (bare EFFECT_TYPE_ACTIVATE
    /// shell) — aligns so downstream consumers can re-anchor.
    Owner,
    /// May or may not own a block (pure passive chain — the .ds
    /// generator sometimes emitted a block for passive text effects,
    /// sometimes not). Skipping it costs nothing.
    Optional,
    /// Clone-hazard ordinal with no skeleton: may own a block, signature
    /// unknown. Matches any block at score 0 and skips at cost 0 — pure
    /// ambiguity that only strong downstream anchors can resolve.
    Phantom,
}

/// Walk-side entity signature. `speed` / `trigger_line` / `mandatory`
/// are EXPECTATIONS about the block (None = no expectation); the
/// `has_*` flags are used only by the mutable-feature veto.
#[derive(Debug, Clone)]
pub struct EntitySig {
    pub kind: EntityKind,
    pub speed: Option<u8>,
    pub trigger_line: Option<bool>,
    pub mandatory: Option<bool>,
    pub once_per_turn: bool,
    pub has_cost: bool,
    pub has_target: bool,
    pub has_condition: bool,
}

impl EntitySig {
    fn phantom() -> Self {
        EntitySig {
            kind: EntityKind::Phantom,
            speed: None,
            trigger_line: None,
            mandatory: None,
            once_per_turn: false,
            // A phantom's chain state is unknown — assume it could have
            // any of the mutable features so the veto never fires on it.
            has_cost: true,
            has_target: true,
            has_condition: true,
        }
    }
}

/// .ds-side block signature. The first five fields are immutable under
/// the apply passes and feed the score; the `has_*` fields are mutable
/// (Passes C/D/E inject them) and feed only the veto.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockSig {
    pub speed: Option<u8>,
    pub trigger_line: bool,
    pub mandatory: bool,
    pub once_per_turn: bool,
    pub has_cost: bool,
    pub has_target: bool,
    pub has_condition: bool,
}

/// Card frame, as far as expected spell speed is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardKind { Monster, Spell, QuickPlaySpell, Trap, CounterTrap }

/// Derive the card frame from the .ds `type:` line.
pub fn card_kind_from_ds(txt: &str) -> CardKind {
    let line = txt.lines()
        .map(str::trim_start)
        .find(|l| l.starts_with("type:"))
        .unwrap_or("");
    if line.contains("Counter Trap") { CardKind::CounterTrap }
    else if line.contains("Trap") { CardKind::Trap }
    else if line.contains("Quick-Play Spell") { CardKind::QuickPlaySpell }
    else if line.contains("Spell") { CardKind::Spell }
    else { CardKind::Monster }
}

/// Parse the signature of every effect block in `txt`, in block order.
/// Only lines at block top level (depth 1) count — `resolve { … }`
/// bodies and `choose { option … }` interiors are skipped so action
/// text can't fake a header line.
pub fn parse_block_sigs(txt: &str) -> Vec<BlockSig> {
    effect_block_ranges(txt).into_iter().map(|(lo, hi)| {
        let block = &txt[lo..hi];
        let mut sig = BlockSig::default();
        // Depth relative to the block's own opening brace.
        let Some(open) = block.find('{') else { return sig };
        let mut depth = 1i32;
        for line in block[open + 1..].lines() {
            let t = line.trim();
            if depth == 1 {
                if let Some(v) = t.strip_prefix("speed:") {
                    sig.speed = v.trim().parse().ok();
                } else if t.starts_with("timing:") || t.starts_with("trigger:") {
                    sig.trigger_line = true;
                } else if t == "mandatory" {
                    sig.mandatory = true;
                } else if t.starts_with("once_per_turn") {
                    sig.once_per_turn = true;
                } else if t.starts_with("cost {") || t == "cost{" {
                    sig.has_cost = true;
                } else if t.starts_with("target ") || t.starts_with("target(") {
                    sig.has_target = true;
                } else if t.starts_with("condition:") {
                    sig.has_condition = true;
                }
            }
            for c in line.chars() {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            if depth <= 0 { break; }
        }
        sig
    }).collect()
}

/// Build the signature of one walk skeleton against the card frame.
pub fn entity_sig(skel: &EffectSkeleton, kind: CardKind) -> EntitySig {
    let ty = skel.set_calls.iter()
        .find(|(m, _)| m == "SetType")
        .and_then(|(_, a)| a.first().map(String::as_str))
        .unwrap_or("");
    let quick = ty.contains("EFFECT_TYPE_QUICK_O") || ty.contains("EFFECT_TYPE_QUICK_F");
    let trigger_o = ty.contains("EFFECT_TYPE_TRIGGER_O");
    let trigger_f = ty.contains("EFFECT_TYPE_TRIGGER_F");
    let flip = ty.contains("EFFECT_TYPE_FLIP");
    let ignition = ty.contains("EFFECT_TYPE_IGNITION");
    let activate = ty.contains("EFFECT_TYPE_ACTIVATE");
    let continuous = ty.contains("EFFECT_TYPE_CONTINUOUS");
    let is_consumer = skel.is_summon_helper() || skel.operation_handler.is_some();
    let ekind = if is_consumer {
        EntityKind::Consumer
    } else if activate {
        EntityKind::Owner
    } else {
        EntityKind::Optional
    };
    // Expected `speed:` — mirrors how the .ds generator labeled blocks.
    // Counter traps are too rare/inconsistent to trust (3 `speed: 3`
    // lines corpus-wide), so they carry no expectation.
    let speed = match kind {
        CardKind::CounterTrap => None,
        CardKind::Trap => Some(2),
        CardKind::QuickPlaySpell => {
            if activate || quick { Some(2) } else { Some(1) }
        }
        CardKind::Spell | CardKind::Monster => {
            if quick { Some(2) } else { Some(1) }
        }
    };
    // Expected presence of a `timing:` / `trigger:` header line.
    let trigger_line = if trigger_o || trigger_f || flip {
        Some(true)
    } else if ignition {
        Some(false)
    } else if activate && kind != CardKind::CounterTrap {
        Some(false)
    } else if ekind == EntityKind::Optional {
        Some(false)
    } else {
        None // quick / continuous-with-op — the generator was inconsistent
    };
    // Expected `mandatory` keyword. Activation shells got `mandatory`
    // noise from the generator, so they carry no expectation.
    let mandatory = if trigger_f || (continuous && is_consumer) {
        Some(true)
    } else if trigger_o || quick || ignition || flip {
        Some(false)
    } else if ekind == EntityKind::Optional {
        Some(true)
    } else {
        None
    };
    let has = |m: &str| skel.set_calls.iter().any(|(name, _)| name == m);
    EntitySig {
        kind: ekind,
        speed,
        trigger_line,
        mandatory,
        once_per_turn: has("SetCountLimit"),
        has_cost: skel.cost_handler.is_some() || has("SetCost"),
        has_target: skel.target_handler.is_some() || has("SetTarget"),
        has_condition: skel.condition_handler.is_some() || has("SetCondition"),
    }
}

// ── The matcher ──────────────────────────────────────────────

/// Cost of leaving a block unmatched (extra text-effect block the walk
/// doesn't model).
const GAP_BLOCK: i64 = -2;
/// Minimum pair score a claim needs — positive signature evidence, not
/// just alignment-by-elimination.
const MIN_CLAIM: i64 = 5;
/// Sentinel for unreachable DP cells.
const NEG_INF: i64 = i64::MIN / 4;

fn gap_entity(e: &EntitySig) -> i64 {
    match e.kind {
        EntityKind::Consumer | EntityKind::Owner => -2,
        EntityKind::Optional | EntityKind::Phantom => 0,
    }
}

/// Signature compatibility score, immutable features only.
fn pair_score(e: &EntitySig, b: &BlockSig) -> i64 {
    if e.kind == EntityKind::Phantom { return 0; }
    let mut s = 0i64;
    if let (Some(es), Some(bs)) = (e.speed, b.speed) {
        s += if es == bs { 3 } else { -4 };
    }
    if let Some(t) = e.trigger_line {
        s += if t == b.trigger_line {
            if t { 2 } else { 1 }
        } else {
            -2
        };
    }
    if let Some(m) = e.mandatory {
        s += if m == b.mandatory { 1 } else { -1 };
    }
    s += match (e.once_per_turn, b.once_per_turn) {
        (true, true) => 2,
        (false, false) => 0,
        _ => -1,
    };
    s
}

/// Mutable-feature contradiction: the block demands a sub-structure the
/// chain doesn't have. Never claimed, even when forced by alignment.
fn veto(e: &EntitySig, b: &BlockSig) -> bool {
    (b.has_cost && !e.has_cost)
        || (b.has_target && !e.has_target)
        || (b.has_condition && !e.has_condition)
}

/// A phantom's block-ownership is unknown, so every subset of phantoms
/// is a possible world; more than this many phantoms (64 worlds) and we
/// refuse to match at all. Real cards have 0–2.
const MAX_PHANTOMS: usize = 6;
/// Gap cost that forces an entity to be matched in a given world.
const MUST_OWN_GAP: i64 = -1_000_000;

/// Align `ents` (walk-side, source order) against `blocks` (.ds order)
/// and return one `Option<block index>` per entity.
///
/// A pair is claimed only when:
///   1. it lies on EVERY optimal alignment (exact path counting — a tie
///      between alignments that disagree about the pair yields None),
///   2. it survives EVERY phantom interpretation: each phantom entity
///      either owns some block or owns none, and the claim must come out
///      identical in all 2^p worlds,
///   3. its own score clears [`MIN_CLAIM`], and
///   4. the mutable-feature [`veto`] doesn't contradict it.
///
/// Monotonicity is structural: alignments cannot cross, so neither can
/// claims.
pub fn match_blocks(ents: &[EntitySig], blocks: &[BlockSig]) -> Vec<Option<usize>> {
    let n = ents.len();
    let out = vec![None; n];
    if n == 0 || blocks.is_empty() { return out; }
    let phantoms: Vec<usize> = (0..n)
        .filter(|&i| ents[i].kind == EntityKind::Phantom)
        .collect();
    if phantoms.len() > MAX_PHANTOMS { return out; }

    let mut merged: Option<Vec<Option<usize>>> = None;
    for mask in 0u32..(1u32 << phantoms.len()) {
        // World: phantoms in `mask` definitely own a block (must be
        // matched); the rest own none (dropped from the alignment).
        let mut world: Vec<usize> = Vec::with_capacity(n);
        let mut must_own: Vec<bool> = Vec::with_capacity(n);
        for (i, e) in ents.iter().enumerate() {
            if e.kind == EntityKind::Phantom {
                let p = phantoms.iter().position(|&x| x == i).unwrap();
                if mask & (1 << p) != 0 {
                    world.push(i);
                    must_own.push(true);
                }
            } else {
                world.push(i);
                must_own.push(false);
            }
        }
        let sub: Vec<&EntitySig> = world.iter().map(|&i| &ents[i]).collect();
        let claims_sub = align_forced(&sub, &must_own, blocks);
        let mut claims = vec![None; n];
        for (k, &i) in world.iter().enumerate() {
            claims[i] = claims_sub[k];
        }
        merged = Some(match merged {
            None => claims,
            Some(prev) => prev.into_iter().zip(claims)
                .map(|(a, b)| if a == b { a } else { None })
                .collect(),
        });
    }
    merged.unwrap_or(out)
}

/// One-world alignment: forced-pair claims for `ents` against `blocks`.
fn align_forced(
    ents: &[&EntitySig],
    must_own: &[bool],
    blocks: &[BlockSig],
) -> Vec<Option<usize>> {
    let n = ents.len();
    let m = blocks.len();
    let mut out = vec![None; n];
    if n == 0 || m == 0 { return out; }

    let gap = |i: usize| if must_own[i] { MUST_OWN_GAP } else { gap_entity(ents[i]) };
    let score: Vec<Vec<i64>> = ents.iter()
        .map(|e| blocks.iter().map(|b| pair_score(e, b)).collect())
        .collect();

    // Forward DP: f[i][j] = best score aligning ents[..i] with
    // blocks[..j]; cf[i][j] = number of optimal paths reaching (i, j).
    let mut f = vec![vec![NEG_INF; m + 1]; n + 1];
    let mut cf = vec![vec![0u128; m + 1]; n + 1];
    f[0][0] = 0;
    cf[0][0] = 1;
    for i in 0..=n {
        for j in 0..=m {
            if i == 0 && j == 0 { continue; }
            let mut best = NEG_INF;
            if i > 0 && j > 0 { best = best.max(f[i - 1][j - 1] + score[i - 1][j - 1]); }
            if i > 0 { best = best.max(f[i - 1][j] + gap(i - 1)); }
            if j > 0 { best = best.max(f[i][j - 1] + GAP_BLOCK); }
            let mut cnt = 0u128;
            if i > 0 && j > 0 && f[i - 1][j - 1] + score[i - 1][j - 1] == best {
                cnt = cnt.saturating_add(cf[i - 1][j - 1]);
            }
            if i > 0 && f[i - 1][j] + gap(i - 1) == best {
                cnt = cnt.saturating_add(cf[i - 1][j]);
            }
            if j > 0 && f[i][j - 1] + GAP_BLOCK == best {
                cnt = cnt.saturating_add(cf[i][j - 1]);
            }
            f[i][j] = best;
            cf[i][j] = cnt;
        }
    }
    // Backward DP: b[i][j] = best score aligning ents[i..] with
    // blocks[j..]; cb[i][j] = number of optimal paths from (i, j).
    let mut bw = vec![vec![NEG_INF; m + 1]; n + 1];
    let mut cb = vec![vec![0u128; m + 1]; n + 1];
    bw[n][m] = 0;
    cb[n][m] = 1;
    for i in (0..=n).rev() {
        for j in (0..=m).rev() {
            if i == n && j == m { continue; }
            let mut best = NEG_INF;
            if i < n && j < m { best = best.max(bw[i + 1][j + 1] + score[i][j]); }
            if i < n { best = best.max(bw[i + 1][j] + gap(i)); }
            if j < m { best = best.max(bw[i][j + 1] + GAP_BLOCK); }
            let mut cnt = 0u128;
            if i < n && j < m && bw[i + 1][j + 1] + score[i][j] == best {
                cnt = cnt.saturating_add(cb[i + 1][j + 1]);
            }
            if i < n && bw[i + 1][j] + gap(i) == best {
                cnt = cnt.saturating_add(cb[i + 1][j]);
            }
            if j < m && bw[i][j + 1] + GAP_BLOCK == best {
                cnt = cnt.saturating_add(cb[i][j + 1]);
            }
            bw[i][j] = best;
            cb[i][j] = cnt;
        }
    }

    let total = f[n][m];
    let n_opt = cf[n][m];
    // Saturated path counts would make the all-optimal test unsound —
    // claim nothing (never happens for real cards: n, m ≲ 20).
    if n_opt == 0 || n_opt == u128::MAX { return out; }

    for i in 0..n {
        for j in 0..m {
            let s = score[i][j];
            if s < MIN_CLAIM { continue; }
            if veto(ents[i], &blocks[j]) { continue; }
            if f[i][j] + s + bw[i + 1][j + 1] != total { continue; }
            let through = cf[i][j].saturating_mul(cb[i + 1][j + 1]);
            if through == n_opt {
                out[i] = Some(j);
            }
        }
    }
    out
}

// ── Per-card assignment (fast path + matcher) ────────────────

/// The block assignment for one card: `by_effect[i]` is the .ds block
/// index the i-th entry of `walk.effects` may fill, or None (skip).
#[derive(Debug, Default)]
pub struct Assignments {
    pub by_effect: Vec<Option<usize>>,
    /// Consumers assigned by position — the historical behavior.
    pub positional: usize,
    /// Hazard-gated consumers the matcher rescued with a forced block.
    pub rescued: usize,
    /// Hazard-gated consumers left unassigned (ambiguous / unforced).
    pub ambiguous: usize,
}

fn is_consumer(e: &EffectSkeleton) -> bool {
    e.is_summon_helper() || e.operation_handler.is_some()
}

/// Compute the per-effect block assignment for one card.
///
/// Hazard-free cards take the positional fast path: the i-th consumer
/// maps to block i, exactly as the pre-Phase-20 counters did —
/// signatures are not even consulted. Cards with hazard-flagged effects
/// keep the positional mapping for the trusted (pre-hazard) prefix and
/// ask the matcher to rescue the flagged consumers; any inconsistency
/// between the matcher and the trusted prefix falls back to the
/// historical skip-everything-flagged behavior.
pub fn compute_assignments(walk: &LuaReport, ds_txt: &str) -> Assignments {
    let n = walk.effects.len();
    let consumers: Vec<usize> = (0..n).filter(|&i| is_consumer(&walk.effects[i])).collect();
    let mut a = Assignments { by_effect: vec![None; n], ..Default::default() };

    // Order-consistency guard (T38 S4): the passes count consumers in
    // `walk.effects` (BTreeMap binding-name) order, but .ds blocks were
    // generated in lua SOURCE order. When the two disagree — out-of-order
    // binding names (`e0` declared after `e1`, `ge1` between `e1`/`e2`),
    // or an anonymous statement-form helper that isn't the first chain
    // (Gishki Shadow's trailing `Ritual.AddWholeLevelTribute` synthesized
    // `__ritual_inline_1`, which sorts FIRST and shifted the search
    // effect one block late) — every positional rank is suspect. Refuse
    // to assign anything rather than fill wrong blocks.
    let mut by_ord = consumers.clone();
    by_ord.sort_by_key(|&i| walk.effects[i].source_ordinal);
    if by_ord != consumers {
        a.ambiguous = consumers.len();
        return a;
    }

    // Fast path: no hazard anywhere — positional, identical to history.
    if !walk.effects.iter().any(|e| e.block_alignment_hazard) {
        for (rank, &i) in consumers.iter().enumerate() {
            a.by_effect[i] = Some(rank);
        }
        a.positional = consumers.len();
        return a;
    }

    // Historical behavior for hazard cards: positional for the trusted
    // prefix, skip for everything flagged.
    let fallback = |a: &mut Assignments| {
        for (rank, &i) in consumers.iter().enumerate() {
            if walk.effects[i].block_alignment_hazard {
                a.ambiguous += 1;
            } else {
                a.by_effect[i] = Some(rank);
                a.positional += 1;
            }
        }
    };

    // Consistency (a) — consumers in BTreeMap order match source order —
    // is guaranteed by the order-consistency guard at the top.

    // Build the entity list in source order: every skeleton plus every
    // phantom clone ordinal.
    let kind = card_kind_from_ds(ds_txt);
    let mut ents: Vec<(usize, Option<usize>)> = walk.effects.iter().enumerate()
        .map(|(i, e)| (e.source_ordinal, Some(i)))
        .collect();
    ents.extend(walk.phantom_block_ordinals.iter().map(|&o| (o, None)));
    ents.sort_by_key(|&(o, _)| o);
    let sigs: Vec<EntitySig> = ents.iter()
        .map(|&(_, wi)| match wi {
            Some(i) => entity_sig(&walk.effects[i], kind),
            None => EntitySig::phantom(),
        })
        .collect();
    let blocks = parse_block_sigs(ds_txt);
    let claims = match_blocks(&sigs, &blocks);

    // Positional rank of each consumer (walk.effects order == source
    // order for consumers, verified above).
    let rank_of = |i: usize| consumers.iter().position(|&c| c == i);

    // Consistency (b): a forced claim that moves a TRUSTED consumer off
    // its positional block is evidence of confusion — fall back.
    for (ei, &(_, wi)) in ents.iter().enumerate() {
        let Some(i) = wi else { continue };
        let eff = &walk.effects[i];
        if !is_consumer(eff) || eff.block_alignment_hazard { continue; }
        if let (Some(j), Some(rank)) = (claims[ei], rank_of(i)) {
            if j != rank {
                fallback(&mut a);
                return a;
            }
        }
    }

    // Compose: trusted prefix positionally, hazard consumers from the
    // matcher — monotonic (c): every accepted block index must exceed
    // the last one handed out, in source order.
    let mut last: i64 = -1;
    for (ei, &(_, wi)) in ents.iter().enumerate() {
        let Some(i) = wi else { continue };
        let eff = &walk.effects[i];
        if !is_consumer(eff) { continue; }
        if !eff.block_alignment_hazard {
            let rank = rank_of(i).unwrap_or(0);
            a.by_effect[i] = Some(rank);
            a.positional += 1;
            last = last.max(rank as i64);
        } else {
            match claims[ei] {
                Some(j) if (j as i64) > last => {
                    a.by_effect[i] = Some(j);
                    a.rescued += 1;
                    last = j as i64;
                }
                _ => a.ambiguous += 1,
            }
        }
    }
    a
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Shorthand: a consumer entity with the given expectations.
    fn ent(speed: u8, trig: bool, opt: bool) -> EntitySig {
        EntitySig {
            kind: EntityKind::Consumer,
            speed: Some(speed),
            trigger_line: Some(trig),
            mandatory: Some(false),
            once_per_turn: opt,
            has_cost: true,
            has_target: true,
            has_condition: true,
        }
    }

    /// Shorthand: a block with the given header facts.
    fn blk(speed: u8, trig: bool, opt: bool) -> BlockSig {
        BlockSig {
            speed: Some(speed),
            trigger_line: trig,
            mandatory: false,
            once_per_turn: opt,
            ..Default::default()
        }
    }

    #[test]
    fn perfect_alignment_is_diagonal() {
        let ents = [ent(1, false, false), ent(1, true, false), ent(2, false, true)];
        let blocks = [blk(1, false, false), blk(1, true, false), blk(2, false, true)];
        assert_eq!(match_blocks(&ents, &blocks), vec![Some(0), Some(1), Some(2)]);
    }

    #[test]
    fn extra_text_block_in_middle_is_skipped() {
        // The .ds generator emitted a block for a text effect the walk
        // doesn't model (speed 2 + trigger, matching neither entity).
        let ents = [ent(1, false, false), ent(1, false, true)];
        let blocks = [blk(1, false, false), blk(2, true, false), blk(1, false, true)];
        assert_eq!(match_blocks(&ents, &blocks), vec![Some(0), Some(2)]);
    }

    #[test]
    fn missing_block_leaves_entity_unmatched() {
        // Three walk consumers, two blocks: the middle entity's block was
        // never generated. Distinct signatures pin the survivors.
        let ents = [ent(1, false, false), ent(2, true, false), ent(1, false, true)];
        let blocks = [blk(1, false, false), blk(1, false, true)];
        assert_eq!(match_blocks(&ents, &blocks), vec![Some(0), None, Some(1)]);
    }

    #[test]
    fn ambiguous_two_entities_one_block_is_none() {
        // Two identical entities compete for one block — which one owns
        // it is a coin flip, so neither may claim.
        let ents = [ent(1, false, false), ent(1, false, false)];
        let blocks = [blk(1, false, false)];
        assert_eq!(match_blocks(&ents, &blocks), vec![None, None]);
    }

    #[test]
    fn ambiguous_one_entity_two_blocks_is_none() {
        let ents = [ent(1, false, false)];
        let blocks = [blk(1, false, false), blk(1, false, false)];
        assert_eq!(match_blocks(&ents, &blocks), vec![None]);
    }

    #[test]
    fn crossing_preference_cannot_produce_crossed_claims() {
        // ent0 strongly matches block1 and ent1 matches block0 — the
        // crossed assignment {0→1, 1→0} is what a greedy best-match
        // would produce. Alignment monotonicity forbids it: only the
        // dominant pair survives, the other entity stays unmatched.
        let ents = [ent(2, true, true), ent(1, false, false)];
        let blocks = [blk(1, false, false), blk(2, true, true)];
        let got = match_blocks(&ents, &blocks);
        assert_eq!(got, vec![Some(1), None]);
        // And never order-inconsistent, by construction.
        if let (Some(a), Some(b)) = (got[0], got[1]) {
            assert!(a < b, "claims must be order-consistent, got {:?}", got);
        }
    }

    #[test]
    fn weak_forced_match_is_below_claim_threshold() {
        // A single entity/block pair is trivially forced, but carries no
        // signature evidence — must stay None.
        let e = EntitySig {
            kind: EntityKind::Consumer,
            speed: None,
            trigger_line: None,
            mandatory: None,
            once_per_turn: false,
            has_cost: true,
            has_target: true,
            has_condition: true,
        };
        let blocks = [BlockSig::default()];
        assert_eq!(match_blocks(&[e], &blocks), vec![None]);
    }

    #[test]
    fn veto_blocks_claim_on_cost_contradiction() {
        // Forced and above threshold, but the block has a cost{} the
        // chain can't account for — never claim.
        let mut e = ent(2, true, true);
        e.has_cost = false;
        let mut b = blk(2, true, true);
        b.has_cost = true;
        assert_eq!(match_blocks(&[e], &[b]), vec![None]);
    }

    #[test]
    fn phantom_absorbs_offset_when_anchored_downstream() {
        // c43698897 / Frightfur-Factory class: an unknown block owner
        // (clone / bare-activate) precedes real consumers. The phantom
        // may or may not own block 0; the consumers' distinct signatures
        // re-anchor them onto blocks 1 and 2 regardless.
        let phantom = EntitySig::phantom();
        let ents = [phantom, ent(2, false, true), ent(2, true, false)];
        let blocks = [blk(2, false, false), blk(2, false, true), blk(2, true, false)];
        let got = match_blocks(&ents, &blocks);
        assert_eq!(got[1], Some(1));
        assert_eq!(got[2], Some(2));
    }

    #[test]
    fn phantom_without_anchor_poisons_everything_after() {
        // Same class, but the consumers' signatures are identical to the
        // shell block's — the phantom's ± one offset stays unresolved and
        // nothing may be claimed.
        let phantom = EntitySig::phantom();
        let ents = [phantom, ent(2, false, false), ent(2, false, false)];
        let blocks = [blk(2, false, false), blk(2, false, false), blk(2, false, false)];
        assert_eq!(match_blocks(&ents, &blocks), vec![None, None, None]);
    }

    // ── compute_assignments: integration shapes ──────────────

    fn walk_of(src: &str) -> LuaReport {
        let parsed = full_moon::parse(src).expect("lua parse");
        crate::lua_ast::walk(&parsed)
    }

    /// Shiranui Style Samsara (c78765160) shape — the Phase 19 report's
    /// stranded emittable. Bare ACTIVATE shell + passive change-code +
    /// two QUICK_O chains; .ds has three blocks ("Effect 1/3/4"). The
    /// pre-Phase-20 positional zip would have put e3 into "Effect 1" —
    /// hazard gates skipped everything; the matcher must rescue e3→1 and
    /// e4→2.
    const SHIRANUI_LUA: &str = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_ACTIVATE)
    e1:SetCode(EVENT_FREE_CHAIN)
    c:RegisterEffect(e1)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_SINGLE)
    e2:SetCode(EFFECT_CHANGE_CODE)
    e2:SetRange(LOCATION_SZONE)
    e2:SetValue(40005099)
    c:RegisterEffect(e2)
    local e3=Effect.CreateEffect(c)
    e3:SetType(EFFECT_TYPE_QUICK_O)
    e3:SetCode(EVENT_FREE_CHAIN)
    e3:SetRange(LOCATION_SZONE)
    e3:SetCountLimit(1,0,EFFECT_COUNT_CODE_SINGLE)
    e3:SetCost(s.damcost)
    e3:SetOperation(s.damop)
    c:RegisterEffect(e3)
    local e4=Effect.CreateEffect(c)
    e4:SetType(EFFECT_TYPE_QUICK_O)
    e4:SetCode(EVENT_FREE_CHAIN)
    e4:SetCountLimit(1,0,EFFECT_COUNT_CODE_SINGLE)
    e4:SetRange(LOCATION_SZONE)
    e4:SetTarget(s.tdtg)
    e4:SetOperation(s.tdop)
    c:RegisterEffect(e4)
end
"#;

    const SHIRANUI_DS: &str = r#"card "Shiranui Style Samsara" {
    id: 78765160
    type: Continuous Trap

    effect "Effect 1" {
        speed: 2
        mandatory
    }

    effect "Effect 3" {
        speed: 2
        once_per_turn: soft
        cost {
            banish self
        }
        resolve { }
    }

    effect "Effect 4" {
        speed: 2
        once_per_turn: soft
        resolve { }
    }
}
"#;

    #[test]
    fn shiranui_rescues_both_quick_chains() {
        let walk = walk_of(SHIRANUI_LUA);
        // e2..e4 are hazard-flagged behind the bare-activate e1.
        assert!(walk.effects.iter().any(|e| e.block_alignment_hazard));
        let a = compute_assignments(&walk, SHIRANUI_DS);
        // walk.effects order: e1, e2, e3, e4.
        assert_eq!(a.by_effect, vec![None, None, Some(1), Some(2)]);
        assert_eq!((a.positional, a.rescued, a.ambiguous), (0, 2, 0));
    }

    /// Advanced Dark (c12644061) shape — the other stranded emittable.
    /// Bare ACTIVATE + field passive + continuous-op + TRIGGER_O + field
    /// passive against two blocks ("Effect 1"/"Effect 4"). Block 0 is
    /// contested (activation shell vs passive vs continuous-op — tie),
    /// so only the trigger chain may claim, onto block 1.
    const ADV_DARK_LUA: &str = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_ACTIVATE)
    e1:SetCode(EVENT_FREE_CHAIN)
    c:RegisterEffect(e1)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_FIELD)
    e2:SetRange(LOCATION_FZONE)
    e2:SetCode(EFFECT_CHANGE_ATTRIBUTE)
    e2:SetTarget(s.tg)
    e2:SetValue(ATTRIBUTE_DARK)
    c:RegisterEffect(e2)
    local e3=Effect.CreateEffect(c)
    e3:SetType(EFFECT_TYPE_FIELD+EFFECT_TYPE_CONTINUOUS)
    e3:SetCode(EVENT_ATTACK_ANNOUNCE)
    e3:SetRange(LOCATION_FZONE)
    e3:SetCondition(s.discon)
    e3:SetOperation(s.disop)
    c:RegisterEffect(e3)
    local e4=Effect.CreateEffect(c)
    e4:SetType(EFFECT_TYPE_FIELD+EFFECT_TYPE_TRIGGER_O)
    e4:SetCode(EVENT_PRE_DAMAGE_CALCULATE)
    e4:SetRange(LOCATION_FZONE)
    e4:SetCondition(s.damcon)
    e4:SetCost(s.damcost)
    e4:SetOperation(s.damop)
    c:RegisterEffect(e4)
    local e5=Effect.CreateEffect(c)
    e5:SetType(EFFECT_TYPE_FIELD)
    e5:SetCode(id)
    e5:SetRange(LOCATION_FZONE)
    e5:SetValue(s.val)
    c:RegisterEffect(e5)
end
"#;

    const ADV_DARK_DS: &str = r#"card "Advanced Dark" {
    id: 12644061
    type: Field Spell

    effect "Effect 1" {
        speed: 1
        mandatory
        resolve { }
    }

    effect "Effect 4" {
        speed: 1
        timing: when
        trigger: summoned
        cost {
            send self to gy
        }
        resolve { }
    }
}
"#;

    #[test]
    fn advanced_dark_rescues_only_the_trigger_chain() {
        let walk = walk_of(ADV_DARK_LUA);
        let a = compute_assignments(&walk, ADV_DARK_DS);
        // walk.effects order: e1..e5. Only e4 (TRIGGER_O + cost) is
        // uniquely forced — onto block 1, not the positional block 1-of-
        // consumers (which would have been block 0 for e3).
        assert_eq!(a.by_effect, vec![None, None, None, Some(1), None]);
        assert_eq!((a.positional, a.rescued, a.ambiguous), (0, 1, 1));
    }

    /// c99634927 class (pre-Phase-14): an UNREGISTERED clone owns a .ds
    /// block the walk can't see. The phantom stands in for it; with no
    /// distinguishing signatures downstream, nothing may be claimed.
    #[test]
    fn unregistered_clone_phantom_stays_conservative() {
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_IGNITION)
    e1:SetOperation(s.op1)
    c:RegisterEffect(e1)
    local e2=e1:Clone()
    e2:SetCode(EVENT_SPSUMMON_SUCCESS)
    local e3=Effect.CreateEffect(c)
    e3:SetType(EFFECT_TYPE_IGNITION)
    e3:SetOperation(s.op3)
    c:RegisterEffect(e3)
end
"#;
        let walk = walk_of(src);
        assert_eq!(walk.phantom_block_ordinals, vec![1]);
        let ds = r#"card "X" {
    id: 1
    type: Effect Monster

    effect "Effect 1" {
        speed: 1
        resolve { }
    }

    effect "Effect 2" {
        speed: 1
        resolve { }
    }
}
"#;
        let a = compute_assignments(&walk, ds);
        // e1 is trusted (pre-hazard) → positional block 0. e3 is flagged
        // and the phantom makes blocks 1/2… wait, only 2 blocks: e3
        // could sit at block 1 (phantom skipped) or behind the phantom —
        // identical ignition signatures give no anchor. Must stay None.
        assert_eq!(a.by_effect, vec![Some(0), None]);
        assert_eq!((a.positional, a.rescued, a.ambiguous), (1, 0, 1));
    }

    /// Fast path: hazard-free cards NEVER consult signatures — the
    /// mapping is positional even when the signatures disagree wildly.
    #[test]
    fn hazard_free_card_is_purely_positional() {
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_IGNITION)
    e1:SetOperation(s.op1)
    c:RegisterEffect(e1)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_SINGLE)
    e2:SetCode(EFFECT_UPDATE_ATTACK)
    e2:SetValue(500)
    c:RegisterEffect(e2)
    local e3=Effect.CreateEffect(c)
    e3:SetType(EFFECT_TYPE_QUICK_O)
    e3:SetCode(EVENT_FREE_CHAIN)
    e3:SetOperation(s.op3)
    c:RegisterEffect(e3)
end
"#;
        let walk = walk_of(src);
        assert!(!walk.effects.iter().any(|e| e.block_alignment_hazard));
        // Deliberately contradictory .ds signatures: fast path must not care.
        let ds = r#"card "X" {
    id: 1
    type: Effect Monster

    effect "Effect 1" {
        speed: 2
        timing: when
        trigger: destroyed
        resolve { }
    }

    effect "Effect 2" {
        speed: 1
        resolve { }
    }
}
"#;
        let a = compute_assignments(&walk, ds);
        // Consumers e1, e3 → positional blocks 0, 1; passive e2 → None.
        assert_eq!(a.by_effect, vec![Some(0), None, Some(1)]);
        assert_eq!((a.positional, a.rescued, a.ambiguous), (2, 0, 0));
    }

    /// A rescue may never land at or before the trusted prefix.
    #[test]
    fn rescue_below_trusted_prefix_is_dropped() {
        // e1 trusted (positional block 0); e2 bare-activate hazard
        // source; e3 flagged. If the matcher somehow forced e3 onto
        // block 0 the monotonic guard must drop it. Construct via a .ds
        // whose only strong-match block is block 0.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_IGNITION)
    e1:SetOperation(s.op1)
    c:RegisterEffect(e1)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_ACTIVATE)
    e2:SetCode(EVENT_FREE_CHAIN)
    c:RegisterEffect(e2)
    local e3=Effect.CreateEffect(c)
    e3:SetType(EFFECT_TYPE_TRIGGER_O)
    e3:SetCode(EVENT_DESTROYED)
    e3:SetCountLimit(1)
    e3:SetOperation(s.op3)
    c:RegisterEffect(e3)
end
"#;
        let walk = walk_of(src);
        let ds = r#"card "X" {
    id: 1
    type: Effect Monster

    effect "Effect 1" {
        speed: 1
        timing: when
        trigger: destroyed
        once_per_turn: soft
        resolve { }
    }
}
"#;
        let a = compute_assignments(&walk, ds);
        // e3's only signature home is block 0 — but block 0 is e1's
        // trusted positional slot. The matcher forcing e3→0 would move a
        // trusted consumer or cross it; either way e3 must end None and
        // e1 must keep block 0.
        assert_eq!(a.by_effect[0], Some(0));
        assert_eq!(a.by_effect[2], None);
    }

    #[test]
    fn block_sigs_parse_headers_and_ignore_bodies() {
        let ds = r#"card "X" {
    id: 1
    type: Continuous Trap

    effect "Effect 1" {
        speed: 2
        timing: if
        trigger: destroyed
        once_per_turn: hard
        condition: self.on_field
        cost {
            banish self
        }
        target (1, monster, you control)
        resolve {
            destroy target
        }
    }

    effect "Effect 2" {
        speed: 1
        mandatory
        choose {
            option "A" {
                resolve {
                    draw 1
                }
            }
        }
    }
}
"#;
        let sigs = parse_block_sigs(ds);
        assert_eq!(sigs.len(), 2);
        assert_eq!(sigs[0], BlockSig {
            speed: Some(2),
            trigger_line: true,
            mandatory: false,
            once_per_turn: true,
            has_cost: true,
            has_target: true,
            has_condition: true,
        });
        assert_eq!(sigs[1], BlockSig {
            speed: Some(1),
            trigger_line: false,
            mandatory: true,
            once_per_turn: false,
            has_cost: false,
            has_target: false,
            has_condition: false,
        });
        assert_eq!(card_kind_from_ds(ds), CardKind::Trap);
    }
}
