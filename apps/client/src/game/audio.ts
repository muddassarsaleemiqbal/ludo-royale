type SoundName = "roll" | "move" | "capture" | "turn" | "victory" | "notification";

class GameAudio {
  private context: AudioContext | null = null;

  unlock() {
    if (!this.context) this.context = new AudioContext();
    if (this.context.state === "suspended") void this.context.resume();
  }

  play(name: SoundName, enabled: boolean) {
    if (!enabled) return;
    this.unlock();
    const context = this.context;
    if (!context) return;
    const notes: Record<SoundName, number[]> = {
      roll: [180, 240, 310],
      move: [330, 440],
      capture: [420, 220],
      turn: [520],
      victory: [392, 523, 659, 784],
      notification: [587, 784]
    };
    notes[name].forEach((frequency, index) => {
      const oscillator = context.createOscillator();
      const gain = context.createGain();
      const start = context.currentTime + index * 0.07;
      oscillator.type = name === "capture" ? "sawtooth" : "sine";
      oscillator.frequency.setValueAtTime(frequency, start);
      gain.gain.setValueAtTime(0.0001, start);
      gain.gain.exponentialRampToValueAtTime(0.09, start + 0.01);
      gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.12);
      oscillator.connect(gain).connect(context.destination);
      oscillator.start(start);
      oscillator.stop(start + 0.14);
    });
  }
}

export const gameAudio = new GameAudio();
window.addEventListener("pointerdown", () => gameAudio.unlock(), { once: true });
