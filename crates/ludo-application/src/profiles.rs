//! Persistent local-player profiles and match statistics.

use ludo_domain::{
    Controller, GameEvent, GameStatus, PlayerColor, PlayerId, RulePreset, TokenPosition,
};
use serde::{Deserialize, Serialize};

use crate::replay::{Replay, ReplayError};

/// Current profile database schema.
pub const PROFILE_SCHEMA_VERSION: u16 = 1;

/// Unlockable profile milestones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Achievement {
    /// Won the first recorded match.
    FirstVictory,
    /// Captured at least 25 opposing tokens.
    CaptureArtist,
    /// Won three recorded matches consecutively.
    HatTrick,
    /// Completed at least 100 tokens.
    HomewardBound,
    /// Won with every board color.
    ColorMaster,
}

/// Compact historical match entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchSummary {
    /// Caller-supplied Unix timestamp.
    pub played_at_unix: u64,
    /// Participant display names.
    pub players: Vec<String>,
    /// Winner name, when determined.
    pub winner: Option<String>,
    /// Named rules when they match a built-in preset.
    pub preset: String,
    /// Number of applied commands.
    pub commands: usize,
}

/// Aggregated statistics for one local person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerProfile {
    /// Stable normalized local identifier.
    pub id: String,
    /// Most recently used display name.
    pub display_name: String,
    /// Recorded matches.
    pub matches: u64,
    /// Recorded victories.
    pub wins: u64,
    /// Wins indexed by `PlayerColor::ALL`.
    pub wins_by_color: [u64; 4],
    /// Wins indexed by `RulePreset::ALL`.
    pub wins_by_preset: [u64; 3],
    /// Opposing tokens captured.
    pub captures: u64,
    /// Owned tokens moved home.
    pub completions: u64,
    /// Current consecutive wins.
    pub current_win_streak: u64,
    /// Best consecutive wins.
    pub best_win_streak: u64,
    /// Earned milestones.
    pub achievements: Vec<Achievement>,
}

impl PlayerProfile {
    fn new(display_name: String) -> Self {
        Self {
            id: profile_id(&display_name),
            display_name,
            matches: 0,
            wins: 0,
            wins_by_color: [0; 4],
            wins_by_preset: [0; 3],
            captures: 0,
            completions: 0,
            current_win_streak: 0,
            best_win_streak: 0,
            achievements: Vec::new(),
        }
    }

    fn refresh_achievements(&mut self) {
        let checks = [
            (self.wins >= 1, Achievement::FirstVictory),
            (self.captures >= 25, Achievement::CaptureArtist),
            (self.best_win_streak >= 3, Achievement::HatTrick),
            (self.completions >= 100, Achievement::HomewardBound),
            (
                self.wins_by_color.iter().all(|wins| *wins > 0),
                Achievement::ColorMaster,
            ),
        ];
        for (earned, achievement) in checks {
            if earned && !self.achievements.contains(&achievement) {
                self.achievements.push(achievement);
            }
        }
    }
}

/// Complete local profile database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileBook {
    /// File schema.
    pub schema_version: u16,
    /// Profiles keyed by normalized IDs.
    pub profiles: Vec<PlayerProfile>,
    /// Most recent matches, newest last.
    pub history: Vec<MatchSummary>,
}

impl Default for ProfileBook {
    fn default() -> Self {
        Self {
            schema_version: PROFILE_SCHEMA_VERSION,
            profiles: Vec::new(),
            history: Vec::new(),
        }
    }
}

impl ProfileBook {
    /// Records a verified finished replay.
    ///
    /// # Errors
    ///
    /// Returns an error when replay validation fails or the match is not
    /// finished.
    pub fn record_match(
        &mut self,
        replay: &Replay,
        played_at_unix: u64,
    ) -> Result<(), ProfileError> {
        let final_state = replay.validate()?;
        if !matches!(final_state.status(), GameStatus::Finished) {
            return Err(ProfileError::MatchNotFinished);
        }
        let initial = &replay.initial_state;
        let winner = final_state.rankings().first().copied();
        let preset = RulePreset::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.rules() == initial.rules());

        for participant in initial
            .players()
            .iter()
            .filter(|state| matches!(state.player.controller, Controller::Human))
        {
            let id = participant.player.id;
            let name = participant.player.name.clone();
            let profile_index = self
                .profiles
                .iter()
                .position(|profile| profile.id == profile_id(&name))
                .unwrap_or_else(|| {
                    self.profiles.push(PlayerProfile::new(name.clone()));
                    self.profiles.len() - 1
                });
            let profile = &mut self.profiles[profile_index];
            profile.display_name = name;
            profile.matches = profile.matches.saturating_add(1);
            profile.captures = profile.captures.saturating_add(event_count(
                replay,
                |event| matches!(event, GameEvent::TokenCaptured { by, .. } if *by == id),
            ));
            profile.completions =
                profile
                    .completions
                    .saturating_add(event_count(replay, |event| {
                        matches!(
                            event,
                            GameEvent::TokenMoved {
                                player,
                                to: TokenPosition::Finished,
                                ..
                            } if *player == id
                        )
                    }));
            let won = winner.is_some_and(|winner| {
                winner == id
                    || (final_state.is_team_game()
                        && final_state.team_id(winner) == final_state.team_id(id))
            });
            if won {
                profile.wins = profile.wins.saturating_add(1);
                profile.current_win_streak = profile.current_win_streak.saturating_add(1);
                profile.best_win_streak = profile.best_win_streak.max(profile.current_win_streak);
                profile.wins_by_color[color_index(participant.player.color)] =
                    profile.wins_by_color[color_index(participant.player.color)].saturating_add(1);
                if let Some(preset) = preset {
                    let index = RulePreset::ALL
                        .iter()
                        .position(|candidate| *candidate == preset)
                        .unwrap_or_default();
                    profile.wins_by_preset[index] = profile.wins_by_preset[index].saturating_add(1);
                }
            } else {
                profile.current_win_streak = 0;
            }
            profile.refresh_achievements();
        }

        self.history.push(MatchSummary {
            played_at_unix,
            players: initial
                .players()
                .iter()
                .map(|state| state.player.name.clone())
                .collect(),
            winner: winner.map(|id| final_state.players()[id.index()].player.name.clone()),
            preset: preset.map_or_else(|| "Custom".to_owned(), |value| value.name().to_owned()),
            commands: replay.records.len(),
        });
        if self.history.len() > 100 {
            self.history.remove(0);
        }
        Ok(())
    }
}

/// Profile-statistics validation failures.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// The supplied replay is corrupt or incompatible.
    #[error(transparent)]
    Replay(#[from] ReplayError),
    /// Statistics only accept completed matches.
    #[error("the replay does not contain a finished match")]
    MatchNotFinished,
}

fn event_count(replay: &Replay, predicate: impl Fn(&GameEvent) -> bool) -> u64 {
    u64::try_from(
        replay
            .records
            .iter()
            .flat_map(|record| &record.events)
            .filter(|event| predicate(event))
            .count(),
    )
    .unwrap_or(u64::MAX)
}

fn color_index(color: PlayerColor) -> usize {
    PlayerColor::ALL
        .iter()
        .position(|candidate| *candidate == color)
        .unwrap_or_default()
}

/// Produces a stable ID suitable for local profile matching.
#[must_use]
pub fn profile_id(name: &str) -> String {
    let normalized = name
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if normalized.is_empty() {
        "player".to_owned()
    } else {
        normalized
    }
}

/// Profile persistence boundary.
pub trait ProfileRepository: Send + Sync {
    /// Loads the profile database or an empty one.
    ///
    /// # Errors
    ///
    /// Returns an adapter-specific message.
    fn load_profiles(&self) -> Result<ProfileBook, String>;

    /// Persists the complete profile database.
    ///
    /// # Errors
    ///
    /// Returns an adapter-specific message.
    fn save_profiles(&self, profiles: &ProfileBook) -> Result<(), String>;
}

/// Resolves a player name from a final state.
#[must_use]
pub fn player_name(players: &[ludo_domain::PlayerState], id: PlayerId) -> Option<&str> {
    players
        .get(id.index())
        .map(|state| state.player.name.as_str())
}

#[cfg(test)]
mod tests {
    use ludo_domain::{
        DiceValue, GameCommand, GameState, GameStatus, RulePreset, Rules, TurnPhase,
        standard_players,
    };

    use super::*;
    use crate::replay::{Replay, ReplayRecord};

    #[test]
    fn profile_ids_are_case_and_space_insensitive() {
        assert_eq!(profile_id(" Ada Lovelace "), "adalovelace");
    }

    #[test]
    fn unfinished_replay_is_not_recorded_as_a_completed_match() {
        let initial = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let mut state = initial.clone();
        let command = GameCommand::Roll(DiceValue::new(2).unwrap_or_else(|| std::process::abort()));
        let events = state
            .apply(command)
            .unwrap_or_else(|_| std::process::abort());
        let replay = Replay {
            schema_version: crate::replay::REPLAY_SCHEMA_VERSION,
            initial_state: initial,
            records: vec![ReplayRecord { command, events }],
        };
        let mut book = ProfileBook::default();
        assert!(matches!(
            book.record_match(&replay, 0),
            Err(ProfileError::MatchNotFinished)
        ));
        assert!(book.history.is_empty());
        assert!(book.profiles.is_empty());
    }

    #[test]
    fn completed_replay_updates_win_completion_and_achievement_statistics() {
        let mut players = standard_players();
        players.truncate(2);
        let initial = GameState::new(players, RulePreset::Quick.rules())
            .unwrap_or_else(|_| std::process::abort());
        let mut state = initial.clone();
        let mut replay = Replay::new(initial);
        let six = DiceValue::new(6).unwrap_or_else(|| std::process::abort());
        for _ in 0..128 {
            if matches!(state.status(), GameStatus::Finished) {
                break;
            }
            let roll = GameCommand::Roll(six);
            let events = state.apply(roll).unwrap_or_else(|_| std::process::abort());
            replay.records.push(ReplayRecord {
                command: roll,
                events,
            });
            if let TurnPhase::AwaitingMove { legal_tokens, .. } = state.phase() {
                let token = legal_tokens
                    .first()
                    .copied()
                    .unwrap_or_else(|| std::process::abort());
                let command = GameCommand::Move(token);
                let events = state
                    .apply(command)
                    .unwrap_or_else(|_| std::process::abort());
                replay.records.push(ReplayRecord { command, events });
            }
        }
        assert!(matches!(state.status(), GameStatus::Finished));

        let mut book = ProfileBook::default();
        assert!(book.record_match(&replay, 123).is_ok());
        assert_eq!(book.profiles.len(), 1);
        let profile = &book.profiles[0];
        assert_eq!(profile.matches, 1);
        assert_eq!(profile.wins, 1);
        assert_eq!(profile.completions, 4);
        assert_eq!(profile.wins_by_color[0], 1);
        assert_eq!(profile.wins_by_preset[1], 1);
        assert!(profile.achievements.contains(&Achievement::FirstVictory));
        assert_eq!(book.history[0].winner.as_deref(), Some("You"));
        assert_eq!(book.history[0].preset, "Quick");
    }

    #[test]
    fn empty_or_symbol_only_profile_names_use_a_stable_fallback() {
        assert_eq!(profile_id(" \t-!? "), "player");
        assert_eq!(profile_id("Éva 42"), "éva42");
    }
}
