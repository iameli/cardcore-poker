/**
 * Tiny WebAudio cues — synthesized, no assets. The AudioContext is created
 * lazily on first use; browsers keep it suspended until the user has
 * interacted with the page, so the first cue after a fresh load may be
 * silent. That's fine — a poker game always involves interaction quickly.
 */
let _ctx = null;

function ctx() {
  if (!_ctx) {
    const AC = window.AudioContext || window.webkitAudioContext;
    if (!AC) return null;
    _ctx = new AC();
  }
  if (_ctx.state === "suspended") _ctx.resume().catch(() => {});
  return _ctx;
}

/**
 * Two-tone "your turn" chime. Increments `window.__cardcoreTurnCues` whether
 * or not audio actually plays — the counter is the observable behavior for
 * tests and headless environments.
 */
export function playTurnCue() {
  window.__cardcoreTurnCues = (window.__cardcoreTurnCues || 0) + 1;
  try {
    const ac = ctx();
    if (!ac) return;
    const t0 = ac.currentTime;
    for (const [freq, start, dur] of [
      [880, 0, 0.12],
      [1174.7, 0.12, 0.18],
    ]) {
      const osc = ac.createOscillator();
      const gain = ac.createGain();
      osc.type = "triangle";
      osc.frequency.value = freq;
      gain.gain.setValueAtTime(0, t0 + start);
      gain.gain.linearRampToValueAtTime(0.18, t0 + start + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.001, t0 + start + dur);
      osc.connect(gain).connect(ac.destination);
      osc.start(t0 + start);
      osc.stop(t0 + start + dur + 0.05);
    }
  } catch {
    // Audio is best-effort; never let a chime break the game.
  }
}
