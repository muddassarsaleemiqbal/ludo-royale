import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useMediaQuery } from "./lib/use-media-query";
import {
  Award,
  Bot,
  Check,
  CheckCircle2,
  ChevronRight,
  Clock3,
  Crown,
  Copy,
  Eye,
  Gamepad2,
  Globe2,
  LockKeyhole,
  History,
  MessageCircle,
  Plus,
  Palette,
  RefreshCw,
  Settings,
  Search,
  Shield,
  Sparkles,
  Trophy,
  UserPlus,
  UserX,
  UsersRound,
  Swords,
  Zap,
  Volume2,
  WifiOff
} from "lucide-react";
import { gameStore, useGameStore } from "./game/store";
import { Button } from "./components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger
} from "./components/ui/dialog";
import { LudoBoard } from "./components/game/board";
import { PlayerCard } from "./components/game/player-card";
import { Dice } from "./components/game/dice";
import { onlineStore, useOnlineStore } from "./game/online";
import { gameAudio } from "./game/audio";
import { preferenceStore, usePreferences } from "./game/preferences";



function LoadingScreen() {
  return (
    <main className="loading-screen">
      <div className="brand-mark"><span>♟</span></div>
      <h1>Ludo Royale</h1>
      <p>Preparing the board…</p>
      <div className="loading-track"><span /></div>
    </main>
  );
}

function OnlineDialog() {
  const online = useOnlineStore();
  const [register, setRegister] = useState(false);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  if (!online.user) {
    return (
      <DialogContent>
        <DialogTitle>{register ? "Create your player account" : "Login for online play"}</DialogTitle>
        <DialogDescription>Accounts are required for fair matches and reconnection.</DialogDescription>
        <form className="online-form" onSubmit={(event) => {
          event.preventDefault();
          setError(null);
          void onlineStore.authenticate(register ? "register" : "login", email, password, name)
            .catch((cause) => setError(String(cause)));
        }}>
          {register && <label>Display name<input autoComplete="nickname" value={name} onChange={(e) => setName(e.target.value)} required /></label>}
          <label>Email<input type="email" autoComplete="email" value={email} onChange={(e) => setEmail(e.target.value)} required /></label>
          <label>Password<input type="password" autoComplete={register ? "new-password" : "current-password"} value={password} onChange={(e) => setPassword(e.target.value)} minLength={10} required /></label>
          {(error ?? online.error) && <p className="online-error">{error ?? online.error}</p>}
          <Button type="submit">{register ? "Create account" : "Login"}</Button>
          <Button type="button" variant="ghost" onClick={() => setRegister(!register)}>
            {register ? "I already have an account" : "Create a new account"}
          </Button>
        </form>
      </DialogContent>
    );
  }
  return (
    <DialogContent>
      <DialogTitle>Your online profile</DialogTitle>
      <DialogDescription>Signed in as {online.user.display_name}. Your active games reconnect automatically.</DialogDescription>
      <div className="online-lobby">
        <p>{online.connected ? "Connected to the royal tables." : "Reconnecting…"}</p>
        <Button variant="ghost" onClick={() => onlineStore.logout()}>Log out</Button>
      </div>
    </DialogContent>
  );
}

const achievementNames: Record<string, string> = {
  first_win: "First Crown", veteran_10: "Table Veteran", streak_3: "Hat Trick",
  level_5: "Rising Royal", ranked_1200: "Elite Contender"
};

function PlayerHubDialog() {
  const online = useOnlineStore();
  const [tab, setTab] = useState<"profile" | "friends" | "matches" | "ranked" | "rewards">("profile");
  const [query, setQuery] = useState("");
  const [frame, setFrame] = useState(0);
  useEffect(() => { onlineStore.getHub(); }, []);
  useEffect(() => { setFrame(0); }, [online.replay?.matchId]);
  const hub = online.hub;
  if (!hub) return <DialogContent><DialogTitle>Royal player hub</DialogTitle><DialogDescription>Loading your profile…</DialogDescription></DialogContent>;
  const profile = hub.profile;
  const replayFrame = online.replay?.frames[frame];
  const canInvite = Boolean(online.lobby && online.lobby.host_user_id === online.user?.id);
  return (
    <DialogContent className="hub-dialog">
      <div className="hub-heading">
        <div><DialogTitle>Royal player hub</DialogTitle><DialogDescription>Friends, progression, ranked play, and rewards.</DialogDescription></div>
        <div className="hub-level"><Crown /><strong>Level {profile.level}</strong><span>{profile.xp % 500}/500 XP</span></div>
      </div>
      <nav className="hub-tabs" aria-label="Player hub">
        {([
          ["profile", <Crown />, "Profile"], ["friends", <UsersRound />, "Friends"],
          ["matches", <History />, "Matches"], ["ranked", <Swords />, "Ranked"],
          ["rewards", <Award />, "Rewards"]
        ] as const).map(([id, icon, label]) => <button key={id} className={tab === id ? "active" : ""} onClick={() => setTab(id)}>{icon}{label}</button>)}
      </nav>
      <div className="hub-content">
        {tab === "profile" && <div className="hub-profile">
          <section className="profile-hero"><div className="profile-avatar">{profile.display_name.slice(0, 1).toUpperCase()}</div><div><span>Royal player</span><h3>{profile.display_name}</h3><small>{profile.rating} season rating</small></div></section>
          <div className="stat-grid"><div><strong>{profile.matches}</strong><span>Matches</span></div><div><strong>{profile.wins}</strong><span>Wins</span></div><div><strong>{profile.matches ? Math.round(profile.wins / profile.matches * 100) : 0}%</strong><span>Win rate</span></div><div><strong>{profile.best_streak}</strong><span>Best streak</span></div></div>
          <div className="xp-card"><div><span>Level {profile.level}</span><strong>{500 - profile.xp % 500} XP to next level</strong></div><progress value={profile.xp % 500} max={500} /></div>
        </div>}
        {tab === "friends" && <div className="hub-stack">
          {hub.invites.map(invite => <div className="hub-notice" key={invite.id}><div><strong>{invite.sender_name} invited you</strong><span>{invite.lobby_name}</span></div><Button size="sm" onClick={() => onlineStore.respondFriendInvite(invite.id, true)}>Join</Button><Button size="sm" variant="ghost" onClick={() => onlineStore.respondFriendInvite(invite.id, false)}>Decline</Button></div>)}
          <form className="player-search" onSubmit={event => { event.preventDefault(); onlineStore.searchPlayers(query); }}><Search /><input value={query} onChange={event => setQuery(event.target.value)} placeholder="Find players by name" minLength={2}/><Button size="sm" type="submit">Search</Button></form>
          {online.playerSearch.map(player => <div className="social-row" key={player.user_id}><span className={`presence-dot ${player.presence}`} /><div><strong>{player.display_name}</strong><small>Level {player.level} · {player.rating} rating</small></div>{player.relationship === "none" && <Button size="sm" variant="secondary" onClick={() => onlineStore.sendFriendRequest(player.user_id)}><UserPlus /> Add</Button>}{player.relationship === "incoming" && <Button size="sm" onClick={() => onlineStore.respondFriendRequest(player.user_id, true)}>Accept</Button>}<span className="relationship-label">{player.relationship !== "none" ? player.relationship : ""}</span></div>)}
          <h3>Your circle</h3>
          {hub.friends.length ? hub.friends.map(friend => <div className="social-row" key={friend.user_id}><span className={`presence-dot ${friend.presence}`} /><div><strong>{friend.display_name}</strong><small>Level {friend.level} · {friend.rating} rating</small></div>{friend.relationship === "incoming" ? <><Button size="sm" onClick={() => onlineStore.respondFriendRequest(friend.user_id, true)}>Accept</Button><Button size="sm" variant="ghost" onClick={() => onlineStore.respondFriendRequest(friend.user_id, false)}>Decline</Button></> : friend.relationship === "friend" ? <>{canInvite && <Button size="sm" variant="secondary" onClick={() => onlineStore.inviteFriend(friend.user_id)}>Invite</Button>}<Button size="sm" variant="ghost" onClick={() => onlineStore.removeFriend(friend.user_id)}>Remove</Button></> : <span className="relationship-label">Request sent</span>}</div>) : <div className="hub-empty"><UsersRound /><span>Find players to build your circle.</span></div>}
        </div>}
        {tab === "matches" && <div className="hub-stack">{hub.matches.length ? hub.matches.map(match => <div className="match-row" key={match.id}><div className={`placement placement-${match.placement}`}>#{match.placement}</div><div><strong>{match.ranked ? "Ranked" : "Classic"} match</strong><small>{match.opponents.length ? `vs ${match.opponents.join(", ")}` : "vs Royal AI"} · {new Date(match.played_at).toLocaleDateString()}</small></div><span>+{match.xp_earned} XP{match.rating_delta ? ` · ${match.rating_delta > 0 ? "+" : ""}${match.rating_delta}` : ""}</span><Button size="sm" variant="ghost" onClick={() => onlineStore.getReplay(match.id)}>Replay</Button></div>) : <div className="hub-empty"><History /><span>Your completed online matches will appear here.</span></div>}</div>}
        {tab === "ranked" && <div className="ranked-hub"><section className="season-card"><div><span>{hub.season_name}</span><h3>{profile.rating} rating</h3><small>{hub.season_ends_at ? `Ends ${new Date(hub.season_ends_at).toLocaleDateString()}` : "Ranked play is between seasons"}</small></div><Button disabled={online.pending === "ranked" || Boolean(online.lobby)} onClick={() => onlineStore.rankedMatch()}><Swords /> Find ranked match</Button></section><div className="leaderboard"><div className="leaderboard-head"><strong>Season leaders</strong><span>Rating</span></div>{hub.leaderboard.map(row => <div className={row.user_id === profile.user_id ? "is-you" : ""} key={row.user_id}><b>#{row.rank}</b><span>{row.display_name}</span><small>{row.wins}/{row.matches} wins</small><strong>{row.rating}</strong></div>)}</div></div>}
        {tab === "rewards" && <div className="rewards-grid"><section><h3><Zap /> Daily challenges</h3>{hub.challenges.map(challenge => <div className="challenge" key={challenge.key}><div><strong>{challenge.title}</strong><span>{challenge.claimed ? "Complete" : `+${challenge.reward} XP`}</span></div><progress value={challenge.progress} max={challenge.target}/><small>{challenge.progress}/{challenge.target}</small></div>)}</section><section><h3><Award /> Achievements</h3><div className="achievement-grid">{Object.entries(achievementNames).map(([key, name]) => <div className={hub.achievements.includes(key) ? "unlocked" : ""} key={key}><Award /><strong>{name}</strong><small>{hub.achievements.includes(key) ? "Unlocked" : "Locked"}</small></div>)}</div></section><section className="cosmetics"><h3><Palette /> Cosmetics</h3><label>Dice theme<select value={profile.selected_dice} onChange={event => onlineStore.setCosmetics(event.target.value, profile.selected_tokens)}>{([["ivory",1],["obsidian",3],["emerald",5],["royal",8]] as [string,number][]).map(([theme, level]) => <option key={theme} value={theme} disabled={profile.level < level}>{theme} · level {level}</option>)}</select></label><label>Token theme<select value={profile.selected_tokens} onChange={event => onlineStore.setCosmetics(profile.selected_dice, event.target.value)}>{([["classic",1],["neon",3],["marble",5],["metallic",8]] as [string,number][]).map(([theme, level]) => <option key={theme} value={theme} disabled={profile.level < level}>{theme} · level {level}</option>)}</select></label></section></div>}
      </div>
      {replayFrame && <div className="replay-overlay"><div className="replay-card"><div className="replay-head"><div><strong>Match replay</strong><span>{replayFrame.status}</span></div><Button size="sm" variant="ghost" onClick={() => onlineStore.closeReplay()}>Close</Button></div><div className="replay-board"><LudoBoard tokens={replayFrame.tokens.map(token => ({ ...token, selectable: false }))} onSelect={() => undefined} showSafeCells animate={false} recentMoveKey={null} capturedKeys={[]} homeKey={null}/></div><div className="replay-controls"><Button size="sm" variant="secondary" disabled={frame === 0} onClick={() => setFrame(value => value - 1)}>Previous</Button><span>{frame + 1} / {online.replay!.frames.length}</span><Button size="sm" disabled={frame + 1 >= online.replay!.frames.length} onClick={() => setFrame(value => value + 1)}>Next</Button></div></div></div>}
    </DialogContent>
  );
}

const presets = [
  { id: "classic", name: "Classic", detail: "Safe stars, blockades and bonus turns", time: "35–50 min" },
  { id: "quick", name: "Quick play", detail: "Faster routes and forgiving home rolls", time: "15–25 min" },
  { id: "tournament", name: "Tournament", detail: "Rank every player with competitive rules", time: "45–60 min" }
];

type LocalSetup = {
  preset: "Classic" | "Quick" | "Tournament";
  botDifficulty: "Easy" | "Medium" | "Hard";
};

function SetupScreen({
  onLocal,
  resumeAvailable,
  onResume
}: {
  onLocal: (setup: LocalSetup) => void;
  resumeAvailable: boolean;
  onResume: () => void;
}) {
  const online = useOnlineStore();
  const [mode, setMode] = useState<"local" | "online">("local");
  const [preset, setPreset] = useState("classic");
  const [difficulty, setDifficulty] = useState("medium");
  const [tableName, setTableName] = useState("");
  const [confirmLeave, setConfirmLeave] = useState(false);
  const [isPublic, setIsPublic] = useState(true);
  const [turnSeconds, setTurnSeconds] = useState(30);
  const [inviteCode, setInviteCode] = useState("");
  const [rulesFilter, setRulesFilter] = useState("all");
  const [occupancyFilter, setOccupancyFilter] = useState("all");
  useEffect(() => {
    if (!online.toast) return;
    const timer = window.setTimeout(() => onlineStore.clearToast(), 5_000);
    return () => window.clearTimeout(timer);
  }, [online.toast]);
  const localSetup = (): LocalSetup => ({
    preset: (preset[0]?.toUpperCase() + preset.slice(1)) as LocalSetup["preset"],
    botDifficulty: (difficulty[0]?.toUpperCase() + difficulty.slice(1)) as LocalSetup["botDifficulty"]
  });

  return (
    <main className="setup-shell">
      <div className="setup-orb orb-a" /><div className="setup-orb orb-b" />
      <header className="setup-header">
        <div className="brand"><div className="brand-mark small"><span>♟</span></div><div><strong>Ludo Royale</strong><span>Choose your table</span></div></div>
        <div className="setup-profile">
          {online.user ? <Dialog><DialogTrigger asChild><button className="profile-trigger"><span className={online.connected ? "online-dot" : ""} /><span>{online.user.display_name}</span>{online.hub && <small>Lv. {online.hub.profile.level}</small>}</button></DialogTrigger><PlayerHubDialog /></Dialog> : <Dialog><DialogTrigger asChild><Button variant="secondary">Sign in</Button></DialogTrigger><OnlineDialog /></Dialog>}
        </div>
      </header>

      <section className="setup-content">
        <div className="setup-intro">
          <span className="eyebrow"><Crown /> The royal board awaits</span>
          <h1>Set the rules.<br /><em>Claim the crown.</em></h1>
          <p>Play your way with three clever rivals, or open a table and let friends take their seats.</p>
        </div>

        <div className="setup-card">
          <div className="setup-tabs">
            <button className={mode === "local" ? "active" : ""} onClick={() => setMode("local")}><Gamepad2 /> Solo game</button>
            <button className={mode === "online" ? "active" : ""} onClick={() => setMode("online")}><Globe2 /> Online tables</button>
          </div>

          {mode === "local" ? <>
            <div className="section-label"><span>1</span><div><strong>Choose a ruleset</strong><small>You can change this before every match</small></div></div>
            <div className="preset-grid">
              {presets.map((item) => <button key={item.id} className={preset === item.id ? "active" : ""} onClick={() => setPreset(item.id)}>
                <span className="preset-check">{preset === item.id && <Check />}</span><strong>{item.name}</strong><small>{item.detail}</small><em><Clock3 /> {item.time}</em>
              </button>)}
            </div>
            <div className="option-row">
              <div><strong>AI difficulty</strong><small>Applies to all three opponents</small></div>
              <div className="segmented">{["easy","medium","hard"].map(level => <button key={level} className={difficulty === level ? "active" : ""} onClick={() => setDifficulty(level)}>{level}</button>)}</div>
            </div>
            <div className="setup-actions">
              {resumeAvailable && <Button size="lg" variant="secondary" onClick={onResume}>Resume saved game</Button>}
              <Button size="lg" className="setup-primary" onClick={() => onLocal(localSetup())}>Start solo game <ChevronRight /></Button>
            </div>
          </> : online.configurationError ? <div className="signin-prompt">
            <WifiOff />
            <h2>Online server unavailable</h2>
            <p>{online.configurationError}</p>
          </div> : !online.user ? <div className="signin-prompt">
            <LockKeyhole /><h2>Sign in to join the tables</h2><p>An account keeps games fair and lets you reconnect on any device.</p>
            <Dialog><DialogTrigger asChild><Button>Sign in or create account</Button></DialogTrigger><OnlineDialog /></Dialog>
          </div> : online.lobby ? <div className="lobby-room">
            <div className="lobby-title"><div><span>Waiting room · {online.lobby.ranked ? "Ranked" : online.lobby.is_public ? "Public" : "Private"}</span><h2>{online.lobby.name}</h2></div><small>{online.lobby.rule_preset} · {online.lobby.bot_difficulty} AI · {online.lobby.turn_seconds}s</small></div>
            <div className="invite-strip"><code>{online.lobby.invite_code}</code><Button size="sm" variant="secondary" onClick={() => void onlineStore.copyInvite()}><Copy /> Copy invite</Button></div>
            <div className="seat-list">{online.lobby.seats.map((seat, index) => <div className={seat.is_bot ? "bot-seat" : "human-seat"} key={seat.seat}><span>{index + 1}</span><div><strong>{seat.name}</strong><small>{seat.is_bot ? "AI fills this seat" : `${seat.ready ? "Ready" : "Not ready"} · ${seat.presence}`}</small></div>{seat.ready && !seat.is_bot && <CheckCircle2 className="ready-icon" />}{online.lobby?.host_user_id === online.user?.id && seat.user_id && seat.user_id !== online.user?.id && <button className="seat-kick" aria-label={`Remove ${seat.name}`} onClick={() => onlineStore.kickPlayer(seat.user_id!)}><UserX /></button>}{online.lobby?.host_user_id === seat.user_id && <Crown />}</div>)}</div>
            {online.lobby.host_user_id === online.user.id && online.lobby.requests.length > 0 && <div className="request-list"><strong>Join requests</strong>{online.lobby.requests.map(request => <div key={request.id}><span>{request.display_name}</span><Button size="sm" disabled={online.pending === `request:${request.id}`} onClick={() => onlineStore.respondJoin(request.id, true)}>{online.pending === `request:${request.id}` ? "Saving…" : "Accept"}</Button><Button size="sm" variant="ghost" disabled={online.pending === `request:${request.id}`} onClick={() => onlineStore.respondJoin(request.id, false)}>Decline</Button></div>)}</div>}
            {online.lobby.host_user_id === online.user.id && <div className="host-controls"><select aria-label="Lobby rules" value={online.lobby.rule_preset} disabled={online.lobby.ranked} onChange={event => onlineStore.updateLobby({ rule_preset:event.target.value, bot_difficulty:online.lobby!.bot_difficulty, is_public:online.lobby!.is_public, turn_seconds:online.lobby!.turn_seconds })}>{presets.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</select><select aria-label="Lobby AI difficulty" value={online.lobby.bot_difficulty} disabled={online.lobby.ranked} onChange={event => onlineStore.updateLobby({ rule_preset:online.lobby!.rule_preset, bot_difficulty:event.target.value, is_public:online.lobby!.is_public, turn_seconds:online.lobby!.turn_seconds })}>{["easy","medium","hard"].map(level => <option key={level} value={level}>{level} AI</option>)}</select><select aria-label="Lobby turn timer" value={online.lobby.turn_seconds} onChange={event => onlineStore.updateLobby({ rule_preset:online.lobby!.rule_preset, bot_difficulty:online.lobby!.bot_difficulty, is_public:online.lobby!.is_public, turn_seconds:Number(event.target.value) })}>{[15,30,45,60].map(seconds => <option key={seconds} value={seconds}>{seconds}s turns</option>)}</select><select aria-label="Rematch setting" value={online.lobby.rematch_mode} onChange={event => onlineStore.updateLobby({ rule_preset:online.lobby!.rule_preset, bot_difficulty:online.lobby!.bot_difficulty, is_public:online.lobby!.is_public, turn_seconds:online.lobby!.turn_seconds, rematch_mode:event.target.value })}><option value="vote">Rematch vote</option><option value="host">Host decides</option><option value="automatic">Quick rematch</option></select><button role="switch" aria-checked={online.lobby.is_public} disabled={online.lobby.ranked} onClick={() => onlineStore.updateLobby({ rule_preset:online.lobby!.rule_preset, bot_difficulty:online.lobby!.bot_difficulty, is_public:!online.lobby!.is_public, turn_seconds:online.lobby!.turn_seconds })}>{online.lobby.is_public ? "Public table" : "Private table"}</button></div>}
            <div className="lobby-actions">{online.lobby.host_user_id === online.user.id ? <><Button variant="secondary" className="lobby-exit-button" onClick={() => setConfirmLeave(true)}>Leave table</Button><Button size="lg" disabled={online.pending === "start" || online.lobby.seats.some(seat => !seat.is_bot && seat.user_id !== online.user!.id && !seat.ready)} onClick={() => onlineStore.startGame()}>{online.pending === "start" ? "Starting…" : "Start with this lineup"}</Button></> : <><Button variant="secondary" onClick={() => onlineStore.setReady(!online.lobby!.seats.find(seat => seat.user_id === online.user!.id)?.ready)}>{online.lobby.seats.find(seat => seat.user_id === online.user!.id)?.ready ? "Not ready" : "I'm ready"}</Button><Button className="lobby-exit-button" variant="ghost" onClick={() => setConfirmLeave(true)}>Leave table</Button></>}</div>
          </div> : <div className="table-browser">
            <div className="browser-heading"><div><strong>Open tables</strong><small>Join friends or watch live games</small></div><Button size="icon" variant="ghost" onClick={() => onlineStore.listLobbies()} aria-label="Refresh tables"><RefreshCw /></Button></div>
            <div className="lobby-filters"><select aria-label="Filter by rules" value={rulesFilter} onChange={event => setRulesFilter(event.target.value)}><option value="all">All rules</option>{presets.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</select><select aria-label="Filter by occupancy" value={occupancyFilter} onChange={event => setOccupancyFilter(event.target.value)}><option value="all">Any seats</option><option value="open">Open seats</option><option value="nearly">Nearly full</option></select></div>
            <div className="table-list">{online.lobbies.filter(lobby => (rulesFilter === "all" || lobby.rule_preset === rulesFilter) && (occupancyFilter === "all" || (occupancyFilter === "open" ? lobby.human_players < 4 : lobby.human_players >= 3))).length ? online.lobbies.filter(lobby => (rulesFilter === "all" || lobby.rule_preset === rulesFilter) && (occupancyFilter === "all" || (occupancyFilter === "open" ? lobby.human_players < 4 : lobby.human_players >= 3))).map(lobby => <div className="table-row" key={lobby.id}>
              <div className="table-icon"><Crown /></div><div><strong>{lobby.name}</strong><small>Hosted by {lobby.host_name} · {lobby.rule_preset}</small></div>
              <span>{lobby.human_players}/4</span>
              {lobby.status === "playing" ? <Button size="sm" variant="secondary" onClick={() => onlineStore.spectate(lobby.id)}><Eye /> Watch</Button> : lobby.is_host ? <Button size="sm" disabled>Hosting</Button> : lobby.requested ? <Button size="sm" variant="ghost" onClick={() => onlineStore.cancelJoin(lobby.id)}>Cancel</Button> : <Button size="sm" variant="secondary" disabled={online.pending === `join:${lobby.id}`} onClick={() => onlineStore.requestJoin(lobby.id)}>{online.pending === `join:${lobby.id}` ? "Sending…" : "Request seat"}</Button>}
            </div>) : <div className="empty-tables"><Globe2 /><strong>No open tables yet</strong><span>Be the first host online.</span></div>}</div>
            <div className="match-actions"><Button variant="secondary" onClick={() => onlineStore.quickMatch(preset,difficulty)}>Quick match</Button><Button variant="secondary" onClick={() => onlineStore.rankedMatch()}><Swords /> Ranked</Button><div><input aria-label="Invite code" placeholder="Invite code" value={inviteCode} onChange={event => setInviteCode(event.target.value.toUpperCase())} maxLength={8}/><Button size="sm" onClick={() => onlineStore.joinByCode(inviteCode)}>Join invite</Button></div></div>
            <div className="create-table">
              <div className="section-label"><span><Plus /></span><div><strong>Host a new table</strong><small>Three AI players fill empty seats</small></div></div>
              <label className="field-label">Table name<input value={tableName} onChange={e => setTableName(e.target.value)} placeholder={`${online.user.display_name}'s table`} maxLength={40} /></label>
              <div className="create-options"><label>Rules<select value={preset} onChange={e => setPreset(e.target.value)}>{presets.map(p => <option value={p.id} key={p.id}>{p.name}</option>)}</select></label><label>AI difficulty<select value={difficulty} onChange={e => setDifficulty(e.target.value)}><option value="easy">Easy AI</option><option value="medium">Medium AI</option><option value="hard">Hard AI</option></select></label><label>Turn timer<select value={turnSeconds} onChange={event => setTurnSeconds(Number(event.target.value))}>{[15,30,45,60].map(seconds => <option key={seconds} value={seconds}>{seconds} seconds</option>)}</select></label><button className="visibility-toggle" role="switch" aria-checked={isPublic} onClick={() => setIsPublic(!isPublic)}>{isPublic ? "Public" : "Private"}</button></div>
              <Button disabled={online.pending === "create"} onClick={() => { onlineStore.createLobby({ name: tableName, rule_preset: preset, bot_difficulty: difficulty, is_public:isPublic, turn_seconds:turnSeconds }); }}>{online.pending === "create" ? "Creating table…" : `Create ${isPublic ? "public" : "private"} table`}</Button>
            </div>
          </div>}
          {online.error && <div className="online-error" role="alert"><span>{online.error}</span><button onClick={() => onlineStore.clearError()} aria-label="Dismiss error">×</button></div>}
          {online.toast && <div className="game-toast" role="status"><span>{online.toast}</span><button onClick={() => onlineStore.clearToast()} aria-label="Dismiss notification">×</button></div>}
        </div>
      </section>
      <Dialog open={confirmLeave} onOpenChange={setConfirmLeave}>
        <DialogContent>
          <DialogTitle>Leave this table?</DialogTitle>
          <DialogDescription>{online.lobby?.host_user_id === online.user?.id ? "Hosting will transfer to the longest-waiting connected player. If nobody remains, the table will close." : "Your seat will return to an AI player. You can request another seat later."}</DialogDescription>
          <div className="dialog-actions">
            <Button variant="ghost" onClick={() => setConfirmLeave(false)}>Stay</Button>
            <Button onClick={() => { onlineStore.leaveLobby(); setConfirmLeave(false); }}>Leave table</Button>
          </div>
        </DialogContent>
      </Dialog>
    </main>
  );
}

function GameHeader({ platform, onlineActive, onLocal }: {
  platform: "web" | "native"; onlineActive: boolean; onLocal: () => void;
}) {
  const preferences = usePreferences();
  return (
    <header className="app-header">
      <div className="brand">
        <div className="brand-mark small"><span>♟</span></div>
        <div>
          <strong>Ludo Royale</strong>
          <span>{platform === "native" ? "Native edition" : "WebAssembly edition"}</span>
        </div>
      </div>
      <nav className="mode-pills" aria-label="Game mode">
        <button className={onlineActive ? "" : "active"} onClick={onLocal}><Gamepad2 /> Local</button>
        <Dialog>
          <DialogTrigger asChild>
            <button className={onlineActive ? "active" : ""}><Globe2 /> Online</button>
          </DialogTrigger>
          <OnlineDialog />
        </Dialog>
        <button disabled title="Tournaments are planned"><Trophy /> Tournaments</button>
      </nav>
      <Dialog>
        <DialogTrigger asChild>
          <Button variant="secondary" size="icon" aria-label="Settings">
            <Settings className="size-4" />
          </Button>
        </DialogTrigger>
        <DialogContent>
          <DialogTitle className="text-xl font-bold">Game settings</DialogTitle>
          <DialogDescription className="mt-1 text-sm text-stone-400">
            Your preferences stay on this device.
          </DialogDescription>
          <div className="settings-list">
            <button role="switch" aria-checked={preferences.sound} onClick={() => {
              const enabled = !preferences.sound;
              preferenceStore.set("sound", enabled);
              if (enabled) gameAudio.play("notification", true);
            }}><Volume2 /> Sound effects <span>{preferences.sound ? "On" : "Muted"}</span></button>
            <button role="switch" aria-checked={preferences.motion} onClick={() => preferenceStore.set("motion", !preferences.motion)}><Sparkles /> Motion effects <span>{preferences.motion ? "Full" : "Reduced"}</span></button>
            <button role="switch" aria-checked={preferences.safeHints} onClick={() => preferenceStore.set("safeHints", !preferences.safeHints)}><Shield /> Safe-cell hints <span>{preferences.safeHints ? "On" : "Off"}</span></button>
          </div>
        </DialogContent>
      </Dialog>
    </header>
  );
}

export default function App() {
  const { model, starting, fatalError, platform, resumeAvailable } = useGameStore();
  const online = useOnlineStore();
  const [confirmNew, setConfirmNew] = useState(false);
  const [inGame, setInGame] = useState(false);
  const [activeSetup, setActiveSetup] = useState<LocalSetup | null>(null);
  const [secondsLeft, setSecondsLeft] = useState<number | null>(null);
  const [chatText, setChatText] = useState("");
  const [turnAnnouncement, setTurnAnnouncement] = useState<string | null>(null);
  const [recentMoveKey, setRecentMoveKey] = useState<string | null>(null);
  const [capturedKeys, setCapturedKeys] = useState<string[]>([]);
  const [homeKey, setHomeKey] = useState<string | null>(null);
  const preferences = usePreferences();
  const previousModel = useRef<typeof model>(null);
  const isMobile = useMediaQuery("(max-width: 960px)");

  useEffect(() => {
    void gameStore.initialize();
    void onlineStore.restore();
  }, []);

  const activeModel = online.model ?? model;
  const current = activeModel?.players.find((player) => player.active);
  const status = activeModel?.error ?? activeModel?.status ?? "";
  const turnStatus = !online.connected && online.model
    ? "Reconnecting — your game is safe"
    : online.syncing && online.model
      ? "Synchronizing the latest board…"
    : activeModel?.winner !== null && activeModel?.winner !== undefined
      ? `${activeModel.players.find(player => player.id === activeModel.winner)?.name ?? "A player"} wins!`
      : activeModel?.busy || (Boolean(online.model) && !activeModel?.human_turn)
        ? "AI is thinking…"
        : online.model && current?.id !== online.player
          ? `Waiting for ${current?.name ?? "the next player"}`
          : "Your turn";
  const boardTokens = useMemo(
    () => (activeModel?.tokens ?? []).map((token) => ({
      ...token,
      selectable: token.selectable && (!online.model || current?.id === online.player)
    })),
    [activeModel?.tokens, current?.id, online.model, online.player]
  );
  const legalTokenNumbers = boardTokens.filter(token => token.selectable).map(token => token.token + 1);
  const legalMovePrompt = legalTokenNumbers.length === 1
    ? `Move token ${legalTokenNumbers[0]}`
    : legalTokenNumbers.length > 1
      ? `Choose token ${legalTokenNumbers.join(" or ")}`
      : null;
  const handleTokenSelect = useCallback((token: number) => {
    gameAudio.unlock();
    if (online.model) {
      onlineStore.move(token);
      return;
    }
    void gameStore.dispatch({ SelectToken: token });
  }, [online.model]);
  const handleRoll = useCallback(() => {
    gameAudio.unlock();
    if (online.model) {
      onlineStore.roll();
      return;
    }
    void gameStore.dispatch("Roll");
  }, [online.model]);

  useEffect(() => {
    if (!activeModel) return;
    const previous = previousModel.current;
    if (previous && activeModel.revision > previous.revision) {
      const changed = activeModel.tokens.filter((token, index) =>
        JSON.stringify(token.position) !== JSON.stringify(previous.tokens[index]?.position)
      );
      const moved = changed.find(token => {
        const before = previous.tokens.find(item => item.player === token.player && item.token === token.token);
        return before && token.position !== "Yard";
      });
      const captured = changed.filter(token => {
        const before = previous.tokens.find(item => item.player === token.player && item.token === token.token);
        return token.position === "Yard" && before?.position !== "Yard";
      });
      if (moved) {
        const key = `${moved.player}:${moved.token}`;
        setRecentMoveKey(key);
        setHomeKey(moved.position === "Finished" ? key : null);
        window.setTimeout(() => {
          setRecentMoveKey(currentKey => currentKey === key ? null : currentKey);
          setHomeKey(currentKey => currentKey === key ? null : currentKey);
        }, 1_500);
      }
      if (captured.length) {
        const keys = captured.map(token => `${token.player}:${token.token}`);
        setCapturedKeys(keys);
        window.setTimeout(() => setCapturedKeys([]), 1_100);
      }
      const previousPlayer = previous.players.find(player => player.active);
      if (current?.id !== previousPlayer?.id && activeModel.winner === null) {
        const message = `${current?.name ?? "Next player"}'s turn`;
        setTurnAnnouncement(message);
        window.setTimeout(() => setTurnAnnouncement(currentMessage =>
          currentMessage === message ? null : currentMessage
        ), 1_200);
      }
      if (activeModel.winner !== null && previous.winner === null) {
        gameAudio.play("victory", preferences.sound);
        if (navigator.vibrate) navigator.vibrate([70, 45, 120]);
      } else if (activeModel.dice !== previous.dice && activeModel.dice !== null) {
        gameAudio.play("roll", preferences.sound);
        if (navigator.vibrate) navigator.vibrate(25);
      } else if (activeModel.tokens.some((token, index) =>
        token.position === "Yard" && previous.tokens[index]?.position !== "Yard"
      )) {
        gameAudio.play("capture", preferences.sound);
        if (navigator.vibrate) navigator.vibrate([45, 35, 70]);
      } else if (activeModel.tokens.some((token, index) =>
        JSON.stringify(token.position) !== JSON.stringify(previous.tokens[index]?.position)
      )) {
        gameAudio.play("move", preferences.sound);
        if (navigator.vibrate) navigator.vibrate(18);
      } else if (current?.id !== previous.players.find(player => player.active)?.id) {
        gameAudio.play("turn", preferences.sound);
      }
    }
    previousModel.current = activeModel;
  }, [activeModel, current?.id, preferences.sound]);

  useEffect(() => {
    if (!online.turnDeadline) {
      setSecondsLeft(null);
      return;
    }
    const update = () => setSecondsLeft(Math.max(0, Math.ceil((online.turnDeadline! - Date.now()) / 1000)));
    update();
    const timer = window.setInterval(update, 250);
    return () => window.clearInterval(timer);
  }, [online.turnDeadline]);

  useEffect(() => {
    if (online.toast) gameAudio.play("notification", preferences.sound);
  }, [online.toast, preferences.sound]);

  if (starting) return <LoadingScreen />;
  if (!activeModel) {
    return (
      <main className="fatal-screen">
        <h1>Unable to start Ludo</h1>
        <p>{fatalError ?? "The game engine did not return a board."}</p>
        <Button onClick={() => location.reload()}>Try again</Button>
      </main>
    );
  }

  if (!inGame && !online.model) {
    return <SetupScreen
      resumeAvailable={resumeAvailable}
      onResume={() => setInGame(true)}
      onLocal={(setup) => {
        setActiveSetup(setup);
        void gameStore.dispatch({
          NewGameWith: {
            preset: setup.preset,
            bot_difficulty: setup.botDifficulty
          }
        });
        setInGame(true);
      }}
    />;
  }

  return (
    <main className={`app-shell theme-dice-${online.hub?.profile.selected_dice ?? "ivory"} theme-tokens-${online.hub?.profile.selected_tokens ?? "classic"}`}>
      <div className="ambient ambient-one" />
      <div className="ambient ambient-two" />
      <GameHeader platform={platform} onlineActive={Boolean(online.model)} onLocal={() => {
        onlineStore.showLocal();
        setInGame(false);
      }} />

      <div className="turn-banner" role="status" aria-live="polite">
        {online.spectating ? "Spectating live" : turnStatus}{secondsLeft !== null && activeModel.winner === null ? ` · ${secondsLeft}s` : ""}
        {online.model && <span className={online.connected ? "connection-good" : "connection-wait"}>
          {online.connected ? "Live" : "Offline"}
        </span>}
      </div>
      {turnAnnouncement && <div className="turn-transition" role="status">
        <span className={`color-dot ${current?.color.toLowerCase()}`} />
        {turnAnnouncement}
      </div>}
      {homeKey && <div className="home-celebration" role="status">Token home! <Crown /></div>}
      <section className="game-layout">
        <aside className="left-panel">
          <div className="panel-heading">
            <div><UsersRound /><span>Players</span></div>
            <small>{online.rulePreset ?? activeSetup?.preset ?? "Classic"} • {online.botDifficulty ?? activeSetup?.botDifficulty ?? "Medium"} AI</small>
          </div>
          <div className="player-list">
            {activeModel.players.map((player) => (
              <PlayerCard key={player.id} player={player} presence={online.presence[player.id]} />
            ))}
          </div>
          <div className="game-tip">
            <Shield />
            <div>
              <strong>Safe squares</strong>
              <span title="Tokens on shield cells cannot be captured">Shield marks protect your token from capture.</span>
            </div>
          </div>
          {online.model && !isMobile && <div className="social-panel">
            <div className="social-title"><MessageCircle /><strong>Match feed</strong>{online.spectating && <span>Watching</span>}</div>
            <div className="event-feed" aria-live="polite">{online.events.length ? online.events.map(event => <div className={`event-${event.kind}`} key={event.id}>{event.message}</div>) : <span>Moves, reactions, and chat appear here.</span>}</div>
            <div className="reaction-row">{["👍","👏","😮","😂","🔥","👑"].map(emoji => <button key={emoji} aria-label={`React ${emoji}`} onClick={() => onlineStore.react(emoji)}>{emoji}</button>)}</div>
            <form className="chat-form" onSubmit={event => { event.preventDefault(); if (chatText.trim()) { onlineStore.chat(chatText); setChatText(""); } }}><label className="sr-only" htmlFor="match-chat">Match chat</label><input id="match-chat" value={chatText} onChange={event => setChatText(event.target.value)} maxLength={240} placeholder="Say something…" /><button type="submit" aria-label="Send chat">Send</button></form>
          </div>}
        </aside>

        <section className="board-section">
          <div className="mobile-players">
            {activeModel.players.map((player) => (
              <PlayerCard key={player.id} player={player} compact presence={online.presence[player.id]} />
            ))}
          </div>
          {online.model && isMobile && <div className="mobile-social-panel social-panel">
            <div className="social-title"><MessageCircle /><strong>Match feed</strong>{online.spectating && <span>Watching</span>}</div>
            <div className="event-feed" aria-live="polite">{online.events.length ? online.events.map(event => <div className={`event-${event.kind}`} key={event.id}>{event.message}</div>) : <span>Moves, reactions, and chat appear here.</span>}</div>
            <div className="reaction-row">{["👍","👏","😮","😂","🔥","👑"].map(emoji => <button key={emoji} aria-label={`React ${emoji}`} onClick={() => onlineStore.react(emoji)}>{emoji}</button>)}</div>
            <form className="chat-form" onSubmit={event => { event.preventDefault(); if (chatText.trim()) { onlineStore.chat(chatText); setChatText(""); } }}><label className="sr-only" htmlFor="mobile-match-chat">Match chat</label><input id="mobile-match-chat" value={chatText} onChange={event => setChatText(event.target.value)} maxLength={240} placeholder="Say something…" /><button type="submit" aria-label="Send chat">Send</button></form>
          </div>}
          <LudoBoard
            tokens={boardTokens}
            onSelect={handleTokenSelect}
            showSafeCells={preferences.safeHints}
            animate={preferences.motion}
            recentMoveKey={recentMoveKey}
            capturedKeys={capturedKeys}
            homeKey={homeKey}
          />
          {legalMovePrompt && <div className="legal-move-hint" role="status">
            <span className={`color-dot ${current?.color.toLowerCase()}`} />
            {legalMovePrompt} — tap the glowing piece
          </div>}
          <div className="mobile-status" aria-live="polite">{turnStatus} · {status}</div>
        </section>

        <aside className="right-panel">
          <div className="turn-label">
            <span className={`color-dot ${current?.color.toLowerCase()}`} />
            {current?.name ?? "Current player"}
          </div>
          <div className="dice-stage">
            <div className="dice-light" />
            <Dice
              value={activeModel.dice}
              busy={activeModel.busy || (Boolean(online.model) && !activeModel.human_turn)}
            />
          </div>
          <Button
            size="lg"
            className="roll-button"
            disabled={online.spectating || !activeModel.can_roll || Boolean(online.model && current?.id !== online.player)}
            onClick={handleRoll}
          >
            {activeModel.busy || (Boolean(online.model) && !activeModel.human_turn)
              ? <><Bot className="size-5" /> AI thinking…</>
              : activeModel.can_roll
                ? "Roll dice"
                : activeModel.human_turn
                  ? legalMovePrompt ?? "Choose a token"
                  : "Waiting for AI"}
          </Button>
          <div className="status-card">
            <span>Match status</span>
            <strong>{status}</strong>
          </div>
          <div className="rule-note" title="A six releases a token from its yard. Captures grant a bonus turn.">
            Roll a six to enter the board. Capture rivals to earn another turn.
          </div>
          {online.model && !online.spectating && <Button
            variant="secondary"
            className="w-full game-exit-action"
            disabled={online.pending === "leave-match" || online.pending === "end-game"}
            onClick={() => online.lobbies.find(lobby => lobby.id === online.lobbyId)?.is_host ? onlineStore.endGame() : onlineStore.leaveMatch()}
          >
            {online.lobbies.find(lobby => lobby.id === online.lobbyId)?.is_host ? "End game for everyone" : "Leave game"}
          </Button>}
          <Dialog open={confirmNew} onOpenChange={setConfirmNew}>
            <DialogTrigger asChild>
              <Button variant="secondary" className="w-full game-new-action">New game</Button>
            </DialogTrigger>
            <DialogContent>
              <DialogTitle className="text-xl font-bold">Start a new game?</DialogTitle>
              <DialogDescription className="mt-2 text-stone-400">
                Your current match progress will be replaced.
              </DialogDescription>
              <div className="mt-6 flex justify-end gap-3">
                <Button variant="ghost" onClick={() => setConfirmNew(false)}>Keep playing</Button>
                <Button onClick={() => {
                  setConfirmNew(false);
                  onlineStore.showLocal();
                  setInGame(false);
                }}>Return to setup</Button>
              </div>
            </DialogContent>
          </Dialog>
        </aside>
      </section>
      {activeModel.winner !== null && <div className="results-overlay" role="dialog" aria-modal="true" aria-labelledby="results-title">
        <div className="results-card">
          <Trophy />
          <span>Match complete</span>
          <h2 id="results-title">{activeModel.players.find(player => player.id === activeModel.winner)?.name} takes the crown!</h2>
          <ol>{[...activeModel.players].sort((a, b) => b.finished - a.finished).map(player => <li key={player.id}><strong>{player.name}</strong><span>{player.finished}/4 home</span></li>)}</ol>
          {online.model && !online.spectating && <Button disabled={online.pending === "rematch"} onClick={() => onlineStore.voteRematch()}>{online.rematchVotes ? `Rematch ${online.rematchVotes.votes}/${online.rematchVotes.needed}` : "Vote for rematch"}</Button>}
          <Button variant="secondary" onClick={() => { onlineStore.showLocal(); setInGame(false); }}>Return to setup</Button>
        </div>
      </div>}
    </main>
  );
}
