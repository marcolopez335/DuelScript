// ============================================================
// DuelScript v2 Constants
// Engine bitfield constants (engine-agnostic, no v1 imports).
// These match EDOPro/YGOPro constant.lua exactly.
// ============================================================

// Effect types
pub const EFFECT_TYPE_SINGLE:     u32 = 0x1;
pub const EFFECT_TYPE_FIELD:      u32 = 0x2;
pub const EFFECT_TYPE_EQUIP:      u32 = 0x4;
pub const EFFECT_TYPE_ACTIVATE:   u32 = 0x10;
pub const EFFECT_TYPE_FLIP:       u32 = 0x20;
pub const EFFECT_TYPE_IGNITION:   u32 = 0x40;
pub const EFFECT_TYPE_TRIGGER_O:  u32 = 0x80;
pub const EFFECT_TYPE_QUICK_O:    u32 = 0x100;
pub const EFFECT_TYPE_TRIGGER_F:  u32 = 0x200;
pub const EFFECT_TYPE_QUICK_F:    u32 = 0x400;
pub const EFFECT_TYPE_CONTINUOUS: u32 = 0x800;
pub const EFFECT_TYPE_XMATERIAL:  u32 = 0x1000;

// Categories
pub const CATEGORY_DESTROY:        u32 = 0x1;
pub const CATEGORY_RELEASE:        u32 = 0x2;
pub const CATEGORY_REMOVE:         u32 = 0x4;
pub const CATEGORY_TOHAND:         u32 = 0x8;
pub const CATEGORY_TODECK:         u32 = 0x10;
pub const CATEGORY_TOGRAVE:        u32 = 0x20;
pub const CATEGORY_DECKDES:        u32 = 0x40;
pub const CATEGORY_HANDES:         u32 = 0x80;
pub const CATEGORY_SUMMON:         u32 = 0x100;
pub const CATEGORY_SPECIAL_SUMMON: u32 = 0x200;
pub const CATEGORY_TOKEN:          u32 = 0x400;
pub const CATEGORY_POSITION:       u32 = 0x1000;
pub const CATEGORY_CONTROL:        u32 = 0x2000;
pub const CATEGORY_DISABLE:        u32 = 0x4000;
pub const CATEGORY_DISABLE_SUMMON: u32 = 0x8000;
pub const CATEGORY_DRAW:           u32 = 0x10000;
pub const CATEGORY_SEARCH:         u32 = 0x20000;
pub const CATEGORY_EQUIP:          u32 = 0x40000;
pub const CATEGORY_DAMAGE:         u32 = 0x80000;
pub const CATEGORY_RECOVER:        u32 = 0x100000;
pub const CATEGORY_ATKCHANGE:      u32 = 0x200000;
pub const CATEGORY_DEFCHANGE:      u32 = 0x400000;
pub const CATEGORY_COUNTER:        u32 = 0x800000;
pub const CATEGORY_NEGATE:         u32 = 0x10000000;
pub const CATEGORY_FUSION_SUMMON:  u32 = 0x40000000;

// Events
pub const EVENT_FLIP:              u32 = 1001;
pub const EVENT_FREE_CHAIN:        u32 = 1002;
pub const EVENT_DESTROY:           u32 = 1010;
pub const EVENT_REMOVE:            u32 = 1011;
pub const EVENT_TO_HAND:           u32 = 1012;
pub const EVENT_TO_DECK:           u32 = 1013;
pub const EVENT_TO_GRAVE:          u32 = 1014;
pub const EVENT_LEAVE_FIELD:       u32 = 1015;
pub const EVENT_RELEASE:           u32 = 1017;
pub const EVENT_CHAINING:          u32 = 1027;
pub const EVENT_CHAIN_SOLVING:     u32 = 1020;
pub const EVENT_CHAIN_SOLVED:      u32 = 1022;
/// Fires when a card is used as a material — Xyz attached, tributed,
/// fused, Synchro/Link/Ritual material. EDOPro code 1108 (see
/// `grammar/edopro_constants.lua:669`). Distinct from `EVENT_RELEASE`
/// (1017), which fires for any `Duel.Release` regardless of reason.
/// Pre-T30, `Trigger::UsedAsMaterial` routed to EVENT_RELEASE; fixed
/// in T30 / AA-II.
pub const EVENT_BE_MATERIAL:       u32 = 1108;
pub const EVENT_DESTROYED:         u32 = 1029;
pub const EVENT_SUMMON_SUCCESS:    u32 = 1100;
pub const EVENT_FLIP_SUMMON_SUCCESS: u32 = 1101;
pub const EVENT_SPSUMMON_SUCCESS:  u32 = 1102;
pub const EVENT_SUMMON:            u32 = 1103;
pub const EVENT_FLIP_SUMMON:       u32 = 1104;
pub const EVENT_SPSUMMON:          u32 = 1105;
pub const EVENT_ATTACK_ANNOUNCE:   u32 = 1130;
pub const EVENT_BE_BATTLE_TARGET:  u32 = 1131;
pub const EVENT_PRE_DAMAGE_CALCULATE: u32 = 1134;
pub const EVENT_BATTLE_DESTROYING: u32 = 1139;
pub const EVENT_BATTLE_DESTROYED:  u32 = 1140;
pub const EVENT_BATTLE_DAMAGE:     u32 = 1143;
pub const EVENT_PREDRAW:           u32 = 1113;

// Phase events: EVENT_PHASE + PHASE_*
pub const EVENT_PHASE:             u32 = 0x1000;
pub const PHASE_DRAW:              u32 = 0x1;
pub const PHASE_STANDBY:           u32 = 0x2;
pub const PHASE_MAIN1:             u32 = 0x4;
pub const PHASE_BATTLE:            u32 = 0x80;
pub const PHASE_MAIN2:             u32 = 0x100;
pub const PHASE_END:               u32 = 0x200;

// Locations
pub const LOCATION_DECK:    u32 = 0x1;
pub const LOCATION_HAND:    u32 = 0x2;
pub const LOCATION_MZONE:   u32 = 0x4;
pub const LOCATION_SZONE:   u32 = 0x8;
pub const LOCATION_GRAVE:   u32 = 0x10;
pub const LOCATION_REMOVED: u32 = 0x20;
pub const LOCATION_EXTRA:   u32 = 0x40;
pub const LOCATION_FZONE:   u32 = 0x100;
pub const LOCATION_PZONE:   u32 = 0x200;
pub const LOCATION_ONFIELD: u32 = LOCATION_MZONE | LOCATION_SZONE;

// Property flags
pub const EFFECT_FLAG_CARD_TARGET:   u32 = 0x10;
pub const EFFECT_FLAG_PLAYER_TARGET: u32 = 0x800;
pub const EFFECT_FLAG_DAMAGE_STEP:   u32 = 0x4000;
pub const EFFECT_FLAG_DELAY:         u32 = 0x10000;
pub const EFFECT_FLAG_SINGLE_RANGE:  u32 = 0x20000;

// Battle-position bitmask (EDOPro POS_*)
pub const POS_FACEUP_ATTACK:    u32 = 0x1;
pub const POS_FACEDOWN_ATTACK:  u32 = 0x2;
pub const POS_FACEUP_DEFENSE:   u32 = 0x4;
pub const POS_FACEDOWN_DEFENSE: u32 = 0x8;
pub const POS_FACEUP:           u32 = POS_FACEUP_ATTACK  | POS_FACEUP_DEFENSE;
pub const POS_FACEDOWN:         u32 = POS_FACEDOWN_ATTACK | POS_FACEDOWN_DEFENSE;
pub const POS_ATTACK:           u32 = POS_FACEUP_ATTACK   | POS_FACEDOWN_ATTACK;
pub const POS_DEFENSE:          u32 = POS_FACEUP_DEFENSE  | POS_FACEDOWN_DEFENSE;

// Reason flags (REASON_* bitmask; `IsReason(REASON_BATTLE|REASON_EFFECT)`)
pub const REASON_DESTROY:  u32 = 0x1;
pub const REASON_RELEASE:  u32 = 0x2;
pub const REASON_MATERIAL: u32 = 0x8;
pub const REASON_SUMMON:   u32 = 0x10;
pub const REASON_BATTLE:   u32 = 0x20;
pub const REASON_EFFECT:   u32 = 0x40;
pub const REASON_COST:     u32 = 0x80;
pub const REASON_RULE:     u32 = 0x400;
pub const REASON_DISCARD:  u32 = 0x4000;
pub const REASON_RETURN:   u32 = 0x20000;
/// Reason: consumed as Fusion material. EDOPro `REASON_FUSION`.
pub const REASON_FUSION:   u32 = 0x40000;
/// Reason: consumed as Synchro material. EDOPro `REASON_SYNCHRO`.
pub const REASON_SYNCHRO:  u32 = 0x80000;
/// Reason: consumed as Ritual material. EDOPro `REASON_RITUAL`.
pub const REASON_RITUAL:   u32 = 0x100000;
/// Reason: consumed as Xyz material (overlaid). EDOPro `REASON_XYZ`.
pub const REASON_XYZ:      u32 = 0x200000;
/// Reason: consumed as Link material. EDOPro `REASON_LINK`.
pub const REASON_LINK:     u32 = 0x10000000;

// ── T31 / CC-II: leave-field redirect (Macro Cosmos pattern) ──

/// Effect code for continuous leave-field destination redirects
/// (Macro Cosmos, Dimensional Fissure, Banisher of the Radiance, …).
/// EDOPro constant `EFFECT_LEAVE_FIELD_REDIRECT` (0x1000018). Distinct
/// from the `EFFECT_DESTROY_REPLACE` family used by event-triggered
/// replacement blocks — redirect is a *continuous* floodgate that
/// retargets leave-field moves while the source card is on the field.
pub const EFFECT_LEAVE_FIELD_REDIRECT: u32 = 0x1000018;

/// Redirect affects only the source card's own moves.
/// Encoded as a bitmask so the adapter can OR multiple flags if a
/// future design wants a redirect that covers several scopes at once.
pub const REDIRECT_SCOPE_SELF:           u32 = 0x1;
/// Redirect affects all cards on the source controller's side.
pub const REDIRECT_SCOPE_FIELD:          u32 = 0x2;
/// Redirect affects all cards on the opposing side.
pub const REDIRECT_SCOPE_OPPONENT_FIELD: u32 = 0x4;
/// Redirect affects all cards on both fields (Macro Cosmos default).
pub const REDIRECT_SCOPE_BOTH_FIELDS:    u32 = REDIRECT_SCOPE_FIELD | REDIRECT_SCOPE_OPPONENT_FIELD;

// ── Redirect filter flags (NN-II) ────────────────────────────────
//
// Compact u32 summary of the `when:` selector on a `redirect {}` block.
// The adapter stores these flags on its `ContinuousEffect::Redirect`
// entry so the engine can scope the redirect to a card class (Monster-
// only redirects ignore Spell/Trap leave-field events, etc.).
//
// Bit 0 indicates any non-default filter is present. Bits 1-3 identify
// the filter's card class. Future extensions can claim bits 4-31 for
// attribute / race / predicate summaries without breaking the seam.

/// Set if any non-default `when:` filter is specified. Zero when the
/// redirect applies universally (no `when:` clause).
pub const REDIRECT_FILTER_HAS_FILTER: u32 = 0x1;
/// Set when the filter restricts to Monster cards.
pub const REDIRECT_FILTER_MONSTER:    u32 = 0x2;
/// Set when the filter restricts to Spell cards.
pub const REDIRECT_FILTER_SPELL:      u32 = 0x4;
/// Set when the filter restricts to Trap cards.
pub const REDIRECT_FILTER_TRAP:       u32 = 0x8;

/// Count limit for effect activation frequency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountLimit {
    /// How many times per period (1 for OPT, 2 for twice-per-turn)
    pub count: u32,
    /// 0 = soft OPT (can re-activate if negated), card_id = hard OPT
    pub code: u32,
}
