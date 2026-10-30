//! 玩家设置（P15c）：文件、加载、落盘，以及「设置说了算」的那一条。
//!
//! 与存档同一个目录、两个文件：存档是**这一局**的，设置是**每一局**的（`settings.json`）。
//! 只有两件真控件 —— 行走速度与调试面板。音量等 P16（今天没有音频管线，摆上去按了
//! 不会有任何东西响），起始资金四档仍归 P2c（等真棋盘）。
//!
//! **`NM_*` 旋钮优先于文件**：截图与冒烟必须可控。`NM_DUMP=1` 强制开、`NM_DUMP=0`
//! 强制关（即便文件里写着开），不设才看文件 —— 判据抽成纯函数 [`debug_panel_on`]，
//! 三种形状各一条用例。
//!
//! 行走速度是**偏好而不是规则**：它住在 `GameRules` 里（core 的走步时钟与表现层的跳步
//! 都读那一个数），但读档会把整份 `GameRules` 换回存档那一刻的。所以这里有一条
//! [`apply_walk_speed`] 每帧把设置写回去 —— 否则「我设了 0.08，读了一个旧档就变回 0.18」
//! 会像一条 bug，而它其实是「规则属于这一局」那条纪律的副作用。

use std::path::Path;

use bevy::prelude::*;
use game_core::prelude::GameRules;
use serde::{Deserialize, Serialize};

use crate::paths::{read_json, settings_path, write_json};

/// 行走速度的档位：`(秒/格, 名字)`。界面用五颗按钮在这条链上走，不摆滑杆 ——
/// 0.20 没有滑杆控件，而「按一下差一档」在截图里也读得出。
///
/// `0.0` 是「瞬间落格」，与 `GameRules::instant()` 同一个语义（无头测试走的正是它）。
pub const SPEED_STEPS: [(f32, &str); 5] = [
    (0.0, "瞬间"),
    (0.08, "很快"),
    (0.18, "默认"),
    (0.30, "慢"),
    (0.45, "很慢"),
];

/// 设置文件的内容。加字段时要给 `serde(default)`（整份 struct 上那个），否则旧文件
/// 缺一格就整份读不出来 —— 而「读不出来」在这个文件里等于「玩家所有设置突然回默认」。
#[derive(Resource, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 行走一格停多久（秒）。
    pub walk_step_interval: f32,
    /// 开局就装只读 dump 面板（等效 `NM_DUMP=1`，但**下次启动**才生效 —— egui 插件
    /// 一旦装上就开始吃输入，不能在运行中装卸）。
    pub debug_panel: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            walk_step_interval: GameRules::default().walk_step_interval,
            debug_panel: false,
        }
    }
}

/// 上一次写盘的结果。失败要看得见 —— 「设置没记住」不该靠下一次启动才发现。
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct SettingsStatus {
    pub text: Option<String>,
    pub failed: bool,
}

impl Settings {
    pub fn load() -> Self {
        Self::load_from(&settings_path())
    }

    /// 读不出来就用默认值并 `warn!` 一句，**不崩**（PLAN P15c 的验收原话）。
    ///
    /// 文件不在是正常情况（第一次运行），不喊；读到了却解析不了才是要喊的那一种。
    pub fn load_from(path: &Path) -> Self {
        if !path.is_file() {
            return Self::default();
        }
        match read_json::<Self>(path) {
            Ok(settings) => settings,
            Err(why) => {
                warn!("设置文件读不出来，改用默认设置：{why}");
                Self::default()
            }
        }
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&settings_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        write_json(path, self)
    }
}

/// 调试面板开不开：**旋钮优先于文件**。
///
/// `NM_DUMP=1` 强制开、`NM_DUMP=0` 强制关；别的值或压根没设才看文件。
/// 抽成纯函数是为了能在不碰进程环境变量的前提下把三种形状各自钉住
/// （测试里改环境变量是全局且并行的，那正是「测试自己打架」的一种）。
pub fn debug_panel_on(env: Option<&str>, from_file: bool) -> bool {
    match env {
        Some("1") => true,
        Some("0") => false,
        _ => from_file,
    }
}

/// 把设置里那条行走速度抄进 `GameRules`（只在不相等时写，免得每帧把资源标脏）。
///
/// **它在 `Update` 里无条件跑**，所以读档那一刻换进来的 `rules` 会在下一帧被改回来：
/// 走一格停多久是玩家偏好，不是这一局的规则 —— 同一份存档在「很快」与「很慢」两台机器
/// 上就该按各自的步子走。别的规则（利率、物价门槛……）照旧以快照为准。
fn apply_walk_speed(settings: Res<Settings>, mut rules: ResMut<GameRules>) {
    if rules.walk_step_interval != settings.walk_step_interval {
        rules.walk_step_interval = settings.walk_step_interval;
    }
}

/// 设置一变就落盘。单写者：界面只改 [`Settings`]，文件由这一条写 ——
/// 三处各写一遍的话，「界面改了但没存」与「存了但不是界面那个值」会各出现一次。
fn persist_settings(
    settings: Res<Settings>,
    mut status: ResMut<SettingsStatus>,
    mut last: Local<Option<Settings>>,
) {
    // `Local` 而不是 `Res::is_changed()`：资源是启动时 `insert` 进来的，第一帧
    // `is_changed()` 必为真 —— 而这一帧该做的事是**读**而不是写。用 `Local` 记账之后，
    // 「启动时那份」也算一次变化，于是文件坏了会被此行改成一份可读的默认值（`load_from`
    // 已经喊过 warn，不必每次启动再喊一遍）。
    if last.as_ref() == Some(&*settings) {
        return;
    }
    *last = Some(*settings);
    *status = match settings.save() {
        Ok(()) => {
            info!("设置已保存：{}", settings_path().display());
            SettingsStatus {
                text: Some("已保存，下次启动仍然有效".to_string()),
                failed: false,
            }
        }
        Err(why) => {
            warn!("设置保存失败：{why}");
            SettingsStatus {
                text: Some(format!("保存失败：{why}")),
                failed: true,
            }
        }
    };
}

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        info!("设置文件：{}", settings_path().display());
        app.init_resource::<Settings>()
            .init_resource::<SettingsStatus>()
            .add_systems(Update, (apply_walk_speed, persist_settings));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("new-money-settings-{}-{name}", std::process::id()))
            .join("settings.json")
    }

    /// 存下去再读回来，逐字段相等 —— 这是「重开进程还是那个值」的最小可验形状。
    #[test]
    fn settings_survive_a_round_trip_through_the_file() {
        let path = scratch("round-trip");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let saved = Settings {
            walk_step_interval: 0.08,
            debug_panel: true,
        };
        saved.save_to(&path).expect("写设置");
        assert_eq!(Settings::load_from(&path), saved);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// 文件不在 = 默认（第一次运行）；文件坏了 = 默认 + 一句 warn，**不崩**。
    #[test]
    fn a_missing_or_broken_file_falls_back_to_defaults() {
        let path = scratch("broken");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        assert_eq!(
            Settings::load_from(&path),
            Settings::default(),
            "第一次运行（没有文件）就该用默认值"
        );

        std::fs::create_dir_all(path.parent().unwrap()).expect("建目录");
        std::fs::write(&path, "{ \"walk_step_interval\": \"不").expect("写坏文件");
        assert_eq!(
            Settings::load_from(&path),
            Settings::default(),
            "坏文件该回落默认，而不是 panic 或者把半份值读进来"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// 只写了半份的旧文件照读：`#[serde(default)]` 让缺的那一格用默认值 ——
    /// 不然以后加一个字段，所有老玩家都会「所有设置突然回默认」。
    #[test]
    fn a_partial_file_keeps_the_fields_it_does_have() {
        let path = scratch("partial");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        crate::paths::write_json(&path, &serde_json::json!({ "walk_step_interval": 0.3_f32 }))
            .expect("写半份");
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.walk_step_interval, 0.3);
        assert!(!loaded.debug_panel, "缺的那一格该是默认值");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// 旋钮优先：`NM_DUMP=1` 压过文件里的「关」，`NM_DUMP=0` 压过文件里的「开」，
    /// 没设才听文件的。三种各一格，少一种就会出现「截图脚本管不住面板」。
    #[test]
    fn the_knob_outranks_the_file_in_both_directions() {
        assert!(debug_panel_on(Some("1"), false), "NM_DUMP=1 没压过文件");
        assert!(!debug_panel_on(Some("0"), true), "NM_DUMP=0 没压过文件");
        assert!(debug_panel_on(None, true), "没设旋钮时该听文件的（开）");
        assert!(!debug_panel_on(None, false), "没设旋钮时该听文件的（关）");
        // 读不懂的值不算「设了」：退到文件，而不是凭一个陌生字符串开面板
        assert!(
            debug_panel_on(Some("yes"), true),
            "读不懂的值该退回文件（开）"
        );
        assert!(
            !debug_panel_on(Some("yes"), false),
            "读不懂的值该退回文件（关）"
        );
    }

    /// 设置里的默认速度必须正好落在某个档位上：否则面板上五颗按钮没有一颗是「当前」，
    /// 玩家会以为设置坏了。
    #[test]
    fn the_default_speed_is_one_of_the_offered_steps() {
        let default = Settings::default().walk_step_interval;
        assert!(
            SPEED_STEPS.iter().any(|(secs, _)| *secs == default),
            "默认速度 {default} 不在档位表里"
        );
    }

    /// 行走速度是**设置说了算**：资源里改了，规则层那个数下一帧就跟着变 ——
    /// 「改完立刻影响行走」在规则层的形状就是这一条（读者每帧现读 `GameRules`）。
    #[test]
    fn the_walk_speed_setting_reaches_the_rule_it_controls() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Settings {
                walk_step_interval: 0.08,
                debug_panel: false,
            })
            .init_resource::<GameRules>()
            .add_systems(Update, apply_walk_speed);
        app.update();
        assert_eq!(app.world().resource::<GameRules>().walk_step_interval, 0.08);

        app.world_mut()
            .resource_mut::<Settings>()
            .walk_step_interval = 0.45;
        app.update();
        assert_eq!(
            app.world().resource::<GameRules>().walk_step_interval,
            0.45,
            "改了设置而规则没跟着变 —— 走起来还是老步子"
        );
    }
}
