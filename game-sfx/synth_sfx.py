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

# ===== P16a：对局里的八条通用音效（同一套管线的第二批）=====
#
# UI 那五条是「按下去了」，这一批是「事情发生了」：掷骰、进账、出账、出卡、放道具、
# 入狱、命运、聚街、破产。命名就是接线时用的键（`audio.rs` 里那张表读这些文件名），
# 所以**改名要两头一起改**。全部走固定种子：`noise_burst` 的每一条噪声流都点名一颗
# 种子，重跑永远得到字节级相同的文件（与 UI 那五条同一条纪律）。

GAME_DST = os.path.normpath(os.path.join(SCRIPT_DIR, "..", ".tmp", "game-sfx", "game"))
os.makedirs(GAME_DST, exist_ok=True)

def noise_burst(dur, lp_cut, gain=1.0, seed=0, decay=None):
    """一记带通噪声（碰撞／纸片／铁栏）。归一到 1.0 再乘 gain，所以 gain 就是它在混音里的位置。"""
    n = max(1, int(SR * dur))
    rng = np.random.default_rng(seed)
    x = rng.standard_normal(n)
    alpha = np.exp(-2 * np.pi * lp_cut / SR)
    out = np.zeros(n)
    acc = 0.0
    for i in range(n):
        acc = (1 - alpha) * x[i] + alpha * acc
        out[i] = acc
    m = np.abs(out).max()
    if m > 0:
        out = out / m
    d = decay if decay is not None else 2.0 / max(dur, 1e-3)
    return out * gain * env_exp(n, d)

def place_at(total_len, sig, offset_s):
    """把 sig 叠进一条 total_len 长的静音里，起点在 offset_s 秒（越界就截掉）。"""
    out = np.zeros(total_len)
    i = int(offset_s * SR)
    if i >= total_len:
        return out
    seg = sig[: total_len - i]
    out[i:i + len(seg)] += seg
    return out

def glide(f0, f1, dur, decay=6.0):
    """频率指数扫移（破产那一声「坠下去」）。相位用 cumsum 累，避免 dur 一变就跑调。"""
    n = int(SR * dur)
    t = np.arange(n) / SR
    freq = f0 * (f1 / f0) ** (t / max(dur, 1e-3))
    phase = 2 * np.pi * np.cumsum(freq) / SR
    return np.sin(phase) * env_exp(n, decay)

def bell(freq, dur, decay, partials=((1.0, 1.0), (1.6, 0.45), (2.76, 0.22))):
    """一个基频加几支**不成整数比**的分音 —— 金属与钟就是这样，正弦叠八度只会听着像电子琴。"""
    return sum(ratio_amp * sine(freq * ratio, dur, decay * (1.0 + 0.25 * i))
               for i, (ratio, ratio_amp) in enumerate(partials))

def make_game():
    files = {}

    # dice_roll：四记越滚越亮的碰撞 + 收尾一记闷响（骰子停住）
    n = int(SR * 0.62)
    x = np.zeros(n)
    for k, (off, dur, lp) in enumerate([(0.00, 0.030, 7000), (0.13, 0.026, 8200),
                                        (0.27, 0.022, 9500), (0.40, 0.018, 11000)]):
        x += place_at(n, noise_burst(dur, lp, gain=0.85, seed=11 + k), off)
    x += place_at(n, sine(320, 0.09, 46, harmonic=1.0, h_level=0.2) * 0.7, 0.50)
    files["dice_roll"] = fade_edges(x)

    # coin_gain：三记上行金属音（钱进袋）
    parts = []
    for i, (f, d) in enumerate([(1180, 0.07), (1560, 0.07), (2050, 0.10)]):
        parts.append(bell(f, d, 30) * (0.8 + 0.1 * i))
        parts.append(noise_burst(0.006, 12000, gain=0.22, seed=21 + i))
    files["coin_gain"] = fade_edges(concat(*parts, gap=0.045))

    # coin_loss：同一族的下行变体（不另立第九个事件，只是钱出去的那一档音高）
    parts = []
    for i, (f, d) in enumerate([(980, 0.07), (760, 0.07), (560, 0.12)]):
        parts.append(bell(f, d, 34) * (0.85 - 0.1 * i))
    files["coin_loss"] = fade_edges(concat(*parts, gap=0.04))

    # card_play：一张纸挥出去 —— 两记由暗到亮的噪声扫 + 收尾一记轻弹
    n = int(SR * 0.26)
    x = place_at(n, noise_burst(0.10, 1800, gain=0.55, seed=41, decay=12), 0.00)
    x += place_at(n, noise_burst(0.09, 4200, gain=0.75, seed=42, decay=14), 0.05)
    x += place_at(n, sine(1450, 0.05, 60) * 0.5, 0.17)
    files["card_play"] = fade_edges(x)

    # item_place：木质落桌 —— 一记闷响带一点板子共振
    body = sine(190, 0.10, 40, harmonic=1.0, h_level=0.3)
    tick = noise_burst(0.012, 3600, gain=0.6, seed=51)
    files["item_place"] = fade_edges(body + np.pad(tick, (0, len(body) - len(tick))))

    # jail：铁门落下 —— 低频一闷 + 三记铁栏震颤
    n = int(SR * 0.52)
    x = place_at(n, sine(92, 0.16, 22, harmonic=1.0, h_level=0.35), 0.00)
    x += place_at(n, sine(150, 0.14, 26) * 0.6, 0.02)
    for k, (off, lp) in enumerate([(0.10, 2400), (0.19, 3100), (0.30, 2000)]):
        x += place_at(n, noise_burst(0.05, lp, gain=0.5, seed=61 + k), off)
    files["jail"] = fade_edges(x)

    # fate：一记钟 —— 神秘感来自不成比的那几支分音，不来自音量
    files["fate"] = fade_edges(bell(523, 0.55, 7.0) * 0.9)

    # monopoly：聚街 = 明亮的三音上行 + 第四音收在长一点的余韵上
    a = bell(523, 0.10, 26)
    b = bell(659, 0.10, 24)
    c = bell(784, 0.12, 22)
    d = bell(1046, 0.26, 12)
    files["monopoly"] = fade_edges(concat(a, b, c, d, gap=0.06))

    # bankrupt：往下坠的一条滑音 + 一记散开的低频碎裂
    n = int(SR * 1.0)
    x = place_at(n, glide(430, 70, 0.55, decay=5.0) * 0.9, 0.00)
    x += place_at(n, noise_burst(0.34, 420, gain=0.7, seed=71, decay=7.0), 0.30)
    x += place_at(n, sine(58, 0.42, 8) * 0.8, 0.52)
    files["bankrupt"] = fade_edges(x)

    return files

print("--- game (P16a) ---")
for name, x in make_game().items():
    save_wav(os.path.join(GAME_DST, f"{name}.wav"), x, -6)
    segs, peak = analyze(os.path.join(GAME_DST, f"{name}.wav"))
    print(f"{name + '.wav':24s} {len(x)/SR:5.2f}s  segments={segs}  peak={peak:.2f}")
