// ============================================================
// DuelScript Expression Evaluator — compiler/expr_eval.rs
// Evaluates Expr AST nodes against game state at runtime.
//
// This is the engine-agnostic evaluator. It takes a trait-based
// context so any engine can provide the backing data.
// ============================================================

use crate::ast::*;

/// Trait that engines implement to provide runtime values for expressions.
///
/// This is the minimal interface DuelScript needs from an engine to
/// evaluate dynamic expressions like `self.atk`, `count(...)`, `your_lp`.
pub trait ExprContext {
    /// Get a stat value for the card that owns this effect
    fn self_stat(&self, stat: &Stat) -> i32;

    /// Get a stat value for the current target card
    fn target_stat(&self, stat: &Stat) -> i32;

    /// Get a player's current life points
    fn player_lp(&self, player: &Player) -> i32;

    /// Count cards matching a target expression, optionally in a specific zone
    fn count_matching(&self, target: &TargetExpr, zone: &Option<Zone>) -> i32;
}

/// Evaluate an expression against a runtime context.
///
/// Returns the computed i32 value. Division by zero returns 0.
pub fn eval_expr(expr: &Expr, ctx: &dyn ExprContext) -> i32 {
    match expr {
        Expr::Literal(n) => *n,
        Expr::SelfStat(stat) => ctx.self_stat(stat),
        Expr::TargetStat(stat) => ctx.target_stat(stat),
        Expr::PlayerLp(player) => ctx.player_lp(player),
        Expr::Count { target, zone } => ctx.count_matching(target, zone),
        Expr::BinOp { left, op, right } => {
            let l = eval_expr(left, ctx);
            let r = eval_expr(right, ctx);
            match op {
                BinOp::Add => l.saturating_add(r),
                BinOp::Sub => l.saturating_sub(r),
                BinOp::Mul => l.saturating_mul(r),
                BinOp::Div => if r == 0 { 0 } else { l / r },
            }
        }
    }
}

/// Evaluate an expression that is expected to be a simple literal.
/// Returns None if the expression is dynamic (needs runtime context).
pub fn eval_literal(expr: &Expr) -> Option<i32> {
    match expr {
        Expr::Literal(n) => Some(*n),
        Expr::BinOp { left, op, right } => {
            let l = eval_literal(left)?;
            let r = eval_literal(right)?;
            Some(match op {
                BinOp::Add => l.saturating_add(r),
                BinOp::Sub => l.saturating_sub(r),
                BinOp::Mul => l.saturating_mul(r),
                BinOp::Div => if r == 0 { 0 } else { l / r },
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockContext;

    impl ExprContext for MockContext {
        fn self_stat(&self, stat: &Stat) -> i32 {
            match stat {
                Stat::Atk => 2500,
                Stat::Def => 2000,
                Stat::Level => 7,
                Stat::Rank => 4,
            }
        }
        fn target_stat(&self, stat: &Stat) -> i32 {
            match stat {
                Stat::Atk => 1800,
                Stat::Def => 1500,
                Stat::Level => 4,
                Stat::Rank => 0,
            }
        }
        fn player_lp(&self, player: &Player) -> i32 {
            match player {
                Player::You => 8000,
                Player::Opponent => 6000,
            }
        }
        fn count_matching(&self, _target: &TargetExpr, _zone: &Option<Zone>) -> i32 {
            3
        }
    }

    #[test]
    fn test_literal() {
        let ctx = MockContext;
        assert_eq!(eval_expr(&Expr::lit(42), &ctx), 42);
    }

    #[test]
    fn test_self_stat() {
        let ctx = MockContext;
        assert_eq!(eval_expr(&Expr::SelfStat(Stat::Atk), &ctx), 2500);
    }

    #[test]
    fn test_arithmetic() {
        let ctx = MockContext;
        // self.level * 200
        let expr = Expr::BinOp {
            left: Box::new(Expr::SelfStat(Stat::Level)),
            op: BinOp::Mul,
            right: Box::new(Expr::lit(200)),
        };
        assert_eq!(eval_expr(&expr, &ctx), 1400);
    }

    #[test]
    fn test_half_lp() {
        let ctx = MockContext;
        // your_lp / 2
        let expr = Expr::BinOp {
            left: Box::new(Expr::PlayerLp(Player::You)),
            op: BinOp::Div,
            right: Box::new(Expr::lit(2)),
        };
        assert_eq!(eval_expr(&expr, &ctx), 4000);
    }

    #[test]
    fn test_count_times_value() {
        let ctx = MockContext;
        // count(monster in gy) * 300
        let expr = Expr::BinOp {
            left: Box::new(Expr::Count {
                target: Box::new(TargetExpr::Filter(CardFilter::Monster)),
                zone: Some(Zone::Graveyard),
            }),
            op: BinOp::Mul,
            right: Box::new(Expr::lit(300)),
        };
        assert_eq!(eval_expr(&expr, &ctx), 900);
    }

    #[test]
    fn test_division_by_zero() {
        let ctx = MockContext;
        let expr = Expr::BinOp {
            left: Box::new(Expr::lit(100)),
            op: BinOp::Div,
            right: Box::new(Expr::lit(0)),
        };
        assert_eq!(eval_expr(&expr, &ctx), 0);
    }

    #[test]
    fn test_eval_literal() {
        assert_eq!(eval_literal(&Expr::lit(42)), Some(42));
        assert_eq!(eval_literal(&Expr::BinOp {
            left: Box::new(Expr::lit(10)),
            op: BinOp::Mul,
            right: Box::new(Expr::lit(5)),
        }), Some(50));
        assert_eq!(eval_literal(&Expr::SelfStat(Stat::Atk)), None);
    }
}
