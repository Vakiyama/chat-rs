#!/usr/bin/env python3
"""
Render the call-cue set in four different timbres so the only thing that changes
is the *sound*, not the intervals/timing. Pick a folder you like; the generator
is parametric so you can keep tuning from there.

Timbres:
  soft_bell   - harmonic partials with upper ones decaying fast (bell, not xylophone)
  sine_pad    - near-pure sine, soft attack, slight detune. Ambient / "premium"
  fm_glass    - 2-op FM, glassy electric-piano character. Modern app feel
  warm_synth  - filtered synth with a gliding low-pass. Round, analog-ish
"""
import numpy as np, wave, os

SR = 48_000
ROOT = "/mnt/user-data/outputs/variants"

def hz(name):
    n = {"C":0,"C#":1,"D":2,"D#":3,"E":4,"F":5,"F#":6,"G":7,"G#":8,"A":9,"A#":10,"B":11}
    return 440.0 * 2 ** ((n[name[:-1]] + (int(name[-1]) - 4) * 12 - 9) / 12)

def attack_ramp(env, attack):
    a = int(attack * SR)
    if a:
        env[:a] *= 0.5 * (1 - np.cos(np.pi * np.arange(a) / a))
    return env

def one_pole(x, cutoff):
    a = np.exp(-2 * np.pi * cutoff / SR)
    y = np.empty_like(x); acc = 0.0
    for i, s in enumerate(x):
        acc = (1 - a) * s + a * acc; y[i] = acc
    return y

def glide_lowpass(x, c_start, c_end):
    n = len(x)
    cutoff = np.linspace(c_start, c_end, n)
    a = np.exp(-2 * np.pi * cutoff / SR)
    y = np.empty_like(x); acc = 0.0
    for i in range(n):
        acc = (1 - a[i]) * x[i] + a[i] * acc; y[i] = acc
    return y

# --- one note per timbre -----------------------------------------------------
def make_note(freq, dur, variant, gain=1.0):
    n = int(dur * SR); t = np.arange(n) / SR

    if variant == "soft_bell":
        # (ratio, amp, decay-multiplier) — upper partials die faster => bell, not toy
        parts = [(1.0,1.0,1.0),(2.0,0.5,1.9),(3.0,0.22,2.7),(4.2,0.10,3.6)]
        sig = sum(a*np.sin(2*np.pi*freq*r*t)*np.exp(-8.0*d*t) for r,a,d in parts)
        sig = attack_ramp_sig(sig, 0.003)

    elif variant == "sine_pad":
        parts = [(1.0,1.0,1.0),(2.0,0.16,1.2),(3.0,0.05,1.5)]
        def voice(detune):
            f = freq * 2**(detune/1200)
            return sum(a*np.sin(2*np.pi*f*r*t)*np.exp(-5.0*d*t) for r,a,d in parts)
        sig = voice(-5) + voice(+5)            # gentle chorus
        sig = attack_ramp_sig(sig, 0.016)      # soft attack, no pluck

    elif variant == "fm_glass":
        mod = np.sin(2*np.pi*freq*2.0*t)       # modulator at 2x carrier
        index = 4.0 * np.exp(-11.0*t)          # bright on attack, mellows to sine
        sig = np.sin(2*np.pi*freq*t + index*mod) * np.exp(-6.0*t)
        sig = attack_ramp_sig(sig, 0.004)

    elif variant == "warm_synth":
        parts = [(1.0,1.0,1.0),(3.0,0.11,1.3),(5.0,0.04,1.6)]  # odd harmonics ~ triangle
        def voice(detune):
            f = freq * 2**(detune/1200)
            return sum(a*np.sin(2*np.pi*f*r*t)*np.exp(-6.0*d*t) for r,a,d in parts)
        sig = voice(-4) + voice(+4)
        sig = attack_ramp_sig(sig, 0.006)
        sig = glide_lowpass(sig, 4500, 850)    # filter sweeps down => "pluck" movement

    else:
        raise ValueError(variant)

    return sig * gain

def attack_ramp_sig(sig, attack):
    a = int(attack * SR)
    if a:
        ramp = 0.5 * (1 - np.cos(np.pi * np.arange(a) / a))
        sig = sig.copy(); sig[:a] *= ramp
    return sig

# --- cue construction (intervals identical to the original set) --------------
def two(n1, n2, variant, lag=0.10, dur=0.40, gain=1.0, muffle=None):
    a = make_note(hz(n1), dur, variant, gain)
    b = make_note(hz(n2), dur, variant, gain)
    buf = np.zeros(int((lag + dur) * SR))
    buf[:len(a)] += a
    i = int(lag * SR); buf[i:i+len(b)] += b
    if muffle:
        buf = one_pole(buf, muffle)
    return buf

def one(note_name, variant, dur=0.22, gain=1.0):
    return make_note(hz(note_name), dur, variant, gain)

def build_cues(variant):
    return {
        "join":       two("E5","A5", variant, lag=0.09),
        "leave":      two("A5","E5", variant, lag=0.09),
        "mute":       one("D5", variant, dur=0.20),
        "unmute":     one("A5", variant, dur=0.20),
        "deafen":     two("A5","D5", variant, lag=0.11, muffle=2400),
        "undeafen":   two("D5","A5", variant, lag=0.11),
        "peer_join":  two("E5","A5", variant, lag=0.085, gain=0.5),
        "peer_leave": two("A5","E5", variant, lag=0.085, gain=0.5),
    }

def norm16(buf, headroom=0.9):
    peak = np.max(np.abs(buf)) or 1.0
    return ((buf/peak)*headroom*32767).astype("<i2")

def write(path, buf):
    fade = min(len(buf), int(0.012*SR))
    buf = buf.copy(); buf[-fade:] *= np.linspace(1,0,fade)
    with wave.open(path, "w") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(SR)
        w.writeframes(norm16(buf).tobytes())

ORDER = ["join","peer_join","mute","unmute","deafen","undeafen","peer_leave","leave"]
VARIANTS = ["soft_bell","sine_pad","fm_glass","warm_synth"]

gap = np.zeros(int(0.4*SR))
compare = []   # join+leave of each timbre, back to back, for quick A/B

for v in VARIANTS:
    d = f"{ROOT}/{v}"; os.makedirs(d, exist_ok=True)
    cues = build_cues(v)
    for name, buf in cues.items():
        write(f"{d}/{name}.wav", buf)
    preview = np.concatenate([np.concatenate([cues[n], gap]) for n in ORDER])
    write(f"{d}/_preview.wav", preview)
    compare.append(np.concatenate([cues["join"], gap, cues["leave"], gap, gap]))
    print("rendered", v)

write(f"{ROOT}/_compare_join_leave.wav", np.concatenate(compare))
print("wrote A/B compare:", " -> ".join(VARIANTS))
