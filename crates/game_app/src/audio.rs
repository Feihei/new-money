//! 音频（P16a）：BGM 的场景机与世界播放列表。
//!
//! 分层红线照旧：规则层不知道有声音这东西。这一模块**只读**世界状态（在哪一屏、开着
//! 哪扇窗口、按没按暂停），一个字都不写回去 —— 于是 `NM_AUDIO=0` 把 bevy 的 `AudioPlugin`
//! 整个不装之后，游戏行为一个字节都不变（截图与无人值守冒烟走的正是这条路）。
//!
//! 三条槽而不是一个「当前曲目」：
//! - **世界**那一格是播放列表（[`WORLD_LIST`]）：每首 `Once`，放完换下一首，走完从头再来。
//! - **事件**那一格（柜台／彩票／魔法屋／保释与拍卖／游乐场）另起一条 `Loop`，而世界那条
//!   **按住不销毁** —— 拍板是「事件结束切回世界时从刚才的断点继续」，落在这里靠的是
//!   rodio 的 `pause` 不丢播放位置，而不是重新起一条再用 `skip_duration` 追那个数。
//! - **独占**那一格给菜单与终局：它们把前两条清空。
//!
//! 「该有什么」写在 [`Bgm`]（意图），「实际有什么」是贴在实体上的 [`BgmTag`]。分成两侧有两条理由：
//! ① `NM_AUDIO=0` 时意图照常算、照常记日志，于是无人值守能验到「换场景就该换曲」这条**判据**，
//! 而不只是验「没崩」；② 判据因此全是纯函数，能在无头 `App` 里逐格钉住 —— `AudioSink` 要真
//! 设备加已加载的资产，测试里摆不出来（而 `check.sh` 是无头的）。

use std::env;

use bevy::{
    asset::LoadState,
    audio::Volume,
    prelude::{
        App, AssetServer, AudioPlayer, AudioSink, AudioSinkPlayback, Commands, Component, Entity,
        IntoScheduleConfigs, PlaybackSettings, Plugin, Query, Res, ResMut, Resource, State, Update,
        debug, info, warn,
    },
};
use game_core::prelude::{Paused, Pending, PendingDecision, TurnPhase};

use crate::scene::Scene;
use crate::ui::finance::StockPanelOpen;

/// 曲库在资产根下的位置。`main.rs::asset_root` 把资产根钉成绝对路径（指向仓库的 `assets/`），
/// 所以这里写的是相对它的那一段。
const BGM_DIR: &str = "audio/bgm";

/// 菜单与终局那两首独占整屏的曲子。
const MENU_TRACK: &str = "bgm_01_menu";
const WIN_TRACK: &str = "bgm_04_win";

/// 世界场景的播放列表。开局与读档都从第 0 格起，走完回到第 0 格。
const WORLD_LIST: [&str; 5] = [
    "bgm_02_town",
    "bgm_05_fiesta",
    "bgm_09_catchy",
    "bgm_10_candy",
    "bgm_11_simple",
];

/// BGM 总线的默认增益。拍板是「三条总线各五档，默认 BGM 50／音效 75／语音 100」，
/// 依据是盘上素材的实测响度（BGM 约 −14.5 LUFS，比 UI 音效的 −6 dBFS 低一截）；
/// 档位与设置文件那一半接进来之前，这里就是那一句「50」。
const BGM_GAIN: f32 = 0.5;

/// 事件场景的五套曲子。**只有整屏场景换曲**，判据在 [`event_music`] 那张穷尽表里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventMusic {
    /// 银行柜台／百货／证券浮层（那三块都是「钱在动」的屏）
    Market,
    /// 彩票中心
    Fortune,
    /// 魔法屋
    Night,
    /// 保释（监狱与医院共用）与拍卖
    Chase,
    /// 游乐场
    Fiesta,
}

impl EventMusic {
    /// 五格清单。生产代码不需要它（换曲的判据全在 [`event_music`] 那张穷尽表里），
    /// 只有「表要覆盖全部」那两条用例逐格走一遍 —— 所以挂在 `cfg(test)` 下，
    /// 不然非测试构建里它就是一条永远不响的死代码。
    #[cfg(test)]
    const ALL: [EventMusic; 5] = [
        EventMusic::Market,
        EventMusic::Fortune,
        EventMusic::Night,
        EventMusic::Chase,
        EventMusic::Fiesta,
    ];

    fn track(self) -> &'static str {
        match self {
            EventMusic::Market => "bgm_03_market",
            EventMusic::Fortune => "bgm_07_fortune",
            EventMusic::Night => "bgm_06_night",
            EventMusic::Chase => "bgm_08_chase",
            EventMusic::Fiesta => "bgm_05_fiesta",
        }
    }

    /// 日志里那一格写什么。与文件名的后缀同字面量，`grep` 得到也认得出。
    fn label(self) -> &'static str {
        match self {
            EventMusic::Market => "event market",
            EventMusic::Fortune => "event fortune",
            EventMusic::Night => "event night",
            EventMusic::Chase => "event chase",
            EventMusic::Fiesta => "event fiesta",
        }
    }
}

/// 独占整屏的两首。胜利那一首发完就静音，**不回世界列表**（拍板）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Solo {
    Menu,
    Win,
}

impl Solo {
    fn track(self) -> &'static str {
        match self {
            Solo::Menu => MENU_TRACK,
            Solo::Win => WIN_TRACK,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Solo::Menu => "menu",
            Solo::Win => "win",
        }
    }
}

/// 现在该听哪一处音乐。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicScene {
    Menu,
    World,
    Event(EventMusic),
    Win,
}

/// 目标场景怎么算：一条优先级链 —— **菜单 ＞ 终局 ＞ 事件窗口 ＞ 证券浮层 ＞ 世界**。
///
/// 参数是值而不是 `Res`：整张表因此能在无头测试里逐格钉住。
fn music_target(
    scene: Scene,
    over: bool,
    pending: Option<&Pending>,
    stock_open: bool,
) -> MusicScene {
    // 菜单压过一切：回主菜单时回合机已经停在 `Boot`，而 `TurnPhase::Over` 那一格
    // 是上一局留下的事实（清场要等 `Last` 里的 `clear_match`），认它会截到一张胜利曲
    if scene == Scene::Menu {
        return MusicScene::Menu;
    }
    if over {
        return MusicScene::Win;
    }
    if let Some(music) = pending.and_then(event_music) {
        return MusicScene::Event(music);
    }
    if stock_open {
        return MusicScene::Event(EventMusic::Market);
    }
    MusicScene::World
}

/// 十六格待决事项 → 换不换曲。**没有 `_`**：新增一种窗口必须在这里当场表态，
/// 否则它会静悄悄地把某一首事件曲让给世界列表（本仓库的穷尽表房规）。
fn event_music(pending: &Pending) -> Option<EventMusic> {
    match pending {
        // 六个整屏窗口
        Pending::BankTeller | Pending::Shop => Some(EventMusic::Market),
        Pending::Lottery => Some(EventMusic::Fortune),
        Pending::MagicSpell(_) => Some(EventMusic::Night),
        Pending::Bail(_) | Pending::Auction => Some(EventMusic::Chase),
        Pending::Minigame(_) => Some(EventMusic::Fiesta),
        // 小模态：画面只占中间一块，世界那一首继续在底下跑
        Pending::Buy(_)
        | Pending::Upgrade(_)
        | Pending::Build(_)
        | Pending::RemoteBuild
        | Pending::RentNotice(_)
        | Pending::ServiceNotice(_)
        | Pending::CompanyNotice
        | Pending::Card
        | Pending::Passive => None,
    }
}

/// 曲目键 → 资产路径。全部 `.ogg`：Bevy 默认只开 `vorbis` 解码器，写成 `.wav` 的症状
/// 是一声不出而一个字不报。
fn track_path(track: &str) -> String {
    format!("{BGM_DIR}/{track}.ogg")
}

/// 一条 BGM 实体身上贴的标签：它替哪一格说话，以及**替的是哪一首**。
///
/// 带上曲目身份是这里的要害：世界列表推进一格，旧标签就「不再被要」，而新标签「还没有实体」
/// —— 换曲因此不需要一段专门的代码，它是协调那两句的自然结果。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgmTag {
    World(usize),
    Event(EventMusic),
    Solo(Solo),
}

impl BgmTag {
    fn track(self) -> &'static str {
        match self {
            BgmTag::World(index) => WORLD_LIST[index],
            BgmTag::Event(music) => music.track(),
            BgmTag::Solo(solo) => solo.track(),
        }
    }

    fn loops(self) -> bool {
        match self {
            BgmTag::World(_) | BgmTag::Solo(Solo::Win) => false,
            BgmTag::Event(_) | BgmTag::Solo(Solo::Menu) => true,
        }
    }
}

/// 三条槽的**意图**。`Default` 是全空 —— 第一帧的 `aim_bgm` 会按目标场景填上，
/// 而菜单是默认场景，所以开局那一首就是菜单曲。
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Bgm {
    /// 世界列表里当前该播的那一格
    world: Option<usize>,
    event: Option<EventMusic>,
    solo: Option<Solo>,
    /// 暂停闸门在本模块的投影。与 `Paused` 每帧同步，单独记一格是为了让
    /// 「暂停/恢复」这一行日志在 `NM_AUDIO=0`（根本没有 sink）时照样说得出来
    paused: bool,
    /// 素材加载失败过一次就不再起新的：`assets/` 整目录不入库，缺文件是常态而不是意外，
    /// 而 Bevy 对失败资产只打一条日志然后永远等下去
    silent: bool,
}

impl Bgm {
    /// 把三条意图对齐到目标场景。**事件场景不清世界那一格** —— 那一句拍板
    /// （「切回世界从刚才的断点继续」）在代码里的形状就是这里没有 `self.world = None`。
    fn aim(&mut self, target: MusicScene) {
        match target {
            MusicScene::Menu | MusicScene::Win => {
                self.world = None;
                self.event = None;
                self.solo = Some(match target {
                    MusicScene::Win => Solo::Win,
                    _ => Solo::Menu,
                });
            }
            MusicScene::World => {
                self.solo = None;
                self.event = None;
                self.reach_world();
            }
            MusicScene::Event(music) => {
                self.solo = None;
                self.reach_world();
                self.event = Some(music);
            }
        }
    }

    /// 世界列表要有内容；没有就是**刚进对局**，从第一格起（拍板：开局／读档从 02 开始）。
    fn reach_world(&mut self) {
        self.world.get_or_insert(0);
    }

    /// 这一首放完了，推到下一格（走完从头再来）。菜单与终局没有世界格可推，空转。
    fn advance_world(&mut self) {
        if let Some(index) = self.world {
            self.world = Some((index + 1) % WORLD_LIST.len());
        }
    }

    /// 该存在的实体标签，最多两条（世界 + 事件）；独占屏只有它自己那一条。
    fn wanted(&self) -> [Option<BgmTag>; 2] {
        match self.solo {
            Some(solo) => [Some(BgmTag::Solo(solo)), None],
            None => [self.world.map(BgmTag::World), self.event.map(BgmTag::Event)],
        }
    }

    /// 这一条标签现在该不该存在 —— 与 [`Self::wanted`] 同一个判据，写在两处会分叉。
    fn wants(&self, tag: BgmTag) -> bool {
        self.wanted().into_iter().flatten().any(|slot| slot == tag)
    }

    /// 现在**该出声**的那一条：`(日志里的槽名, 曲目键)`。判据与 [`Self::audible`] 同源，
    /// 两处各写一遍就会在「事件场景里世界还在」那一格分叉。
    fn sounding(&self) -> Option<(&'static str, &'static str)> {
        match (self.solo, self.event, self.world) {
            (Some(solo), _, _) => Some((solo.label(), solo.track())),
            (None, Some(music), _) => Some((music.label(), music.track())),
            (None, None, Some(index)) => Some(("world", WORLD_LIST[index])),
            (None, None, None) => None,
        }
    }

    /// 这一条槽该不该出声。事件盖住世界，独占屏盖住一切，暂停闸门由调用方另问。
    fn audible(&self, tag: BgmTag) -> bool {
        match tag {
            BgmTag::World(_) => self.event.is_none() && self.solo.is_none(),
            BgmTag::Event(_) => self.solo.is_none(),
            BgmTag::Solo(_) => true,
        }
    }
}

/// 两个开发旋钮，启动时读一次。
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
struct AudioConfig {
    /// 装不装真出声：`NM_AUDIO=0` 时主入口连 bevy 的 `AudioPlugin` 都不装，
    /// 本模块也算意图、也记日志，但一个实体都不起。
    playback: bool,
    /// 每一首换曲、每一次暂停各记一行。
    log: bool,
}

impl AudioConfig {
    fn from_env() -> Self {
        Self {
            playback: playback_on(env::var("NM_AUDIO").ok().as_deref()),
            log: log_on(env::var("NM_AUDIO_LOG").ok().as_deref()),
        }
    }
}

/// `NM_AUDIO=0` 关出声。**只有字面的 `0` 关**：静音是看得见的损失，而旋钮名拼错一个字母
/// 不该让整局没声（与 `NM_DUMP` 那条「读不懂的值退回默认」同一个方向，只是这里的默认是开）。
fn playback_on(env: Option<&str>) -> bool {
    !matches!(env.map(str::trim), Some("0"))
}

/// `NM_AUDIO_LOG=1` 开决策日志。**只有字面的 `1` 开**（日志是默认关的那一类）。
fn log_on(env: Option<&str>) -> bool {
    matches!(env.map(str::trim), Some("1"))
}

/// 主入口要在 `add_plugins` **之前**问这一次：`disable` 是 `PluginGroupBuilder` 上的方法，
/// 插件装下去之后就退不回来了。判据与 [`AudioConfig::from_env`] 同一条，不另抄一份。
pub(crate) fn playback_on_from_env() -> bool {
    playback_on(env::var("NM_AUDIO").ok().as_deref())
}

/// 每帧算目标场景，把三条意图对齐过去；只在**该出声的那一条**真的换了时记一行。
///
/// 先 `aim` 再让别的 system 动实体：意图是唯一的真相来源，`play_bgm` 与 `duck_bgm` 都只读它。
fn aim_bgm(
    mut bgm: ResMut<Bgm>,
    scene: Res<State<Scene>>,
    phase: Res<State<TurnPhase>>,
    paused: Res<Paused>,
    pending: Res<PendingDecision>,
    stock: Res<StockPanelOpen>,
    cfg: Res<AudioConfig>,
) {
    let was = (bgm.sounding(), bgm.paused);
    bgm.aim(music_target(
        *scene.get(),
        phase.get() == &TurnPhase::Over,
        pending.about.as_ref(),
        stock.0,
    ));
    bgm.paused = paused.0;
    if !cfg.log {
        return;
    }
    let now = (bgm.sounding(), bgm.paused);
    if now.0 != was.0
        && let Some((slot, track)) = now.0
    {
        info!("audio: bgm {slot} -> {track}");
    }
    if now.1 != was.1 {
        info!("audio: bgm {}", if now.1 { "pause" } else { "resume" });
    }
}

/// 世界列表里那一首**放完**（`Once` 播到底）就推进一格；素材**加载失败**则整块关掉。
///
/// 这里只改意图，不在同一格里起下一条：起与停是 `play_bgm` 的活（它要 `AssetServer`）。
/// 于是「放完换下一首」在 `NM_AUDIO=0` 下照样发生、照样记日志 —— 世界列表的循环因此
/// 能被一条无声的长跑验到。
fn retire_finished_world(
    mut bgm: ResMut<Bgm>,
    cfg: Res<AudioConfig>,
    server: Res<AssetServer>,
    live: Query<(&BgmTag, &AudioPlayer, Option<&AudioSink>)>,
) {
    if !cfg.playback || bgm.silent {
        return;
    }
    for (tag, player, sink) in &live {
        if let LoadState::Failed(why) = server.load_state(&player.0) {
            warn!(
                "audio: BGM 素材加载失败：{}（{why}）—— 本进程不再起新的 BGM",
                tag.track()
            );
            bgm.silent = true;
            return;
        }
        // 暂停中的那条不算「放完」：`empty()` 在 pause 期间不会是 `true`，但那句判据
        // 是 rodio 的行为而不是我们的规则，把它写在这一格里才不会被下一次升级静悄悄改掉
        let BgmTag::World(index) = *tag else { continue };
        if Some(index) == bgm.world && !bgm.paused && sink.is_some_and(|sink| sink.empty()) {
            bgm.advance_world();
            if cfg.log
                && let Some((slot, track)) = bgm.sounding()
            {
                info!("audio: bgm {slot} -> {track}");
            }
        }
    }
}

/// 把意图落成实体：缺的起一条，多的收走。
///
/// 协调比的是**标签**而不是句柄，所以「换一首」「进事件」「回菜单」三条路共用这一段。
/// 同一帧里旧标签被收走、新标签被起出来是安全的：`Commands` 在下一条 system 之前落地，
/// 而两者本来就不同名。
fn play_bgm(
    mut commands: Commands,
    bgm: Res<Bgm>,
    cfg: Res<AudioConfig>,
    server: Res<AssetServer>,
    live: Query<(Entity, &BgmTag)>,
) {
    // 连 `AudioSource` 这份资产类型都没注册（bevy 的 `AudioPlugin` 没装），所以这里
    // 一个 `load` 都不能发 —— 而实体也不会存在，收尸那一步自然无事可做
    if !cfg.playback {
        return;
    }
    // 素材加载失败过：把还在播的收走，之后不再起新的。`warn!` 已经在
    // [`retire_finished_world`] 里喊过一次，这里只负责收尸，不再重复
    if bgm.silent {
        for (entity, _) in &live {
            commands.entity(entity).despawn();
        }
        return;
    }
    for (entity, tag) in &live {
        if !bgm.wants(*tag) {
            commands.entity(entity).despawn();
        }
    }
    for slot in bgm.wanted().into_iter().flatten() {
        if live.iter().any(|(_, live_tag)| *live_tag == slot) {
            continue;
        }
        let base = if slot.loops() {
            PlaybackSettings::LOOP
        } else {
            PlaybackSettings::ONCE
        };
        let with_volume = base.with_volume(Volume::Linear(BGM_GAIN));
        // 出生时就按它该在的状态起：事件场景里被按住的那条世界槽、暂停中进的对局都是
        let settings = if bgm.audible(slot) && !bgm.paused {
            with_volume
        } else {
            with_volume.paused()
        };
        commands.spawn((
            AudioPlayer::new(server.load(track_path(slot.track()))),
            settings,
            slot,
        ));
    }
}

/// 谁该出声：事件场景把世界那条按住（不销毁，位置留着），暂停闸门把全部按住。
///
/// 每条都只在状态不一致时才调 `pause`/`play` —— 那两个方法是发到音频线程的，
/// 每帧无脑调一次会白占那条线程（而症状是噪音与延迟，不是错）。
fn duck_bgm(bgm: Res<Bgm>, paused: Res<Paused>, live: Query<(&AudioSink, &BgmTag)>) {
    for (sink, tag) in &live {
        let hold = !bgm.audible(*tag) || paused.0;
        if sink.is_paused() != hold {
            if hold {
                sink.pause();
            } else {
                sink.play();
            }
        }
    }
}

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        let cfg = AudioConfig::from_env();
        debug!(
            "音频：出声{}，决策日志{}",
            if cfg.playback {
                "开"
            } else {
                "关（NM_AUDIO=0）"
            },
            if cfg.log { "开" } else { "关" }
        );
        app.insert_resource(cfg)
            .init_resource::<Bgm>()
            // 严格串起来：意图 → 退役 → 起停 → 出声。并行元组里没有顺序可言
            // （§6.1 坑 36），而这一条链上每一步都读上一步写的那个意图
            .add_systems(
                Update,
                (aim_bgm, retire_finished_world, play_bgm, duck_bgm).chain(),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        prelude::{AppExtStates, MinimalPlugins, NextState},
        state::app::StatesPlugin,
    };
    use game_core::prelude::{DetentionPlace, GameKind, LandId, MagicFilter};

    fn world(index: usize) -> Bgm {
        Bgm {
            world: Some(index),
            ..Default::default()
        }
    }

    /// 六个整屏窗口（七个变体）各自换哪一首，九种小模态一律不换。这张表是「事件场景」的定义本身。
    #[test]
    fn only_the_full_screen_scenes_change_the_track() {
        let cases: [(Pending, Option<EventMusic>); 17] = [
            (Pending::BankTeller, Some(EventMusic::Market)),
            (Pending::Shop, Some(EventMusic::Market)),
            (Pending::Lottery, Some(EventMusic::Fortune)),
            (
                Pending::MagicSpell(MagicFilter::AllGirls),
                Some(EventMusic::Night),
            ),
            (Pending::Bail(DetentionPlace::Jail), Some(EventMusic::Chase)),
            (
                Pending::Bail(DetentionPlace::Hospital),
                Some(EventMusic::Chase),
            ),
            (Pending::Auction, Some(EventMusic::Chase)),
            (
                Pending::Minigame(GameKind::PenguinDig),
                Some(EventMusic::Fiesta),
            ),
            (Pending::Buy(LandId(3)), None),
            (Pending::Upgrade(LandId(3)), None),
            (Pending::Build(LandId(3)), None),
            (Pending::RemoteBuild, None),
            (Pending::RentNotice(LandId(3)), None),
            (Pending::ServiceNotice(LandId(3)), None),
            (Pending::CompanyNotice, None),
            (Pending::Card, None),
            (Pending::Passive, None),
        ];
        for (pending, expected) in cases {
            assert_eq!(
                music_target(Scene::Board, false, Some(&pending), false),
                expected.map_or(MusicScene::World, MusicScene::Event),
                "窗口 {pending:?} 的目标曲目写错了"
            );
        }
    }

    /// 优先级链：菜单 ＞ 终局 ＞ 事件窗口。回菜单那一帧 `TurnPhase::Over` 还是上一局的值
    /// （清场排在 `Last`），认它就会在菜单上放胜利曲。
    #[test]
    fn the_priority_chain_is_menu_then_over_then_window() {
        assert_eq!(
            music_target(Scene::Menu, true, Some(&Pending::Lottery), true),
            MusicScene::Menu,
            "菜单上不该放事件曲，也不该放胜利曲"
        );
        assert_eq!(
            music_target(Scene::Board, true, Some(&Pending::Lottery), false),
            MusicScene::Win,
            "终局压过还挂着的窗口"
        );
        assert_eq!(
            music_target(Scene::Board, false, Some(&Pending::Lottery), true),
            MusicScene::Event(EventMusic::Fortune),
            "窗口压过证券浮层"
        );
        // 证券浮层是**整屏**的（盖住地图与浮层），所以它换曲；ATM／手牌／道具是浮层，
        // 而 `music_target` 根本没有那三样的参数 —— 少一条判据就是少一条出口
        assert_eq!(
            music_target(Scene::Board, false, None, true),
            MusicScene::Event(EventMusic::Market)
        );
        assert_eq!(
            music_target(Scene::Board, false, None, false),
            MusicScene::World
        );
    }

    /// 十一首曲子全部有归属：菜单 1 ＋ 终局 1 ＋ 世界列表 5 ＋ 事件场景 5，其中游乐场与
    /// 世界列表共用那一首，于是**不同的键**正好 11。少一格会在这里红，而不是变成一段静默的空白。
    #[test]
    fn every_track_on_disk_has_a_home() {
        let mut keys: Vec<&str> = vec![MENU_TRACK, WIN_TRACK];
        keys.extend(WORLD_LIST);
        keys.extend(EventMusic::ALL.map(EventMusic::track));
        let distinct: std::collections::HashSet<&str> = keys.iter().copied().collect();
        assert_eq!(keys.len(), 12, "菜单＋终局＋五格世界＋五个事件场景");
        assert_eq!(
            distinct.len(),
            11,
            "表里多出一处重名（世界列表与某一首事件曲共用那一首是唯一的合法重合）：{distinct:?}"
        );
    }

    /// 世界列表的顺序与回绕（走完从第一格再来）。第 0 格必须是 `bgm_02_town` ——
    /// 验收那句「世界曲在切换到对局后从 02 开始」钉的是它。
    #[test]
    fn the_world_list_is_five_tracks_and_wraps_back_to_the_first() {
        assert_eq!(WORLD_LIST[0], "bgm_02_town");
        assert_eq!(WORLD_LIST.len(), 5);
        let mut bgm = world(4);
        bgm.advance_world();
        assert_eq!(bgm.world, Some(0), "走完最后一格该回到第一格");
        // 菜单与终局没有世界格可推：空转而不是凭空起一条
        let mut idle = Bgm::default();
        idle.advance_world();
        assert_eq!(idle.world, None);
    }

    /// 路径拼写：`.ogg` 而不在 `assets/` 之下再套一层。Bevy 默认只开 `vorbis`，
    /// 键名写成 `.wav` 的症状是一声不出而一个字不报。
    #[test]
    fn tracks_resolve_to_ogg_under_the_bgm_dir() {
        assert_eq!(track_path("bgm_02_town"), "audio/bgm/bgm_02_town.ogg");
        for key in [MENU_TRACK, WIN_TRACK].iter().chain(WORLD_LIST.iter()) {
            assert!(track_path(key).ends_with(".ogg"), "{key} 不是 ogg");
        }
    }

    /// 事件场景**不清世界那一格**，出去时还是同一格 —— 这是「从刚才的断点继续」
    /// 在意图侧的全部形状（另一侧是 sink 的 pause 不丢位置）。
    #[test]
    fn an_event_scene_holds_the_world_slot_in_place() {
        let mut bgm = world(2);
        bgm.aim(MusicScene::Event(EventMusic::Chase));
        assert_eq!(bgm.world, Some(2), "进事件把世界格清了，出去就只能从头放");
        assert_eq!(bgm.event, Some(EventMusic::Chase));
        bgm.aim(MusicScene::World);
        assert_eq!(bgm.world, Some(2));
        assert_eq!(bgm.event, None);
        // 事件之间换曲不重开世界：拍卖轮转、魔法屋那一屏都靠这一条
        bgm.aim(MusicScene::Event(EventMusic::Fortune));
        assert_eq!(bgm.event, Some(EventMusic::Fortune));
        assert_eq!(bgm.world, Some(2));
    }

    /// 离开对局把世界格清掉，所以**重新进对局**从第一格起（开局／读档都从 02 开始）。
    #[test]
    fn leaving_the_board_rewinds_the_world_list() {
        let mut bgm = world(3);
        bgm.aim(MusicScene::Menu);
        assert_eq!(
            (bgm.world, bgm.event, bgm.solo),
            (None, None, Some(Solo::Menu))
        );
        bgm.aim(MusicScene::World);
        assert_eq!(bgm.world, Some(0), "第二局的世界曲该从第一格起");

        let mut bgm = world(1);
        bgm.aim(MusicScene::Win);
        assert_eq!(
            (bgm.world, bgm.event, bgm.solo),
            (None, None, Some(Solo::Win))
        );
    }

    /// 该出声的那一条与「该存在的实体」两处都从同一份意图算出来。
    /// 分开写就会在「事件场景里世界还在」那一格分叉 —— 日志说 A、实体放 B。
    #[test]
    fn the_wanted_slots_and_the_audible_one_agree() {
        assert_eq!(
            world(1).wanted(),
            [Some(BgmTag::World(1)), None],
            "世界场景只有一条实体"
        );
        let mut bgm = Bgm::default();
        bgm.aim(MusicScene::Event(EventMusic::Night));
        assert_eq!(
            bgm.wanted(),
            [
                Some(BgmTag::World(0)),
                Some(BgmTag::Event(EventMusic::Night))
            ],
            "事件场景里两条实体并存，下面那条被按住"
        );
        assert!(bgm.audible(BgmTag::Event(EventMusic::Night)));
        assert!(
            !bgm.audible(BgmTag::World(0)),
            "世界那条在事件场景里必须是被按住的，否则两首一起响"
        );
        assert_eq!(bgm.sounding(), Some(("event night", "bgm_06_night")));
        // `play_bgm` 收尸与起条读的是 `wants`，不是 `wanted` —— 那一句也要有牙
        assert!(bgm.wants(BgmTag::World(0)));
        assert!(bgm.wants(BgmTag::Event(EventMusic::Night)));
        assert!(
            !bgm.wants(BgmTag::Event(EventMusic::Market)),
            "另一首事件曲该被收走"
        );
        assert!(
            !bgm.wants(BgmTag::World(1)),
            "列表里别的那一格不是此刻该有的实体"
        );

        bgm.aim(MusicScene::Menu);
        assert_eq!(bgm.wanted(), [Some(BgmTag::Solo(Solo::Menu)), None]);
        assert!(bgm.audible(BgmTag::Solo(Solo::Menu)));
        // 日志的槽名是验收 grep 的字面量
        assert_eq!(bgm.sounding(), Some(("menu", "bgm_01_menu")));
    }

    /// 菜单／世界／事件三种场景下，`sounding()` 的曲目键与 `BgmTag::track()` 必须同一个数 ——
    /// 否则日志念的是 A，耳朵听到的是 B。
    #[test]
    fn the_log_key_names_the_track_that_actually_plays() {
        for (index, key) in WORLD_LIST.iter().enumerate() {
            assert_eq!(BgmTag::World(index).track(), *key);
            assert_eq!(world(index).sounding(), Some(("world", *key)));
        }
        for music in EventMusic::ALL {
            assert_eq!(BgmTag::Event(music).track(), music.track());
        }
        for solo in [Solo::Menu, Solo::Win] {
            assert_eq!(BgmTag::Solo(solo).track(), solo.track());
            let bgm = Bgm {
                solo: Some(solo),
                ..Default::default()
            };
            assert_eq!(bgm.sounding(), Some((solo.label(), solo.track())));
        }
    }

    /// 世界列表是 `Once`（要靠 `empty()` 换下一首），其余循环。胜利曲也是 `Once` ——
    /// 拍板那句「放完静音，不回世界列表」。
    #[test]
    fn only_the_world_list_and_the_win_track_are_played_once() {
        assert!(!BgmTag::World(0).loops());
        assert!(!BgmTag::Solo(Solo::Win).loops());
        assert!(BgmTag::Solo(Solo::Menu).loops());
        for music in EventMusic::ALL {
            assert!(BgmTag::Event(music).loops(), "{music:?} 是整屏场景，该循环");
        }
    }

    /// 旋钮：`NM_AUDIO=0` 才关出声，`NM_AUDIO_LOG=1` 才开日志。读不懂的值各退回自己的默认。
    #[test]
    fn the_two_knobs_only_answer_their_own_literal() {
        assert!(playback_on(None), "没设就该有声");
        assert!(!playback_on(Some("0")), "NM_AUDIO=0 该关");
        assert!(!playback_on(Some(" 0 ")), "旋钮值该剥空白");
        assert!(playback_on(Some("1")), "NM_AUDIO=1 是「开」而不是别的");
        assert!(playback_on(Some("no")), "拼错的值不该让整局没声");

        assert!(!log_on(None), "没设就不该刷日志");
        assert!(log_on(Some("1")));
        assert!(!log_on(Some("0")));
        assert!(!log_on(Some("yes")), "读不懂的值不算「开」");
    }

    /// 接线：`aim_bgm` 真的从场景与窗口算出意图，且**一帧之内**跟得上窗口。
    /// 上面的纯函数一条条都对，这一条不管内容对不对 —— 它管的是「有没有人调它」。
    #[test]
    fn the_director_follows_the_window_from_one_frame_to_the_next() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(StatesPlugin)
            .init_state::<Scene>()
            .init_state::<TurnPhase>()
            .init_resource::<Bgm>()
            .init_resource::<PendingDecision>()
            .init_resource::<Paused>()
            .insert_resource(StockPanelOpen(false))
            .insert_resource(AudioConfig {
                playback: false,
                log: false,
            })
            .add_systems(Update, aim_bgm);
        app.world_mut()
            .resource_mut::<NextState<Scene>>()
            .set(Scene::Board);
        app.update();
        app.update();
        assert_eq!(
            app.world().resource::<Bgm>().world,
            Some(0),
            "进对局第一帧之后世界格该是列表第 0 格"
        );

        // 开一扇整屏窗口：事件槽挂上，而世界格**留在原地**
        let somebody = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<PendingDecision>()
            .offer(somebody, Pending::Auction);
        app.update();
        let bgm = *app.world().resource::<Bgm>();
        assert_eq!(bgm.event, Some(EventMusic::Chase));
        assert_eq!(bgm.world, Some(0), "进拍卖把世界格挤掉了，出去就得从头放");

        // 关掉窗口 —— 回到世界，还是同一格
        app.world_mut().resource_mut::<PendingDecision>().clear();
        app.update();
        assert_eq!(app.world().resource::<Bgm>().event, None);
        assert_eq!(app.world().resource::<Bgm>().world, Some(0));

        // 暂停闸门：意图侧记一格，日志才在 `NM_AUDIO=0` 下也说得出「当前曲暂停了」
        *app.world_mut().resource_mut::<Paused>() = Paused(true);
        app.update();
        assert!(app.world().resource::<Bgm>().paused);

        // 终局：胜利曲独占，世界与事件两条都清走
        app.world_mut()
            .resource_mut::<NextState<TurnPhase>>()
            .set(TurnPhase::Over);
        app.update();
        assert_eq!(
            app.world().resource::<Bgm>().solo,
            Some(Solo::Win),
            "终局该切胜利曲"
        );
        assert_eq!(app.world().resource::<Bgm>().world, None);
    }
}
