use std::time::Instant;

use ludo_ai::{BotRequest, Difficulty, ParallelBot};
use ludo_domain::{
    DiceValue, GameCommand, GameState, GameStatus, Rules, TurnPhase, standard_players,
};
use rand::{RngExt, SeedableRng, rngs::SmallRng};
use rayon::prelude::*;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--ai-bench") {
        benchmark_ai();
        return;
    }
    let games = std::env::args()
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1_000);
    let start = Instant::now();
    let completed = (0..games)
        .into_par_iter()
        .filter(|seed| simulate(*seed))
        .count();
    let elapsed = start.elapsed();
    println!(
        "Completed {completed}/{games} games in {:.2?} ({:.0} games/s)",
        elapsed,
        u32::try_from(completed).map_or(f64::INFINITY, f64::from) / elapsed.as_secs_f64()
    );
}

fn benchmark_ai() {
    let Ok(mut state) = GameState::new(standard_players(), Rules::default()) else {
        return;
    };
    let Some(six) = DiceValue::new(6) else {
        return;
    };
    if state.apply(GameCommand::Roll(six)).is_err() {
        return;
    }
    let iterations = std::env::args()
        .nth(2)
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(50);
    for (label, difficulty, budget) in [
        ("Easy", Difficulty::Easy, 0),
        ("Medium", Difficulty::Medium, 20),
        ("Hard", Difficulty::Hard, 120),
    ] {
        let request = BotRequest::new(state.clone(), difficulty).with_thinking_time_ms(budget);
        let start = Instant::now();
        let simulations = (0..iterations)
            .map(|_| ParallelBot::choose(&request).simulations)
            .sum::<u32>();
        let elapsed = start.elapsed();
        println!(
            "{label}: {iterations} decisions, {simulations} rollouts, {:.2?}, {:.0} decisions/s",
            elapsed,
            f64::from(iterations) / elapsed.as_secs_f64()
        );
    }
}

fn simulate(seed: u64) -> bool {
    let Ok(mut state) = GameState::new(standard_players(), Rules::default()) else {
        return false;
    };
    let mut rng = SmallRng::seed_from_u64(seed);
    for _ in 0..100_000 {
        if matches!(state.status(), GameStatus::Finished) {
            return true;
        }
        match state.phase() {
            TurnPhase::AwaitingRoll => {
                let value = rng.random_range(1..=6);
                let Some(dice) = DiceValue::new(value) else {
                    return false;
                };
                if state.apply(GameCommand::Roll(dice)).is_err() {
                    return false;
                }
            }
            TurnPhase::AwaitingMove { .. } => {
                let decision = ParallelBot::choose(
                    &BotRequest::new(state.clone(), Difficulty::Hard).with_thinking_time_ms(0),
                );
                let Some(token) = decision.token else {
                    return false;
                };
                if state.apply(GameCommand::Move(token)).is_err() {
                    return false;
                }
            }
        }
    }
    false
}
