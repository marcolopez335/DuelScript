// ============================================================
// DuelScript AST v0.5 — ast.rs
// Complete Rust types for all DuelScript constructs
// ============================================================

use std::fmt;

// ── File ──────────────────────────────��───────────────────────

#[derive(Debug, Clone)]
pub struct DuelScriptFile {
    pub cards: Vec<Card>,
}

// ── Card ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Card {
    pub name:                String,
    pub card_types:          Vec<CardType>,
    pub attribute:           Option<Attribute>,
    pub stats:               Stats,
    pub race:                Option<Race>,
    pub level:               Option<u32>,
    pub rank:                Option<u32>,
    pub link:                Option<u32>,
    pub scale:               Option<u32>,
    pub flavor:              Option<String>,
    pub password:            Option<u32>,
    pub archetypes:          Vec<String>,
    pub link_arrows:         Vec<LinkArrow>,
    pub summon_conditions:   Vec<SummonRule>,
    pub materials:           Option<MaterialsBlock>,
    pub counter_system:      Option<CounterSystem>,
    pub pendulum_effect:     Option<EffectBody>,
    pub effects:             Vec<Effect>,
    pub continuous_effects:  Vec<ContinuousEffect>,
    pub replacement_effects: Vec<ReplacementEffect>,
    pub equip_effects:       Vec<EquipEffect>,
    pub win_condition:       Option<WinCondition>,
}

impl Card {
    pub fn is_monster(&self) -> bool {
        self.card_types.iter().any(|t| matches!(t,
            CardType::NormalMonster | CardType::EffectMonster |
            CardType::RitualMonster | CardType::FusionMonster |
            CardType::SynchroMonster | CardType::XyzMonster |
            CardType::LinkMonster | CardType::PendulumMonster
        ))
    }

    pub fn is_spell(&self) -> bool {
        self.card_types.iter().any(|t| matches!(t,
            CardType::NormalSpell | CardType::QuickPlaySpell |
            CardType::ContinuousSpell | CardType::EquipSpell |
            CardType::FieldSpell | CardType::RitualSpell
        ))
    }

    pub fn is_trap(&self) -> bool {
        self.card_types.iter().any(|t| matches!(t,
            CardType::NormalTrap | CardType::CounterTrap | CardType::ContinuousTrap
        ))
    }

    pub fn is_extra_deck(&self) -> bool {
        self.card_types.iter().any(|t| matches!(t,
            CardType::FusionMonster | CardType::SynchroMonster |
            CardType::XyzMonster | CardType::LinkMonster
        ))
    }
}

// ── Stats ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub atk: Option<StatValue>,
    pub def: Option<StatValue>,
}

#[derive(Debug, Clone)]
pub enum StatValue {
    Number(i32),
    Variable, // "?"
}

// ── Card Types ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CardType {
    NormalMonster, EffectMonster, RitualMonster, FusionMonster,
    SynchroMonster, XyzMonster, LinkMonster, PendulumMonster,
    Tuner, SynchroTuner, Gemini, Union, Spirit, Flip, Toon,
    NormalSpell, QuickPlaySpell, ContinuousSpell, EquipSpell,
    FieldSpell, RitualSpell,
    NormalTrap, CounterTrap, ContinuousTrap,
}

// ── Attribute ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Attribute { Light, Dark, Fire, Water, Earth, Wind, Divine }

// ── Race ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Race {
    Dragon, Spellcaster, Zombie, Warrior, BeastWarrior, Beast,
    WingedBeast, Fiend, Fairy, Insect, Dinosaur, Reptile,
    Fish, SeaSerpent, Aqua, Pyro, Thunder, Rock, Plant, Machine,
    Psychic, DivineBeast, Wyrm, Cyberse,
}

// ── Link Arrows ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkArrow {
    TopLeft, Top, TopRight,
    Left,         Right,
    BottomLeft, Bottom, BottomRight,
}

// ── Expression System ─────────────────────────────────────────
// Dynamic value expressions — replaces static u32 in actions,
// costs, modifiers, etc.

#[derive(Debug, Clone)]
pub enum Expr {
    /// Literal integer
    Literal(i32),
    /// self.atk, self.def, self.level, self.rank
    SelfStat(Stat),
    /// target.atk, target.def, target.level
    TargetStat(Stat),
    /// your_lp, opponent_lp
    PlayerLp(Player),
    /// count(target_expr in zone)
    Count { target: Box<TargetExpr>, zone: Option<Zone> },
    /// Binary operation: left op right
    BinOp { left: Box<Expr>, op: BinOp, right: Box<Expr> },
}

impl Expr {
    /// Create a literal expression
    pub fn lit(n: i32) -> Self { Expr::Literal(n) }

    /// Returns Some(n) if this is a simple literal, None otherwise
    pub fn as_literal(&self) -> Option<i32> {
        match self {
            Expr::Literal(n) => Some(*n),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinOp { Add, Sub, Mul, Div }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stat { Atk, Def, Level, Rank }

// ── Summon Conditions ─────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum SummonRule {
    TributesRequired(u32),
    CannotNormalSummon,
    CannotSpecialSummon,
    SpecialSummonOnly,
    MustBeSummonedBy(SummonSource),
    SummonOncePerTurn,
    TributeMaterial(CardFilter),
    SpecialSummonFrom(Vec<Zone>),
}

#[derive(Debug, Clone)]
pub enum SummonSource {
    OwnEffect,
    RitualSpell,
    FusionSpell,
    SpecificCard(String),
    Method(SummonMethod),
}

// ── Materials ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MaterialsBlock {
    pub slots:        Vec<MaterialSlot>,
    pub constraints:  Vec<MaterialConstraint>,
    pub alternatives: Vec<AlternativeMaterials>,
}

#[derive(Debug, Clone)]
pub enum MaterialSlot {
    Named(Vec<String>),
    Generic(GenericMaterialSlot),
}

#[derive(Debug, Clone)]
pub struct GenericMaterialSlot {
    pub count:           u32,
    pub count_or_more:   bool,
    pub qualifiers:      Vec<MaterialQualifier>,
    pub attribute:       Option<Attribute>,
    pub race:            Option<Race>,
    pub level:           Option<LevelConstraint>,
    pub extra_deck_type: Option<ExtraDeckType>,
    pub filter:          CardFilter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterialQualifier {
    Tuner, NonTuner, NonToken, NonSpecial,
    NonFusion, NonSynchro, NonXyz, NonLink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtraDeckType { Synchro, Fusion, Xyz, Link, Ritual }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LevelConstraint {
    Exact(u32),
    Range(u32, u32),
    Min(u32),
    Max(u32),
}

impl LevelConstraint {
    pub fn satisfied_by(&self, level: u32) -> bool {
        match self {
            LevelConstraint::Exact(n)      => level == *n,
            LevelConstraint::Range(lo, hi) => level >= *lo && level <= *hi,
            LevelConstraint::Min(n)        => level >= *n,
            LevelConstraint::Max(n)        => level <= *n,
        }
    }
}

#[derive(Debug, Clone)]
pub enum MaterialConstraint {
    SameLevel,
    SameAttribute,
    SameRace,
    MustInclude(String),
    CannotUse(MaterialCannotTarget),
    Method(SummonMethodType),
}

#[derive(Debug, Clone)]
pub enum MaterialCannotTarget {
    Token, Fusion, Synchro, Xyz, Link, Pendulum, Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummonMethodType { Fusion, Synchro, Xyz, Link, Ritual }

#[derive(Debug, Clone)]
pub struct AlternativeMaterials {
    pub slots:       Vec<MaterialSlot>,
    pub constraints: Vec<MaterialConstraint>,
}

// ── Counter System ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CounterSystem {
    pub name:        String,
    pub placed_when: Option<TriggerExpr>,
    pub max:         Option<CounterMax>,
    pub effects:     Vec<Effect>,
}

#[derive(Debug, Clone)]
pub enum CounterMax {
    Limited(u32),
    Unlimited,
}

// ── Trigger Effect ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Effect {
    pub name: Option<String>,
    pub body: EffectBody,
}

#[derive(Debug, Clone)]
pub struct EffectBody {
    pub speed:        SpellSpeed,
    pub frequency:    Frequency,
    pub optional:     bool,
    pub timing:       TimingQualifier,
    /// Whether timing was explicitly declared in the source (vs defaulted)
    pub timing_explicit: bool,
    pub condition:    Option<ConditionExpr>,
    pub trigger:      Option<TriggerExpr>,
    pub cost:         Vec<CostAction>,
    pub on_activate:  Vec<GameAction>,
    pub on_resolve:   Vec<GameAction>,
    pub restrictions: Vec<RestrictionRule>,
}

impl Default for EffectBody {
    fn default() -> Self {
        EffectBody {
            speed:        SpellSpeed::SpellSpeed1,
            frequency:    Frequency::Unlimited,
            optional:     false,
            timing:       TimingQualifier::When,
            timing_explicit: false,
            condition:    None,
            trigger:      None,
            cost:         vec![],
            on_activate:  vec![],
            on_resolve:   vec![],
            restrictions: vec![],
        }
    }
}

// ── Continuous Effect ─────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ContinuousEffect {
    pub name:         Option<String>,
    pub while_cond:   Option<ConditionExpr>,
    pub apply_to:     Option<TargetExpr>,
    pub modifiers:    Vec<ModifierDecl>,
    pub restrictions: Vec<RestrictionRule>,
    pub cannots:      Vec<CannotBlock>,
}

#[derive(Debug, Clone)]
pub enum ModifierDecl {
    Atk { sign: Sign, value: Expr, duration: Option<Duration> },
    Def { sign: Sign, value: Expr, duration: Option<Duration> },
    Level { sign: Sign, value: Expr, duration: Option<Duration> },
    Grant(GrantedAbility),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sign { Plus, Minus }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantedAbility {
    Piercing,
    DoubleAttack,
    DirectAttack,
    CannotBeDestroyedByBattle,
    CannotBeDestroyedByEffect,
    UnaffectedBySpellEffects,
    UnaffectedByTrapEffects,
    UnaffectedByMonsterEffects,
    UnaffectedByCardEffects,
    ImmuneToTargeting,
    CannotActivateEffects,
}

// ── Replacement Effect ────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ReplacementEffect {
    pub name:        Option<String>,
    pub instead_of:  ReplaceableEvent,
    pub do_actions:  Vec<GameAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceableEvent {
    DestroyedByBattle,
    DestroyedByEffect,
    DestroyedByAny,
    SentToGyByEffect,
    SentToGyByBattle,
    SentToGy,
    Banished,
    ReturnedToHand,
    ReturnedToDeck,
}

// ── Equip Effect ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct EquipEffect {
    pub target:                TargetExpr,
    pub while_equipped:        Vec<WhileEquippedClause>,
    pub on_equipped_destroyed: Vec<GameAction>,
    pub on_unequipped:         Vec<GameAction>,
}

#[derive(Debug, Clone)]
pub enum WhileEquippedClause {
    Modifier(ModifierDecl),
    Cannot(CannotBlock),
}

// ── Win Condition ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WinCondition {
    pub trigger: WinTrigger,
    pub result:  WinResult,
}

#[derive(Debug, Clone)]
pub enum WinTrigger {
    AllPiecesInHand,
    OpponentCannotDraw,
    TurnCount(u32),
    SpecificCardsOnField(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WinResult { WinDuel, LoseDuel, DrawDuel }

// ── Spell Speed ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpellSpeed {
    SpellSpeed1,
    SpellSpeed2,
    SpellSpeed3,
}

impl Default for SpellSpeed {
    fn default() -> Self { SpellSpeed::SpellSpeed1 }
}

impl fmt::Display for SpellSpeed {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SpellSpeed::SpellSpeed1 => write!(f, "Spell Speed 1"),
            SpellSpeed::SpellSpeed2 => write!(f, "Spell Speed 2"),
            SpellSpeed::SpellSpeed3 => write!(f, "Spell Speed 3"),
        }
    }
}

// ── Frequency ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frequency {
    Unlimited,
    OncePerTurn(OptKind),
    TwicePerTurn,
    OncePerDuel,
    EachTurn,
}

impl Default for Frequency {
    fn default() -> Self { Frequency::Unlimited }
}

/// Soft OPT = can activate again if negated (code=0)
/// Hard OPT = cannot activate again even if negated (code=card_id)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptKind { Soft, Hard }

impl Default for OptKind {
    fn default() -> Self { OptKind::Hard }
}

// ── Timing Qualifier ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimingQualifier {
    When, // Strict — can miss the timing
    If,   // Soft — cannot miss the timing
}

impl Default for TimingQualifier {
    fn default() -> Self { TimingQualifier::When }
}

// ── Duration ──────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Duration {
    UntilEndOfTurn,
    UntilEndPhase,
    UntilEndOfDamageStep,
    UntilNextTurn,
    Permanently,
    ThisTurn,
}

// ── Condition ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ConditionExpr {
    Simple(SimpleCondition),
    And(Vec<SimpleCondition>),
    Or(Vec<SimpleCondition>),
}

#[derive(Debug, Clone)]
pub enum SimpleCondition {
    InZone(Zone),
    OnField,
    YouControlNoMonsters,
    OpponentControlsNoMonsters,
    YouControl(TargetExpr),
    OpponentControls(TargetExpr),
    FieldIsEmpty,
    LpCondition { player: Player, op: CompareOp, value: u32 },
    HandSize { op: CompareOp, value: u32 },
    CardsInGy { op: CompareOp, value: u32 },
    YouControlCount { op: CompareOp, value: u32 },
    BanishedCount { op: CompareOp, value: u32 },
    /// Chain link includes one of these categories (for hand traps)
    ChainIncludes(Vec<ChainCategory>),
}

/// Categories that can appear in a chain link — maps to engine CATEGORY_* constants
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainCategory {
    Search,
    SpecialSummon,
    SendToGy,
    AddToHand,
    Draw,
    Banish,
    Mill,
    Destroy,
    Negate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Player { You, Opponent }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp { Gte, Lte, Gt, Lt, Eq, Neq }

// ── Trigger ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TriggerExpr {
    OpponentActivates(Vec<TriggerAction>),
    WhenSummoned(Option<SummonMethod>),
    WhenTributeSummoned(Option<CardFilter>),
    WhenTributed(Option<TributeFor>),
    WhenDestroyed(Option<DestructionCause>),
    WhenSentTo { zone: Zone, cause: Option<DestructionCause> },
    WhenFlipped,
    WhenAttacked,
    OnNthSummon(u32),
    DuringStandbyPhase(Option<PhaseOwner>),
    DuringEndPhase,
    DuringPhase(Phase),
    WhenAction(TriggerAction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TributeFor { Summon, Cost, Any }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummonMethod {
    ByNormalSummon, BySpecialSummon, ByFlipSummon,
    ByRitualSummon, ByFusionSummon, BySynchroSummon,
    ByXyzSummon, ByLinkSummon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DestructionCause {
    Battle, CardEffect, YourEffect, OpponentEffect, Any,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhaseOwner { Yours, Opponents, Either }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerAction {
    Search, SpecialSummon, SendToGy, AddToHand,
    Draw, Banish, Mill, TokenSpawn,
    ActivateSpell, ActivateTrap, ActivateMonsterEffect,
    FusionSummon, SynchroSummon, XyzSummon, LinkSummon,
    RitualSummon, NormalSummon, SetCard,
    ChangeBattlePosition, TakeDamage, GainLp,
    AttackDeclared,
}

// ── Phase ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    DrawPhase, StandbyPhase, MainPhase1,
    DamageStep, DamageCalculation, BattlePhase,
    MainPhase2, EndPhase,
}

// ── Duration ──────────────────────────────────────────────────

// (Already defined above)

// ── Cost ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum CostAction {
    None,
    PayLp(Expr),
    Discard(SelfOrTarget),
    Tribute(SelfOrTarget),
    Banish { target: SelfOrTarget, from: Option<Zone> },
    Send { target: SelfOrTarget, to: Zone },
    RemoveCounter { count: u32, name: String, from: SelfOrTarget },
    Detach { count: u32, from: SelfOrTarget },
    Reveal(SelfOrTarget),
}

#[derive(Debug, Clone)]
pub enum SelfOrTarget {
    Self_,
    Target(TargetExpr),
}

// ── Game Actions ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum GameAction {
    Draw { count: Expr },

    SpecialSummon {
        target:   SelfOrTarget,
        from:     Option<Zone>,
        position: Option<BattlePosition>,
    },

    Negate {
        what:        Option<NegateTarget>,
        and_destroy: bool,
    },

    Destroy { target: TargetExpr },

    SendToZone { target: SelfOrTarget, zone: Zone },

    Search { target: TargetExpr, from: Zone },

    AddToHand { target: TargetExpr, from: Zone },

    ModifyAtk {
        kind:     AtkModKind,
        target:   Option<TargetExpr>,
        duration: Option<Duration>,
    },

    ModifyDef {
        kind:     DefModKind,
        target:   Option<TargetExpr>,
        duration: Option<Duration>,
    },

    Banish {
        target:    SelfOrTarget,
        from:      Option<Zone>,
        face_down: bool,
    },

    Return {
        target:  SelfOrTarget,
        to:      ReturnZone,
        shuffle: bool,
    },

    SetFaceDown { target: TargetExpr },
    FlipFaceDown { target: TargetExpr },
    ChangeBattlePosition { target: TargetExpr },

    TakeControl {
        target:   TargetExpr,
        duration: Option<TakeControlDuration>,
    },

    PlaceCounter { count: u32, name: String, on: SelfOrTarget },
    RemoveCounter { count: u32, name: String, from: SelfOrTarget },

    LookAt { target: TargetExpr, from: Option<Zone> },
    Reveal { target: SelfOrTarget },

    CopyEffect { from: TargetExpr },

    Equip { card: TargetExpr, to: TargetExpr },

    Detach { count: u32, from: SelfOrTarget },

    /// Attach card(s) as overlay material to an Xyz monster
    Attach { target: TargetExpr, to: SelfOrTarget },

    FusionSummon  { target: TargetExpr, materials: Vec<TargetExpr> },
    SynchroSummon { target: TargetExpr, materials: Vec<TargetExpr> },
    XyzSummon     { target: TargetExpr, materials: Vec<TargetExpr> },
    RitualSummon  { target: TargetExpr, materials: Vec<TargetExpr> },
    PendulumSummon { targets: TargetExpr, from: Vec<Zone> },

    CreateToken { spec: TokenSpec },

    DealDamage { to: DamageTarget, amount: Expr },
    GainLp { amount: Expr },

    Shuffle { zone: Zone },

    Mill { count: Expr, from: MillSource },

    Discard { target: SelfOrTarget },
    Tribute { target: SelfOrTarget },

    SetScale { target: SelfOrTarget, value: Expr },

    If {
        condition:    ConditionExpr,
        then_actions: Vec<GameAction>,
        else_actions: Vec<GameAction>,
    },

    ForEach {
        target:  TargetExpr,
        in_zone: Zone,
        actions: Vec<GameAction>,
    },

    ApplyUntil {
        actions:  Vec<GameAction>,
        duration: Duration,
    },

    /// Player picks one of N options at resolution time
    Choose { options: Vec<ChoiceOption> },

    /// Activate now, resolve later (e.g., "destroy this card during the End Phase")
    Delayed {
        until:   Phase,
        actions: Vec<GameAction>,
    },

    /// Dynamically register a new effect on a target during resolution
    RegisterEffect {
        target:   TargetExpr,
        effect:   Box<InlineEffect>,
        duration: Option<Duration>,
    },

    /// Store cards/value for cross-phase persistence
    Store { label: String, value: StoreValue },

    /// Recall previously stored cards/value
    Recall { label: String },
}

#[derive(Debug, Clone)]
pub struct ChoiceOption {
    pub label:   String,
    pub actions: Vec<GameAction>,
}

/// An effect defined inline within a RegisterEffect action
#[derive(Debug, Clone)]
pub struct InlineEffect {
    pub modifiers:    Vec<ModifierDecl>,
    pub grants:       Vec<GrantedAbility>,
    pub restrictions: Vec<RestrictionRule>,
}

#[derive(Debug, Clone)]
pub enum StoreValue {
    SelectedTargets,
    Expression(Expr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NegateTarget { Trigger, Effect, Activation, Summon, Attack }

#[derive(Debug, Clone)]
pub enum AtkModKind {
    Delta { sign: Sign, value: Expr },
    SetTo(Expr),
    Double,
    Halve,
}

#[derive(Debug, Clone)]
pub enum DefModKind {
    Delta { sign: Sign, value: Expr },
    SetTo(Expr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BattlePosition {
    AttackPosition, DefensePosition, FaceDownDefense,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnZone { Hand, Deck, ExtraDeck }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TakeControlDuration { EndPhase, EndOfTurn }

#[derive(Debug, Clone)]
pub struct TokenSpec {
    pub name:      Option<String>,
    pub attribute: Option<Attribute>,
    pub race:      Option<Race>,
    pub atk:       StatValue,
    pub def:       StatValue,
    pub count:     u32,
    pub position:  Option<BattlePosition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DamageTarget { Opponent, You, BothPlayers }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MillSource { YourDeck, OpponentDeck }

// ── Restrictions ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum RestrictionRule {
    Cannot(CannotBlock),
    Must(MustBlock),
    Limit(LimitBlock),
}

#[derive(Debug, Clone)]
pub struct CannotBlock {
    pub action: CannotAction,
    pub scope:  Option<RestrictionScope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CannotAction {
    BeTargeted, BeDestroyed, BeNegated, BeBanished,
    BeReturned, ChangeBattlePosition, BeTributed,
    Attack, AttackDirectly, ActivateEffects, SpecialSummon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MustBlock {
    AttackIfAble, AttackAllMonsters, ChangeToAttackPosition,
}

#[derive(Debug, Clone)]
pub enum LimitBlock {
    AttacksPerTurn(u32),
    SpecialSummonsPerTurn(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestrictionScope {
    Battle, CardEffects, SpellEffects, TrapEffects,
    MonsterEffects, OpponentCardEffects, YourCardEffects, Any,
}

// ── Target Expressions ────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TargetExpr {
    SelfCard,
    Counted {
        count:         u32,
        count_or_more: bool,
        filter:        CardFilter,
        controller:    Option<ControllerRef>,
        zone:          Option<Zone>,
        qualifiers:    Vec<TargetQualifier>,
    },
    Filter(CardFilter),
}

#[derive(Debug, Clone)]
pub enum CardFilter {
    Monster,
    Spell,
    Trap,
    Card,
    Token,
    NonTokenMonster,
    TunerMonster,
    NonTunerMonster,
    NormalMonster,
    EffectMonster,
    FusionMonster,
    SynchroMonster,
    XyzMonster,
    LinkMonster,
    RitualMonster,
    ArchetypeMonster(String),
    ArchetypeCard(String),
    NamedCard(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerRef { You, Opponent, EitherPlayer }

#[derive(Debug, Clone)]
pub enum TargetQualifier {
    FaceUp,
    FaceDown,
    InAttackPosition,
    InDefensePosition,
    WithCounter(String),
    WithAtk(CompareOp, u32),
    WithDef(CompareOp, u32),
    WithLevel(CompareOp, u32),
    OfAttribute(Attribute),
    OfRace(Race),
    OfArchetype(String),
    ThatWasNormalSummoned,
    ThatWasSpecialSummoned,
    OtherThanSelf,
}

// ── Zones ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Zone {
    Hand, Field, Graveyard, Banished, Deck, ExtraDeck,
    SpellTrapZone, MonsterZone, ExtraMonsterZone,
    TopOfDeck, BottomOfDeck,
    /// Pendulum cards destroyed go face-up in the Extra Deck
    ExtraDeckFaceUp,
    /// Pendulum zones specifically
    PendulumZone,
    /// Field spell zone
    FieldZone,
}

impl fmt::Display for Zone {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Zone::Hand             => write!(f, "hand"),
            Zone::Field            => write!(f, "field"),
            Zone::Graveyard        => write!(f, "graveyard"),
            Zone::Banished         => write!(f, "banished"),
            Zone::Deck             => write!(f, "deck"),
            Zone::ExtraDeck        => write!(f, "extra deck"),
            Zone::SpellTrapZone    => write!(f, "spell/trap zone"),
            Zone::MonsterZone      => write!(f, "monster zone"),
            Zone::ExtraMonsterZone => write!(f, "extra monster zone"),
            Zone::TopOfDeck        => write!(f, "top of deck"),
            Zone::BottomOfDeck     => write!(f, "bottom of deck"),
            Zone::ExtraDeckFaceUp  => write!(f, "extra deck (face-up)"),
            Zone::PendulumZone     => write!(f, "pendulum zone"),
            Zone::FieldZone        => write!(f, "field zone"),
        }
    }
}
