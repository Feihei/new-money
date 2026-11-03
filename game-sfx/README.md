# UI 音效（算法合成）

最终采用的 UI 音效由 `synth_sfx.py` **程序化合成**（numpy + wave，无外部模型依赖）：
固定种子、参数全部为字面量，重跑永远得到字节级相同的文件。产物已提升进
`assets/audio/sfx/ui_*.wav`，本脚本是它们唯一的再生来源。

## 音效 → 资产映射

| 脚本产物（soft / crisp 两变体） | 资产 |
|---|---|
| `synth_menu_click_*.wav` | `assets/audio/sfx/ui_click_*.wav` |
| `synth_hover_*.wav` | `assets/audio/sfx/ui_hover_*.wav` |
| `synth_confirm_*.wav` | `assets/audio/sfx/ui_confirm_*.wav` |
| `synth_cancel_*.wav` | `assets/audio/sfx/ui_cancel_*.wav` |
| `synth_error_alert_*.wav` | `assets/audio/sfx/ui_error_*.wav` |

- **soft**：圆钝低频（click 主体 850Hz、confirm 640→960Hz），hover 峰值 −12 dBFS，其余 −6。
- **crisp**：明亮尖锐（click 1150Hz + 更响的噪声 tick、confirm 700→1320Hz）。
- 音频文件本身**不入库**（`assets/*` 整目录 gitignore）；脚本入库即保住可复现性。

## 重跑

```
python game-sfx/synth_sfx.py
```

草稿固定输出到 `.tmp/game-sfx/synth/`（被 gitignore），不污染本目录。脚本自带
校验段（transient 段数 + 峰值），跑完直接在 stdout 核对。选定后拷贝改名进
`assets/audio/sfx/` 即可。

## 历史：MiniMax Music 3 路线（已否决）

2026-10-28 曾用 ComfyUI + 本地 MiniMax Music 3 权重生成过三批候选
（seed 11/42/77 与 v2 的 7/23/99，留档 `.tmp/game-sfx/`），听感不达标，一个都
没采用。工作流模板一并退到 `.tmp/game-sfx/sfx_workflow_template.json.rejected.bak`，
不入库。
