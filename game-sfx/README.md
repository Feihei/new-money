# 音效（算法合成）

最终采用的音效由 `synth_sfx.py` **程序化合成**（numpy + wave，无外部模型依赖）：
固定种子、参数全部为字面量，重跑永远得到字节级相同的文件。分两批：**UI 五条**
（按下去了）与**对局九条**（事情发生了，P16a）。产物已提升进 `assets/audio/sfx/*.ogg`，
本脚本是它们唯一的再生来源。

> ⚠️ 盘上是 **`.ogg`（vorbis）而不是 `.wav`**：Bevy 的默认 feature 只开 `bevy_audio` +
> `vorbis`，wav 不在里面 —— 接线时若把文件名写成 `.wav`，症状不是报错而是**一声不出**
> （资产加载失败只留一条 warn）。所以转换那一步是管线的一部分，不是可选的收尾。

## 音效 → 资产映射

### UI（soft / crisp 两变体，实际采用 soft）

| 脚本产物 | 资产 |
|---|---|
| `synth_menu_click_*.wav` | `assets/audio/sfx/ui_click_*.ogg` |
| `synth_hover_*.wav` | `assets/audio/sfx/ui_hover_*.ogg` |
| `synth_confirm_*.wav` | `assets/audio/sfx/ui_confirm_*.ogg` |
| `synth_cancel_*.wav` | `assets/audio/sfx/ui_cancel_*.ogg` |
| `synth_error_alert_*.wav` | `assets/audio/sfx/ui_error_*.ogg` |

- **soft**：圆钝低频（click 主体 850Hz、confirm 640→960Hz），hover 峰值 −12 dBFS，其余 −6。
- **crisp**：明亮尖锐（click 1150Hz + 更响的噪声 tick、confirm 700→1320Hz）。今天**没接**
  （接哪一套是 `audio.rs` 那张表里的一格，换它只改那一个后缀）。
- 音频文件本身**不入库**（`assets/*` 整目录 gitignore）；脚本入库即保住可复现性。

### 对局九条（P16a，键名就是 `audio.rs` 里读的那个文件名）

| 脚本产物 | 用途 | 形状（verify 段的 segments 就是它） |
|---|---|---|
| `dice_roll.wav` | 掷骰 | 四记越滚越亮的碰撞 + 收尾一记闷响（0.62s / 5 段） |
| `coin_gain.wav` | 进账 | 三记上行金属音（0.48s / 3 段） |
| `coin_loss.wav` | 出账 | 同一族的下行变体，低一档（0.34s / 3 段）—— 不另立第九个事件 |
| `card_play.wav` | 出卡 | 两记由暗到亮的噪声扫 + 一记轻弹（0.26s / 2 段） |
| `item_place.wav` | 放道具 | 木质落桌：190Hz 闷响带板子共振（0.10s / 1 段） |
| `jail.wav` | 入狱／保释 | 低频一闷 + 三记铁栏震颤（0.52s / 5 段） |
| `fate.wav` | 命运 | 一记钟，神秘感来自**不成整数比**的三支分音（0.55s / 1 段） |
| `monopoly.wav` | 聚街 | 三音上行 + 第四音收在长余韵上（0.76s / 4 段） |
| `bankrupt.wav` | 破产 | 430→70Hz 的指数滑音 + 一记散开的低频碎裂（1.00s） |

金属与钟一律用 `bell()`（分音比 1 : 1.6 : 2.76）而不是叠八度的正弦 —— 后者只会听着像电子琴。

## 重跑

```
uv run --with numpy python game-sfx/synth_sfx.py     # 或 python（要 numpy）
```

草稿固定输出到 `.tmp/game-sfx/synth/`（UI）与 `.tmp/game-sfx/game/`（对局），都在 gitignore
里，不污染库目录。脚本自带校验段（transient 段数 + 峰值），跑完直接在 stdout 核对。
提升进库要过一道转换（**同一条命令、固定参数**，逐键替换 `<键>`）：

```bash
ffmpeg -y -i .tmp/game-sfx/game/<键>.wav -ac 2 -ar 44100 -c:a libvorbis -q:a 5 assets/audio/sfx/<键>.ogg
```

UI 那五条改名进 `assets/audio/sfx/ui_*.ogg` 时同样走这条转换，不要直接拷 wav。

## 历史：MiniMax Music 3 路线（已否决）

2026-10-28 曾用 ComfyUI + 本地 MiniMax Music 3 权重生成过三批候选
（seed 11/42/77 与 v2 的 7/23/99，留档 `.tmp/game-sfx/`），听感不达标，一个都
没采用。工作流模板一并退到 `.tmp/game-sfx/sfx_workflow_template.json.rejected.bak`，
不入库。

## 历史：MiniMax Music 3 路线（已否决）

2026-10-28 曾用 ComfyUI + 本地 MiniMax Music 3 权重生成过三批候选
（seed 11/42/77 与 v2 的 7/23/99，留档 `.tmp/game-sfx/`），听感不达标，一个都
没采用。工作流模板一并退到 `.tmp/game-sfx/sfx_workflow_template.json.rejected.bak`，
不入库。
