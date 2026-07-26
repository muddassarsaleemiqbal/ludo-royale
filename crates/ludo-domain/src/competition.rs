//! Team rosters and deterministic tournament scheduling.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable tournament participant identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ParticipantId(pub u16);

/// Tournament competitor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Participant {
    /// Stable ID.
    pub id: ParticipantId,
    /// Display name.
    pub name: String,
}

/// Supported competition structures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TournamentFormat {
    /// Every participant plays every other participant once.
    RoundRobin,
    /// Single loss eliminates a participant.
    SingleElimination,
}

/// Scheduled or completed pairing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fixture {
    /// Stable fixture index.
    pub id: usize,
    /// Bracket or league round, starting at one.
    pub round: usize,
    /// First participant.
    pub home: ParticipantId,
    /// Second participant.
    pub away: ParticipantId,
    /// Winner once reported.
    pub winner: Option<ParticipantId>,
}

/// Standings row derived from reported fixtures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing {
    /// Participant.
    pub participant: ParticipantId,
    /// Completed fixtures.
    pub played: u32,
    /// Wins.
    pub wins: u32,
    /// Losses.
    pub losses: u32,
    /// Three points per win.
    pub points: u32,
}

/// Versioned competition state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tournament {
    /// Schema for future migrations.
    pub schema_version: u16,
    /// Scheduling format.
    pub format: TournamentFormat,
    /// Registered competitors.
    pub participants: Vec<Participant>,
    /// All currently scheduled fixtures.
    pub fixtures: Vec<Fixture>,
}

impl Tournament {
    /// Builds the initial schedule.
    ///
    /// # Errors
    ///
    /// Returns an error for fewer than two participants, blank/duplicate
    /// identities, or a non-power-of-two elimination field.
    pub fn new(
        format: TournamentFormat,
        participants: Vec<Participant>,
    ) -> Result<Self, CompetitionError> {
        validate_participants(&participants)?;
        let fixtures = match format {
            TournamentFormat::RoundRobin => round_robin(&participants),
            TournamentFormat::SingleElimination => {
                if !participants.len().is_power_of_two() {
                    return Err(CompetitionError::EliminationField);
                }
                participants
                    .chunks_exact(2)
                    .enumerate()
                    .map(|(id, pair)| Fixture {
                        id,
                        round: 1,
                        home: pair[0].id,
                        away: pair[1].id,
                        winner: None,
                    })
                    .collect()
            }
        };
        Ok(Self {
            schema_version: 1,
            format,
            participants,
            fixtures,
        })
    }

    /// Reports one result and grows an elimination bracket when a round ends.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown fixtures, invalid winners, or duplicate
    /// reporting.
    pub fn report_winner(
        &mut self,
        fixture_id: usize,
        winner: ParticipantId,
    ) -> Result<(), CompetitionError> {
        let fixture = self
            .fixtures
            .iter_mut()
            .find(|fixture| fixture.id == fixture_id)
            .ok_or(CompetitionError::UnknownFixture)?;
        if fixture.winner.is_some() {
            return Err(CompetitionError::AlreadyReported);
        }
        if winner != fixture.home && winner != fixture.away {
            return Err(CompetitionError::InvalidWinner);
        }
        fixture.winner = Some(winner);
        if matches!(self.format, TournamentFormat::SingleElimination) {
            self.schedule_next_elimination_round();
        }
        Ok(())
    }

    /// Computes sorted current standings.
    #[must_use]
    pub fn standings(&self) -> Vec<Standing> {
        let mut standings = self
            .participants
            .iter()
            .map(|participant| Standing {
                participant: participant.id,
                played: 0,
                wins: 0,
                losses: 0,
                points: 0,
            })
            .collect::<Vec<_>>();
        for fixture in self
            .fixtures
            .iter()
            .filter(|fixture| fixture.winner.is_some())
        {
            let winner = fixture.winner.unwrap_or(fixture.home);
            for row in &mut standings {
                if row.participant == fixture.home || row.participant == fixture.away {
                    row.played = row.played.saturating_add(1);
                    if row.participant == winner {
                        row.wins = row.wins.saturating_add(1);
                        row.points = row.points.saturating_add(3);
                    } else {
                        row.losses = row.losses.saturating_add(1);
                    }
                }
            }
        }
        standings.sort_by(|left, right| {
            right
                .points
                .cmp(&left.points)
                .then_with(|| right.wins.cmp(&left.wins))
                .then_with(|| left.participant.0.cmp(&right.participant.0))
        });
        standings
    }

    fn schedule_next_elimination_round(&mut self) {
        let current_round = self
            .fixtures
            .iter()
            .map(|fixture| fixture.round)
            .max()
            .unwrap_or(1);
        let current = self
            .fixtures
            .iter()
            .filter(|fixture| fixture.round == current_round)
            .collect::<Vec<_>>();
        if current.is_empty()
            || current.iter().any(|fixture| fixture.winner.is_none())
            || current.len() < 2
            || self
                .fixtures
                .iter()
                .any(|fixture| fixture.round > current_round)
        {
            return;
        }
        let winners = current
            .iter()
            .filter_map(|fixture| fixture.winner)
            .collect::<Vec<_>>();
        let first_id = self.fixtures.len();
        self.fixtures.extend(
            winners
                .chunks_exact(2)
                .enumerate()
                .map(|(offset, pair)| Fixture {
                    id: first_id + offset,
                    round: current_round + 1,
                    home: pair[0],
                    away: pair[1],
                    winner: None,
                }),
        );
    }
}

fn round_robin(participants: &[Participant]) -> Vec<Fixture> {
    // Circle scheduling guarantees that nobody appears twice in one round.
    // An odd field receives one rotating bye.
    let mut rotation = participants
        .iter()
        .map(|participant| Some(participant.id))
        .collect::<Vec<_>>();
    if rotation.len() % 2 != 0 {
        rotation.push(None);
    }
    let rounds = rotation.len().saturating_sub(1);
    let pairings = rotation.len() / 2;
    let mut fixtures =
        Vec::with_capacity(participants.len().saturating_mul(participants.len() - 1) / 2);
    for round in 0..rounds {
        for pairing in 0..pairings {
            if let (Some(home), Some(away)) =
                (rotation[pairing], rotation[rotation.len() - 1 - pairing])
            {
                fixtures.push(Fixture {
                    id: fixtures.len(),
                    round: round + 1,
                    home,
                    away,
                    winner: None,
                });
            }
        }
        rotation[1..].rotate_right(1);
    }
    fixtures
}

fn validate_participants(participants: &[Participant]) -> Result<(), CompetitionError> {
    if participants.len() < 2 {
        return Err(CompetitionError::TooFewParticipants);
    }
    for (index, participant) in participants.iter().enumerate() {
        if participant.name.trim().is_empty()
            || participants[index + 1..].iter().any(|other| {
                other.id == participant.id
                    || other
                        .name
                        .trim()
                        .eq_ignore_ascii_case(participant.name.trim())
            })
        {
            return Err(CompetitionError::InvalidParticipant);
        }
    }
    Ok(())
}

/// Competition configuration and reporting failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CompetitionError {
    /// At least two competitors are required.
    #[error("a tournament requires at least two participants")]
    TooFewParticipants,
    /// Names and IDs must be valid and unique.
    #[error("participants require non-empty names and unique IDs")]
    InvalidParticipant,
    /// Single-elimination fields require a power of two.
    #[error("single elimination requires a power-of-two participant count")]
    EliminationField,
    /// Fixture ID is unknown.
    #[error("fixture does not exist")]
    UnknownFixture,
    /// Fixture already has a result.
    #[error("fixture result was already reported")]
    AlreadyReported,
    /// Winner did not play in the fixture.
    #[error("winner must belong to the fixture")]
    InvalidWinner,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn participants(count: u16) -> Vec<Participant> {
        (0..count)
            .map(|id| Participant {
                id: ParticipantId(id),
                name: format!("Player {id}"),
            })
            .collect()
    }

    #[test]
    fn round_robin_schedules_every_pair_once() {
        let tournament = Tournament::new(TournamentFormat::RoundRobin, participants(4))
            .unwrap_or_else(|_| std::process::abort());
        assert_eq!(tournament.fixtures.len(), 6);
        for round in 1..=3 {
            let fixtures = tournament
                .fixtures
                .iter()
                .filter(|fixture| fixture.round == round)
                .collect::<Vec<_>>();
            assert_eq!(fixtures.len(), 2);
            let players = fixtures
                .iter()
                .flat_map(|fixture| [fixture.home, fixture.away])
                .collect::<Vec<_>>();
            assert_eq!(players.len(), 4);
            assert!(
                players
                    .iter()
                    .enumerate()
                    .all(|(index, player)| { !players[index + 1..].contains(player) })
            );
        }
    }

    #[test]
    fn odd_round_robin_rotates_one_bye_per_round() {
        let tournament = Tournament::new(TournamentFormat::RoundRobin, participants(5))
            .unwrap_or_else(|_| std::process::abort());
        assert_eq!(tournament.fixtures.len(), 10);
        assert_eq!(
            tournament
                .fixtures
                .iter()
                .map(|fixture| fixture.round)
                .max(),
            Some(5)
        );
        for round in 1..=5 {
            assert_eq!(
                tournament
                    .fixtures
                    .iter()
                    .filter(|fixture| fixture.round == round)
                    .count(),
                2
            );
        }
    }

    #[test]
    fn elimination_advances_completed_round() {
        let mut tournament = Tournament::new(TournamentFormat::SingleElimination, participants(4))
            .unwrap_or_else(|_| std::process::abort());
        assert!(tournament.report_winner(0, ParticipantId(0)).is_ok());
        assert!(tournament.report_winner(1, ParticipantId(2)).is_ok());
        assert_eq!(tournament.fixtures.len(), 3);
        assert!(matches!(
            tournament.report_winner(0, ParticipantId(0)),
            Err(CompetitionError::AlreadyReported)
        ));
        assert!(matches!(
            tournament.report_winner(2, ParticipantId(1)),
            Err(CompetitionError::InvalidWinner)
        ));
    }

    #[test]
    fn invalid_fields_are_rejected_before_scheduling() {
        assert!(matches!(
            Tournament::new(TournamentFormat::RoundRobin, participants(1)),
            Err(CompetitionError::TooFewParticipants)
        ));
        assert!(matches!(
            Tournament::new(TournamentFormat::SingleElimination, participants(3)),
            Err(CompetitionError::EliminationField)
        ));
        let mut duplicate_names = participants(2);
        duplicate_names[1].name = " player 0 ".to_owned();
        assert!(matches!(
            Tournament::new(TournamentFormat::RoundRobin, duplicate_names),
            Err(CompetitionError::InvalidParticipant)
        ));
    }
}
