"""Procedural one-shot UI SFX synthesis. Drafts go to .tmp/game-sfx/synth/."""
import numpy as np, wave, os

SR = 44100

def env_exp(n, decay):
    t = np.arange(n) / SR
    return np.exp(-t * decay)

def sine(freq, dur, decay, harmonic=0.0, h_level=0.3):
    n = int(SR * dur)
    t = np.arange(n) / SR
    x = np.sin(2 * np.pi * freq * t) + h_level * harmonic * np.sin(4 * np.pi * freq * t)
    return x * env_exp(n, decay)

def noise_click(dur, lp_cut):
    n = int(SR * dur)
    rng = np.random.default_rng(0)
    x = rng.standard_normal(n)
    # simple one-pole low-pass
    alpha = np.exp(-2 * np.pi * lp_cut / SR)
    out = np.zeros(n)
    acc = 0.0
    for i in range(n):
        acc = (1 - alpha) * x[i] + alpha * acc
        out[i] = acc
    return out * env_exp(n, 1 / max(dur, 1e-3) * 2)

def fade_edges(x, ms=1.5):
    n = int(SR * ms / 1000)
    w = np.linspace(0, 1, n)
    x[:n] *= w
    x[-n:] *= w[::-1]
    return x

def concat(*parts, gap=0.0):
    out = []
    g = np.zeros(int(SR * gap))
    for i, p in enumerate(parts):
        if i: out.append(g.copy())
        out.append(p)
    return np.concatenate(out)

def norm(x, peak_db):
    target = 10 ** (peak_db / 20)
    m = np.abs(x).max()
    return x * (target / m) if m > 0 else x

def save_wav(path, x, peak_db=-6):
    x = norm(x, peak_db)
    st = np.stack([x, x], axis=1)
    pcm = (np.clip(st, -1, 1) * 32767).astype(np.int16)
    with wave.open(path, "w") as w:
        w.setnchannels(2); w.setsampwidth(2); w.setframerate(SR)
        w.writeframes(pcm.tobytes())

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
# 草稿产物一律落仓库根 .tmp/（被 gitignore），脚本放在哪里跑都不污染库目录
DST = os.path.normpath(os.path.join(SCRIPT_DIR, "..", ".tmp", "game-sfx", "synth"))
os.makedirs(DST, exist_ok=True)

def synth_click(thump_freq, thump_decay, click_gain, lp):
    body = sine(thump_freq, 0.09, thump_decay, harmonic=1.0, h_level=0.25)
    tick = noise_click(0.006, lp) * click_gain
    tick = np.pad(tick, (0, len(body) - len(tick)))
    return fade_edges(body + tick)

def make_all(variant):
    """variant: 0 = soft round, 1 = crisp bright"""
    if variant == 0:
        c_t, c_d, cg, lp, conf_notes, canc = 850, 40, 0.15, 6000, (640, 960), (520, 410)
        err_freq, err_style = 110, "square"
        hov = sine(1900, 0.04, 90)
    else:
        c_t, c_d, cg, lp, conf_notes, canc = 1150, 55, 0.3, 9000, (700, 1320), (560, 380)
        err_freq, err_style = 130, "square"
        hov = sine(2300, 0.03, 120)

    files = {}
    # menu_click: single pulse
    files["menu_click"] = synth_click(c_t, c_d, cg, lp)
    # hover: single micro tick
    files["hover"] = fade_edges(hov)
    # confirm: two ascending notes
    a = sine(conf_notes[0], 0.11, 26, harmonic=1.0, h_level=0.2)
    b = sine(conf_notes[1], 0.15, 20, harmonic=1.0, h_level=0.2)
    files["confirm"] = concat(a, b, gap=0.02)
    # cancel: two descending, shorter & softer
    a = sine(canc[0], 0.08, 38)
    b = sine(canc[1], 0.11, 30)
    files["cancel"] = concat(a, b, gap=0.02)
    # error_alert: double buzz
    n = int(SR * 0.09); t = np.arange(n) / SR
    raw = np.sign(np.sin(2 * np.pi * err_freq * t)) if err_style == "square" else np.sin(2 * np.pi * err_freq * t)
    raw = raw * env_exp(n, 18)
    buzz1 = fade_edges(raw); buzz2 = fade_edges(raw * 1.1)
    files["error_alert"] = concat(buzz1, buzz2, gap=0.05)

    return files

for variant in (0, 1):
    tag = "soft" if variant == 0 else "crisp"
    for name, x in make_all(variant).items():
        peak_db = -12 if name == "hover" else -6
        save_wav(os.path.join(DST, f"synth_{name}_{tag}.wav"), x, peak_db)
        print(f"synth_{name}_{tag}.wav  {len(x)/SR:.2f}s  {peak_db}dBFS")

# verify: transient count + peak
def analyze(path):
    with wave.open(path) as w:
        sr = w.getframerate()
        x = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(np.float32)/32768
    win = int(sr*0.01); n = len(x)//win
    env = np.sqrt((x[:n*win].reshape(n,win)**2).mean(axis=1))
    active = env > env.max()*0.15
    segs, on = [], False
    for i, a in enumerate(active):
        if a and not on: s=i; on=True
        elif not a and on: segs.append(1); on=False
    if on: segs.append(1)
    return len(segs), np.abs(x).max()

print("--- verify ---")
for f in sorted(os.listdir(DST)):
    if f.startswith("synth_") and f.endswith(".wav"):
        segs, peak = analyze(os.path.join(DST, f))
        print(f"{f:32s} segments={segs}  peak={peak:.2f}")
