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

/// 同一个人的四种姿态，各是一条横条。
///
/// `Stand` 就是现有那批 1024×128 的待机条，**文件名不带后缀** —— 已经烘出来的二十几条不用改名。
/// 其余三条各带一个后缀，而盘上没有是正常状态：素材不入库，而这一轮管线只跑到待机。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pose {
    Stand,
    Walk,
    Bike,
    Car,
}

impl Pose {
    /// 横条后缀，与 [`Pose::FRAMES`] 同序。
    const SUFFIX: [&'static str; 4] = ["", "_walk", "_bike", "_car"];
    /// 表序 == [`Pose::index`]，`render::token` 那个 `[Option<Track>; 4]` 靠它对齐。
    pub const ALL: [Pose; 4] = [Pose::Stand, Pose::Walk, Pose::Bike, Pose::Car];
    /// 一条横条切几帧。
    ///
    /// 这是**设计常量而不是素材自述**：PNG 只说得出总宽，说不出「分成几格」，
    /// 而 cell 宽 = 总宽 / 帧数。bake 那边按这个数出宽度，代码这边按它切 uv ——
    /// 两边唯一的耦合就是这两个数，写进文件头比写进 JSON 更难悄悄跑掉。
    const FRAMES: [usize; 4] = [8, 8, 4, 4];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn frames(self) -> usize {
        Self::FRAMES[self.index()]
    }

    fn path(self, key: &str) -> String {
        format!("sprite/{key}{}.png", Self::SUFFIX[self.index()])
    }
}

/// 一个人身上那四条横条在盘上的路径。`stand` 是唯一的必需项 ——
/// 缺它整个返回值是 `None`（这颗棋子退回球），缺其余三条只是那一种姿态退回站立。
pub struct Strips {
    pub stand: String,
    pub walk: Option<String>,
    pub bike: Option<String>,
    pub car: Option<String>,
}

impl Strips {
    /// 按姿态取路径；`Stand` 之外缺条就 `None`，降级由调用方决定。
    pub fn get(&self, pose: Pose) -> Option<&str> {
        let p = match pose {
            Pose::Stand => Some(&self.stand),
            Pose::Walk => self.walk.as_ref(),
            Pose::Bike => self.bike.as_ref(),
            Pose::Car => self.car.as_ref(),
        };
        p.map(String::as_str)
    }
}

/// 一条横条的 cell 像素尺寸：总宽按帧数平分，总高就是一格的高。
///
/// 待机是 128×128（正方形，与从前一样）；载具那条会算出 192×128 —— 车身比人宽，
/// 而**人不能因为上了车就变矮**，所以多出来的是横向而不是缩放，见 `render::token` 的 quad。
pub fn cell(size: (u32, u32), frames: usize) -> Option<(f32, f32)> {
    let (w, h) = size;
    let frames = frames.max(1) as f32;
    (w > 0 && h > 0).then(|| (w as f32 / frames, h as f32))
}

/// 横条文件的像素尺寸：只读 IHDR 那 24 个字节，不解码像素。
/// 读不到就 `None`，调用方按方形 cell 退 —— 与其余闸门同一条纪律，一声不响。
pub fn png_size(rel: &str) -> Option<(u32, u32)> {
    let mut head = [0u8; 24];
    let mut file = std::fs::File::open(asset_root().join(rel)).ok()?;
    std::io::Read::read_exact(&mut file, &mut head).ok()?;
    ihdr_size(&head)
}

/// PNG 头 24 字节 → 宽高。块布局是「签名 8 + 长度 4 + 类型 4 + 宽 4 + 高 4」，
/// 所以宽高在 16 与 20 —— 按「签名之后就是 IHDR」去数会差 4 格，而差的那 4 格读出来
/// 是一个看着很合理的数（长度字段恒为 13）。
fn ihdr_size(head: &[u8]) -> Option<(u32, u32)> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let h = head.get(..24)?;
    if h[..8] != SIG || h[12..16] != *b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(h[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(h[20..24].try_into().unwrap());
    Some((w, h))
}

/// 诨名 → sprite 键 + **语音目录**。表序同时是「名字对不上时按座位领人」的领人顺序。
///
/// 十六位 = `CHARACTERS.md` §五 首发十二人 + 隐藏位沈万三 + §六 三位已烘出来的候补。
/// 演示盘的默认阵容（姬百年／范一文）就在这份表里，名字与脸一一对上；表里查不到的
/// 名字（测试夹具、P14 之后临时摆上来的对手）才走按座位那一支。
///
/// 第三列是语音素材的目录名（`audio/voice/<列>/`）。它与 sprite 键**不是同一套拼写**
/// （`a-xiang` vs `axiang`），是按音节重新断过的，所以另起一列而不是加个前缀算出来；
/// 空串 = 这一位没有录音（隐藏位与三位候补），静默是设计而不是缺素材。
const ROSTER: [(&str, &str, &str); 16] = [
    ("姬百年", "jibainian", "ji-bainian"),
    ("莱息", "laixi", "lai-xi"),
    ("玛租", "mazu", "ma-zu"),
    ("贺兰山", "helanshan", "he-lanshan"),
    ("温久枝", "wenjiuzhi", "wen-jiuzhi"),
    ("范一文", "fanyiwen", "fan-yiwen"),
    ("多吉", "duoji", "duoji"),
    ("阿香", "axiang", "a-xiang"),
    ("陆仁佳", "lvrenjia", "lu-renjia"),
    ("塔萝", "taluo", "ta-luo"),
    ("索元", "suoyuan", "suo-yuan"),
    ("滑铁卢", "huatielu", "hua-tielu"),
    ("沈万三", "shenwansan", ""),
    ("包太太", "baotai", ""),
    ("雷当", "leidang", ""),
    ("葛守田", "geshoutian", ""),
];

/// 这一位该用哪条待机横条（`sprite/<键>.png`）。
pub fn strip_for(name: &str, seat: usize) -> Option<String> {
    resolve_key(name, seat).map(|key| Pose::Stand.path(key))
}

/// 这一位的四条横条，逐条问盘。键与 [`strip_for`] 同一条解析，所以「名字查得到而待机缺」
/// 仍然是整颗退回球 —— 不会出现「有步行条却没有脸」那种半套人。
pub fn strips_for(name: &str, seat: usize) -> Option<Strips> {
    let key = resolve_key(name, seat)?;
    let probe = |pose: Pose| on_disk(&pose.path(key));
    Some(Strips {
        stand: Pose::Stand.path(key),
        walk: probe(Pose::Walk),
        bike: probe(Pose::Bike),
        car: probe(Pose::Car),
    })
}

/// 键的解析：`seat` 只在名字查不到时用来分人，保证同桌两位不会领到同一张脸。
/// 盘上缺某几条时往后顺延一位 —— 少一条素材不该让某一颗棋子独自退回球；但**名字查得到
/// 而文件缺**时宁可退回球，也不换一张脸：那会把「温久枝」画成「姬百年」，而棋盘上没人看得出。
fn resolve_key(name: &str, seat: usize) -> Option<&'static str> {
    let known = ROSTER.iter().any(|(who, ..)| *who == name);
    if known {
        // 名单里的人只认自己那张脸：待机条缺就退回球，不顺延到隔壁那张
        let key = sprite_key(name, seat);
        return on_disk(&Pose::Stand.path(key)).map(|_| key);
    }
    (0..ROSTER.len())
        .map(|k| sprite_key(name, seat + k))
        .find(|key| on_disk(&Pose::Stand.path(key)).is_some())
}

/// 名字 → 横条键。不碰盘，所以能单测（`assets/sprite/` 不入库，「盘上有没有」在别的
/// 机器上是另一种答案，把它写进断言就是写一条只有这台机器会跑的测试）。
fn sprite_key(name: &str, seat: usize) -> &'static str {
    ROSTER
        .iter()
        .find(|(who, ..)| *who == name)
        .map_or(ROSTER[seat % ROSTER.len()].1, |(_, key, _)| *key)
}

/// 名字 → 角色键。sprite 横条与归属标志物共用这一套键。
///
/// ⚠️ 与 [`strip_for`] 的兜底**不一样**：这里查不到就是查不到，不按座位换人。
/// 领错一张脸至少还是「一个人」，领错一件道具等于在别人的地上插了你的手杖 ——
/// 那是给玩家的一条假信息，比没有标记更糟。
pub fn prop_key(name: &str) -> Option<&'static str> {
    ROSTER
        .iter()
        .find(|(who, ..)| *who == name)
        .map(|(_, k, _)| *k)
}

/// 名字 → 语音目录键（`audio/voice/<键>/`）。
///
/// 查不到（无名座位、测试夹具）与没有录音（隐藏位与三位候补）都是 `None`：静默是设计，
/// 不 `warn!` —— 每局每件事喊一遍会把日志刷满，而这条判据本来就允许「说不出话的人」。
pub fn voice_key(name: &str) -> Option<&'static str> {
    ROSTER
        .iter()
        .find(|(who, ..)| *who == name)
        .map(|(_, _, voice)| *voice)
        .filter(|voice| !voice.is_empty())
}

/// 全表角色键，装载器按它逐键问盘上有没有。
pub fn roster_keys() -> impl Iterator<Item = &'static str> {
    ROSTER.iter().map(|(_, key, _)| *key)
}

/// 归属标志物：`prop/<键>.png`（本体）与 `prop/<键>_edge.png`（归属色蒙版）。
///
/// **成套才上**：只有本体没有蒙版时，那块地看上去只是「地上放了个东西」，而不是
/// 「这是谁的地」—— 归属色正是这层素材存在的全部理由，缺一张就两张都不给。
pub fn prop(key: &str) -> Option<(String, String)> {
    let body = on_disk(&format!("prop/{key}.png"))?;
    let edge = on_disk(&format!("prop/{key}_edge.png"))?;
    Some((body, edge))
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

/// 道路单元图（`textures/road/units/<键>.png`，256²，俯视平面）。街面只用 `straight_orth`
/// 这一张，每格按街道朝向转 —— 斜街与直街用同一张图，没有第二套接边词汇表。
pub fn road_unit(key: &str) -> Option<String> {
    on_disk(&format!("textures/road/units/{key}.png"))
}

/// 整盘垫底的草坪（`textures/road/grass.png`，无缝）。每格 UV 各自 0..1 平铺。
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

    /// 十二位有录音、四位没有；语音目录的拼写与 sprite 键不是同一套（`a-xiang` vs `axiang`）。
    #[test]
    fn only_the_twelve_voiced_characters_have_a_voice_directory() {
        let voiced: Vec<&str> = ROSTER
            .iter()
            .filter_map(|(who, ..)| voice_key(who))
            .collect();
        assert_eq!(voiced.len(), 12, "有录音的该是十二位：{voiced:?}");
        assert_eq!(voice_key("阿香"), Some("a-xiang"));
        assert_eq!(voice_key("陆仁佳"), Some("lu-renjia"));
        assert_eq!(voice_key("姬百年"), Some("ji-bainian"));
        // 隐藏位与三位候补没有录音：静默是设计
        assert_eq!(voice_key("沈万三"), None);
        assert_eq!(voice_key("包太太"), None);
        // 表里查不到的名字（测试夹具、临时对手）也是静默，而不是按座位领一个别人的嗓子
        assert_eq!(voice_key("甲"), None);
        // `Some("")` 这种半截答案不许漏出来 —— 它会拼出一个目录级的路径
        assert!(voiced.iter().all(|key| !key.is_empty()));
    }

    #[test]
    fn the_moving_poses_hang_off_the_same_key_as_the_face() {
        // 待机条不带后缀：已经烘出来的那二十几条不改名
        assert_eq!(Pose::Stand.path("jibainian"), "sprite/jibainian.png");
        assert_eq!(Pose::Walk.path("jibainian"), "sprite/jibainian_walk.png");
        assert_eq!(Pose::Car.path("jibainian"), "sprite/jibainian_car.png");
    }

    #[test]
    fn the_pose_table_indexes_itself() {
        // `[Option<Track>; 4]` 按 `index()` 取，而 `ALL` 是装配那一头写的表序。
        // 两者一错开，症状是「步行放的是汽车那几帧」——画面会动，没人看得出是串了。
        for (i, pose) in Pose::ALL.iter().enumerate() {
            assert_eq!(pose.index(), i);
        }
    }

    #[test]
    fn a_wider_cell_is_wider_and_not_shorter() {
        // 载具条 768×128 切四帧：一格 192 宽、128 高 —— 车身横向溢出，人的身高一个像素不挪
        assert_eq!(cell((768, 128), Pose::Car.frames()), Some((192.0, 128.0)));
        assert_eq!(
            cell((1024, 128), Pose::Stand.frames()),
            Some((128.0, 128.0))
        );
    }

    #[test]
    fn a_header_that_is_not_a_png_has_no_size() {
        let mut head = [0u8; 24];
        head[..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        head[12..16].copy_from_slice(b"IHDR");
        head[16..20].copy_from_slice(&768u32.to_be_bytes());
        head[20..24].copy_from_slice(&128u32.to_be_bytes());
        assert_eq!(ihdr_size(&head), Some((768, 128)));
        // 签名对、块名不对（比如把调色板块当头）→ 没有尺寸，而不是读出隔壁块的数
        head[12..16].copy_from_slice(b"PLTE");
        assert_eq!(ihdr_size(&head), None);
        assert_eq!(ihdr_size(&[0u8; 23]), None);
    }
}
