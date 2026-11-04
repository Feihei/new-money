//! 本地美术素材的闸门：谁该画成图、图在不在盘上、开关在哪。
//!
//! 三件事集中在一个模块里，是因为它们的答案在每一处都一样：**先问开关，再问盘上有没有，
//! 两个问题任一个答「没有」就退回程序化几何**。散到 `render/` 与 `ui/` 各自 `is_file()`
//! 一遍，就会长出「贴图缺了会退回方块而 sprite 缺了会退回球」这种两套规矩。
//!
//! `assets/` 除 `data/` 之外整目录不入库（AGENTS.md 硬约束 8），所以「盘上没有」在别的
//! 机器上是正常状态而不是错误 —— 这里不 panic、不 `warn!`，一句都不响。唯一的例外是
//! `examples/character_preview.rs`，它是一次性试装场景，缺文件要指出重烘方法。

use std::sync::OnceLock;

use crate::asset_root;

/// 素材总开关。`NM_ART=0` 整屏退回程序化几何 —— 留这一条是为了能逐像素 A/B：
/// 「加了素材感觉不对」要能立刻看出是哪一处不对，而不是只能拿旧 commit 重编一次。
fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("NM_ART").map(|v| v != "0").unwrap_or(true))
}

/// 盘上有就给出相对路径（`AssetServer` 与 `is_file` 用的同一个根，见 `asset_root`）。
fn on_disk(rel: &str) -> Option<String> {
    enabled().then(|| asset_root().join(rel).is_file().then(|| rel.to_owned()))?
}

/// 诨名 → sprite 键。表序同时是「名字对不上时按座位领人」的领人顺序。
///
/// 十六位 = `CHARACTERS.md` §五 首发十二人 + 隐藏位沈万三 + §六 三位已烘出来的候补。
/// 演示盘的默认阵容（姬百年／范一文）就在这份表里，名字与脸一一对上；表里查不到的
/// 名字（测试夹具、P14 之后临时摆上来的对手）才走按座位那一支。
const ROSTER: [(&str, &str); 16] = [
    ("姬百年", "jibainian"),
    ("莱息", "laixi"),
    ("玛租", "mazu"),
    ("贺兰山", "helanshan"),
    ("温久枝", "wenjiuzhi"),
    ("范一文", "fanyiwen"),
    ("多吉", "duoji"),
    ("阿香", "axiang"),
    ("陆仁佳", "lvrenjia"),
    ("塔萝", "taluo"),
    ("索元", "suoyuan"),
    ("滑铁卢", "huatielu"),
    ("沈万三", "shenwansan"),
    ("包太太", "baotai"),
    ("雷当", "leidang"),
    ("葛守田", "geshoutian"),
];

/// 这一位该用哪条横条（`sprite/<键>.png`，八帧 128×128）。
///
/// `seat` 只在名字查不到时用来分人，保证同桌两位不会领到同一张脸。盘上缺某几条时
/// 往后顺延一位 —— 少一条素材不该让某一颗棋子独自退回球；但**名字查得到而文件缺**时
/// 宁可退回球，也不换一张脸：那会把「温久枝」画成「姬百年」，而棋盘上没人看得出。
pub fn strip_for(name: &str, seat: usize) -> Option<String> {
    if let Some((_, key)) = ROSTER.iter().find(|(who, _)| *who == name) {
        return on_disk(&format!("sprite/{key}.png"));
    }
    (0..ROSTER.len())
        .map(|k| sprite_key(name, seat + k))
        .find_map(|key| on_disk(&format!("sprite/{key}.png")))
}

/// 名字 → 横条键。不碰盘，所以能单测（`assets/sprite/` 不入库，「盘上有没有」在别的
/// 机器上是另一种答案，把它写进断言就是写一条只有这台机器会跑的测试）。
fn sprite_key(name: &str, seat: usize) -> &'static str {
    ROSTER
        .iter()
        .find(|(who, _)| *who == name)
        .map_or(ROSTER[seat % ROSTER.len()].1, |(_, key)| *key)
}

/// 卡面（`cards/<卡 id>.png`，带剪纸边框的整张竖图）。
pub fn card_face(id: &str) -> Option<String> {
    on_disk(&format!("cards/{id}.png"))
}

/// 主菜单背景。
pub fn menu_bg() -> Option<String> {
    on_disk("ui/bg_title.png")
}

/// 主菜单标题 logo（书法「暴富」+ 方孔铜钱 + NEW MONEY 绶带，AI 生成后抠透明底）。
/// 与 bg_title 同一目录同一套门：盘上没有就由调用方退回文字标题。
pub fn menu_logo() -> Option<String> {
    on_disk("ui/logo_title.png")
}

/// UI 底板皮肤（`ui/skin_<键>.webp`，中央素面 + 透明外沿的剪纸底板）。
/// 键与边距表见 `ui/skin.rs`；盘上没有时调用方退回纯色面板，不报错。
pub fn ui_skin(key: &str) -> Option<String> {
    on_disk(&format!("ui/skin_{key}.webp"))
}

/// 选角色那一屏的剪纸世界地图（`ui/world_map.png`，2:1）。盘上没有时设置页
/// 仍按同一套锚点摆头像，只是背景退回素色纸面（与其余素材同一条闸门）。
pub fn world_map() -> Option<String> {
    on_disk("ui/world_map.png")
}

/// 棋盘格子的素面纸卡（`textures/board/plate.png`，圆角 + 透明角，不带徽记）。
/// 配色仍由 `palette` 出，这张只出形与纸纹 —— 一块图盖全部 18 种格子类型。
pub fn board_plate() -> Option<String> {
    on_disk("textures/board/plate.png")
}

/// 棋盘地面（`textures/board/ground.png`，满幅无缝纸面，给环内环外那些空格）。
pub fn board_ground() -> Option<String> {
    on_disk("textures/board/ground.png")
}

/// 道路单元图（`textures/road/units/<键>.png`，256²，俯视平面：直路／直角／丁字／断头／斜路／45° 弯）。
/// 与 `board_plate` 不同一条纪律：那一张是「缺一块少一块」的皮肤，这六张是**一套接边表**，
/// 缺哪张路面就在那一格断缝，所以成套才上 —— 判据在 `render::RoadTiles::table`。
pub fn road_unit(key: &str) -> Option<String> {
    on_disk(&format!("textures/road/units/{key}.png"))
}

/// 单元图之外那大片草坪（`textures/road/grass.png`）。草坪均值与六张单元图里的草是同一个数
/// (116,135,61)，这是「无缝」唯一的前提。
pub fn road_grass() -> Option<String> {
    on_disk("textures/road/grass.png")
}

/// 地块边界的粉笔方框（`textures/board/chalk_frame.png`，透明底，只画四条边）。给 1×1 的地。
pub fn chalk_frame() -> Option<String> {
    on_disk("textures/board/chalk_frame.png")
}

/// 粉笔角片（`textures/board/chalk_corner.png`，透明底，左上角那两条边）。
/// 2×2 的地用四张各转一档，拼出的是一整圈**外框**而不是田字格 —— 中间那道缝不该有线。
pub fn chalk_corner() -> Option<String> {
    on_disk("textures/board/chalk_corner.png")
}

/// 住宅建筑模型（`model/dwell_L<level>.glb`，level 1..=5 正好对上 `BuildingLevel` 的
/// 1..=5）。与贴图同一条闸门：盘上没有就退回程序化方体，不报错。
///
/// **成套才上**的判据不在这里 —— 这里是逐级问，成套由 [`crate::render::BuildingModels`] 判。
pub fn dwell_model(level: u8) -> Option<String> {
    on_disk(&format!("model/dwell_L{level}.glb"))
}

/// 特殊格薄饼上的圆形图标（`textures/disc/<格类型>.png`，圆外真透明）。
///
/// 与道路单元不同一条纪律：那六张是接边表，缺一就断缝；这十六张各盖各的饼，
/// 缺一张只是那一格没花纹、饼还在。所以按键单独问，不做「成套才上」。
pub fn disc_icon(key: &str) -> Option<String> {
    on_disk(&format!("textures/disc/{key}.png"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_roster_name_wins_over_the_seat() {
        // 座位 0 本来该领姬百年，而他叫温久枝
        assert_eq!(sprite_key("温久枝", 0), "wenjiuzhi");
        assert_eq!(sprite_key("滑铁卢", 3), "huatielu");
    }

    #[test]
    fn an_unknown_name_picks_by_seat_and_two_seats_never_pick_the_same_face() {
        // 表里查不到的名字（测试夹具、临时对手）按座位领人
        assert_eq!(sprite_key("甲", 0), "jibainian");
        assert_eq!(sprite_key("乙", 1), "laixi");
        let first: Vec<&str> = (0..ROSTER.len()).map(|s| sprite_key("无名", s)).collect();
        let mut sorted = first.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ROSTER.len(), "有一张脸被两位同时领走了");
    }

    #[test]
    fn the_seat_wraps_instead_of_running_off_the_table() {
        assert_eq!(sprite_key("无名", ROSTER.len()), sprite_key("无名", 0));
    }
}
