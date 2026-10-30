//! 存档文件的路与落盘（P15b）。
//!
//! core 只负责把世界拍成 `GameSnapshot`（`SnapshotTaken`），不碰文件系统 ——
//! 「存到哪里、写坏了怎么办」是表现层的事。槽位只有一个（PLAN §四・十六 ②）。
//!
//! 位置：`NM_SAVE_DIR` 优先，其次 `%APPDATA%\new-money`（Windows）／
//! `$XDG_DATA_HOME/new-money`／`~/.local/share/new-money`。旋钮优先是截图与冒烟的老规矩：
//! 指一个干净目录，「读档」那几张图才不受本机旧存档影响。
//!
//! **回执只在真落盘之后才写**（缺陷 #11：Godot 在新开对局时误发 `game_saved`）。
//! [`SaveNotice`] 只有两处会写：落盘失败/成功的那条 system，与读档失败那一支
//! （`ui::dispatch_ui_actions`）。没有第三条路径能说出「已保存」—— 这就是那条缺陷
//! 的结构性修法，而不是「记得别在新开对局时发」。

use std::fs;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use game_core::prelude::{GameSnapshot, SAVE_VERSION, SnapshotTaken};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// 唯一的槽位文件名。
pub const SAVE_FILE: &str = "save.json";

/// 存档目录。`NM_SAVE_DIR` 优先（截图与冒烟要一个干净目录）。
pub fn save_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NM_SAVE_DIR") {
        return PathBuf::from(dir);
    }
    platform_data_dir().join("new-money")
}

fn platform_data_dir() -> PathBuf {
    #[cfg(windows)]
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata);
    }
    #[cfg(not(windows))]
    {
        if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(data);
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".local/share");
        }
    }
    // 走到这里说明连用户目录都问不出来（容器、被清空的环境变量）：
    // 存到临时目录也好过「点保存没反应」
    std::env::temp_dir()
}

pub fn save_path() -> PathBuf {
    save_dir().join(SAVE_FILE)
}

/// 主菜单那颗「读档」按不按得动，就看这一句。
pub fn save_exists() -> bool {
    save_path().is_file()
}

/// 存档/读档最后那句话。菜单与暂停面板都念它。
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct SaveNotice {
    pub text: Option<String>,
    /// 失败用一种颜色、成功另一种 —— 两句话共用一格，颜色是唯一的区别。
    pub failed: bool,
}

/// 原子写：先写 `.tmp` 再改名。中途崩掉不会留下半个 JSON 把下一次读档毒死。
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("建目录 {} 失败：{e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| format!("序列化失败：{e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|e| format!("写 {} 失败：{e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("替换 {} 失败：{e}", path.display()))?;
    Ok(())
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("读 {} 失败：{e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} 解析失败：{e}", path.display()))
}

/// 版本不对就不读：字段对不上的旧档「半读出来」比直接拒绝更危险。
pub fn check_version(version: u32) -> Result<(), String> {
    if version == SAVE_VERSION {
        Ok(())
    } else {
        Err(format!("存档版本 {version} 与本版 {SAVE_VERSION} 不兼容"))
    }
}

pub fn write_snapshot_to(path: &Path, snapshot: &GameSnapshot) -> Result<(), String> {
    write_json(path, snapshot)
}

pub fn read_snapshot_from(path: &Path) -> Result<GameSnapshot, String> {
    let snapshot: GameSnapshot = read_json(path)?;
    check_version(snapshot.version)?;
    Ok(snapshot)
}

/// 把 core 拍好的快照落盘，并把那一句话留给面板。**这是唯一写「已保存」的地方。**
fn persist_snapshot(mut taken: ResMut<SnapshotTaken>, mut notice: ResMut<SaveNotice>) {
    let Some(snapshot) = taken.0.take() else {
        return;
    };
    let path = save_path();
    *notice = match write_snapshot_to(&path, &snapshot) {
        Ok(()) => {
            info!("已存档：{}", path.display());
            SaveNotice {
                // 界面上只念文件名：完整路径在日志里，让面板那一行放得下它该说的事
                text: Some(format!("已保存到 {SAVE_FILE}")),
                failed: false,
            }
        }
        Err(why) => {
            warn!("存档失败：{why}");
            SaveNotice {
                text: Some(format!("保存失败：{why}")),
                failed: true,
            }
        }
    };
}

pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        // 启动时印一次路径：排查「存哪儿去了」不该靠翻代码
        info!("存档位置：{}", save_path().display());
        app.init_resource::<SaveNotice>()
            .add_systems(Update, persist_snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("new-money-paths-{}-{name}", std::process::id()))
            .join("save.json")
    }

    /// 写进一个还不存在的目录、再原样读回来。这条钉的是三件容易同时坏的事：
    /// 目录要自己建、写入要真的落到 `path`（而不是留在 `.tmp`）、读回来的是同一份值。
    #[test]
    fn a_file_written_under_a_fresh_directory_reads_back() {
        let path = scratch("round-trip");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        let value = vec![("P0".to_string(), 7_u32), ("P1".to_string(), 9)];
        write_json(&path, &value).expect("写");
        assert!(path.is_file(), "文件没落到目标路径");
        assert!(
            !path.with_extension("json.tmp").exists(),
            "临时文件留下来了：改名那一步没走完"
        );
        let back: Vec<(String, u32)> = read_json(&path).expect("读");
        assert_eq!(back, value);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// 读不出与读得出两种形状都要有 —— 只测成功那一半的话，「失败也返回 Ok」不会红。
    #[test]
    fn a_missing_file_and_garbage_both_come_back_as_errors() {
        let path = scratch("missing");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        assert!(read_json::<Vec<u32>>(&path).is_err(), "没有文件却读成功了");

        let path = scratch("garbage");
        fs::create_dir_all(path.parent().unwrap()).expect("建目录");
        fs::write(&path, "{ 这不是 JSON").expect("写坏档");
        let err = read_json::<Vec<u32>>(&path).expect_err("坏档不该读成功");
        assert!(err.contains("解析失败"), "报错没说清是哪一步：{err}");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    /// 版本是**先看再解释**：同一份字段，版本号差一位就必须拒 —— 不拒就等于
    /// 拿新字段的默认值冒充旧局的事实。
    #[test]
    fn only_the_current_version_is_accepted() {
        assert!(check_version(SAVE_VERSION).is_ok());
        assert!(
            check_version(SAVE_VERSION + 1).is_err(),
            "更新的一版没有迁移却过了"
        );
        if SAVE_VERSION > 0 {
            assert!(
                check_version(SAVE_VERSION - 1).is_err(),
                "旧版没有迁移却过了"
            );
        }
    }
}
