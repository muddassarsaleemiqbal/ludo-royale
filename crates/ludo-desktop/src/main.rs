use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    App, Application, Bounds, Context, Entity, Rgba, Task, Timer, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_component::{
    Disableable, Root, Sizable, StyledExt, Theme, ThemeMode, WindowExt as _,
    button::{Button, ButtonRounded, ButtonVariants},
    input::{Input, InputState},
    notification::Notification,
    scroll::ScrollableElement,
};
use ludo_ai::{BotRequest, BotWorker, Difficulty};
use ludo_application::{
    GameSession, SoundCue, SoundPlayer, UndoPolicy,
    profiles::{Achievement, ProfileBook, ProfileRepository},
    replay::{ReplayPlayer, ReplayRepository},
    rule_presets::{NamedRulePreset, RulePresetRepository},
};
use ludo_domain::{
    BotDifficulty, Controller, GameEvent, GameState, GameStatus, Player, PlayerColor, PlayerId,
    RulePreset, Rules, SafeCellRule, TokenId, TokenPosition, TurnPhase, WinCondition,
    competition::{Participant, ParticipantId, Tournament, TournamentFormat},
};
use ludo_infrastructure::{
    AudioWorker, BackgroundGameRepository, JsonProfileRepository, JsonRulePresetRepository,
    RandomDice, ReplayFileRepository,
};
use ludo_network::{
    ClientAction, DiscoveryEvent, HostMessage, JoinRequest, JoinRequestId, LanClientWorker,
    LanDiscovery, LanHost, LanWorkerEvent, LanWorkerRequest, LobbyPhase, LobbySeatKind,
    LobbySnapshot, NearbyGame, ReconnectToken, local_lan_addresses,
};
use ludo_presentation::{
    AnimationCue, AnimationFrame, GameViewModel, PlayerViewModel, TokenViewModel, animation_frames,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Setup,
    Game,
    HotSeat,
    Pause,
    Help,
    Settings,
    ConfirmNew,
    Results,
    Replay,
    Profiles,
    CustomRules,
    UndoConfirm,
    Tournament,
    Lan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeKind {
    Royale,
    Classic,
    Midnight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupSection {
    Local,
    Lan,
    Tournament,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LanConnectionState {
    Offline,
    Connecting,
    AwaitingApproval,
    Connected,
    Reconnecting,
    Disconnected,
}

impl LanConnectionState {
    const fn label(self) -> &'static str {
        match self {
            Self::Offline => "Local game",
            Self::Connecting => "Connecting",
            Self::AwaitingApproval => "Awaiting host",
            Self::Connected => "Connected",
            Self::Reconnecting => "Reconnecting",
            Self::Disconnected => "Disconnected",
        }
    }

    const fn color(self) -> u32 {
        match self {
            Self::Offline => 0x0088_9890,
            Self::Connecting => 0x0048_a9e6,
            Self::AwaitingApproval | Self::Reconnecting => 0x00ed_b34f,
            Self::Connected => 0x0044_c47a,
            Self::Disconnected => 0x00e2_626d,
        }
    }
}

struct JoinRequestNotification;

fn join_request_notification_id(request: &JoinRequestId) -> u64 {
    let mut hasher = DefaultHasher::new();
    request.hash(&mut hasher);
    hasher.finish()
}

impl ThemeKind {
    const ALL: [Self; 3] = [Self::Royale, Self::Classic, Self::Midnight];

    const fn name(self) -> &'static str {
        match self {
            Self::Royale => "Royale",
            Self::Classic => "Classic",
            Self::Midnight => "Midnight",
        }
    }

    const fn palette(self) -> Palette {
        match self {
            Self::Royale => Palette {
                canvas: 0x0007_120f,
                surface: 0x000d_1e18,
                raised: 0x0014_2a22,
                line: 0x0028_473b,
                accent: 0x00f2_c766,
                foreground: 0x00f7_faf8,
                muted: 0x00a6_b8af,
                table: 0x006c_4129,
                table_edge: 0x00bd_8654,
            },
            Self::Classic => Palette {
                canvas: 0x00f0_e8d8,
                surface: 0x00ff_fbf1,
                raised: 0x00f8_f0df,
                line: 0x00c5_b89f,
                accent: 0x008c_2f24,
                foreground: 0x0028_211c,
                muted: 0x006d_6257,
                table: 0x0092_653f,
                table_edge: 0x006d_452a,
            },
            Self::Midnight => Palette {
                canvas: 0x0008_1020,
                surface: 0x0010_1c35,
                raised: 0x0017_2847,
                line: 0x0032_4a70,
                accent: 0x0079_d8ff,
                foreground: 0x00f1_f6ff,
                muted: 0x0097_a9c3,
                table: 0x0022_3656,
                table_edge: 0x0046_6593,
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Palette {
    canvas: u32,
    surface: u32,
    raised: u32,
    line: u32,
    accent: u32,
    foreground: u32,
    muted: u32,
    table: u32,
    table_edge: u32,
}

#[derive(Debug, Clone, Copy)]
struct UiSettings {
    theme: ThemeKind,
    sound: bool,
    reduced_motion: bool,
    high_contrast: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: ThemeKind::Royale,
            sound: true,
            reduced_motion: false,
            high_contrast: false,
        }
    }
}

fn apply_component_theme(theme_kind: ThemeKind, cx: &mut App) {
    let palette = theme_kind.palette();
    let theme = Theme::global_mut(cx);
    theme.mode = if matches!(theme_kind, ThemeKind::Classic) {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    theme.background = rgb(palette.canvas).into();
    theme.foreground = rgb(palette.foreground).into();
    theme.primary = rgb(palette.accent).into();
    theme.primary_foreground = rgb(if matches!(theme_kind, ThemeKind::Classic) {
        0x00ff_ffff
    } else {
        0x0018_211c
    })
    .into();
    theme.primary_hover = Rgba {
        a: 0.88,
        ..rgb(palette.accent)
    }
    .into();
    theme.primary_active = Rgba {
        a: 0.76,
        ..rgb(palette.accent)
    }
    .into();
    theme.secondary = rgb(palette.raised).into();
    theme.secondary_foreground = rgb(palette.foreground).into();
    theme.secondary_hover = rgb(palette.line).into();
    theme.secondary_active = rgb(palette.surface).into();
    theme.accent = rgb(palette.raised).into();
    theme.accent_foreground = rgb(palette.foreground).into();
    theme.muted = rgb(palette.raised).into();
    theme.muted_foreground = rgb(palette.muted).into();
    theme.input = rgb(palette.line).into();
    theme.border = rgb(palette.line).into();
    theme.ring = rgb(palette.accent).into();
    theme.popover = rgb(palette.surface).into();
    theme.popover_foreground = rgb(palette.foreground).into();
    theme.radius = px(10.0);
    theme.radius_lg = px(16.0);
}

struct SetupState {
    player_count: usize,
    names: Vec<Entity<InputState>>,
    controllers: [Controller; 4],
    difficulties: [BotDifficulty; 4],
    colors: [PlayerColor; 4],
    preset: RulePreset,
    custom_rules: Rules,
    custom_name: Entity<InputState>,
    use_custom_rules: bool,
    team_mode: bool,
    ai_thinking_time_ms: u64,
    lan_address: Entity<InputState>,
    lan_code: Entity<InputState>,
}

impl SetupState {
    fn new(window: &mut Window, cx: &mut Context<GameView>) -> Self {
        let names = (0..4)
            .map(|index| {
                cx.new(|cx| {
                    InputState::new(window, cx).default_value(if index == 0 {
                        "You".to_owned()
                    } else {
                        format!("Player {}", index + 1)
                    })
                })
            })
            .collect();
        Self {
            player_count: 4,
            names,
            controllers: [
                Controller::Human,
                Controller::Bot,
                Controller::Bot,
                Controller::Bot,
            ],
            difficulties: [BotDifficulty::Hard; 4],
            colors: PlayerColor::ALL,
            preset: RulePreset::Classic,
            custom_rules: RulePreset::Classic.rules(),
            custom_name: cx
                .new(|cx| InputState::new(window, cx).default_value("My House Rules".to_owned())),
            use_custom_rules: false,
            team_mode: false,
            ai_thinking_time_ms: 120,
            lan_address: cx
                .new(|cx| InputState::new(window, cx).default_value("127.0.0.1:42042".to_owned())),
            lan_code: cx.new(|cx| InputState::new(window, cx)),
        }
    }
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "independent UI, animation, bot, and network activity flags are orthogonal states"
)]
struct GameView {
    screen: Screen,
    setup_section: SetupSection,
    overlay_return: Screen,
    setup: SetupState,
    session: GameSession,
    view_model: GameViewModel,
    repository: Arc<BackgroundGameRepository>,
    resume_available: bool,
    bot_worker: BotWorker,
    bot_pending: bool,
    last_error: Option<String>,
    bot_task: Option<Task<()>>,
    audio: AudioWorker,
    settings: UiSettings,
    animation_task: Option<Task<()>>,
    animating: bool,
    visual_positions: HashMap<(usize, usize), TokenPosition>,
    animated_dice: Option<u8>,
    effect_message: Option<String>,
    replay_repository: Arc<ReplayFileRepository>,
    profile_repository: Arc<JsonProfileRepository>,
    rule_repository: Arc<JsonRulePresetRepository>,
    profiles: ProfileBook,
    custom_presets: Vec<NamedRulePreset>,
    replay_player: Option<ReplayPlayer>,
    replay_task: Option<Task<()>>,
    replay_return: Screen,
    replay_exit_state: Option<GameState>,
    result_recorded_revision: Option<u64>,
    tournament: Option<Tournament>,
    tournament_format: TournamentFormat,
    tournament_match: Option<usize>,
    lan_host: Option<LanHost>,
    lan_worker: LanClientWorker,
    lan_token: Option<ReconnectToken>,
    lan_player: Option<PlayerId>,
    lan_join_request: Option<JoinRequestId>,
    lan_join_requests: Vec<JoinRequest>,
    lan_pending: bool,
    lan_task: Option<Task<()>>,
    host_join_task: Option<Task<()>>,
    lan_previous: Option<PlayerId>,
    lan_endpoint: Option<std::net::SocketAddr>,
    lan_share_endpoint: Option<std::net::SocketAddr>,
    lan_room: Option<String>,
    lan_lobby: Option<LobbySnapshot>,
    lan_connection: LanConnectionState,
    lan_latency_ms: Option<u128>,
    lan_last_seen: Option<Instant>,
    lan_advertised_humans: usize,
    discovery: Option<LanDiscovery>,
    nearby_games: Vec<NearbyGame>,
    discovery_task: Option<Task<()>>,
}

impl GameView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let repository = Arc::new(BackgroundGameRepository::new(save_path()));
        let resume_available = repository.exists();
        let state = default_game();
        let view_model = GameViewModel::from(&state);
        let audio = AudioWorker::new();
        let replay_repository = Arc::new(ReplayFileRepository::new(replay_path()));
        let profile_repository = Arc::new(JsonProfileRepository::new(profile_path()));
        let rule_repository = Arc::new(JsonRulePresetRepository::new(
            rule_collection_path(),
            rule_exchange_path(),
        ));
        let profiles = profile_repository.load_profiles().unwrap_or_default();
        let custom_presets = rule_repository.load_rule_presets().unwrap_or_default();
        let mut view = Self {
            screen: Screen::Setup,
            setup_section: SetupSection::Local,
            overlay_return: Screen::Setup,
            setup: SetupState::new(window, cx),
            session: GameSession::new(state, Arc::new(RandomDice))
                .with_repository(repository.clone()),
            view_model,
            repository,
            resume_available,
            bot_worker: BotWorker::new(),
            bot_pending: false,
            last_error: None,
            bot_task: None,
            audio,
            settings: UiSettings::default(),
            animation_task: None,
            animating: false,
            visual_positions: HashMap::new(),
            animated_dice: None,
            effect_message: None,
            replay_repository,
            profile_repository,
            rule_repository,
            profiles,
            custom_presets,
            replay_player: None,
            replay_task: None,
            replay_return: Screen::Setup,
            replay_exit_state: None,
            result_recorded_revision: None,
            tournament: None,
            tournament_format: TournamentFormat::RoundRobin,
            tournament_match: None,
            lan_host: None,
            lan_worker: LanClientWorker::new(),
            lan_token: None,
            lan_player: None,
            lan_join_request: None,
            lan_join_requests: Vec::new(),
            lan_pending: false,
            lan_task: None,
            host_join_task: None,
            lan_previous: None,
            lan_endpoint: None,
            lan_share_endpoint: None,
            lan_room: None,
            lan_lobby: None,
            lan_connection: LanConnectionState::Offline,
            lan_latency_ms: None,
            lan_last_seen: None,
            lan_advertised_humans: 0,
            discovery: LanDiscovery::new().ok(),
            nearby_games: Vec::new(),
            discovery_task: None,
        };
        view.start_discovery_poll(cx);
        view
    }

    fn change_player_count(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.setup.player_count = self
            .setup
            .player_count
            .saturating_add_signed(delta)
            .clamp(2, 4);
        cx.notify();
    }

    fn toggle_controller(&mut self, index: usize, cx: &mut Context<Self>) {
        self.setup.controllers[index] = match self.setup.controllers[index] {
            Controller::Human => Controller::Bot,
            Controller::Bot => Controller::Human,
        };
        cx.notify();
    }

    fn cycle_difficulty(&mut self, index: usize, cx: &mut Context<Self>) {
        let current = self.setup.difficulties[index];
        let next = BotDifficulty::ALL
            .iter()
            .position(|difficulty| *difficulty == current)
            .map_or(0, |position| (position + 1) % BotDifficulty::ALL.len());
        self.setup.difficulties[index] = BotDifficulty::ALL[next];
        cx.notify();
    }

    fn cycle_color(&mut self, index: usize, cx: &mut Context<Self>) {
        let current = self.setup.colors[index];
        let next_index = PlayerColor::ALL
            .iter()
            .position(|color| *color == current)
            .map_or(0, |position| (position + 1) % PlayerColor::ALL.len());
        let next_color = PlayerColor::ALL[next_index];
        if let Some(owner) = self.setup.colors[..self.setup.player_count]
            .iter()
            .position(|color| *color == next_color)
        {
            self.setup.colors.swap(index, owner);
        } else {
            self.setup.colors[index] = next_color;
        }
        cx.notify();
    }

    fn cycle_ai_time(&mut self, cx: &mut Context<Self>) {
        self.setup.ai_thinking_time_ms = match self.setup.ai_thinking_time_ms {
            40 => 120,
            120 => 300,
            _ => 40,
        };
        cx.notify();
    }

    fn start_game(&mut self, cx: &mut Context<Self>) {
        let players = (0..self.setup.player_count)
            .filter_map(|index| {
                let id = PlayerId::new(u8::try_from(index).ok()?)?;
                let entered = self.setup.names[index].read(cx).value().to_string();
                let name = if entered.trim().is_empty() {
                    format!("Player {}", index + 1)
                } else {
                    entered.trim().to_owned()
                };
                Some(Player {
                    id,
                    name,
                    color: self.setup.colors[index],
                    controller: self.setup.controllers[index],
                    bot_difficulty: self.setup.difficulties[index],
                })
            })
            .collect();
        let rules = if self.setup.use_custom_rules {
            self.setup.custom_rules
        } else {
            self.setup.preset.rules()
        };
        let state = if self.setup.team_mode {
            GameState::new_team(players, rules, [0, 1, 0, 1])
        } else {
            GameState::new(players, rules)
        };
        match state {
            Ok(state) => {
                self.bot_task = None;
                self.animation_task = None;
                self.bot_worker = BotWorker::new();
                let _ = self.repository.delete();
                self.session.restore(state);
                self.session.set_undo_policy(
                    if self.setup.team_mode || matches!(rules.win_condition, WinCondition::RankAll)
                    {
                        UndoPolicy::Competitive
                    } else {
                        UndoPolicy::Allowed
                    },
                );
                self.screen = Screen::Game;
                self.resume_available = false;
                self.bot_pending = false;
                self.animating = false;
                self.visual_positions.clear();
                self.animated_dice = None;
                self.effect_message = None;
                self.result_recorded_revision = None;
                self.tournament_match = None;
                self.lan_token = None;
                self.lan_player = None;
                self.lan_join_request = None;
                self.lan_join_requests.clear();
                self.host_join_task = None;
                if let Some(discovery) = &mut self.discovery {
                    discovery.stop_advertising();
                }
                self.lan_host = None;
                self.lan_worker = LanClientWorker::new();
                self.lan_endpoint = None;
                self.lan_share_endpoint = None;
                self.lan_room = None;
                self.lan_lobby = None;
                self.lan_connection = LanConnectionState::Offline;
                self.lan_latency_ms = None;
                self.lan_last_seen = None;
                self.lan_advertised_humans = 0;
                self.last_error = None;
                let _ = self.session.save();
                self.refresh(cx);
                self.drive_bot(cx);
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn resume_game(&mut self, cx: &mut Context<Self>) {
        match self.session.load() {
            Ok(true) => {
                self.bot_task = None;
                self.animation_task = None;
                self.bot_worker = BotWorker::new();
                self.screen = Screen::Game;
                self.resume_available = false;
                self.last_error = None;
                self.animating = false;
                self.visual_positions.clear();
                self.animated_dice = None;
                self.effect_message = None;
                self.refresh(cx);
                self.drive_bot(cx);
            }
            Ok(false) => {
                self.resume_available = false;
                self.last_error = Some("No saved match was found.".to_owned());
                cx.notify();
            }
            Err(error) => {
                self.resume_available = self.repository.exists();
                self.last_error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn return_to_setup(&mut self, cx: &mut Context<Self>) {
        self.bot_task = None;
        self.animation_task = None;
        self.bot_worker = BotWorker::new();
        self.screen = Screen::Setup;
        self.bot_pending = false;
        self.animating = false;
        self.visual_positions.clear();
        self.animated_dice = None;
        self.effect_message = None;
        self.tournament_match = None;
        self.lan_token = None;
        self.lan_player = None;
        self.lan_join_request = None;
        self.lan_join_requests.clear();
        self.lan_pending = false;
        if let Some(discovery) = &mut self.discovery {
            discovery.stop_advertising();
        }
        self.lan_host = None;
        self.lan_task = None;
        self.host_join_task = None;
        self.lan_worker = LanClientWorker::new();
        self.lan_endpoint = None;
        self.lan_share_endpoint = None;
        self.lan_room = None;
        self.lan_lobby = None;
        self.lan_connection = LanConnectionState::Offline;
        self.lan_latency_ms = None;
        self.lan_last_seen = None;
        self.lan_advertised_humans = 0;
        self.resume_available = self.repository.exists();
        cx.notify();
    }

    fn request_new_game(&mut self, cx: &mut Context<Self>) {
        if self.session.state().revision() > 0
            && matches!(self.session.state().status(), GameStatus::Playing)
        {
            self.bot_task = None;
            self.bot_pending = false;
            self.screen = Screen::ConfirmNew;
            cx.notify();
        } else {
            self.return_to_setup(cx);
        }
    }

    fn confirm_new_game(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.repository.delete() {
            self.last_error = Some(error);
        }
        self.return_to_setup(cx);
        self.resume_available = false;
    }

    fn pause(&mut self, cx: &mut Context<Self>) {
        if self.animating {
            return;
        }
        self.bot_task = None;
        self.bot_pending = false;
        self.screen = Screen::Pause;
        cx.notify();
    }

    fn resume_from_pause(&mut self, cx: &mut Context<Self>) {
        self.screen = Screen::Game;
        cx.notify();
        if self.lan_token.is_some() {
            self.schedule_lan_sync(cx);
        } else {
            self.drive_bot(cx);
        }
    }

    fn open_help(&mut self, cx: &mut Context<Self>) {
        if self.animating {
            return;
        }
        self.overlay_return = self.screen;
        if matches!(self.screen, Screen::Game) {
            self.bot_task = None;
            self.bot_pending = false;
        }
        self.screen = Screen::Help;
        cx.notify();
    }

    fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.overlay_return = self.screen;
        self.screen = Screen::Settings;
        cx.notify();
    }

    fn close_overlay(&mut self, cx: &mut Context<Self>) {
        self.screen = self.overlay_return;
        cx.notify();
        if matches!(self.screen, Screen::Game) {
            if self.lan_token.is_some() {
                self.schedule_lan_sync(cx);
            } else {
                self.drive_bot(cx);
            }
        }
    }

    fn cycle_theme(&mut self, cx: &mut Context<Self>) {
        let next = ThemeKind::ALL
            .iter()
            .position(|theme| *theme == self.settings.theme)
            .map_or(0, |position| (position + 1) % ThemeKind::ALL.len());
        self.settings.theme = ThemeKind::ALL[next];
        apply_component_theme(self.settings.theme, cx);
        cx.notify();
    }

    fn toggle_sound(&mut self, cx: &mut Context<Self>) {
        self.settings.sound = !self.settings.sound;
        self.audio.set_enabled(self.settings.sound);
        if self.settings.sound {
            self.audio.play(SoundCue::Turn);
        }
        cx.notify();
    }

    fn toggle_reduced_motion(&mut self, cx: &mut Context<Self>) {
        self.settings.reduced_motion = !self.settings.reduced_motion;
        cx.notify();
    }

    fn toggle_high_contrast(&mut self, cx: &mut Context<Self>) {
        self.settings.high_contrast = !self.settings.high_contrast;
        cx.notify();
    }

    fn toggle_team_mode(&mut self, cx: &mut Context<Self>) {
        self.setup.team_mode = !self.setup.team_mode;
        if self.setup.team_mode {
            self.setup.player_count = 4;
        }
        cx.notify();
    }

    fn show_setup_section(&mut self, section: SetupSection, cx: &mut Context<Self>) {
        self.setup_section = section;
        self.last_error = None;
        cx.notify();
    }

    fn select_preset(&mut self, preset: RulePreset, cx: &mut Context<Self>) {
        self.setup.preset = preset;
        self.setup.custom_rules = preset.rules();
        self.setup.use_custom_rules = false;
        cx.notify();
    }

    fn open_custom_rules(&mut self, cx: &mut Context<Self>) {
        self.setup.use_custom_rules = true;
        self.screen = Screen::CustomRules;
        cx.notify();
    }

    fn toggle_custom_rule(&mut self, index: usize, cx: &mut Context<Self>) {
        let rules = &mut self.setup.custom_rules;
        match index {
            0 => {
                rules.extra_turn_on_six = !rules.extra_turn_on_six;
                if !rules.extra_turn_on_six {
                    rules.three_sixes_forfeit = false;
                }
            }
            1 => rules.extra_turn_on_capture = !rules.extra_turn_on_capture,
            2 => rules.extra_turn_on_home = !rules.extra_turn_on_home,
            3 => rules.three_sixes_forfeit = !rules.three_sixes_forfeit,
            4 => rules.blockades = !rules.blockades,
            5 => rules.exact_home_roll = !rules.exact_home_roll,
            6 => {
                rules.safe_cells = match rules.safe_cells {
                    SafeCellRule::None => SafeCellRule::Starts,
                    SafeCellRule::Starts => SafeCellRule::StartsAndStars,
                    SafeCellRule::StartsAndStars => SafeCellRule::None,
                };
            }
            7 => {
                rules.win_condition = match rules.win_condition {
                    WinCondition::FirstWinner => WinCondition::RankAll,
                    WinCondition::RankAll => WinCondition::FirstWinner,
                };
            }
            _ => {}
        }
        self.last_error = rules.validate().err().map(|error| error.to_string());
        cx.notify();
    }

    fn save_custom_rules(&mut self, cx: &mut Context<Self>) {
        let name = self.setup.custom_name.read(cx).value().to_string();
        match NamedRulePreset::new(name, self.setup.custom_rules) {
            Ok(preset) => {
                if let Some(existing) = self
                    .custom_presets
                    .iter()
                    .position(|candidate| candidate.name == preset.name)
                {
                    self.custom_presets[existing] = preset;
                } else {
                    self.custom_presets.push(preset);
                }
                self.last_error = self
                    .rule_repository
                    .save_rule_presets(&self.custom_presets)
                    .err();
            }
            Err(error) => self.last_error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn export_custom_rules(&mut self, cx: &mut Context<Self>) {
        let name = self.setup.custom_name.read(cx).value().to_string();
        self.last_error = NamedRulePreset::new(name, self.setup.custom_rules)
            .map_err(|error| error.to_string())
            .and_then(|preset| self.rule_repository.export_rule_preset(&preset))
            .err();
        cx.notify();
    }

    fn import_custom_rules(&mut self, cx: &mut Context<Self>) {
        match self.rule_repository.import_rule_preset() {
            Ok(Some(preset)) => {
                self.setup.custom_rules = preset.rules;
                self.setup.use_custom_rules = true;
                self.last_error = None;
            }
            Ok(None) => self.last_error = Some("No exported rule file was found.".to_owned()),
            Err(error) => self.last_error = Some(error),
        }
        cx.notify();
    }

    fn load_custom_preset(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(preset) = self.custom_presets.get(index) {
            self.setup.custom_rules = preset.rules;
            self.setup.use_custom_rules = true;
            self.last_error = None;
            cx.notify();
        }
    }

    fn request_undo(&mut self, cx: &mut Context<Self>) {
        if self.session.can_undo() {
            self.screen = Screen::UndoConfirm;
        } else {
            self.last_error = Some(match self.session.undo_policy() {
                UndoPolicy::Allowed => "Nothing has been played yet.".to_owned(),
                UndoPolicy::Competitive => {
                    "Undo is disabled for team and tournament matches.".to_owned()
                }
            });
        }
        cx.notify();
    }

    fn confirm_undo(&mut self, cx: &mut Context<Self>) {
        match self.session.undo() {
            Ok(()) => {
                self.screen = Screen::Game;
                self.last_error = None;
                self.refresh(cx);
                self.drive_bot(cx);
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                self.screen = Screen::Pause;
                cx.notify();
            }
        }
    }

    fn save_replay(&mut self, cx: &mut Context<Self>) {
        self.last_error = self
            .replay_repository
            .save_replay(self.session.replay())
            .err();
        cx.notify();
    }

    fn open_replay(&mut self, return_to: Screen, cx: &mut Context<Self>) {
        match self.replay_repository.load_replay() {
            Ok(Some(replay)) => match ReplayPlayer::new(replay) {
                Ok(player) => {
                    self.replay_task = None;
                    self.replay_exit_state = Some(self.session.state().clone());
                    self.session.restore(player.state().clone());
                    self.replay_player = Some(player);
                    self.replay_return = return_to;
                    self.screen = Screen::Replay;
                    self.last_error = None;
                    self.refresh(cx);
                }
                Err(error) => {
                    self.last_error = Some(error.to_string());
                    cx.notify();
                }
            },
            Ok(None) => {
                self.last_error = Some("No replay file has been saved yet.".to_owned());
                cx.notify();
            }
            Err(error) => {
                self.last_error = Some(error);
                cx.notify();
            }
        }
    }

    fn step_replay(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(player) = &mut self.replay_player else {
            return false;
        };
        match player.step() {
            Ok(Some(_)) => {
                self.session.restore(player.state().clone());
                self.refresh(cx);
                true
            }
            Ok(None) => {
                player.set_playing(false);
                cx.notify();
                false
            }
            Err(error) => {
                player.set_playing(false);
                self.last_error = Some(error.to_string());
                cx.notify();
                false
            }
        }
    }

    fn toggle_replay_playback(&mut self, cx: &mut Context<Self>) {
        let Some(player) = &mut self.replay_player else {
            return;
        };
        if player.is_playing() {
            player.set_playing(false);
            self.replay_task = None;
            cx.notify();
            return;
        }
        player.set_playing(true);
        self.replay_task = Some(cx.spawn(async move |view, cx| {
            loop {
                let delay = cx
                    .update(|cx| {
                        let view = view.upgrade()?;
                        view.read(cx).replay_player.as_ref().map(|player| {
                            let (numerator, denominator) = player.speed().ratio();
                            420_u64.saturating_mul(numerator) / denominator
                        })
                    })
                    .ok()
                    .flatten();
                let Some(delay) = delay else {
                    break;
                };
                Timer::after(Duration::from_millis(delay)).await;
                let keep_playing = cx
                    .update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return false;
                        };
                        view.update(cx, |this, cx| {
                            this.replay_player
                                .as_ref()
                                .is_some_and(ReplayPlayer::is_playing)
                                && this.step_replay(cx)
                        })
                    })
                    .unwrap_or(false);
                if !keep_playing {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn seek_replay(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(player) = &mut self.replay_player else {
            return;
        };
        let target = player
            .cursor()
            .saturating_add_signed(delta)
            .min(player.len());
        if let Err(error) = player.seek(target) {
            self.last_error = Some(error.to_string());
        } else {
            self.session.restore(player.state().clone());
            self.refresh(cx);
        }
    }

    fn cycle_replay_speed(&mut self, cx: &mut Context<Self>) {
        if let Some(player) = &mut self.replay_player {
            player.cycle_speed();
            cx.notify();
        }
    }

    fn close_replay(&mut self, cx: &mut Context<Self>) {
        self.replay_task = None;
        self.replay_player = None;
        if let Some(state) = self.replay_exit_state.take() {
            self.session.restore(state);
        }
        self.screen = self.replay_return;
        self.refresh(cx);
    }

    fn open_profiles(&mut self, cx: &mut Context<Self>) {
        self.profiles = self.profile_repository.load_profiles().unwrap_or_default();
        self.screen = Screen::Profiles;
        cx.notify();
    }

    fn create_tournament(&mut self, cx: &mut Context<Self>) {
        let participants = (0..self.setup.player_count)
            .filter_map(|index| {
                let id = u16::try_from(index).ok()?;
                let entered = self.setup.names[index].read(cx).value().to_string();
                Some(Participant {
                    id: ParticipantId(id),
                    name: if entered.trim().is_empty() {
                        format!("Player {}", index + 1)
                    } else {
                        entered.trim().to_owned()
                    },
                })
            })
            .collect();
        match Tournament::new(self.tournament_format, participants) {
            Ok(tournament) => {
                self.tournament = Some(tournament);
                self.screen = Screen::Tournament;
                self.last_error = None;
            }
            Err(error) => self.last_error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn toggle_tournament_format(&mut self, cx: &mut Context<Self>) {
        self.tournament_format = match self.tournament_format {
            TournamentFormat::RoundRobin => TournamentFormat::SingleElimination,
            TournamentFormat::SingleElimination => TournamentFormat::RoundRobin,
        };
        cx.notify();
    }

    fn start_fixture(&mut self, fixture_id: usize, cx: &mut Context<Self>) {
        let Some(tournament) = &self.tournament else {
            return;
        };
        let Some(fixture) = tournament
            .fixtures
            .iter()
            .find(|fixture| fixture.id == fixture_id && fixture.winner.is_none())
        else {
            return;
        };
        let competitors = [fixture.home, fixture.away]
            .into_iter()
            .enumerate()
            .filter_map(|(index, participant_id)| {
                let participant = tournament
                    .participants
                    .iter()
                    .find(|participant| participant.id == participant_id)?;
                Some(Player {
                    id: PlayerId::new(u8::try_from(index).ok()?)?,
                    name: participant.name.clone(),
                    color: PlayerColor::ALL[index],
                    controller: Controller::Human,
                    bot_difficulty: BotDifficulty::Hard,
                })
            })
            .collect();
        match GameState::new(competitors, RulePreset::Classic.rules()) {
            Ok(state) => {
                self.session.restore(state);
                self.session.set_undo_policy(UndoPolicy::Competitive);
                self.tournament_match = Some(fixture_id);
                self.result_recorded_revision = None;
                self.bot_task = None;
                self.animation_task = None;
                self.bot_pending = false;
                self.animating = false;
                self.screen = Screen::Game;
                self.last_error = None;
                let _ = self.session.save();
                self.refresh(cx);
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn return_to_tournament(&mut self, cx: &mut Context<Self>) {
        self.tournament_match = None;
        self.screen = Screen::Tournament;
        cx.notify();
    }

    fn start_discovery_poll(&mut self, cx: &mut Context<Self>) {
        if self.discovery.is_none() {
            return;
        }
        self.discovery_task = Some(cx.spawn(async move |view, cx| {
            loop {
                Timer::after(Duration::from_millis(250)).await;
                let keep_running = cx
                    .update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return false;
                        };
                        view.update(cx, |this, cx| {
                            let mut changed = false;
                            while let Some(event) =
                                this.discovery.as_ref().and_then(LanDiscovery::try_event)
                            {
                                match event {
                                    DiscoveryEvent::Upsert(game) => {
                                        if let Some(index) = this
                                            .nearby_games
                                            .iter()
                                            .position(|candidate| candidate.id == game.id)
                                        {
                                            if this.nearby_games[index] != game {
                                                this.nearby_games[index] = game;
                                                changed = true;
                                            }
                                        } else {
                                            this.nearby_games.push(game);
                                            changed = true;
                                        }
                                    }
                                    DiscoveryEvent::Removed(id) => {
                                        let previous = this.nearby_games.len();
                                        this.nearby_games.retain(|candidate| candidate.id != id);
                                        changed |= previous != this.nearby_games.len();
                                    }
                                }
                            }
                            if changed {
                                this.nearby_games
                                    .sort_by(|left, right| left.name.cmp(&right.name));
                                cx.notify();
                            }
                            true
                        })
                    })
                    .unwrap_or(false);
                if !keep_running {
                    break;
                }
            }
        }));
    }

    fn local_player_name(&self, cx: &Context<Self>) -> String {
        self.setup.names[0].read(cx).value().trim().to_owned()
    }

    fn lan_host_state(&self, cx: &Context<Self>) -> Result<GameState, String> {
        if !matches!(self.screen, Screen::Setup) {
            return Err("Create LAN rooms from the new-game setup screen.".to_owned());
        }
        let players = (0..self.setup.player_count)
            .filter_map(|index| {
                let is_host = index == 0;
                Some(Player {
                    id: PlayerId::new(u8::try_from(index).ok()?)?,
                    name: if is_host {
                        let entered = self.setup.names[index].read(cx).value().to_string();
                        if entered.trim().is_empty() {
                            "Host".to_owned()
                        } else {
                            entered.trim().to_owned()
                        }
                    } else {
                        format!("{} Computer", self.setup.colors[index].name())
                    },
                    color: self.setup.colors[index],
                    controller: if is_host {
                        Controller::Human
                    } else {
                        Controller::Bot
                    },
                    bot_difficulty: self.setup.difficulties[index],
                })
            })
            .collect();
        let rules = if self.setup.use_custom_rules {
            self.setup.custom_rules
        } else {
            self.setup.preset.rules()
        };
        if self.setup.team_mode {
            GameState::new_team(players, rules, [0, 1, 0, 1])
        } else {
            GameState::new(players, rules)
        }
        .map_err(|error| error.to_string())
    }

    fn advertise_lan_host(&mut self, room_name: &str, port: u16, humans: usize) {
        if let Some(discovery) = &mut self.discovery
            && let Err(error) = discovery.advertise(
                room_name,
                port,
                humans,
                self.setup.player_count,
                self.setup.preset.name(),
            )
        {
            self.last_error = Some(format!(
                "The room is running, but automatic discovery failed: {error}"
            ));
        } else {
            self.lan_advertised_humans = humans;
        }
    }

    fn refresh_lan_advertisement(&mut self) {
        let (Some(host), Some(lobby)) = (&self.lan_host, &self.lan_lobby) else {
            return;
        };
        let humans = lobby.human_count();
        if humans == self.lan_advertised_humans || !matches!(lobby.phase, LobbyPhase::Waiting) {
            return;
        }
        let room_name = lobby.seats.first().map_or_else(
            || "Ludo game".to_owned(),
            |seat| format!("{}'s game", seat.name),
        );
        let port = host.address().port();
        self.advertise_lan_host(&room_name, port, humans);
    }

    fn refresh_host_join_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = &self.lan_host else {
            return;
        };
        let Ok(requests) = host.pending_join_requests() else {
            return;
        };
        for request in &requests {
            if self
                .lan_join_requests
                .iter()
                .any(|known| known.id == request.id)
            {
                continue;
            }
            window.push_notification(
                Notification::warning(format!("{} wants to join your LAN game.", request.name))
                    .title("New join request")
                    .id1::<JoinRequestNotification>((
                        "join-request",
                        join_request_notification_id(&request.id),
                    ))
                    .autohide(false),
                cx,
            );
        }
        for request in &self.lan_join_requests {
            if requests.iter().any(|pending| pending.id == request.id) {
                continue;
            }
            window.push_notification(
                Notification::info(format!("{}'s request is no longer pending.", request.name))
                    .title("Join request closed")
                    .id1::<JoinRequestNotification>((
                        "join-request",
                        join_request_notification_id(&request.id),
                    )),
                cx,
            );
        }
        if self.lan_join_requests != requests {
            self.lan_join_requests = requests;
            cx.notify();
        }
    }

    fn start_host_join_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.host_join_task = Some(cx.spawn_in(window, async move |view, window| {
            Timer::after(Duration::from_millis(300)).await;
            let _ = view.update_in(window, |this, window, cx| {
                if this.lan_host.is_none() || !matches!(this.screen, Screen::Lan) {
                    return;
                }
                this.refresh_host_join_requests(window, cx);
                this.start_host_join_poll(window, cx);
            });
        }));
    }

    fn accept_join_request(
        &mut self,
        request: &JoinRequestId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = self
            .lan_join_requests
            .iter()
            .find(|pending| &pending.id == request)
            .map_or_else(|| "Player".to_owned(), |pending| pending.name.clone());
        let Some(host) = &self.lan_host else {
            return;
        };
        match host.accept_join_request(request) {
            Ok(lobby) => {
                self.lan_lobby = Some(lobby);
                self.lan_join_requests
                    .retain(|pending| &pending.id != request);
                self.last_error = None;
                window.push_notification(
                    Notification::success(format!("{name} can now join the table."))
                        .title("Player accepted")
                        .id1::<JoinRequestNotification>((
                            "join-request",
                            join_request_notification_id(request),
                        )),
                    cx,
                );
                self.refresh_lan_advertisement();
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                window.push_notification(
                    Notification::error(error.to_string()).title("Could not accept player"),
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn reject_join_request(
        &mut self,
        request: &JoinRequestId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = self
            .lan_join_requests
            .iter()
            .find(|pending| &pending.id == request)
            .map_or_else(|| "Player".to_owned(), |pending| pending.name.clone());
        let Some(host) = &self.lan_host else {
            return;
        };
        match host.reject_join_request(request) {
            Ok(lobby) => {
                self.lan_lobby = Some(lobby);
                self.lan_join_requests
                    .retain(|pending| &pending.id != request);
                self.last_error = None;
                window.push_notification(
                    Notification::info(format!("{name}'s request was declined."))
                        .title("Request declined")
                        .id1::<JoinRequestNotification>((
                            "join-request",
                            join_request_notification_id(request),
                        )),
                    cx,
                );
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                window.push_notification(
                    Notification::error(error.to_string()).title("Could not decline request"),
                    cx,
                );
            }
        }
        cx.notify();
    }

    fn host_lan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.lan_host.is_some() {
            self.screen = Screen::Lan;
            self.start_host_join_poll(window, cx);
            cx.notify();
            return;
        }
        let host_state = match self.lan_host_state(cx) {
            Ok(state) => state,
            Err(error) => {
                self.last_error = Some(error);
                cx.notify();
                return;
            }
        };
        match "0.0.0.0:0"
            .parse()
            .map_err(|error: std::net::AddrParseError| error.to_string())
            .and_then(|address| {
                LanHost::bind(address, host_state).map_err(|error| error.to_string())
            }) {
            Ok(host) => {
                let code = host
                    .room_code()
                    .map_or_else(|_| String::new(), |code| code.as_str().to_owned());
                let credentials = host.host_credentials();
                let lobby = host.lobby().ok();
                let connect_address =
                    std::net::SocketAddr::from(([127, 0, 0, 1], host.address().port()));
                let host_name = self.local_player_name(cx);
                let room_name = format!(
                    "{}'s game",
                    if host_name.is_empty() {
                        "Host"
                    } else {
                        &host_name
                    }
                );
                self.advertise_lan_host(&room_name, host.address().port(), 1);
                self.lan_host = Some(host);
                self.lan_endpoint = Some(connect_address);
                self.lan_share_endpoint = local_lan_addresses(connect_address.port())
                    .into_iter()
                    .next()
                    .or(Some(connect_address));
                self.lan_room = Some(code.clone());
                self.lan_lobby = lobby;
                self.screen = Screen::Lan;
                self.lan_connection = LanConnectionState::Connecting;
                self.lan_pending = credentials.is_ok_and(|(_, token)| {
                    self.lan_token = Some(token.clone());
                    self.lan_worker
                        .request(LanWorkerRequest::Reconnect {
                            address: connect_address,
                            room: Some(code),
                            token,
                        })
                        .is_ok()
                });
                self.start_lan_poll(cx);
                self.start_host_join_poll(window, cx);
            }
            Err(error) => self.last_error = Some(error),
        }
        cx.notify();
    }

    fn join_lan(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let address = self.setup.lan_address.read(cx).value().to_string();
        let room = self
            .setup
            .lan_code
            .read(cx)
            .value()
            .trim()
            .to_ascii_uppercase();
        let name = self.local_player_name(cx);
        if name.is_empty() {
            self.last_error = Some("Enter your name in the first player field.".to_owned());
            cx.notify();
            return;
        }
        if room.is_empty() {
            self.last_error = Some("Enter the host's room code.".to_owned());
            cx.notify();
            return;
        }
        match address.parse() {
            Ok(address) => {
                self.connect_to_lan(address, Some(room), name, window, cx);
            }
            Err(error) => {
                self.last_error = Some(format!("Invalid host address: {error}"));
                cx.notify();
            }
        }
    }

    fn join_nearby(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.nearby_games.get(index) else {
            return;
        };
        let name = self.local_player_name(cx);
        if name.is_empty() {
            self.last_error = Some("Enter your name before requesting to join.".to_owned());
            cx.notify();
            return;
        }
        self.connect_to_lan(game.address, None, name, window, cx);
    }

    fn connect_to_lan(
        &mut self,
        address: std::net::SocketAddr,
        room: Option<String>,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.lan_endpoint = Some(address);
        self.lan_room.clone_from(&room);
        self.lan_connection = LanConnectionState::AwaitingApproval;
        let request = JoinRequestId::generate();
        self.lan_join_request = Some(request.clone());
        self.screen = Screen::Lan;
        self.lan_pending = self
            .lan_worker
            .request(LanWorkerRequest::RequestJoin {
                address,
                request,
                room,
                name,
            })
            .is_ok();
        self.last_error = None;
        window.push_notification(
            Notification::info("The host will be asked to approve your player.")
                .title("Join request sent"),
            cx,
        );
        self.start_lan_poll(cx);
        cx.notify();
    }

    fn start_lan_poll(&mut self, cx: &mut Context<Self>) {
        self.lan_task = Some(cx.spawn(async move |view, cx| {
            loop {
                Timer::after(Duration::from_millis(20)).await;
                let done = cx
                    .update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return true;
                        };
                        view.update(cx, |this, cx| {
                            let Some(event) = this.lan_worker.try_event() else {
                                return false;
                            };
                            this.handle_lan_event(event, cx);
                            true
                        })
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        }));
    }

    fn mark_lan_connected(&mut self, round_trip: Duration) {
        self.lan_connection = LanConnectionState::Connected;
        self.lan_latency_ms = Some(round_trip.as_millis());
        self.lan_last_seen = Some(Instant::now());
    }

    #[allow(
        clippy::too_many_lines,
        reason = "all protocol response variants update the shared LAN session state here"
    )]
    fn handle_lan_event(&mut self, event: LanWorkerEvent, cx: &mut Context<Self>) {
        self.lan_pending = false;
        match event {
            LanWorkerEvent::Error(error) => {
                self.last_error = Some(error);
                self.lan_connection = if self.lan_join_request.is_some() {
                    LanConnectionState::AwaitingApproval
                } else if self.lan_token.is_some() {
                    LanConnectionState::Reconnecting
                } else {
                    LanConnectionState::Disconnected
                };
                cx.notify();
                if self.lan_join_request.is_some() {
                    self.schedule_join_status(cx);
                } else {
                    self.schedule_lan_reconnect(cx);
                }
            }
            LanWorkerEvent::Message {
                message,
                round_trip,
            } => {
                self.mark_lan_connected(round_trip);
                match message {
                    HostMessage::JoinPending { request, lobby, .. } => {
                        self.lan_join_request = Some(request.id);
                        self.lan_lobby = Some(lobby);
                        self.lan_connection = LanConnectionState::AwaitingApproval;
                        self.screen = Screen::Lan;
                        self.last_error = None;
                        self.schedule_join_status(cx);
                        cx.notify();
                    }
                    HostMessage::Welcome {
                        player,
                        token,
                        state,
                        lobby,
                        ..
                    } => match state.validated() {
                        Ok(state) => {
                            let playing = matches!(lobby.phase, LobbyPhase::Playing);
                            self.lan_player = Some(player);
                            self.lan_token = Some(token);
                            self.lan_join_request = None;
                            self.lan_lobby = Some(lobby);
                            self.session.restore(state);
                            self.session.set_undo_policy(UndoPolicy::Competitive);
                            self.screen = if playing { Screen::Game } else { Screen::Lan };
                            self.last_error = None;
                            self.refresh(cx);
                            self.schedule_lan_sync(cx);
                        }
                        Err(error) => self.last_error = Some(error.to_string()),
                    },
                    HostMessage::Applied {
                        events,
                        state,
                        lobby,
                        ..
                    } => match state.validated() {
                        Ok(state) => {
                            let previous = self
                                .lan_previous
                                .take()
                                .unwrap_or_else(|| state.current().player.id);
                            self.lan_lobby = Some(lobby);
                            self.session.restore(state);
                            self.last_error = None;
                            self.present_events(previous, &events, cx);
                        }
                        Err(error) => {
                            self.last_error = Some(error.to_string());
                            cx.notify();
                        }
                    },
                    HostMessage::Snapshot { state, lobby, .. } => match state.validated() {
                        Ok(state) => {
                            let should_enter = matches!(lobby.phase, LobbyPhase::Playing)
                                && matches!(self.screen, Screen::Lan);
                            self.lan_lobby = Some(lobby);
                            if state.revision() != self.session.state().revision()
                                || state != *self.session.state()
                            {
                                self.session.restore(state);
                                self.refresh(cx);
                            }
                            if should_enter {
                                self.screen = Screen::Game;
                                cx.notify();
                            }
                            self.schedule_lan_sync(cx);
                            self.drive_bot(cx);
                        }
                        Err(error) => self.last_error = Some(error.to_string()),
                    },
                    HostMessage::UpToDate { .. } => {
                        self.last_error = None;
                        self.schedule_lan_sync(cx);
                        self.drive_bot(cx);
                    }
                    HostMessage::Rejected {
                        reason,
                        state,
                        lobby,
                        ..
                    } => {
                        if let Some(state) = state.and_then(|state| state.validated().ok()) {
                            self.session.restore(state);
                            self.refresh(cx);
                        }
                        if let Some(lobby) = lobby {
                            self.lan_lobby = Some(lobby);
                        }
                        self.last_error = Some(reason);
                        if self.lan_token.is_some() {
                            self.schedule_lan_sync(cx);
                        } else {
                            self.lan_join_request = None;
                            self.lan_connection = LanConnectionState::Disconnected;
                        }
                    }
                }
                self.refresh_lan_advertisement();
            }
        }
    }

    fn send_lan_action(&mut self, action: ClientAction, cx: &mut Context<Self>) -> bool {
        let Some(token) = self.lan_token.clone() else {
            return false;
        };
        let current = &self.session.state().current().player;
        let host_controls_computer =
            self.lan_host.is_some() && matches!(current.controller, Controller::Bot);
        if self.lan_pending || (self.lan_player != Some(current.id) && !host_controls_computer) {
            return true;
        }
        self.lan_previous = Some(self.session.state().current().player.id);
        self.lan_pending = self
            .lan_worker
            .request(LanWorkerRequest::Action {
                token,
                expected_revision: self.session.state().revision(),
                action,
            })
            .is_ok();
        self.start_lan_poll(cx);
        cx.notify();
        true
    }

    fn schedule_lan_sync(&mut self, cx: &mut Context<Self>) {
        let Some(token) = self.lan_token.clone() else {
            return;
        };
        let known_state_revision = self.session.state().revision();
        let known_lobby_revision = self
            .lan_lobby
            .as_ref()
            .map_or(u64::MAX, |lobby| lobby.revision);
        if self.lan_pending || !matches!(self.screen, Screen::Game | Screen::Lan) {
            return;
        }
        self.lan_task = Some(cx.spawn(async move |view, cx| {
            Timer::after(Duration::from_millis(450)).await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    if this
                        .lan_worker
                        .request(LanWorkerRequest::Sync {
                            token,
                            known_state_revision,
                            known_lobby_revision,
                        })
                        .is_ok()
                    {
                        this.lan_pending = true;
                        if !matches!(this.lan_connection, LanConnectionState::Connected) {
                            this.lan_connection = LanConnectionState::Reconnecting;
                        }
                        this.start_lan_poll(cx);
                    }
                });
            });
        }));
    }

    fn schedule_join_status(&mut self, cx: &mut Context<Self>) {
        let (Some(request), Some(address)) = (self.lan_join_request.clone(), self.lan_endpoint)
        else {
            return;
        };
        if self.lan_pending || !matches!(self.screen, Screen::Lan) {
            return;
        }
        self.lan_task = Some(cx.spawn(async move |view, cx| {
            Timer::after(Duration::from_millis(450)).await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    if this
                        .lan_worker
                        .request(LanWorkerRequest::JoinStatus { address, request })
                        .is_ok()
                    {
                        this.lan_pending = true;
                        this.lan_connection = LanConnectionState::AwaitingApproval;
                        this.start_lan_poll(cx);
                    }
                });
            });
        }));
    }

    fn schedule_lan_reconnect(&mut self, cx: &mut Context<Self>) {
        let (Some(token), Some(address)) = (self.lan_token.clone(), self.lan_endpoint) else {
            return;
        };
        let room = self.lan_room.clone();
        self.lan_task = Some(cx.spawn(async move |view, cx| {
            Timer::after(Duration::from_millis(750)).await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    if this
                        .lan_worker
                        .request(LanWorkerRequest::Reconnect {
                            address,
                            room,
                            token,
                        })
                        .is_ok()
                    {
                        this.lan_pending = true;
                        this.lan_connection = LanConnectionState::Reconnecting;
                        this.start_lan_poll(cx);
                    }
                });
            });
        }));
    }

    fn enter_lan_match(&mut self, cx: &mut Context<Self>) {
        if self.lan_token.is_none() {
            self.last_error = Some("The host seat is still connecting.".to_owned());
            cx.notify();
            return;
        }
        if let Some(host) = &self.lan_host {
            if let Err(error) = host.start_match() {
                self.last_error = Some(error.to_string());
                cx.notify();
                return;
            }
        } else {
            self.last_error = Some("Only the host can start the match.".to_owned());
            cx.notify();
            return;
        }
        if let Some(discovery) = &mut self.discovery {
            discovery.stop_advertising();
        }
        if let Some(lobby) = &mut self.lan_lobby {
            lobby.phase = LobbyPhase::Playing;
        }
        self.screen = Screen::Game;
        self.last_error = None;
        self.refresh(cx);
        self.schedule_lan_sync(cx);
        self.drive_bot(cx);
    }

    fn rematch(&mut self, cx: &mut Context<Self>) {
        let players = self
            .session
            .state()
            .players()
            .iter()
            .map(|state| state.player.clone())
            .collect();
        match GameState::new(players, self.session.state().rules()) {
            Ok(state) => {
                self.bot_task = None;
                self.animation_task = None;
                self.bot_worker = BotWorker::new();
                self.session.restore(state);
                self.screen = Screen::Game;
                self.bot_pending = false;
                self.animating = false;
                self.visual_positions.clear();
                self.animated_dice = None;
                self.effect_message = None;
                self.result_recorded_revision = None;
                self.last_error = None;
                let _ = self.session.save();
                self.refresh(cx);
                self.drive_bot(cx);
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn reveal_turn(&mut self, cx: &mut Context<Self>) {
        self.screen = Screen::Game;
        cx.notify();
    }

    fn roll(&mut self, cx: &mut Context<Self>) {
        if self.bot_pending || self.animating || !self.view_model.can_roll {
            return;
        }
        if self.send_lan_action(ClientAction::Roll, cx) {
            return;
        }
        let previous = self.session.state().current().player.id;
        match self.session.roll() {
            Ok(events) => {
                self.last_error = None;
                self.present_events(previous, &events, cx);
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                self.refresh(cx);
            }
        }
    }

    fn move_token(&mut self, token: TokenId, cx: &mut Context<Self>) {
        if self.bot_pending || self.animating {
            return;
        }
        if self.send_lan_action(ClientAction::Move(token), cx) {
            return;
        }
        let previous = self.session.state().current().player.id;
        match self.session.move_token(token) {
            Ok(events) => {
                self.last_error = None;
                self.present_events(previous, &events, cx);
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                self.refresh(cx);
            }
        }
    }

    fn present_events(&mut self, previous: PlayerId, events: &[GameEvent], cx: &mut Context<Self>) {
        self.play_event_sounds(events);
        self.refresh(cx);
        let frames = animation_frames(events);
        if self.settings.reduced_motion || frames.is_empty() {
            self.complete_command(previous, cx);
            return;
        }
        for event in events {
            if let GameEvent::TokenMoved {
                player,
                token,
                from,
                ..
            } = event
            {
                self.visual_positions
                    .insert((player.index(), token.index()), *from);
            }
        }
        self.animating = true;
        self.animation_task = Some(cx.spawn(async move |view, cx| {
            for frame in frames {
                Timer::after(Duration::from_millis(frame.delay_ms)).await;
                let should_continue = cx
                    .update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return false;
                        };
                        view.update(cx, |this, cx| {
                            this.apply_animation_frame(frame);
                            cx.notify();
                        });
                        true
                    })
                    .unwrap_or(false);
                if !should_continue {
                    return;
                }
            }
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this.animating = false;
                    this.visual_positions.clear();
                    this.animated_dice = None;
                    this.effect_message = None;
                    this.complete_command(previous, cx);
                });
            });
        }));
    }

    fn apply_animation_frame(&mut self, frame: AnimationFrame) {
        match frame.cue {
            AnimationCue::Dice(value) => self.animated_dice = Some(value),
            AnimationCue::TokenAt {
                player,
                token,
                position,
            } => {
                self.visual_positions
                    .insert((player.index(), token.index()), position);
            }
            AnimationCue::Capture { player, token } => {
                self.visual_positions
                    .insert((player.index(), token.index()), TokenPosition::Yard);
                self.effect_message = Some("CAPTURE!".to_owned());
            }
            AnimationCue::Ranked { player, place } => {
                let name = &self.session.state().players()[player.index()].player.name;
                self.effect_message = Some(format!("{name} takes place #{place}!"));
            }
            AnimationCue::Turn(player) => {
                let name = &self.session.state().players()[player.index()].player.name;
                self.effect_message = Some(format!("{name}'s turn"));
            }
            AnimationCue::Victory(player) => {
                let name = &self.session.state().players()[player.index()].player.name;
                self.effect_message = Some(format!("{name} wins!"));
            }
        }
    }

    fn play_event_sounds(&self, events: &[GameEvent]) {
        for event in events {
            let cue = match event {
                GameEvent::DiceRolled { .. } => Some(SoundCue::Dice),
                GameEvent::TokenMoved { to, .. } => {
                    Some(if matches!(to, TokenPosition::Finished) {
                        SoundCue::Home
                    } else {
                        SoundCue::Move
                    })
                }
                GameEvent::TokenCaptured { .. } => Some(SoundCue::Capture),
                GameEvent::TurnChanged { .. } => Some(SoundCue::Turn),
                GameEvent::GameFinished { .. } => Some(SoundCue::Victory),
                GameEvent::PlayerRanked { .. } => None,
            };
            if let Some(cue) = cue {
                self.audio.play(cue);
            }
        }
    }

    fn complete_command(&mut self, previous: PlayerId, cx: &mut Context<Self>) {
        self.refresh(cx);
        if matches!(self.session.state().status(), GameStatus::Finished) {
            self.finalize_match();
            self.screen = Screen::Results;
            cx.notify();
            return;
        }
        if self.lan_token.is_some() {
            self.schedule_lan_sync(cx);
            self.drive_bot(cx);
            return;
        }
        if should_show_hotseat(self.session.state(), previous) {
            self.screen = Screen::HotSeat;
            cx.notify();
        } else {
            self.drive_bot(cx);
        }
    }

    fn finalize_match(&mut self) {
        let revision = self.session.state().revision();
        if self.result_recorded_revision == Some(revision) {
            return;
        }
        if self.lan_token.is_some() {
            self.result_recorded_revision = Some(revision);
            return;
        }
        if let Err(error) = self.replay_repository.save_replay(self.session.replay()) {
            self.last_error = Some(error);
            return;
        }
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        if let Err(error) = self.profiles.record_match(self.session.replay(), timestamp) {
            self.last_error = Some(error.to_string());
            return;
        }
        if let Err(error) = self.profile_repository.save_profiles(&self.profiles) {
            self.last_error = Some(error);
            return;
        }
        if let Some(fixture_id) = self.tournament_match
            && let Some(winner) = self.session.state().rankings().first()
            && let Some(tournament) = &mut self.tournament
            && let Some(fixture) = tournament
                .fixtures
                .iter()
                .find(|fixture| fixture.id == fixture_id)
                .cloned()
        {
            let participant = if winner.index() == 0 {
                fixture.home
            } else {
                fixture.away
            };
            if let Err(error) = tournament.report_winner(fixture_id, participant) {
                self.last_error = Some(error.to_string());
                return;
            }
        }
        self.result_recorded_revision = Some(revision);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.view_model = GameViewModel::from(self.session.state());
        cx.notify();
    }

    fn drive_bot(&mut self, cx: &mut Context<Self>) {
        if self.bot_pending
            || self.animating
            || (self.lan_token.is_some() && self.lan_host.is_none())
            || !matches!(self.screen, Screen::Game)
            || !matches!(self.session.state().status(), GameStatus::Playing)
            || !matches!(
                self.session.state().current().player.controller,
                Controller::Bot
            )
        {
            return;
        }
        self.bot_pending = true;
        match self.session.state().phase() {
            TurnPhase::AwaitingRoll => {
                self.bot_task = Some(cx.spawn(async move |view, cx| {
                    Timer::after(Duration::from_millis(360)).await;
                    let _ = cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            let previous = this.session.state().current().player.id;
                            this.bot_pending = false;
                            if this.lan_token.is_some() {
                                let _ = this.send_lan_action(ClientAction::Roll, cx);
                                return;
                            }
                            match this.session.roll() {
                                Ok(events) => {
                                    this.last_error = None;
                                    this.present_events(previous, &events, cx);
                                }
                                Err(error) => {
                                    this.last_error = Some(error.to_string());
                                    this.complete_command(previous, cx);
                                }
                            }
                        });
                    });
                }));
            }
            TurnPhase::AwaitingMove { .. } => {
                let request = BotRequest::new(
                    self.session.state().clone(),
                    self.session.state().current().player.bot_difficulty,
                )
                .with_thinking_time_ms(self.setup.ai_thinking_time_ms);
                if self.bot_worker.request(request).is_err() {
                    self.bot_pending = false;
                    return;
                }
                self.bot_task = Some(cx.spawn(async move |view, cx| {
                    loop {
                        Timer::after(Duration::from_millis(20)).await;
                        let done = cx
                            .update(|cx| {
                                let Some(view) = view.upgrade() else {
                                    return true;
                                };
                                view.update(cx, |this, cx| {
                                    let Some(decision) = this.bot_worker.try_decision() else {
                                        return false;
                                    };
                                    let previous = this.session.state().current().player.id;
                                    if decision.revision == this.session.state().revision()
                                        && decision.player == previous
                                        && let Some(token) = decision.token
                                    {
                                        if this.lan_token.is_some() {
                                            this.bot_pending = false;
                                            let _ =
                                                this.send_lan_action(ClientAction::Move(token), cx);
                                            return true;
                                        }
                                        match this.session.move_token(token) {
                                            Ok(events) => {
                                                this.last_error = None;
                                                this.bot_pending = false;
                                                this.present_events(previous, &events, cx);
                                                return true;
                                            }
                                            Err(error) => {
                                                this.last_error = Some(error.to_string());
                                            }
                                        }
                                    }
                                    this.bot_pending = false;
                                    this.complete_command(previous, cx);
                                    true
                                })
                            })
                            .unwrap_or(true);
                        if done {
                            break;
                        }
                    }
                }));
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the declarative setup view tree is clearest as one contiguous layout"
    )]
    fn render_setup(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.settings.theme.palette();
        let compact = f32::from(window.viewport_size().width) < 860.0;
        let content_width = (f32::from(window.viewport_size().width)
            - if compact { 32.0 } else { 64.0 })
        .clamp(320.0, 1040.0);
        let seat_width = if compact {
            content_width
        } else {
            (content_width - 12.0) / 2.0
        };
        let mut seats = div().flex().flex_wrap().gap_3();
        for index in 0..self.setup.player_count {
            let controller = self.setup.controllers[index];
            let difficulty = self.setup.difficulties[index];
            let color_name = self.setup.colors[index].name();
            seats = seats.child(
                div()
                    .w(px(seat_width))
                    .p_4()
                    .rounded_xl()
                    .border_1()
                    .border_color(if index == 0 {
                        color(self.setup.colors[index])
                    } else {
                        rgb(palette.line)
                    })
                    .bg(rgb(palette.surface))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .size_8()
                            .rounded_full()
                            .bg(color(self.setup.colors[index]))
                            .flex()
                            .items_center()
                            .justify_center()
                            .font_bold()
                            .text_color(rgb(0x00ff_ffff))
                            .child((index + 1).to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.setup.names[index]).small()),
                    )
                    .child(
                        Button::new(("color", index))
                            .ghost()
                            .label(color_name)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.cycle_color(index, cx);
                            })),
                    )
                    .child(
                        Button::new(("controller", index))
                            .ghost()
                            .label(match controller {
                                Controller::Human => "Human",
                                Controller::Bot => "Bot",
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_controller(index, cx);
                            })),
                    )
                    .when(matches!(controller, Controller::Bot), |element| {
                        element.child(
                            Button::new(("difficulty", index))
                                .ghost()
                                .label(difficulty.name())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.cycle_difficulty(index, cx);
                                })),
                        )
                    }),
            );
        }
        let mut nearby = div().flex().flex_col().gap_2();
        if self.nearby_games.is_empty() {
            nearby = nearby.child(
                div()
                    .p_3()
                    .rounded_lg()
                    .bg(rgb(palette.raised))
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(if self.discovery.is_some() {
                        "Scanning your local network for joinable games…"
                    } else {
                        "Automatic discovery is unavailable. Use Direct address below."
                    }),
            );
        } else {
            for (index, game) in self.nearby_games.iter().enumerate() {
                nearby = nearby.child(
                    div()
                        .p_3()
                        .rounded_xl()
                        .border_1()
                        .border_color(rgb(palette.line))
                        .bg(rgb(palette.raised))
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(div().size_3().rounded_full().bg(rgb(0x0044_c47a)))
                        .child(
                            div()
                                .flex_1()
                                .child(div().font_semibold().child(game.name.clone()))
                                .child(div().text_xs().text_color(rgb(palette.muted)).child(
                                    format!(
                                        "{} • {} human{} • {}/{} seats",
                                        game.preset,
                                        game.humans,
                                        if game.humans == 1 { "" } else { "s" },
                                        game.humans,
                                        game.capacity
                                    ),
                                )),
                        )
                        .child(
                            Button::new(("join-nearby", index))
                                .primary()
                                .label("Request to join")
                                .disabled(self.lan_pending)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.join_nearby(index, window, cx);
                                })),
                        ),
                );
            }
        }

        div()
            .size_full()
            .overflow_y_scrollbar()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.foreground))
            .py_6()
            .px_4()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(content_width))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_start()
                            .gap_4()
                            .child(
                                div()
                                    .flex_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_bold()
                                            .text_color(rgb(palette.accent))
                                            .child("LUDO ROYALE"),
                                    )
                                    .child(
                                        div()
                                            .mt_1()
                                            .text_3xl()
                                            .font_bold()
                                            .child("Choose your table"),
                                    )
                                    .child(
                                        div()
                                            .mt_1()
                                            .text_sm()
                                            .text_color(rgb(palette.muted))
                                            .child("A timeless race, built for the room you’re in."),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        Button::new("profiles")
                                            .ghost()
                                            .label("Stats")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.open_profiles(cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("load-replay")
                                            .ghost()
                                            .label("Replays")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.open_replay(Screen::Setup, cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("setup-help")
                                            .ghost()
                                            .label("Help")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.open_help(cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("setup-settings")
                                            .ghost()
                                            .label("Settings")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.open_settings(cx);
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .p_1()
                            .rounded_xl()
                            .bg(rgb(palette.surface))
                            .border_1()
                            .border_color(rgb(palette.line))
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .child(
                                Button::new("setup-local")
                                    .when(
                                        matches!(self.setup_section, SetupSection::Local),
                                        ButtonVariants::primary,
                                    )
                                    .when(
                                        !matches!(self.setup_section, SetupSection::Local),
                                        ButtonVariants::ghost,
                                    )
                                    .rounded(ButtonRounded::Large)
                                    .label("Local game")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_setup_section(SetupSection::Local, cx);
                                    })),
                            )
                            .child(
                                Button::new("setup-lan")
                                    .when(
                                        matches!(self.setup_section, SetupSection::Lan),
                                        ButtonVariants::primary,
                                    )
                                    .when(
                                        !matches!(self.setup_section, SetupSection::Lan),
                                        ButtonVariants::ghost,
                                    )
                                    .rounded(ButtonRounded::Large)
                                    .label("LAN party")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_setup_section(SetupSection::Lan, cx);
                                    })),
                            )
                            .child(
                                Button::new("setup-tournament")
                                    .when(
                                        matches!(self.setup_section, SetupSection::Tournament),
                                        ButtonVariants::primary,
                                    )
                                    .when(
                                        !matches!(self.setup_section, SetupSection::Tournament),
                                        ButtonVariants::ghost,
                                    )
                                    .rounded(ButtonRounded::Large)
                                    .label("Tournament")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_setup_section(SetupSection::Tournament, cx);
                                    })),
                            ),
                    )
                    .when(
                        matches!(
                            self.setup_section,
                            SetupSection::Local | SetupSection::Tournament
                        ),
                        |element| {
                            element.child(
                                div()
                            .p_4()
                            .rounded_2xl()
                            .bg(rgb(palette.surface))
                            .border_1()
                            .border_color(rgb(palette.line))
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_3()
                            .child("Players")
                            .child(
                                Button::new("players-minus")
                                    .label("−")
                                    .disabled(self.setup.player_count == 2)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.change_player_count(-1, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .w_8()
                                    .text_center()
                                    .font_bold()
                                    .child(self.setup.player_count.to_string()),
                            )
                            .child(
                                Button::new("players-plus")
                                    .label("+")
                                    .disabled(self.setup.player_count == 4)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.change_player_count(1, cx);
                                    })),
                            )
                            .child(div().ml_6().child("Rules"))
                            .child(
                                Button::new("preset-classic")
                                    .when(
                                        !self.setup.use_custom_rules
                                            && matches!(self.setup.preset, RulePreset::Classic),
                                        ButtonVariants::primary,
                                    )
                                    .label("Classic")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.select_preset(RulePreset::Classic, cx);
                                    })),
                            )
                            .child(
                                Button::new("preset-quick")
                                    .when(
                                        !self.setup.use_custom_rules
                                            && matches!(self.setup.preset, RulePreset::Quick),
                                        ButtonVariants::primary,
                                    )
                                    .label("Quick")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.select_preset(RulePreset::Quick, cx);
                                    })),
                            )
                            .child(
                                Button::new("preset-tournament")
                                    .when(
                                        !self.setup.use_custom_rules
                                            && matches!(self.setup.preset, RulePreset::Tournament),
                                        ButtonVariants::primary,
                                    )
                                    .label("Tournament")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.select_preset(RulePreset::Tournament, cx);
                                    })),
                            )
                            .child(
                                Button::new("custom-rules")
                                    .ghost()
                                    .label(if self.setup.use_custom_rules {
                                        "Custom ✓"
                                    } else {
                                        "Advanced"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_custom_rules(cx);
                                    })),
                            )
                            .child(
                                Button::new("team-mode")
                                    .label(if self.setup.team_mode {
                                        "2v2: On"
                                    } else {
                                        "2v2: Off"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_team_mode(cx);
                                    })),
                            )
                            .child(
                                Button::new("ai-time")
                                    .label(format!("AI {}ms", self.setup.ai_thinking_time_ms))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cycle_ai_time(cx);
                                    })),
                            ),
                            )
                        },
                    )
                    .when(
                        matches!(
                            self.setup_section,
                            SetupSection::Local | SetupSection::Tournament
                        ),
                        |element| element.child(seats),
                    )
                    .when_some(self.last_error.clone(), |element, error| {
                        element.child(
                            div()
                                .p_3()
                                .rounded_lg()
                                .bg(rgb(0x003c_2024))
                                .text_color(rgb(0x00ff_b4b8))
                                .child(error),
                        )
                    })
                    .when(matches!(self.setup_section, SetupSection::Local), |element| {
                        element.child(
                            div()
                            .flex()
                            .flex_wrap()
                            .gap_3()
                            .child(
                                Button::new("start")
                                    .primary()
                                    .large()
                                    .label("Start match")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.start_game(cx);
                                    })),
                            )
                            .when(self.resume_available, |element| {
                                element.child(
                                    Button::new("resume")
                                        .large()
                                        .label("Resume saved match")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                        this.resume_game(cx);
                                    })),
                                )
                            })
                        )
                    })
                    .when(
                        matches!(self.setup_section, SetupSection::Tournament),
                        |element| {
                            element.child(
                                div()
                            .p_5()
                            .rounded_2xl()
                            .bg(rgb(palette.surface))
                            .border_1()
                            .border_color(rgb(palette.line))
                            .flex()
                            .when(compact, Styled::flex_col)
                            .items_center()
                            .gap_4()
                            .child(
                                div()
                                    .flex_1()
                                    .child(div().text_xl().font_bold().child("Competition hub"))
                                    .child(
                                        div()
                                            .mt_1()
                                            .text_sm()
                                            .text_color(rgb(palette.muted))
                                            .child("Create a bracket from the players configured above. Results and standings update after every match."),
                                    ),
                            )
                            .child(
                                Button::new("tournament-format")
                                    .large()
                                    .label(match self.tournament_format {
                                        TournamentFormat::RoundRobin => "Round robin",
                                        TournamentFormat::SingleElimination => "Elimination",
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_tournament_format(cx);
                                    })),
                            )
                            .child(
                                Button::new("tournament-center")
                                    .primary()
                                    .large()
                                    .label("Create tournament")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.create_tournament(cx);
                                    })),
                            ),
                            )
                        },
                    )
                    .when(matches!(self.setup_section, SetupSection::Lan), |element| {
                        element.child(
                            div()
                            .p_4()
                            .rounded_2xl()
                            .bg(rgb(palette.surface))
                            .border_1()
                            .border_color(rgb(palette.line))
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .font_bold()
                                    .text_color(rgb(palette.accent))
                                    .child("Join a nearby table"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(palette.muted))
                                    .child(
                                        "Enter your name and choose a discovered game. The host will approve or decline your request.",
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_3()
                                    .child("Your name")
                                    .child(
                                        div()
                                            .w(px(180.0))
                                            .child(Input::new(&self.setup.names[0]).small()),
                                    ),
                            )
                            .child(nearby),
                        )
                    })
                    .when(matches!(self.setup_section, SetupSection::Lan), |element| {
                        element.child(
                            div()
                            .p_3()
                            .rounded_xl()
                            .bg(rgb(palette.surface))
                            .border_1()
                            .border_color(rgb(palette.line))
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .w_full()
                                    .child(div().font_bold().child("Room-code fallback"))
                                    .child(
                                        div()
                                            .mt_1()
                                            .text_sm()
                                            .text_color(rgb(palette.muted))
                                            .child("Use this only when automatic discovery cannot find the host."),
                                    ),
                            )
                            .child("Room code")
                            .child(
                                div()
                                    .w(px(150.0))
                                    .child(Input::new(&self.setup.lan_code).small()),
                            )
                            .child("Direct address")
                            .child(
                                div()
                                    .w(px(210.0))
                                    .child(Input::new(&self.setup.lan_address).small()),
                            )
                            .child(
                                Button::new("lan-join")
                                    .primary()
                                    .label(if self.lan_pending {
                                        "Connecting…"
                                    } else {
                                        "Request access"
                                    })
                                    .disabled(self.lan_pending)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.join_lan(window, cx);
                                    })),
                            ),
                        )
                    })
                    .when(matches!(self.setup_section, SetupSection::Lan), |element| {
                        element.child(
                            div()
                                .p_5()
                                .rounded_2xl()
                                .bg(rgb(palette.surface))
                                .border_1()
                                .border_color(rgb(palette.line))
                                .flex()
                                .when(compact, Styled::flex_col)
                                .items_center()
                                .gap_4()
                                .child(
                                    div()
                                        .flex_1()
                                        .child(div().text_xl().font_bold().child("Host this table"))
                                        .child(
                                            div()
                                                .mt_1()
                                                .text_sm()
                                                .text_color(rgb(palette.muted))
                                                .child("Nearby players can request access automatically. You approve each player before they receive a seat; the room code remains a fallback."),
                                        ),
                                )
                                .child(
                                    Button::new("lan-host")
                                        .primary()
                                        .large()
                                        .label("Host private game")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.host_lan(window, cx);
                                        })),
                                ),
                        )
                    }),
            )
    }

    fn render_hotseat(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let player = self.session.state().current();
        let palette = self.settings.theme.palette();
        div()
            .size_full()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.foreground))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(panel_width(window, 440.0)))
                    .p_8()
                    .rounded_2xl()
                    .border_2()
                    .border_color(color(player.player.color))
                    .bg(rgb(palette.surface))
                    .shadow_2xl()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_5()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child("PASS THE DEVICE"),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .whitespace_nowrap()
                            .text_center()
                            .text_3xl()
                            .font_bold()
                            .text_color(color(player.player.color))
                            .child(format!("{}'s turn", ellipsize(&player.player.name, 28))),
                    )
                    .child("The board is hidden until the next player is ready.")
                    .child(
                        Button::new("reveal")
                            .primary()
                            .large()
                            .label("Reveal board")
                            .on_click(cx.listener(|this, _, _, cx| this.reveal_turn(cx))),
                    ),
            )
    }

    fn render_pause(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.settings.theme.palette();
        div()
            .size_full()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.foreground))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(panel_width(window, 460.0)))
                    .p_8()
                    .rounded_2xl()
                    .border_1()
                    .border_color(rgb(palette.line))
                    .bg(rgb(palette.surface))
                    .shadow_2xl()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_4()
                    .child(
                        div()
                            .text_xs()
                            .font_bold()
                            .text_color(rgb(palette.muted))
                            .child("MATCH PAUSED"),
                    )
                    .child(
                        div()
                            .text_3xl()
                            .font_bold()
                            .text_color(rgb(palette.accent))
                            .child("Take a breather"),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette.muted))
                            .child("Your match has been saved automatically."),
                    )
                    .child(
                        Button::new("pause-resume")
                            .primary()
                            .large()
                            .label("Resume match")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.resume_from_pause(cx);
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_3()
                            .child(Button::new("pause-rules").ghost().label("Rules").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.open_help(cx);
                                }),
                            ))
                            .child(
                                Button::new("pause-settings")
                                    .ghost()
                                    .label("Settings")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_settings(cx);
                                    })),
                            )
                            .child(
                                Button::new("pause-new")
                                    .danger()
                                    .label("New game")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.request_new_game(cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_3()
                            .child(
                                Button::new("pause-undo")
                                    .ghost()
                                    .label("Undo last action")
                                    .disabled(!self.session.can_undo())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.request_undo(cx);
                                    })),
                            )
                            .child(
                                Button::new("pause-save-replay")
                                    .ghost()
                                    .label("Save replay")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.save_replay(cx);
                                    })),
                            ),
                    ),
            )
    }

    fn render_confirmation(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.settings.theme.palette();
        div()
            .size_full()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.foreground))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(panel_width(window, 480.0)))
                    .p_8()
                    .rounded_2xl()
                    .border_1()
                    .border_color(rgb(palette.line))
                    .bg(rgb(palette.surface))
                    .shadow_2xl()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(div().text_2xl().font_bold().child("Start a new match?"))
                    .child(
                        div().text_color(rgb(palette.muted)).child(
                            "This clears the current autosave. This action cannot be undone.",
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_3()
                            .child(
                                Button::new("confirm-cancel")
                                    .ghost()
                                    .large()
                                    .label("Keep playing")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.resume_from_pause(cx);
                                    })),
                            )
                            .child(
                                Button::new("confirm-new")
                                    .danger()
                                    .large()
                                    .label("Clear & new game")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.confirm_new_game(cx);
                                    })),
                            ),
                    ),
            )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the rules reference is one declarative screen with a single data-driven row list"
    )]
    fn render_help(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.settings.theme.palette();
        let rules = if matches!(self.overlay_return, Screen::Setup) {
            self.setup.preset.rules()
        } else {
            self.session.state().rules()
        };
        let preset = if matches!(self.overlay_return, Screen::Setup) {
            self.setup.preset.name()
        } else {
            active_preset_name(self.session.state())
        };
        let rule_rows = [
            (
                "Enter the board",
                "Roll a 6 to move a token out of its yard.",
            ),
            (
                "Bonus turns",
                if rules.extra_turn_on_home {
                    "A 6, capture, or token reaching home grants another roll."
                } else {
                    "A 6 or capture grants another roll; reaching home does not."
                },
            ),
            (
                "Three sixes",
                if rules.three_sixes_forfeit {
                    "Three consecutive sixes forfeit the turn."
                } else {
                    "Consecutive sixes never trigger a forfeit."
                },
            ),
            (
                "Safe cells",
                match rules.safe_cells {
                    SafeCellRule::None => "No shared cells protect tokens.",
                    SafeCellRule::Starts => "Colored starting cells protect tokens.",
                    SafeCellRule::StartsAndStars => "Colored starts and ★ cells protect tokens.",
                },
            ),
            (
                "Blockades",
                if rules.blockades {
                    "Two allied tokens block opponents from passing."
                } else {
                    "Tokens never form impassable blockades."
                },
            ),
            (
                "Reach home",
                if rules.exact_home_roll {
                    "The final move requires the exact dice value."
                } else {
                    "An overshoot is allowed and completes the token."
                },
            ),
            (
                "Victory",
                match rules.win_condition {
                    WinCondition::FirstWinner => "The first player with all four tokens home wins.",
                    WinCondition::RankAll => "Play continues until every placement is known.",
                },
            ),
        ];
        let mut rows = div().flex().flex_col().gap_2();
        for (title, description) in rule_rows {
            rows = rows.child(
                div()
                    .p_3()
                    .rounded_xl()
                    .bg(rgb(palette.raised))
                    .border_1()
                    .border_color(rgb(palette.line))
                    .child(
                        div()
                            .font_semibold()
                            .text_color(rgb(palette.foreground))
                            .child(title),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child(description),
                    ),
            );
        }
        div()
            .size_full()
            .overflow_y_scrollbar()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.foreground))
            .p_8()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(panel_width(window, 680.0)))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .text_3xl()
                                    .font_bold()
                                    .text_color(rgb(palette.accent))
                                    .child("How to play"),
                            )
                            .child(
                                div()
                                    .px_3()
                                    .py_1()
                                    .rounded_full()
                                    .bg(rgb(palette.raised))
                                    .child(format!("{preset} rules")),
                            ),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette.muted))
                            .child("Roll, choose a glowing token, capture rivals, and bring all four tokens home."),
                    )
                    .child(rows)
                    .child(
                        div()
                            .p_3()
                            .rounded_xl()
                            .border_1()
                            .border_color(rgb(palette.accent))
                            .child("Tip: ★ marks a safe cell. Numbered tokens and the status panel show every available action."),
                    )
                    .child(
                        Button::new("help-back")
                            .primary()
                            .large()
                            .label("Got it")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_overlay(cx);
                            })),
                    ),
            )
    }

    fn render_settings(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.settings.theme.palette();
        div()
            .size_full()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.foreground))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(panel_width(window, 560.0)))
                    .p_8()
                    .rounded_2xl()
                    .border_1()
                    .border_color(rgb(palette.line))
                    .bg(rgb(palette.surface))
                    .shadow_2xl()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_3xl()
                            .font_bold()
                            .text_color(rgb(palette.accent))
                            .child("Display & accessibility"),
                    )
                    .child(setting_row(
                        "Theme",
                        "Changes the table and surrounding interface.",
                        Button::new("setting-theme")
                            .label(self.settings.theme.name())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cycle_theme(cx);
                            })),
                        palette,
                    ))
                    .child(setting_row(
                        "Sound effects",
                        "Dice, moves, captures, turns, and victory cues.",
                        Button::new("setting-sound")
                            .label(on_off(self.settings.sound))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_sound(cx);
                            })),
                        palette,
                    ))
                    .child(setting_row(
                        "Reduced motion",
                        "Shows final states immediately without timed movement.",
                        Button::new("setting-motion")
                            .label(on_off(self.settings.reduced_motion))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_reduced_motion(cx);
                            })),
                        palette,
                    ))
                    .child(setting_row(
                        "High contrast tokens",
                        "Adds dark outlines and color initials to every token.",
                        Button::new("setting-contrast")
                            .label(on_off(self.settings.high_contrast))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_high_contrast(cx);
                            })),
                        palette,
                    ))
                    .child(
                        Button::new("settings-back")
                            .primary()
                            .large()
                            .label("Done")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_overlay(cx);
                            })),
                    ),
            )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the standings and result actions form one declarative screen"
    )]
    fn render_results(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.settings.theme.palette();
        let rankings = self.session.state().rankings();
        let mut result_players = rankings.to_vec();
        result_players.extend(
            self.session
                .state()
                .players()
                .iter()
                .map(|state| state.player.id)
                .filter(|player| !rankings.contains(player)),
        );
        let mut standings = div().flex().flex_col().gap_2();
        for (index, player_id) in result_players.iter().enumerate() {
            let player = &self.session.state().players()[player_id.index()].player;
            let is_ranked = index < rankings.len();
            let finished = self.session.state().players()[player_id.index()]
                .tokens
                .iter()
                .filter(|token| matches!(token.position, TokenPosition::Finished))
                .count();
            standings = standings.child(
                div()
                    .p_4()
                    .rounded_xl()
                    .border_2()
                    .border_color(color(player.color))
                    .bg(rgb(palette.raised))
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_shrink_0()
                            .w_10()
                            .text_2xl()
                            .font_bold()
                            .text_color(rgb(palette.accent))
                            .child(if is_ranked {
                                format!("#{}", index + 1)
                            } else {
                                "—".to_owned()
                            }),
                    )
                    .child(
                        div()
                            .size_4()
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(color(player.color)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_nowrap()
                            .font_semibold()
                            .child(ellipsize(&player.name, 24)),
                    )
                    .child(div().flex_shrink_0().child(if index == 0 {
                        "CHAMPION".to_owned()
                    } else if is_ranked {
                        "FINISHED".to_owned()
                    } else {
                        format!("{finished}/4 home")
                    })),
            );
        }
        div()
            .size_full()
            .overflow_y_scrollbar()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.foreground))
            .p_8()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(panel_width(window, 620.0)))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_5()
                    .child(div().text_3xl().child("♛"))
                    .child(
                        div()
                            .text_3xl()
                            .font_bold()
                            .text_color(rgb(palette.accent))
                            .child("Match complete"),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette.muted))
                            .child("Final standings"),
                    )
                    .child(div().w_full().child(standings))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_3()
                            .when(
                                self.tournament_match.is_none() && self.lan_token.is_none(),
                                |element| {
                                    element.child(
                                        Button::new("results-rematch")
                                            .primary()
                                            .large()
                                            .label("Rematch")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.rematch(cx);
                                            })),
                                    )
                                },
                            )
                            .when(self.tournament_match.is_some(), |element| {
                                element.child(
                                    Button::new("results-tournament")
                                        .primary()
                                        .large()
                                        .label("Tournament standings")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.return_to_tournament(cx);
                                        })),
                                )
                            })
                            .child(
                                Button::new("results-new")
                                    .large()
                                    .label("New setup")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.confirm_new_game(cx);
                                    })),
                            )
                            .child(
                                Button::new("results-rules")
                                    .large()
                                    .label("Review rules")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_help(cx);
                                    })),
                            ),
                    )
                    .when(self.lan_token.is_none(), |element| {
                        element.child(
                            Button::new("results-replay")
                                .label("Watch this match")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.open_replay(Screen::Results, cx);
                                })),
                        )
                    }),
            )
    }

    fn render_undo_confirmation(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let palette = self.settings.theme.palette();
        div()
            .size_full()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.foreground))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(panel_width(window, 460.0)))
                    .p_8()
                    .rounded_2xl()
                    .bg(rgb(palette.surface))
                    .border_1()
                    .border_color(rgb(palette.line))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(div().text_2xl().font_bold().child("Undo last action?"))
                    .child(div().text_color(rgb(palette.muted)).child(
                        "The match returns to the exact snapshot before the latest command.",
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_3()
                            .child(Button::new("undo-cancel").label("Cancel").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.screen = Screen::Pause;
                                    cx.notify();
                                }),
                            ))
                            .child(
                                Button::new("undo-confirm")
                                    .primary()
                                    .label("Undo")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.confirm_undo(cx);
                                    })),
                            ),
                    ),
            )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the custom-rule switches form one data-driven editor screen"
    )]
    fn render_custom_rules(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.settings.theme.palette();
        let rules = self.setup.custom_rules;
        let name_width = (panel_width(window, 720.0) - 32.0).clamp(180.0, 260.0);
        let entries = [
            ("Bonus turn on six", on_off(rules.extra_turn_on_six)),
            ("Bonus turn on capture", on_off(rules.extra_turn_on_capture)),
            ("Bonus turn on home", on_off(rules.extra_turn_on_home)),
            ("Three sixes forfeit", on_off(rules.three_sixes_forfeit)),
            ("Blockades", on_off(rules.blockades)),
            ("Exact home roll", on_off(rules.exact_home_roll)),
            (
                "Safe cells",
                match rules.safe_cells {
                    SafeCellRule::None => "None",
                    SafeCellRule::Starts => "Starts",
                    SafeCellRule::StartsAndStars => "Starts + stars",
                },
            ),
            (
                "Win condition",
                match rules.win_condition {
                    WinCondition::FirstWinner => "First winner",
                    WinCondition::RankAll => "Rank everyone",
                },
            ),
        ];
        let mut rows = div().flex().flex_col().gap_2();
        for (index, (name, value)) in entries.into_iter().enumerate() {
            rows = rows.child(
                div()
                    .p_3()
                    .rounded_xl()
                    .bg(rgb(palette.raised))
                    .border_1()
                    .border_color(rgb(palette.line))
                    .flex()
                    .items_center()
                    .child(div().flex_1().font_semibold().child(name))
                    .child(Button::new(("custom-toggle", index)).label(value).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.toggle_custom_rule(index, cx);
                        }),
                    )),
            );
        }
        let mut saved = div().flex().flex_wrap().gap_2();
        for (index, preset) in self.custom_presets.iter().enumerate() {
            saved = saved.child(
                Button::new(("saved-rule", index))
                    .label(preset.name.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.load_custom_preset(index, cx);
                    })),
            );
        }
        div()
            .size_full()
            .overflow_y_scrollbar()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.foreground))
            .p_8()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(panel_width(window, 720.0)))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_3xl()
                            .font_bold()
                            .text_color(rgb(palette.accent))
                            .child("Custom rules"),
                    )
                    .child(
                        div().text_color(rgb(palette.muted)).child(
                            "Every switch is validated by the domain before a match can start.",
                        ),
                    )
                    .child(rows)
                    .child(
                        div()
                            .w(px(name_width))
                            .child(Input::new(&self.setup.custom_name)),
                    )
                    .when(!self.custom_presets.is_empty(), |element| {
                        element.child(div().child("Saved presets")).child(saved)
                    })
                    .when_some(self.last_error.clone(), |element, error| {
                        element.child(
                            div()
                                .p_3()
                                .rounded_lg()
                                .bg(rgb(0x003c_2024))
                                .text_color(rgb(0x00ff_b4b8))
                                .child(error),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(
                                Button::new("custom-save")
                                    .primary()
                                    .label("Save named preset")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.save_custom_rules(cx);
                                    })),
                            )
                            .child(Button::new("custom-export").label("Export").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.export_custom_rules(cx);
                                }),
                            ))
                            .child(Button::new("custom-import").label("Import").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.import_custom_rules(cx);
                                }),
                            ))
                            .child(
                                Button::new("custom-done")
                                    .label("Use these rules")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.setup.custom_rules.validate().is_ok() {
                                            this.setup.use_custom_rules = true;
                                            this.screen = Screen::Setup;
                                            this.last_error = None;
                                        }
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "profile totals and recent history form one data-driven dashboard"
    )]
    fn render_profiles(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.settings.theme.palette();
        let mut cards = div().flex().flex_col().gap_3();
        for profile in &self.profiles.profiles {
            let achievements = if profile.achievements.is_empty() {
                "None yet".to_owned()
            } else {
                profile
                    .achievements
                    .iter()
                    .map(|achievement| achievement_label(*achievement))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            cards = cards.child(
                div()
                    .p_4()
                    .rounded_xl()
                    .border_1()
                    .border_color(rgb(palette.line))
                    .bg(rgb(palette.surface))
                    .child(
                        div()
                            .text_xl()
                            .font_bold()
                            .text_color(rgb(palette.accent))
                            .child(profile.display_name.clone()),
                    )
                    .child(div().mt_2().text_color(rgb(palette.muted)).child(format!(
                        "{} matches • {} wins • {} captures • {} tokens home",
                        profile.matches, profile.wins, profile.captures, profile.completions
                    )))
                    .child(div().mt_1().text_sm().child(format!(
                        "Wins by color — R {} • G {} • Y {} • B {}",
                        profile.wins_by_color[0],
                        profile.wins_by_color[1],
                        profile.wins_by_color[2],
                        profile.wins_by_color[3]
                    )))
                    .child(div().mt_1().text_sm().child(format!(
                        "Classic {} • Quick {} • Tournament {} • Best streak {}",
                        profile.wins_by_preset[0],
                        profile.wins_by_preset[1],
                        profile.wins_by_preset[2],
                        profile.best_win_streak
                    )))
                    .child(
                        div()
                            .mt_1()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child(format!("Achievements: {achievements}")),
                    ),
            );
        }
        let mut history = div().flex().flex_col().gap_2();
        for match_summary in self.profiles.history.iter().rev().take(10) {
            history = history.child(div().p_3().rounded_lg().bg(rgb(palette.raised)).child(
                format!(
                    "{} • {} • winner: {} • {} commands",
                    match_summary.preset,
                    match_summary.players.join(" vs "),
                    match_summary.winner.as_deref().unwrap_or("undetermined"),
                    match_summary.commands
                ),
            ));
        }
        div()
            .size_full()
            .overflow_y_scrollbar()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.foreground))
            .p_8()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(panel_width(window, 720.0)))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_3xl()
                            .font_bold()
                            .text_color(rgb(palette.accent))
                            .child("Profiles & statistics"),
                    )
                    .child(if self.profiles.profiles.is_empty() {
                        div()
                            .p_5()
                            .rounded_xl()
                            .bg(rgb(palette.surface))
                            .child("Complete a local match to create your first profile.")
                    } else {
                        div().child(cards)
                    })
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} matches in local history",
                                self.profiles.history.len()
                            )),
                    )
                    .when(!self.profiles.history.is_empty(), |element| {
                        element
                            .child(div().font_bold().child("Recent matches"))
                            .child(history)
                    })
                    .child(
                        Button::new("profiles-back")
                            .primary()
                            .label("Back to setup")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.screen = Screen::Setup;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn render_replay(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.settings.theme.palette();
        let viewport = window.viewport_size();
        let (_, cell) = board_layout(f32::from(viewport.width), f32::from(viewport.height));
        let (cursor, total, playing, speed) =
            self.replay_player
                .as_ref()
                .map_or((0, 0, false, "1×"), |player| {
                    (
                        player.cursor(),
                        player.len(),
                        player.is_playing(),
                        player.speed().label(),
                    )
                });
        div()
            .size_full()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.foreground))
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .p_4()
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_2xl()
                            .font_bold()
                            .text_color(rgb(palette.accent))
                            .child("Replay studio"),
                    )
                    .child(format!("{cursor}/{total} commands")),
            )
            .child(self.render_board(cell, cx))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        Button::new("replay-back-five")
                            .label("−5")
                            .disabled(cursor == 0)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.seek_replay(-5, cx);
                            })),
                    )
                    .child(
                        Button::new("replay-play")
                            .primary()
                            .label(if playing { "Pause" } else { "Play" })
                            .disabled(total == 0)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_replay_playback(cx);
                            })),
                    )
                    .child(
                        Button::new("replay-step")
                            .label("Step")
                            .disabled(cursor >= total)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let _ = this.step_replay(cx);
                            })),
                    )
                    .child(
                        Button::new("replay-forward-five")
                            .label("+5")
                            .disabled(cursor >= total)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.seek_replay(5, cx);
                            })),
                    )
                    .child(
                        Button::new("replay-speed")
                            .label(speed)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cycle_replay_speed(cx);
                            })),
                    )
                    .child(
                        Button::new("replay-close")
                            .label("Close")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_replay(cx);
                            })),
                    ),
            )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "fixtures and standings are rendered together as one tournament dashboard"
    )]
    fn render_tournament(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.settings.theme.palette();
        let mut fixtures = div().flex().flex_col().gap_2();
        let mut standings = div().flex().flex_col().gap_2();
        if let Some(tournament) = &self.tournament {
            for fixture in &tournament.fixtures {
                let home = tournament
                    .participants
                    .iter()
                    .find(|participant| participant.id == fixture.home)
                    .map_or_else(
                        || "Unknown".to_owned(),
                        |participant| participant.name.clone(),
                    );
                let away = tournament
                    .participants
                    .iter()
                    .find(|participant| participant.id == fixture.away)
                    .map_or_else(
                        || "Unknown".to_owned(),
                        |participant| participant.name.clone(),
                    );
                let fixture_id = fixture.id;
                fixtures = fixtures.child(
                    div()
                        .p_3()
                        .rounded_xl()
                        .bg(rgb(palette.surface))
                        .border_1()
                        .border_color(rgb(palette.line))
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .w_16()
                                .text_sm()
                                .text_color(rgb(palette.muted))
                                .child(format!("R{}", fixture.round)),
                        )
                        .child(div().flex_1().child(format!("{home} vs {away}")))
                        .when(fixture.winner.is_none(), |element| {
                            element.child(
                                Button::new(("fixture-play", fixture_id))
                                    .primary()
                                    .label("Play match")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.start_fixture(fixture_id, cx);
                                    })),
                            )
                        })
                        .when_some(fixture.winner, |element, winner| {
                            element.child(format!("Winner: {}", winner.0))
                        }),
                );
            }
            for (position, standing) in tournament.standings().iter().enumerate() {
                let name = tournament
                    .participants
                    .iter()
                    .find(|participant| participant.id == standing.participant)
                    .map_or_else(
                        || "Unknown".to_owned(),
                        |participant| participant.name.clone(),
                    );
                standings = standings.child(
                    div()
                        .p_3()
                        .rounded_lg()
                        .bg(rgb(palette.raised))
                        .flex()
                        .child(div().w_10().font_bold().child(format!("#{}", position + 1)))
                        .child(div().flex_1().child(name))
                        .child(format!(
                            "{} pts • {}W {}L",
                            standing.points, standing.wins, standing.losses
                        )),
                );
            }
        }
        div()
            .size_full()
            .overflow_y_scrollbar()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.foreground))
            .p_8()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(panel_width(window, 820.0)))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_3xl()
                            .font_bold()
                            .text_color(rgb(palette.accent))
                            .child(match self.tournament_format {
                                TournamentFormat::RoundRobin => "Round-robin tournament",
                                TournamentFormat::SingleElimination => "Elimination bracket",
                            }),
                    )
                    .child(fixtures)
                    .child(div().text_xl().font_bold().child("Standings"))
                    .child(standings)
                    .child(
                        Button::new("tournament-back")
                            .primary()
                            .label("Back to setup")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.screen = Screen::Setup;
                                cx.notify();
                            })),
                    ),
            )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the lobby keeps connection details and host actions visible together"
    )]
    fn render_lan(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.settings.theme.palette();
        let viewport_width = f32::from(window.viewport_size().width);
        let compact = viewport_width < 760.0;
        let content_width = (viewport_width - 32.0).clamp(320.0, 920.0);
        let is_host = self.lan_host.is_some();
        let visible_endpoint = if is_host {
            self.lan_share_endpoint
        } else {
            self.lan_endpoint
        };
        let address = visible_endpoint.map_or_else(
            || "Resolving host…".to_owned(),
            |address| address.to_string(),
        );
        let code = self.lan_room.clone().unwrap_or_else(|| "------".to_owned());
        let mut roster = div().w_full().flex().flex_col().gap_2();
        if let Some(lobby) = &self.lan_lobby {
            for seat in &lobby.seats {
                let game_player = self.session.state().player(seat.player);
                let tint =
                    game_player.map_or(rgb(palette.muted), |player| color(player.player.color));
                let role = match seat.kind {
                    LobbySeatKind::Host => "Host",
                    LobbySeatKind::RemoteHuman => "Player",
                    LobbySeatKind::Computer => {
                        game_player.map_or("Computer", |player| player.player.bot_difficulty.name())
                    }
                };
                let is_you = self.lan_player == Some(seat.player);
                let status_color = if seat.connected {
                    0x0044_c47a
                } else {
                    0x00e2_626d
                };
                roster = roster.child(
                    div()
                        .p_3()
                        .rounded_xl()
                        .bg(rgb(palette.raised))
                        .border_1()
                        .border_color(tint)
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(div().size_3().flex_shrink_0().rounded_full().bg(tint))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .whitespace_nowrap()
                                .font_semibold()
                                .child(ellipsize(&seat.name, 28)),
                        )
                        .when(is_you, |element| {
                            element.child(
                                div()
                                    .flex_shrink_0()
                                    .px_2()
                                    .py_1()
                                    .rounded_full()
                                    .bg(rgb(palette.surface))
                                    .text_xs()
                                    .child("YOU"),
                            )
                        })
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(role),
                        )
                        .child(
                            div()
                                .size_2()
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(rgb(status_color)),
                        ),
                );
            }
        }
        let mut join_requests = div().w_full().flex().flex_col().gap_2();
        for (index, request) in self.lan_join_requests.iter().enumerate() {
            let accept_id = request.id.clone();
            let reject_id = request.id.clone();
            join_requests = join_requests.child(
                div()
                    .w_full()
                    .p_3()
                    .rounded_xl()
                    .bg(rgb(palette.raised))
                    .border_1()
                    .border_color(rgb(0x00ed_b34f))
                    .flex()
                    .when(compact, Styled::flex_col)
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .font_semibold()
                                    .whitespace_nowrap()
                                    .child(ellipsize(&request.name, 32)),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .text_sm()
                                    .text_color(rgb(palette.muted))
                                    .child("Nearby player is waiting for your approval"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .gap_2()
                            .child(
                                Button::new(("lan-reject-request", index))
                                    .danger()
                                    .label("Decline")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.reject_join_request(&reject_id, window, cx);
                                    })),
                            )
                            .child(
                                Button::new(("lan-accept-request", index))
                                    .primary()
                                    .label("Accept player")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.accept_join_request(&accept_id, window, cx);
                                    })),
                            ),
                    ),
            );
        }
        let connection = self.lan_connection;
        let awaiting_approval = !is_host && self.lan_join_request.is_some();
        let waiting_for_players = is_host
            && self.lan_lobby.as_ref().is_some_and(|lobby| {
                lobby
                    .seats
                    .iter()
                    .any(|seat| matches!(seat.kind, LobbySeatKind::RemoteHuman) && !seat.connected)
            });
        div()
            .size_full()
            .overflow_y_scrollbar()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.foreground))
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .child(
                div()
                    .w(px(content_width))
                    .p_6()
                    .rounded_2xl()
                    .bg(rgb(palette.surface))
                    .border_1()
                    .border_color(rgb(palette.line))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_4()
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_bold()
                                            .text_color(rgb(palette.accent))
                                            .child("PRIVATE TABLE"),
                                    )
                                    .child(
                                        div()
                                            .mt_1()
                                            .text_3xl()
                                            .font_bold()
                                            .child(if is_host {
                                                "Your lobby is ready"
                                            } else if awaiting_approval {
                                                "Approval requested"
                                            } else {
                                                "Waiting for the host"
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .px_3()
                                    .py_2()
                                    .rounded_full()
                                    .bg(rgb(palette.raised))
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .size_3()
                                            .rounded_full()
                                            .bg(rgb(connection.color())),
                                    )
                                    .child(
                                        div()
                                            .font_bold()
                                            .text_color(rgb(connection.color()))
                                            .child(connection.label()),
                                    )
                                    .when_some(self.lan_latency_ms, |element, latency| {
                                        element.child(
                                            div()
                                                .text_sm()
                                                .text_color(rgb(palette.muted))
                                                .child(format!("• {latency} ms")),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .when(compact, Styled::flex_col)
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .p_4()
                                    .rounded_xl()
                                    .bg(rgb(palette.raised))
                                    .border_1()
                                    .border_color(rgb(palette.line))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(palette.muted))
                                            .child(if is_host {
                                                "FALLBACK ROOM CODE"
                                            } else if awaiting_approval {
                                                "REQUEST STATUS"
                                            } else {
                                                "ROOM ACCESS"
                                            }),
                                    )
                                    .child(
                                        div()
                                            .mt_1()
                                            .text_3xl()
                                            .font_bold()
                                            .text_color(rgb(palette.accent))
                                            .child(if is_host {
                                                code
                                            } else if awaiting_approval {
                                                "Waiting for approval".to_owned()
                                            } else {
                                                "Approved".to_owned()
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .p_4()
                                    .rounded_xl()
                                    .bg(rgb(palette.raised))
                                    .border_1()
                                    .border_color(rgb(palette.line))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(palette.muted))
                                            .child(if is_host {
                                                "DIRECT FALLBACK"
                                            } else {
                                                "CONNECTED HOST"
                                            }),
                                    )
                                    .child(
                                        div()
                                            .mt_1()
                                            .font_bold()
                                            .child(address),
                                    ),
                            ),
                    )
                    .when(is_host && !self.lan_join_requests.is_empty(), |element| {
                        element.child(
                            div()
                                .w_full()
                                .p_4()
                                .rounded_xl()
                                .bg(rgb(0x0036_2c18))
                                .border_1()
                                .border_color(rgb(0x00ed_b34f))
                                .child(
                                    div()
                                        .w_full()
                                        .mb_3()
                                        .flex()
                                        .items_center()
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_xl()
                                                .font_bold()
                                                .child("Join requests"),
                                        )
                                        .child(
                                            div()
                                                .px_2()
                                                .py_1()
                                                .rounded_full()
                                                .bg(rgb(0x00ed_b34f))
                                                .text_color(rgb(0x0021_1a0d))
                                                .font_bold()
                                                .child(self.lan_join_requests.len().to_string()),
                                        ),
                                )
                                .child(join_requests),
                        )
                    })
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .child(div().flex_1().text_xl().font_bold().child("Players"))
                            .child(
                                div()
                                    .whitespace_nowrap()
                                    .text_sm()
                                    .text_color(rgb(palette.muted))
                                    .child(
                                        "Human players replace computers automatically",
                                    ),
                            ),
                    )
                    .child(roster)
                    .child(
                        div()
                            .p_3()
                            .w_full()
                            .rounded_xl()
                            .bg(rgb(palette.raised))
                            .text_center()
                            .text_color(rgb(palette.muted))
                            .child(if is_host {
                                "Nearby players can request access automatically. Review each request above; share the room code only as a fallback."
                            } else if awaiting_approval {
                                "Your request is with the host. Keep this window open—you will enter the lobby automatically when accepted."
                            } else {
                                "You are connected. The board will open automatically when the host starts."
                            }),
                    )
                    .when_some(self.last_error.clone(), |element, error| {
                        element.child(
                            div()
                                .w_full()
                                .p_3()
                                .rounded_lg()
                                .bg(rgb(0x003c_2024))
                                .text_color(rgb(0x00ff_b4b8))
                                .child(error),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .when(is_host, |element| {
                                element.child(
                                    Button::new("lan-enter")
                                        .primary()
                                        .large()
                                        .label(if waiting_for_players {
                                            "Waiting for players…"
                                        } else if self.lan_pending {
                                            "Synchronizing…"
                                        } else {
                                            "Start match"
                                        })
                                        .disabled(
                                            self.lan_token.is_none()
                                                || self.lan_pending
                                                || !self.lan_join_requests.is_empty()
                                                || waiting_for_players
                                                || !matches!(
                                                    self.lan_connection,
                                                    LanConnectionState::Connected
                                                ),
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.enter_lan_match(cx);
                                        })),
                                )
                            })
                            .child(
                                Button::new("lan-close")
                                    .ghost()
                                    .large()
                                    .label(if is_host { "Stop host" } else { "Leave lobby" })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.return_to_setup(cx);
                                    })),
                            ),
                    ),
            )
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the board grid, tokens, accessibility labels, and effect overlay render together"
    )]
    fn render_board(&self, cell: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let board_size = cell * 15.0;
        let mut board = div()
            .relative()
            .size(px(board_size))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded_xl()
            .border_2()
            .border_color(rgb(0x001d_2a24))
            .shadow_2xl()
            .bg(rgb(0x00f8_f3e7));
        for row in 0_u8..15 {
            let mut row_element = div().flex().h(px(cell));
            for column in 0_u8..15 {
                row_element = row_element.child(render_cell(row, column, cell));
            }
            board = board.child(row_element);
        }
        for token in &self.view_model.tokens {
            let mut visual = *token;
            if let Some(position) = self
                .visual_positions
                .get(&(token.player.index(), token.token.index()))
            {
                visual.position = *position;
            }
            if let Some((row, column)) = token_coordinate(visual) {
                let player = token.player;
                let token_id = token.token;
                let selectable = token.selectable
                    && !self.animating
                    && if self.lan_token.is_some() {
                        self.lan_player == Some(self.session.state().current().player.id)
                            && !self.lan_pending
                    } else {
                        matches!(
                            self.session.state().current().player.controller,
                            Controller::Human
                        )
                    };
                let token_size = (cell * 0.66).max(12.0);
                let (offset_x, offset_y) = token_offset(visual, cell);
                let token_element = div()
                    .id(("token", player.index() * 4 + token_id.index()))
                    .absolute()
                    .left(px(f32::from(column) * cell + offset_x))
                    .top(px(f32::from(row) * cell + offset_y))
                    .size(px(token_size))
                    .rounded_full()
                    .border_2()
                    .border_color(if self.settings.high_contrast {
                        rgb(0x0000_0000)
                    } else {
                        rgb(0x00ff_ffff)
                    })
                    .bg(color(token.color))
                    .shadow_lg()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .font_semibold()
                    .text_color(rgb(0x00ff_ffff))
                    .cursor_pointer()
                    .when(selectable, |element| {
                        element.border_3().shadow_xl().on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.move_token(token_id, cx);
                            },
                        ))
                    })
                    .child(if self.settings.high_contrast {
                        format!(
                            "{}{}",
                            token.color.name().chars().next().unwrap_or('?'),
                            token_id.index() + 1
                        )
                    } else {
                        (token_id.index() + 1).to_string()
                    });
                board = board.child(token_element);
            }
        }
        board.when_some(self.effect_message.clone(), |element, message| {
            element.child(
                div()
                    .absolute()
                    .left(px(board_size * 0.31))
                    .top(px(board_size * 0.45))
                    .w(px(board_size * 0.38))
                    .p_3()
                    .rounded_xl()
                    .bg(rgb(0x00ff_f4cf))
                    .border_2()
                    .border_color(rgb(0x00c8_9235))
                    .shadow_2xl()
                    .text_center()
                    .font_bold()
                    .text_color(rgb(0x0030_2517))
                    .child(message),
            )
        })
    }

    fn network_player_badge(&self, player: PlayerId) -> Option<(String, u32)> {
        self.lan_lobby.as_ref().and_then(|lobby| {
            lobby
                .seats
                .iter()
                .find(|seat| seat.player == player)
                .map(|seat| {
                    let label = if self.lan_player == Some(player) {
                        "You"
                    } else {
                        match seat.kind {
                            LobbySeatKind::Host => "Host",
                            LobbySeatKind::RemoteHuman => "Online",
                            LobbySeatKind::Computer => "Computer",
                        }
                    };
                    (
                        label.to_owned(),
                        if seat.connected {
                            0x0044_c47a
                        } else {
                            0x00e2_626d
                        },
                    )
                })
        })
    }

    fn player_card(&self, player: &PlayerViewModel, compact: bool) -> impl IntoElement {
        let tint = color(player.color);
        let palette = self.settings.theme.palette();
        let initial = player.name.chars().next().unwrap_or('?').to_string();
        let network_badge = self.network_player_badge(player.id);
        let name_limit = if compact {
            10
        } else if player.active || network_badge.is_some() {
            12
        } else {
            18
        };
        let display_name = ellipsize(&player.name, name_limit);
        div()
            .when(compact, |element| element.flex_1().min_w(px(160.0)))
            .when(!compact, Styled::w_full)
            .min_w_0()
            .overflow_hidden()
            .p_2()
            .rounded_xl()
            .border_1()
            .border_color(if player.active {
                tint
            } else {
                rgb(palette.line)
            })
            .bg(if player.active {
                Rgba { a: 0.20, ..tint }
            } else {
                rgb(palette.surface)
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .size_8()
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(tint)
                            .flex()
                            .items_center()
                            .justify_center()
                            .font_bold()
                            .text_color(rgb(0x00ff_ffff))
                            .child(initial),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .text_color(rgb(palette.foreground))
                                    .child(display_name),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("{}/4 home", player.finished)),
                            ),
                    )
                    .when(player.active, |element| {
                        element.child(
                            div()
                                .flex_shrink_0()
                                .px_2()
                                .py_1()
                                .rounded_full()
                                .bg(tint)
                                .text_xs()
                                .font_bold()
                                .text_color(rgb(0x00ff_ffff))
                                .child("TURN"),
                        )
                    })
                    .when_some(network_badge, |element, (label, status_color)| {
                        element.child(
                            div()
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(div().size_2().rounded_full().bg(rgb(status_color)))
                                .when(!compact, |element| element.child(label)),
                        )
                    }),
            )
    }

    fn dice_face(&self, compact: bool) -> impl IntoElement {
        let value = self.animated_dice.or(self.view_model.dice).unwrap_or(0);
        div()
            .size(px(if compact { 54.0 } else { 78.0 }))
            .rounded_2xl()
            .bg(rgb(0x00f9_f5e9))
            .border_2()
            .border_color(rgb(0x00d8_cda9))
            .shadow_xl()
            .flex()
            .items_center()
            .justify_center()
            .text_3xl()
            .font_bold()
            .text_color(rgb(0x0017_221c))
            .child(if value == 0 {
                "•".to_owned()
            } else {
                dice_glyph(value).to_owned()
            })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the responsive declarative game view is clearest as one contiguous layout"
    )]
    fn render_game(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.settings.theme.palette();
        let viewport = window.viewport_size();
        let width = f32::from(viewport.width);
        let height = f32::from(viewport.height);
        let (compact, cell) = board_layout(width, height);
        let can_human_roll = self.view_model.can_roll
            && !self.bot_pending
            && !self.animating
            && !self.lan_pending
            && if self.lan_token.is_some() {
                self.lan_player == Some(self.session.state().current().player.id)
            } else {
                matches!(
                    self.session.state().current().player.controller,
                    Controller::Human
                )
            };
        let status = self.last_error.as_deref().map_or_else(
            || {
                if self.bot_pending {
                    "Computer is choosing a move…".to_owned()
                } else if self.lan_pending {
                    "Synchronizing with the host…".to_owned()
                } else if self.animating {
                    "Moving token…".to_owned()
                } else {
                    match self.session.state().phase() {
                        TurnPhase::AwaitingRoll => "Roll to begin your move".to_owned(),
                        TurnPhase::AwaitingMove { .. } => "Choose a highlighted token".to_owned(),
                    }
                }
            },
            |error| ellipsize(error, 56),
        );
        let current_name = ellipsize(
            self.view_model
                .players
                .iter()
                .find(|player| player.active)
                .map_or("Current player", |player| player.name.as_str()),
            if compact { 10 } else { 16 },
        );
        let player_cards = div()
            .flex()
            .flex_wrap()
            .when(!compact, |element| element.flex_col().w(px(220.0)))
            .when(compact, Styled::w_full)
            .gap_2()
            .children(
                self.view_model
                    .players
                    .iter()
                    .map(|player| self.player_card(player, compact)),
            );
        let controls = div()
            .when(!compact, |element| {
                element.w(px(210.0)).flex_col().items_center()
            })
            .when(compact, |element| element.items_center().justify_center())
            .flex()
            .gap_3()
            .p_4()
            .rounded_2xl()
            .bg(rgb(palette.surface))
            .border_1()
            .border_color(rgb(palette.line))
            .child(
                div()
                    .when(!compact, Styled::w_full)
                    .when(compact, Styled::flex_1)
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .text_xs()
                            .font_bold()
                            .text_color(rgb(palette.accent))
                            .child("CURRENT TURN"),
                    )
                    .child(
                        div()
                            .mt_1()
                            .whitespace_nowrap()
                            .font_bold()
                            .child(current_name),
                    ),
            )
            .child(self.dice_face(compact))
            .child(
                Button::new("roll-dice")
                    .primary()
                    .large()
                    .label(if self.bot_pending {
                        "Bot thinking…"
                    } else if self.lan_pending {
                        "Synchronizing…"
                    } else if self.animating {
                        "In motion…"
                    } else {
                        "Roll dice"
                    })
                    .disabled(!can_human_roll)
                    .on_click(cx.listener(|this, _, _, cx| this.roll(cx))),
            )
            .child(
                div()
                    .when(!compact, Styled::w_full)
                    .when(compact, Styled::flex_1)
                    .line_clamp(if compact { 2 } else { 3 })
                    .text_sm()
                    .when(!compact, Styled::text_center)
                    .text_color(rgb(palette.foreground))
                    .child(status),
            );
        let table = div()
            .p_2()
            .rounded_2xl()
            .bg(rgb(palette.table))
            .border_1()
            .border_color(rgb(palette.table_edge))
            .shadow_2xl()
            .child(self.render_board(cell, cx));
        let content = if compact {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .overflow_y_scrollbar()
                .items_center()
                .justify_center()
                .gap_3()
                .p_3()
                .child(player_cards)
                .child(table)
                .child(controls)
                .into_any_element()
        } else {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .gap_4()
                .p_3()
                .child(player_cards)
                .child(table)
                .child(controls)
                .into_any_element()
        };
        div()
            .size_full()
            .overflow_hidden()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.foreground))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(56.0))
                    .px_4()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(palette.line))
                    .bg(rgb(palette.surface))
                    .child(
                        div()
                            .flex_1()
                            .font_bold()
                            .text_color(rgb(palette.accent))
                            .child("Ludo Royale"),
                    )
                    .child(
                        div()
                            .mr_3()
                            .px_3()
                            .py_1()
                            .rounded_full()
                            .bg(rgb(palette.raised))
                            .text_xs()
                            .font_bold()
                            .text_color(rgb(palette.muted))
                            .child(active_preset_name(self.session.state())),
                    )
                    .when(self.lan_token.is_some(), |element| {
                        let connection = self.lan_connection;
                        element.child(
                            div()
                                .mr_3()
                                .px_3()
                                .py_1()
                                .rounded_full()
                                .bg(rgb(palette.raised))
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_xs()
                                .font_bold()
                                .text_color(rgb(connection.color()))
                                .child(div().size_2().rounded_full().bg(rgb(connection.color())))
                                .child(connection.label())
                                .when_some(self.lan_latency_ms, |badge, latency| {
                                    badge.child(format!("{latency} ms"))
                                }),
                        )
                    })
                    .child(
                        Button::new("help")
                            .ghost()
                            .label("Rules")
                            .disabled(self.animating)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_help(cx);
                            })),
                    )
                    .child(
                        Button::new("pause")
                            .ghost()
                            .label("Pause")
                            .disabled(self.animating)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pause(cx);
                            })),
                    ),
            )
            .child(content)
    }
}

impl Render for GameView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sheet_layer = Root::render_sheet_layer(window, cx);
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let notification_layer = Root::render_notification_layer(window, cx);
        let content = match self.screen {
            Screen::Setup => self.render_setup(window, cx).into_any_element(),
            Screen::Game => self.render_game(window, cx).into_any_element(),
            Screen::HotSeat => self.render_hotseat(window, cx).into_any_element(),
            Screen::Pause => self.render_pause(window, cx).into_any_element(),
            Screen::Help => self.render_help(window, cx).into_any_element(),
            Screen::Settings => self.render_settings(window, cx).into_any_element(),
            Screen::ConfirmNew => self.render_confirmation(window, cx).into_any_element(),
            Screen::Results => self.render_results(window, cx).into_any_element(),
            Screen::Replay => self.render_replay(window, cx).into_any_element(),
            Screen::Profiles => self.render_profiles(window, cx).into_any_element(),
            Screen::CustomRules => self.render_custom_rules(window, cx).into_any_element(),
            Screen::UndoConfirm => self.render_undo_confirmation(window, cx).into_any_element(),
            Screen::Tournament => self.render_tournament(window, cx).into_any_element(),
            Screen::Lan => self.render_lan(window, cx).into_any_element(),
        };
        div()
            .relative()
            .size_full()
            .child(content)
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
    }
}

fn setting_row(
    title: &'static str,
    description: &'static str,
    control: Button,
    palette: Palette,
) -> impl IntoElement {
    div()
        .p_4()
        .rounded_xl()
        .border_1()
        .border_color(rgb(palette.line))
        .bg(rgb(palette.raised))
        .flex()
        .items_center()
        .gap_4()
        .child(
            div()
                .flex_1()
                .child(div().font_semibold().child(title))
                .child(
                    div()
                        .mt_1()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child(description),
                ),
        )
        .child(control)
}

const fn on_off(enabled: bool) -> &'static str {
    if enabled { "On" } else { "Off" }
}

fn ellipsize(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let prefix = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    let mut shortened = prefix.trim_end().to_owned();
    if max_chars > 0 {
        shortened.push('…');
    }
    shortened
}

const fn dice_glyph(value: u8) -> &'static str {
    match value {
        1 => "⚀",
        2 => "⚁",
        3 => "⚂",
        4 => "⚃",
        5 => "⚄",
        6 => "⚅",
        _ => "•",
    }
}

const fn achievement_label(achievement: Achievement) -> &'static str {
    match achievement {
        Achievement::FirstVictory => "First Victory",
        Achievement::CaptureArtist => "Capture Artist",
        Achievement::HatTrick => "Hat Trick",
        Achievement::HomewardBound => "Homeward Bound",
        Achievement::ColorMaster => "Color Master",
    }
}

fn active_preset_name(state: &GameState) -> &'static str {
    RulePreset::ALL
        .iter()
        .copied()
        .find(|preset| preset.rules() == state.rules())
        .map_or("Custom", RulePreset::name)
}

fn default_game() -> GameState {
    let players = PlayerColor::ALL[..2]
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, color)| {
            Some(Player {
                id: PlayerId::new(u8::try_from(index).ok()?)?,
                name: format!("Player {}", index + 1),
                color,
                controller: Controller::Human,
                bot_difficulty: Difficulty::Hard,
            })
        })
        .collect();
    GameState::new(players, RulePreset::Classic.rules()).unwrap_or_else(|error| {
        tracing::error!(%error, "default game configuration is invalid");
        std::process::abort();
    })
}

fn save_path() -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join("ludo-save.json")
}

fn replay_path() -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join("last-match.ludo-replay.json")
}

fn profile_path() -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join("ludo-profiles.json")
}

fn rule_collection_path() -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join("ludo-rule-presets.json")
}

fn rule_exchange_path() -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join("shared-rules.ludo.json")
}

fn panel_width(window: &Window, maximum: f32) -> f32 {
    (f32::from(window.viewport_size().width) - 64.0)
        .max(280.0)
        .min(maximum)
}

fn board_layout(width: f32, height: f32) -> (bool, f32) {
    let compact = width < 1_020.0;
    let cell = if compact {
        ((width - 32.0) / 15.0)
            .min((height - 240.0) / 15.0)
            .clamp(18.0, 34.0)
    } else {
        ((width - 500.0) / 15.0)
            .min((height - 125.0) / 15.0)
            .clamp(22.0, 42.0)
    };
    (compact, cell)
}

fn should_show_hotseat(state: &GameState, previous: PlayerId) -> bool {
    matches!(state.status(), GameStatus::Playing)
        && state.current().player.id != previous
        && matches!(state.current().player.controller, Controller::Human)
        && state
            .players()
            .iter()
            .filter(|player| matches!(player.player.controller, Controller::Human))
            .count()
            > 1
}

fn render_cell(row: u8, column: u8, cell: f32) -> impl IntoElement {
    let background = cell_color(row, column);
    let is_track = is_track_cell(row, column);
    let is_safe = matches!(
        (row, column),
        (6, 1 | 12) | (1 | 12, 8) | (8, 13 | 2) | (13 | 2, 6)
    );
    div()
        .size(px(cell))
        .flex_none()
        .bg(background)
        .border_1()
        .border_color(if is_track {
            rgb(0x00b9_b29e)
        } else {
            background
        })
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .font_bold()
        .text_color(rgb(0x00ff_ffff))
        .when(is_safe && cell >= 24.0, |element| element.child("★"))
}

fn cell_color(row: u8, column: u8) -> Rgba {
    if row < 6 && column < 6 {
        return rgb(0x00d9_404b);
    }
    if row < 6 && column > 8 {
        return rgb(0x002b_ab70);
    }
    if row > 8 && column > 8 {
        return rgb(0x00e9_b934);
    }
    if row > 8 && column < 6 {
        return rgb(0x0034_7ed1);
    }
    if (row == 7 && (1..=5).contains(&column)) || (row == 6 && column == 1) {
        return color(PlayerColor::Red);
    }
    if (column == 7 && (1..=5).contains(&row)) || (row == 1 && column == 8) {
        return color(PlayerColor::Green);
    }
    if (row == 7 && (9..=13).contains(&column)) || (row == 8 && column == 13) {
        return color(PlayerColor::Yellow);
    }
    if (column == 7 && (9..=13).contains(&row)) || (row == 13 && column == 6) {
        return color(PlayerColor::Blue);
    }
    if (6..=8).contains(&row) && (6..=8).contains(&column) {
        return match (row, column) {
            (6 | 7, 6) => color(PlayerColor::Red),
            (6, 7 | 8) => color(PlayerColor::Green),
            (7 | 8, 8) => color(PlayerColor::Yellow),
            _ => color(PlayerColor::Blue),
        };
    }
    rgb(0x00fa_f7ed)
}

fn is_track_cell(row: u8, column: u8) -> bool {
    ((6..=8).contains(&row) && !(6..=8).contains(&column))
        || ((6..=8).contains(&column) && !(6..=8).contains(&row))
}

fn token_coordinate(token: TokenViewModel) -> Option<(u8, u8)> {
    match token.position {
        TokenPosition::Yard => Some(yard_coordinate(token.color, token.token)),
        TokenPosition::Path(progress) if progress < 52 => {
            let index = (token.color.start_index() + progress) % 52;
            Some(TRACK[index as usize])
        }
        TokenPosition::Path(progress) => {
            let offset = progress.saturating_sub(52);
            match token.color {
                PlayerColor::Red => Some((7, 1 + offset)),
                PlayerColor::Green => Some((1 + offset, 7)),
                PlayerColor::Yellow => Some((7, 13 - offset)),
                PlayerColor::Blue => Some((13 - offset, 7)),
            }
        }
        TokenPosition::Finished => None,
    }
}

fn yard_coordinate(color: PlayerColor, token: TokenId) -> (u8, u8) {
    let offset = match token.index() {
        0 => (0, 0),
        1 => (0, 2),
        2 => (2, 0),
        _ => (2, 2),
    };
    let base = match color {
        PlayerColor::Red => (2, 2),
        PlayerColor::Green => (2, 10),
        PlayerColor::Yellow => (10, 10),
        PlayerColor::Blue => (10, 2),
    };
    (base.0 + offset.0, base.1 + offset.1)
}

fn token_offset(token: TokenViewModel, cell: f32) -> (f32, f32) {
    if matches!(token.position, TokenPosition::Yard) {
        return (cell * 0.17, cell * 0.14);
    }
    let shift = cell * 0.10;
    match token.token.index() {
        0 => (cell * 0.08, cell * 0.08),
        1 => (cell * 0.25, cell * 0.08),
        2 => (cell * 0.08, cell * 0.25),
        _ => (cell * 0.25, cell * 0.25),
    }
    .map(|value| value.max(shift))
}

trait TupleMap {
    fn map(self, function: impl Fn(f32) -> f32) -> Self;
}

impl TupleMap for (f32, f32) {
    fn map(self, function: impl Fn(f32) -> f32) -> Self {
        (function(self.0), function(self.1))
    }
}

fn color(color: PlayerColor) -> Rgba {
    match color {
        PlayerColor::Red => rgb(0x00df_3545),
        PlayerColor::Green => rgb(0x001f_a966),
        PlayerColor::Yellow => rgb(0x00e2_aa21),
        PlayerColor::Blue => rgb(0x0028_78cf),
    }
}

const TRACK: [(u8, u8); 52] = [
    (6, 1),
    (6, 2),
    (6, 3),
    (6, 4),
    (6, 5),
    (5, 6),
    (4, 6),
    (3, 6),
    (2, 6),
    (1, 6),
    (0, 6),
    (0, 7),
    (0, 8),
    (1, 8),
    (2, 8),
    (3, 8),
    (4, 8),
    (5, 8),
    (6, 9),
    (6, 10),
    (6, 11),
    (6, 12),
    (6, 13),
    (6, 14),
    (7, 14),
    (8, 14),
    (8, 13),
    (8, 12),
    (8, 11),
    (8, 10),
    (8, 9),
    (9, 8),
    (10, 8),
    (11, 8),
    (12, 8),
    (13, 8),
    (14, 8),
    (14, 7),
    (14, 6),
    (13, 6),
    (12, 6),
    (11, 6),
    (10, 6),
    (9, 6),
    (8, 5),
    (8, 4),
    (8, 3),
    (8, 2),
    (8, 1),
    (8, 0),
    (7, 0),
    (6, 0),
];

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ludo=info".into()),
        )
        .init();
    Application::new().run(|cx: &mut App| {
        gpui_component::init(cx);
        apply_component_theme(ThemeKind::Royale, cx);
        let bounds = Bounds::centered(None, size(px(1_120.0), px(800.0)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Ludo Royale".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                let game = cx.new(|cx| GameView::new(window, cx));
                cx.new(|cx| Root::new(game, window, cx))
            },
        );
        if let Err(error) = result {
            tracing::error!(%error, "failed to open the Ludo window");
            return;
        }
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn track_contains_52_unique_board_cells() {
        assert_eq!(TRACK.iter().copied().collect::<HashSet<_>>().len(), 52);
    }

    #[test]
    fn responsive_layout_selects_compact_and_wide_bounds() {
        let (compact, compact_cell) = board_layout(700.0, 650.0);
        assert!(compact);
        assert!((18.0..=34.0).contains(&compact_cell));

        let (wide, wide_cell) = board_layout(1_400.0, 900.0);
        assert!(!wide);
        assert!((22.0..=42.0).contains(&wide_cell));
    }

    #[test]
    fn player_names_are_shortened_without_breaking_unicode() {
        assert_eq!(ellipsize("You", 12), "You");
        assert_eq!(ellipsize("Alexandria the Great", 12), "Alexandria…");
        assert_eq!(ellipsize("محمد عبداللہ", 7), "محمد ع…");
    }

    #[test]
    fn every_yard_token_has_a_valid_coordinate() {
        for color in PlayerColor::ALL {
            for index in 0_u8..4 {
                let Some(token) = TokenId::new(index) else {
                    return;
                };
                let (row, column) = yard_coordinate(color, token);
                assert!(row < 15);
                assert!(column < 15);
            }
        }
    }

    #[test]
    fn one_human_with_bots_never_needs_privacy_screen() {
        let state = GameState::new(ludo_domain::standard_players(), RulePreset::Classic.rules())
            .unwrap_or_else(|_| std::process::abort());
        assert!(!should_show_hotseat(
            &state,
            PlayerId::new(1).unwrap_or_else(|| std::process::abort())
        ));
    }

    #[test]
    fn multiple_humans_need_privacy_screen_on_player_change() {
        let state = default_game();
        assert!(should_show_hotseat(
            &state,
            PlayerId::new(1).unwrap_or_else(|| std::process::abort())
        ));
    }
}
