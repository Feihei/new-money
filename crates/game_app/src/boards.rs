//! 可选棋盘（P2d 之后）：启动时把每张真盘的句柄装进来，设置页按它摆地图卡。
//!
//! 规则层只认 `DataHandles.board` 这一个句柄 —— 换盘就是换它。这里不复制任何
//! 拓扑真相：`label` 只上屏，尺寸/地块数/街数/默认档全从 `BoardDef` 现读。
//!
//! 新增一张盘＝在 [`BOARD_OPTIONS`] 加一行（数据文件本身另有 `validate` 与
//! `check.sh` 的棋盘校验那两道门）。

use bevy::prelude::*;
use game_core::prelude::{BoardDef, DataHandles};

pub const BOARD_OPTIONS: [(&str, &str, &str); 2] = [
    ("taiwan", "台湾", "data/boards/taiwan.json"),
    ("china", "中国大陆", "data/boards/china.json"),
];

#[derive(Clone)]
pub struct BoardEntry {
    /// 与 `BoardDef.board_name` 同一个键：存档按它认盘，`NM_BOARD` 也按它选。
    pub key: &'static str,
    /// 只上屏的名字。
    pub label: &'static str,
    pub handle: Handle<BoardDef>,
}

#[derive(Resource, Clone)]
pub struct BoardLibrary {
    pub entries: Vec<BoardEntry>,
}

impl BoardLibrary {
    pub fn load(server: &AssetServer, data: &DataHandles) -> Self {
        Self {
            entries: BOARD_OPTIONS
                .iter()
                .map(|(key, label, path)| BoardEntry {
                    key,
                    label,
                    // 默认盘已经由 `DataHandles::load` 载入，直接复用那个句柄
                    handle: if *path == game_core::prelude::BOARD_PATH {
                        data.board.clone()
                    } else {
                        server.load(*path)
                    },
                })
                .collect(),
        }
    }

    pub fn index_of(&self, key: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.key == key)
    }
}

/// 设置页上选中的那张盘。与阵容一样跨局保留；读档会把它改到存档那张盘上。
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BoardPick(pub usize);

impl BoardPick {
    /// `NM_BOARD=<taiwan|china>` 钉一张盘（截图与 `NM_SCENE=board` 用）。
    /// 认不出就喊一句并回台湾盘 —— 与 `NM_CASH` 四档之外的处理同一条。
    pub fn from_env(library: &BoardLibrary) -> Self {
        let Ok(raw) = std::env::var("NM_BOARD") else {
            return Self::default();
        };
        match library.index_of(raw.trim()) {
            Some(index) => {
                info!("棋盘（NM_BOARD）：{}", library.entries[index].label);
                Self(index)
            }
            None => {
                warn!("NM_BOARD={raw} 不是可选棋盘（taiwan/china），改用台湾盘");
                Self::default()
            }
        }
    }
}
