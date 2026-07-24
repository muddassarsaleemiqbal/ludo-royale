use std::time::Instant;

use ludo_ai::{BotRequest, Difficulty, ParallelBot};
use ludo_domain::{
    DiceValue, GameCommand, GameState, GameStatus, Rules, TurnPhase, standard_players,
};
use rand::{Rng, SeedableRng, rngs::StdRng};
use rayon::prelude::*;

fn main() {
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

fn simulate(seed: u64) -> bool {
    let Ok(mut state) = GameState::new(standard_players(), Rules::default()) else {
        return false;
    };
    let mut rng = StdRng::seed_from_u64(seed);
    for _ in 0..100_000 {
        if matches!(state.status, GameStatus::Won(_)) {
            return true;
        }
        match state.phase {
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
                let decision = ParallelBot::choose(&BotRequest {
                    state: state.clone(),
                    difficulty: Difficulty::Hard,
                });
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
