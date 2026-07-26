export type PlayerColor = "Red" | "Green" | "Yellow" | "Blue";
export type TokenPosition = "Yard" | "Finished" | { Path: number };

export interface PlayerViewModel {
  id: number;
  name: string;
  color: PlayerColor;
  active: boolean;
  finished: number;
}

export interface TokenViewModel {
  player: number;
  token: number;
  color: PlayerColor;
  position: TokenPosition;
  selectable: boolean;
}

export interface GameViewModel {
  players: PlayerViewModel[];
  tokens: TokenViewModel[];
  status: string;
  dice: number | null;
  can_roll: boolean;
  human_turn: boolean;
  busy: boolean;
  error: string | null;
  revision: number;
  winner: number | null;
}

export type BotDecision = {
  revision: number;
  player: number;
  token: number | null;
};

export type GameAction =
  | "NewGame"
  | "Roll"
  | { SelectToken: number }
  | { ContinueBot: number }
  | { DiceReady: { effect: number; value: number } }
  | { BotReady: { effect: number; decision: BotDecision } };

export type RuntimeEffect =
  | { DelayBot: { effect: number; milliseconds: number } }
  | { GenerateDice: { effect: number } }
  | {
      EvaluateBot: {
        effect: number;
        request: unknown;
      };
    };

export interface RuntimeUpdate {
  model: GameViewModel;
  effects: RuntimeEffect[];
}

export interface GameClient {
  initialize(): Promise<GameViewModel>;
  dispatch(action: GameAction): Promise<RuntimeUpdate>;
  evaluateBot(request: unknown): Promise<BotDecision>;
  randomDice(): Promise<number>;
  close(): void;
}
