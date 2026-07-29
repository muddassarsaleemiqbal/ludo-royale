export default async function initialize() {}

export class WasmGame {
  snapshot_json() {
    return JSON.stringify({
      players: [], tokens: [], status: "Ready", dice: null, can_roll: true,
      human_turn: true, busy: false, error: null, revision: 0, winner: null
    });
  }

  state_json() {
    return "{}";
  }

  restore_json() {
    return this.snapshot_json();
  }

  dispatch_json() {
    return JSON.stringify({ model: JSON.parse(this.snapshot_json()), effects: [] });
  }
}
