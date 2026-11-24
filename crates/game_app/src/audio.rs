//! 音频（P16a）：BGM 的场景机与世界播放列表。
//!
//! 分层红线照旧：规则层不知道有声音这东西。这一模块**只读**世界状态（在哪一屏、开着
//! 哪扇窗口、按没按暂停），唯一的例外是 P16b 的全局闸 [`hold_for_voice`]：它写 core 的
//! `PerformanceHold`，说的是「台词在念，现在别推进」而不是「推进到下一步」—— 与 `Paused`
//! 同一个形状，也是 §4.6 那条红线的延长而不是破例。`NM_AUDIO=0` 把 bevy 的 `AudioPlugin`
//! 整个不装之后，闸门恒开，游戏行为一个字节都不变（截图与无人值守冒烟走的正是这条路）。
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
    ecs::change_detection::DetectChanges,
    prelude::{
        App, AssetServer, AudioPlayer, AudioSink, AudioSinkPlayback, Commands, Component, Entity,
        First, IntoScheduleConfigs, MessageReader, OnEnter, PlaybackSettings, Plugin,
        PointerButton, PointerClick, Query, Res, ResMut, Resource, State, Time, Update, With,
        debug, info, warn,
    },
};
use game_core::prelude::{
    ActiveBoard, AuctionWon, BankOpDone, CardDrawn, CardKind, CardPlayed, CashChanged, DiceRolled,
    EffectOutcome, FacilityBuilt, FinancingDrawn, FinancingRecalled, GameOver, GameSession,
    ItemKind, ItemPlaced, LandBought, LandId, LandUpgraded, LotteryBought, MortgageDone, Owner,
    Paused, Pending, PendingDecision, PerformanceHold, PlayerBailed, PlayerBankrupt,
    PlayerDetained, PlayerId, PlayerName, RentDue, StockTraded, TurnOrder, TurnPhase, TurnStarted,
};

use crate::dialogue::DialogueLines;
use crate::scene::Scene;
use crate::settings::{Settings, VolumeBus};
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
    /// 台词能不能把回合机按住（P16b 的全局闸，见 [`hold_for_voice`]）。
    /// `NM_AUDIO=0` 与 `NM_AUTOROLL=1` 时为假 —— 前者的队列根本没有「在播」这回事，
    /// 后者是长跑，等台词会把几百回合拖成十几分钟。
    hold: bool,
    /// 每一首换曲、每一次暂停各记一行。
    log: bool,
}

impl AudioConfig {
    fn from_env() -> Self {
        let playback = playback_on(env::var("NM_AUDIO").ok().as_deref());
        Self {
            playback,
            hold: playback && env::var_os("NM_AUTOROLL").is_none(),
            log: log_on(env::var("NM_AUDIO_LOG").ok().as_deref()),
        }
    }
}

/// 全局闸（P16b）：台词队列里还有东西要念，就把回合机按住。
///
/// 写的是 core 的 [`PerformanceHold`] —— 规则层只读它，表现层只写它。这不是 §4.6 红线的
/// 破例，是同一条红线的延长：它说的是「现在别推进」，不是「推进到下一步」，与 `Paused`
/// 同一个形状（谁按的谁松，松闸之后的推进仍由规则层的钟来做）。
///
/// 挂在 `First`：它必须在 `PreUpdate` 的 `ai_roll_system` 与 `Update` 的掷骰/走步之前
/// 写好，否则第一条台词要晚一帧才按得住 —— 而那一帧正是「骰子还在翻、人已经走了」。
///
/// `NM_AUDIO_SELFTEST` 也不闸：自检的队列是验听用的，它不是剧情。
fn hold_for_voice(
    cfg: Res<AudioConfig>,
    queue: Res<VoiceQueue>,
    selftest: Res<Selftest>,
    mut hold: ResMut<PerformanceHold>,
) {
    let playing = queue.current.is_some() || !queue.pending.is_empty();
    hold.0 = cfg.hold && !selftest.on && playing;
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
    settings: Res<Settings>,
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
        let with_volume = base.with_volume(Volume::Linear(VolumeBus::Bgm.gain(&settings)));
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

/// 设置里的音量档一改，把新增益按到**已经在响**的那几条上。
///
/// 起新声时读档位是不够的：世界列表里一首 BGM 放几分钟，不按下去的话「拉到静音」要等到
/// 换曲才生效。只在 `Settings` 变了的那一帧动手 —— `set_volume` 与 `pause` 一样是发到
/// 音频线程的消息，每帧无脑调一次会白占那条线程。
///
/// 一条查询同时认两种声：`BgmTag` 与 `VoiceLine` 是两条互斥的槽，分成两条 `&mut AudioSink`
/// 查询则过不了 Bevy 的冲突检查（过滤器证明不了两者不交）。音效是一次性的短声，
/// 按下那一刻的档位就是它的全部寿命，没有「已经在响的那条」要改。
fn apply_audio_volumes(
    settings: Res<Settings>,
    live: Query<(&mut AudioSink, Option<&BgmTag>, Option<&VoiceLine>)>,
) {
    if !settings.is_changed() {
        return;
    }
    for (mut sink, tag, line) in live {
        if let Some(bus) = bus_of(tag, line) {
            sink.set_volume(Volume::Linear(bus.gain(&settings)));
        }
    }
}

/// 这条 sink 归哪条总线。抽出来是为了能单测：`AudioSink` 要真音频设备才造得出来，
/// 而「BGM 的 sink 被按语音档」这种串线错误不该只有上机才看得见。
fn bus_of(tag: Option<&BgmTag>, line: Option<&VoiceLine>) -> Option<VolumeBus> {
    if tag.is_some() {
        Some(VolumeBus::Bgm)
    } else if line.is_some() {
        Some(VolumeBus::Voice)
    } else {
        None
    }
}

// ===== 语音（P16a③）=====
//
// 一句一句地放：一帧里可能同时来三件事（收租、聚街、被关），而耳朵只能接一句。
// 队列元素带**事件主语**的回合序与子序，排完序再进队；当前那条播完（`Despawn`）才轮到下一条。
// 点击打断一次只丢一条 —— 一条点击消息配一次 `take()`。

/// UI 那一层要放的四声。派发侧只认这个枚举，路径与音量都留在本模块 ——
/// 按钮不该知道 `.ogg` 在哪。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiSound {
    /// 任何**可用**按钮的底噪
    Click,
    /// 接受／确认类：买地、开始对局、存取档、柜台里那几笔
    Confirm,
    /// 放弃／关闭／返回类
    Cancel,
    /// 点到禁用按钮：给一声，理由仍由那颗灰按钮自己写
    Error,
}

/// UI 想放的一声。写成消息而不是直接调本模块：派发侧只写愿望，
/// `NM_AUDIO=0` 时要不要出声由这里说了算（消息还在，日志照记）。
#[derive(bevy::prelude::Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiSoundRequest(pub UiSound);

impl UiSound {
    fn file(self) -> &'static str {
        match self {
            UiSound::Click => "ui_click_soft",
            UiSound::Confirm => "ui_confirm_soft",
            UiSound::Cancel => "ui_cancel_soft",
            UiSound::Error => "ui_error_soft",
        }
    }
}

/// 音效总线同样走 [`Settings`] 的档位（默认 75%：UI 素材峰值 −6 dBFS，比 BGM 高一大截）。
/// 十五个语音事件：文件名（去 `.ogg`）与日志键同字面量。`CHARACTERS-dialogue.md` §三
/// 那张触发矩阵是这里的地基，改口径先改那张表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceEvent {
    Enter,
    CashBigGain,
    CashBigLoss,
    Monopoly,
    Detained,
    FateGain,
    FateLoss,
    ItemFrame,
    Framed,
    ItemTurn,
    ItemStay,
    ItemTurtle,
    Mine,
    Bankrupt,
    Win,
}

impl VoiceEvent {
    /// 十五格清单：自检那一屏按它逐条放，验收也按它核对「十五条一条不少」。
    const ALL: [VoiceEvent; 15] = [
        VoiceEvent::Enter,
        VoiceEvent::CashBigGain,
        VoiceEvent::CashBigLoss,
        VoiceEvent::Monopoly,
        VoiceEvent::Detained,
        VoiceEvent::FateGain,
        VoiceEvent::FateLoss,
        VoiceEvent::ItemFrame,
        VoiceEvent::Framed,
        VoiceEvent::ItemTurn,
        VoiceEvent::ItemStay,
        VoiceEvent::ItemTurtle,
        VoiceEvent::Mine,
        VoiceEvent::Bankrupt,
        VoiceEvent::Win,
    ];

    fn file(self) -> &'static str {
        match self {
            VoiceEvent::Enter => "enter",
            VoiceEvent::CashBigGain => "cash_big_gain",
            VoiceEvent::CashBigLoss => "cash_big_loss",
            VoiceEvent::Monopoly => "monopoly",
            VoiceEvent::Detained => "detained",
            VoiceEvent::FateGain => "fate_gain",
            VoiceEvent::FateLoss => "fate_loss",
            VoiceEvent::ItemFrame => "item_frame",
            VoiceEvent::Framed => "framed",
            VoiceEvent::ItemTurn => "item_turn",
            VoiceEvent::ItemStay => "item_stay",
            VoiceEvent::ItemTurtle => "item_turtle",
            VoiceEvent::Mine => "mine",
            VoiceEvent::Bankrupt => "bankrupt",
            VoiceEvent::Win => "win",
        }
    }
}

/// 语音总线同样走 [`Settings`] 的档位（默认 100%：素材约 −20 LUFS，比音效低一截）。
/// 队列上限。满了丢队尾而**不丢队头**：队头是此刻该说的那一句。
const MAX_QUEUE: usize = 8;
/// 大额现金的门槛（未乘物价）。乘完之后才是判据 —— 门槛钉死的话，物价涨一倍时
/// 「2,000」就只值一半，语音会越到后期越哑。
const CASH_FLOOR: f64 = 2000.0;

/// 队列里的一句话。`seat` 是**事件主语**在 `TurnOrder` 里的下标（不是说话人的），
/// `sub` 是同一件事内部的次序（收租：收款方 0、付款方 1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VoiceCue {
    player: Entity,
    event: VoiceEvent,
    seat: usize,
    sub: u8,
}

#[derive(Resource, Default)]
struct VoiceQueue {
    /// 此刻正在播的那一句（`Despawn` 模式，放完实体自己消失，所以这里只是一个句柄）
    current: Option<Entity>,
    pending: std::collections::VecDeque<VoiceCue>,
    /// 已经念过开场白的玩家（P16b 修正）：判据是「谁的回合开始而他还没念过」，
    /// 于是**每人第一回合前念自己的那句** —— 第一位上场、第一位行动、第二位上场……
    /// 而不是第一位回合前把全员连成串。实体带 generation，跨局复用 index 也不会
    /// 误挡新玩家；回菜单时 `clear_voice` 照样清，账不过夜。
    entered: std::collections::HashSet<Entity>,
    /// 当前这一句是什么时候起播的（`Time::elapsed_secs`）。只给「单击跳过」用：
    /// 起播那一刻之前就在路上的点击不许把这一句掐掉（见 [`SKIP_GRACE`]）。
    started_at: f32,
}

#[derive(Component)]
struct VoiceLine;

/// 此刻该上屏的那一句（P16d）。语音**真的起播**时才写，念完、跳过、回菜单时清 ——
/// 字幕跟着声音走，不跟队列走：队列里排着的那几句现在没人说。
///
/// `seq` 每换一句 +1：UI 只在它变的时候重写文字（与各浮层「按签名重建」同一条）。
#[derive(Resource, Default)]
pub(crate) struct VoiceCaption {
    /// 说话的人（画面上的棋子），`None` = 没有该上屏的台词
    pub(crate) player: Option<Entity>,
    pub(crate) text: String,
    pub(crate) seq: u64,
}

impl VoiceCaption {
    fn show(&mut self, player: Entity, text: &str) {
        if text.trim().is_empty() {
            self.clear();
            return;
        }
        self.player = Some(player);
        self.text = text.to_string();
        self.seq = self.seq.wrapping_add(1);
    }

    fn clear(&mut self) {
        if self.player.take().is_some() {
            self.text.clear();
            self.seq = self.seq.wrapping_add(1);
        }
    }
}

/// `NM_AUDIO=0` 时字幕自己走的钟。有声时不参与 —— 一句的长度由声音素材决定，
/// 那是唯一与「观众听到的」对得上的长度。静音模式没有 sink 可等，只能按字数估一个
/// 停留时长，否则队列一帧走一句、字还没读完就换下一条。
#[derive(Resource, Default)]
struct CaptionClock(f32);

/// 一句台词的停留时长（秒）：中文按每秒约 5–6 字估，短句留 1.2 秒起步，
/// 长句封顶 6 秒。估长了只是多停一会儿，估短了是读不完 —— 所以宁长勿短。
fn caption_secs(text: &str) -> f32 {
    let chars = text.chars().count() as f32;
    (1.2 + chars * 0.18).clamp(1.2, 6.0)
}

/// 名字 + 事件 → 资产路径。查不到嗓子（四恶人、无名座位、没录音的候补）就是 `None`。
fn voice_path(name: &str, event: VoiceEvent) -> Option<String> {
    let key = crate::art::voice_key(name)?;
    Some(format!("audio/voice/{key}/{}.ogg", event.file()))
}

/// 这一笔钱够不够响。边界取**含等号**：正好 2,000（物价 1.0 时）该说。
fn loud_enough(amount: i64, price_index: f32) -> bool {
    f64::from(amount.unsigned_abs() as u32) >= CASH_FLOOR * f64::from(price_index)
}

/// 现金这一族的语音决定。抽成纯函数是因为它是全模块最容易写错的一块：
/// 五条规矩（租金成对、命运接管、静默族、门槛随物价、同帧合并）挤在同一个函数里，
/// 而其中三条只有「两条消息同帧到场」时才显形 —— 拿 SystemParam 去测它们要摆半局棋。
///
/// 返回值：`(谁说话, 事件, 子序, 事件主语)`。租金的**两个人共用一个主语**（付款那位），
/// 于是「得钱者先说、亏钱者后说」与「按回合顺序」两条排序规则不会打架。
fn cash_voices(
    price_index: f32,
    rents: &[(Entity, Entity, i64)],
    flows: &[(Entity, i64)],
    fates: &[(Entity, VoiceEvent)],
    silent: &std::collections::HashSet<Entity>,
) -> Vec<(Entity, VoiceEvent, u8, Entity)> {
    let mut out = Vec::new();
    let mut taken = silent.clone();
    // ① 租金是**显式接管**：不论金额大小，这两个人的现金都不再走通用那一句
    for (payer, owner, amount) in rents {
        taken.insert(*payer);
        taken.insert(*owner);
        if payer != owner && loud_enough(*amount, price_index) {
            out.push((*owner, VoiceEvent::CashBigGain, 0, *payer));
            out.push((*payer, VoiceEvent::CashBigLoss, 1, *payer));
        }
    }
    // ② 命运那笔由 fate_gain／fate_loss 接管，同一个人不再出 cash_big_*
    for (who, event) in fates {
        taken.insert(*who);
        out.push((*who, *event, 0, *who));
    }
    // ③ 其余现金：同一个人同帧只取绝对值最大的那一笔
    let mut best: Vec<(Entity, i64)> = Vec::new();
    for (who, delta) in flows {
        if *delta == 0 || taken.contains(who) {
            continue;
        }
        match best.iter_mut().find(|(other, _)| other == who) {
            Some((_, amount)) => {
                if delta.abs() > amount.abs() {
                    *amount = *delta;
                }
            }
            None => best.push((*who, *delta)),
        }
    }
    for (who, delta) in best {
        if loud_enough(delta, price_index) {
            let event = if delta > 0 {
                VoiceEvent::CashBigGain
            } else {
                VoiceEvent::CashBigLoss
            };
            out.push((who, event, 0, who));
        }
    }
    out
}

/// 一帧里所有与声音有关的消息，一次读齐（语音与音效各取所需，两条 system 各有一份游标）。
///
/// 打包是房规（§六 坑 8）：光「静默族」就九条 reader，而它们存在的理由只有一句
/// 「这些钱已经在别处说过了」—— 散在参数表里就没人记得住为什么要有它们。
#[derive(bevy::ecs::system::SystemParam)]
struct AudioInbox<'w, 's> {
    order: Option<Res<'w, TurnOrder>>,
    session: Option<Res<'w, GameSession>>,
    board: Option<Res<'w, ActiveBoard>>,
    players: Query<'w, 's, (&'static PlayerName, &'static PlayerId)>,
    owners: Query<'w, 's, (&'static LandId, &'static Owner)>,
    cash: MessageReader<'w, 's, CashChanged>,
    rent: MessageReader<'w, 's, RentDue>,
    drawn: MessageReader<'w, 's, CardDrawn>,
    played: MessageReader<'w, 's, CardPlayed>,
    placed: MessageReader<'w, 's, ItemPlaced>,
    bought: MessageReader<'w, 's, LandBought>,
    detained: MessageReader<'w, 's, PlayerDetained>,
    bankrupt: MessageReader<'w, 's, PlayerBankrupt>,
    over: MessageReader<'w, 's, GameOver>,
    started: MessageReader<'w, 's, TurnStarted>,
    rolled: MessageReader<'w, 's, DiceRolled>,
    bailed: MessageReader<'w, 's, PlayerBailed>,
    ui: MessageReader<'w, 's, UiSoundRequest>,
    // 静默族：这些流水同帧都有更专门的那一声，或者根本不该出声
    bank: MessageReader<'w, 's, BankOpDone>,
    traded: MessageReader<'w, 's, StockTraded>,
    mortgage: MessageReader<'w, 's, MortgageDone>,
    financing: MessageReader<'w, 's, FinancingDrawn>,
    recalled: MessageReader<'w, 's, FinancingRecalled>,
    upgraded: MessageReader<'w, 's, LandUpgraded>,
    built: MessageReader<'w, 's, FacilityBuilt>,
    lottery: MessageReader<'w, 's, LotteryBought>,
    auction: MessageReader<'w, 's, AuctionWon>,
}

/// 这一帧的事件 → 队列。排序键是 `(事件主语的回合序, 子序)`，走稳定排序，于是
/// 「同帧发射序」就是入列顺序 —— 不需要再记一个序号。
fn collect_voice(
    mut inbox: AudioInbox<'_, '_>,
    mut queue: ResMut<VoiceQueue>,
    cfg: Res<AudioConfig>,
    mut commands: Commands,
    live: Query<Entity, With<VoiceLine>>,
) {
    // 先读这一份（它借走整个 `inbox`），再拿那些只借用某几个字段的局部引用
    let (silent, purchases, _) = cash_context(&mut inbox);
    let order = inbox.order.as_ref();
    let players = &inbox.players;
    let seat_of = |who: Entity| -> usize {
        order
            .as_ref()
            .and_then(|order| order.order.iter().position(|each| *each == who))
            .or_else(|| players.get(who).ok().map(|(_, id)| id.0 as usize))
            .unwrap_or(usize::MAX)
    };
    let mut cues: Vec<VoiceCue> = Vec::new();
    let cue = |cues: &mut Vec<VoiceCue>, player, event, sub, actor| {
        cues.push(VoiceCue {
            player,
            event,
            seat: seat_of(actor),
            sub,
        });
    };

    // 命运：得/失都接管同一个人这一帧的现金语音
    let mut fates: Vec<(Entity, VoiceEvent)> = Vec::new();
    for drawn in inbox.drawn.read() {
        if drawn.card.kind != CardKind::Fate {
            continue;
        }
        let (mut gain, mut lost) = (false, false);
        for outcome in &drawn.card.outcomes {
            match outcome {
                EffectOutcome::Gained(amount) if *amount > 0 => gain = true,
                EffectOutcome::Lost(amount) if *amount > 0 => lost = true,
                _ => {}
            }
        }
        if gain {
            fates.push((drawn.player, VoiceEvent::FateGain));
        }
        if lost {
            fates.push((drawn.player, VoiceEvent::FateLoss));
        }
    }

    // 现金：租金、命运、静默族、其余
    let mut rents: Vec<(Entity, Entity, i64)> = Vec::new();
    for due in inbox.rent.read() {
        rents.push((due.payer, due.owner, due.amount));
    }
    let mut flows: Vec<(Entity, i64)> = Vec::new();
    for changed in inbox.cash.read() {
        flows.push((changed.player, changed.delta));
    }
    let index = inbox.session.as_ref().map_or(1.0, |s| s.price_index);
    for (who, event, sub, actor) in cash_voices(index, &rents, &flows, &fates, &silent) {
        cue(&mut cues, who, event, sub, actor);
    }

    // 其余事件
    for done in &purchases {
        let Ok((_, id)) = inbox.players.get(done.player) else {
            continue;
        };
        if reaches_three(inbox.board.as_deref(), &inbox.owners, *id, done.land) {
            cue(&mut cues, done.player, VoiceEvent::Monopoly, 0, done.player);
        }
    }
    for held in inbox.detained.read() {
        cue(&mut cues, held.player, VoiceEvent::Detained, 0, held.player);
    }
    for played in inbox.played.read() {
        // 同一件事里的两个人共用一个主语（出牌的人）：`子序` 决定谁说先
        match played.card.id.as_str() {
            "item_frame" => {
                cue(
                    &mut cues,
                    played.player,
                    VoiceEvent::ItemFrame,
                    0,
                    played.player,
                );
                if played.victim != played.player {
                    cue(
                        &mut cues,
                        played.victim,
                        VoiceEvent::Framed,
                        1,
                        played.player,
                    );
                }
            }
            "item_turn" => cue(
                &mut cues,
                played.player,
                VoiceEvent::ItemTurn,
                0,
                played.player,
            ),
            "item_stay" => cue(
                &mut cues,
                played.player,
                VoiceEvent::ItemStay,
                0,
                played.player,
            ),
            "item_turtle" => cue(
                &mut cues,
                played.player,
                VoiceEvent::ItemTurtle,
                0,
                played.player,
            ),
            _ => {}
        }
    }
    for placed in inbox.placed.read() {
        if placed.item == ItemKind::Mine {
            cue(&mut cues, placed.player, VoiceEvent::Mine, 0, placed.player);
        }
    }
    // 开场白（P16b 修正）：**谁的回合开始而他还没念过，谁念自己的那句**。节奏是
    // 「第一位上场 → 第一位行动 → 第二位上场 → 第二位行动……」—— 旧版用
    // `turn_count == 0` 当判据，它只认得出第一位，于是四句连串挤在第一位行动之前。
    // 拘留跳过的回合也发 `TurnStarted`（turn.rs 三处都在回合机推进里），所以被押着
    // 的第一回合照样开口。读档不重跑 `TurnStarted`（restore 不走回合机），回到
    // 第一回合的存档也不会再念 —— 判据靠回合机本来的形状，不靠 `StartMode`。
    for started in inbox.started.read() {
        if queue.entered.insert(started.player) {
            cues.push(VoiceCue {
                player: started.player,
                event: VoiceEvent::Enter,
                seat: seat_of(started.player),
                sub: 0,
            });
        }
    }

    // 破产与终局是低频高光，不排在三条收租后面：清空队列、掐掉正在念的那一句、插队
    let urgent = inbox
        .bankrupt
        .read()
        .map(|poor| (poor.player, VoiceEvent::Bankrupt))
        .chain(
            inbox
                .over
                .read()
                .filter_map(|over| over.winner)
                .map(|winner| (winner, VoiceEvent::Win)),
        )
        .next();
    if let Some((player, event)) = urgent {
        queue.pending.clear();
        if let Some(entity) = queue.current.take()
            && live.contains(entity)
        {
            commands.entity(entity).despawn();
        }
        queue.pending.push_back(VoiceCue {
            player,
            event,
            seat: seat_of(player),
            sub: 0,
        });
        if cfg.log {
            info!("audio: voice 插队 {}", event.file());
        }
        return;
    }

    let dropped = line_up(&mut cues, &mut queue.pending, MAX_QUEUE);
    if dropped > 0 && cfg.log {
        info!("audio: voice drop (queue full) ×{dropped}");
    }
}

/// 排队：先按 `(事件主语回合序, 子序)` **稳定**排序（同键保持同帧发射序），再逐条进队；
/// 满了丢队尾的后来者并返回丢掉的条数 —— 队头是此刻该说的那一句，不能丢。
fn line_up(
    cues: &mut [VoiceCue],
    pending: &mut std::collections::VecDeque<VoiceCue>,
    cap: usize,
) -> usize {
    cues.sort_by_key(|cue| (cue.seat, cue.sub));
    let mut dropped = 0;
    for cue in cues.iter() {
        if pending.len() >= cap {
            dropped += 1;
            continue;
        }
        pending.push_back(*cue);
    }
    dropped
}

/// 这一帧里哪些人的现金「已经在别处说过了」，以及买地那几条（聚街还要用）。
///
/// 九条流水 + 买地：它们同帧都有一条更专门的凭据（银行回执、成交流水、收据），
/// 或者根本不该出声（买地那一笔是「换资产」不是「丢钱」）。语音与音效**共用这一份判断** ——
/// 两处各写一遍的话，迟早会出现「语音不吭声而音效在响」。
fn cash_context(
    inbox: &mut AudioInbox<'_, '_>,
) -> (std::collections::HashSet<Entity>, Vec<LandBought>, bool) {
    // 买地有两种用法（静默 + 聚街），所以先落一份 —— `MessageReader` 读一次就到底，
    // 两处各 `read()` 一遍的话第二遍一条都读不到，而症状恰好是「买地不静音」或
    // 「聚街永远不响」二选一（第一次实现就漏了静默那一半）
    let purchases: Vec<LandBought> = inbox.bought.read().copied().collect();
    let mut quiet: std::collections::HashSet<Entity> = std::collections::HashSet::new();
    for bought in &purchases {
        quiet.insert(bought.player);
    }
    for done in inbox.bank.read() {
        quiet.insert(done.player);
    }
    for trade in inbox.traded.read() {
        quiet.insert(trade.player);
    }
    for done in inbox.mortgage.read() {
        quiet.insert(done.player);
    }
    for drawn in inbox.financing.read() {
        quiet.insert(drawn.chairman);
    }
    for recalled in inbox.recalled.read() {
        quiet.insert(recalled.player);
    }
    // 升级也算「东西落下去」（P16b 的摆放音效与它同步）：顺手记一个布尔，
    // 因为这条 reader 读完就到底，`sfx_for_frame` 不能再去读第二遍
    let mut placed = !purchases.is_empty();
    for up in inbox.upgraded.read() {
        quiet.insert(up.player);
        placed = true;
    }
    for built in inbox.built.read() {
        quiet.insert(built.player);
    }
    for bought in inbox.lottery.read() {
        quiet.insert(bought.player);
    }
    for won in inbox.auction.read() {
        quiet.insert(won.player);
    }
    (quiet, purchases, placed)
}

/// 事件音效：一帧里同一键最多一条（`keys` 去重），播完自清（`Despawn`）。
///
/// 与语音分两条 system 而不是合一条：音效**不排队**也不打断，它要的是「这一下有个响」；
/// 语音要的是「一次只说一句」。把两种时序塞进同一个函数，两边都会被对方的规矩绑住。
fn collect_sfx(
    mut inbox: AudioInbox<'_, '_>,
    cfg: Res<AudioConfig>,
    settings: Res<Settings>,
    server: Res<AssetServer>,
    mut commands: Commands,
) {
    let keys = sfx_for_frame(&mut inbox);
    if keys.is_empty() {
        return;
    }
    if cfg.log {
        info!("audio: sfx {}", keys.join(" + "));
    }
    if !cfg.playback {
        return;
    }
    let gain = VolumeBus::Sfx.gain(&settings);
    for key in keys {
        commands.spawn((
            AudioPlayer::new(server.load(format!("audio/sfx/{key}.ogg"))),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(gain)),
        ));
    }
}

/// 这一帧该放的音效键（同帧同键只留一条）。读消息的那一半单独成函数，
/// 测试才能在无音频设备的机器上把「该放哪几条」钉住 —— 与 `cash_voices` 同一条理由。
fn sfx_for_frame(inbox: &mut AudioInbox<'_, '_>) -> Vec<&'static str> {
    let (quiet, purchases, placed) = cash_context(inbox);
    let mut keys: Vec<&'static str> = Vec::new();
    let mut want = |key: &'static str| {
        if !keys.contains(&key) {
            keys.push(key);
        }
    };

    // `read()` 要**读干净**：只取一条的话剩下的会留到下一帧再响一次（同一掷两下，听得出）
    if inbox.rolled.read().count() > 0 {
        want("dice_roll");
    }
    for changed in inbox.cash.read() {
        if changed.delta == 0 || quiet.contains(&changed.player) {
            continue;
        }
        want(if changed.delta > 0 {
            "coin_gain"
        } else {
            "coin_loss"
        });
    }
    for _ in inbox.played.read() {
        want("card_play");
    }
    for _ in inbox.placed.read() {
        want("item_place");
    }
    // 摆放标志物（P16b）：买地与升级同一声。它与 `item_place` 分开 ——
    // 那一声是道具放在**路上**，这一声是地契/楼落在**自家地上**
    if placed {
        want("land_place");
    }
    if inbox.detained.read().count() > 0 || inbox.bailed.read().count() > 0 {
        want("jail");
    }
    for drawn in inbox.drawn.read() {
        if drawn.card.kind == CardKind::Fate {
            want("fate");
        }
    }
    for done in &purchases {
        let Ok((_, id)) = inbox.players.get(done.player) else {
            continue;
        };
        if reaches_three(inbox.board.as_deref(), &inbox.owners, *id, done.land) {
            want("monopoly");
        }
    }
    if inbox.bankrupt.read().count() > 0 {
        want("bankrupt");
    }
    for asked in inbox.ui.read() {
        want(asked.0.file());
    }
    keys
}

/// 买家在这一刻是不是**刚凑齐同一条街的第三块**。
///
/// 没有「聚街了」这条消息（经济侧只在算租金时现问），所以这里自己数一遍：同组成员、
/// 归属是买家。判 `== 3` 而不是 `>= 3`，否则第四块也会再喊一遍。
fn reaches_three(
    board: Option<&ActiveBoard>,
    owners: &Query<(&LandId, &Owner)>,
    buyer: PlayerId,
    land: LandId,
) -> bool {
    let Some(board) = board else { return false };
    let Some(group) = board
        .def
        .lands
        .iter()
        .find(|each| each.id == land.0)
        .and_then(|each| each.group.as_ref())
    else {
        return false;
    };
    let owned = board
        .def
        .lands
        .iter()
        .filter(|each| each.group.as_ref() == Some(group))
        .filter(|each| {
            owners
                .iter()
                .any(|(id, owner)| id.0 == each.id && owner.0 == Some(buyer))
        })
        .count();
    owned == 3
}

/// 念下一句：当前那句的实体已经没了（`Despawn` 播完）就取队头。
///
/// 一次只起一条；没有嗓子的人（四恶人、无名座位）当场跳过 —— 静默是设计，不是错误。
/// 字幕那几样总是同进同出：谁在说、静音时那一句要停多久、正文表与墙钟。
/// 打包也是房规（§六 坑 8）：全摊在 `advance_voice` 的参数表上会越过 clippy 的七参数线。
/// 正文表取 `Option`：单测里的 App 不装 `DialoguePlugin`，缺表时照常放声音、只是没字幕。
#[derive(bevy::ecs::system::SystemParam)]
struct CaptionDesk<'w> {
    caption: ResMut<'w, VoiceCaption>,
    clock: ResMut<'w, CaptionClock>,
    dialogue: Option<Res<'w, DialogueLines>>,
    time: Res<'w, Time>,
}

/// 起播一条语音要的三样（开关与日志、音量档位、素材服务），同样只在这里打包一次。
#[derive(bevy::ecs::system::SystemParam)]
struct PlaybackDesk<'w> {
    cfg: Res<'w, AudioConfig>,
    settings: Res<'w, Settings>,
    server: Res<'w, AssetServer>,
}

fn advance_voice(
    mut commands: Commands,
    mut queue: ResMut<VoiceQueue>,
    mut desk: CaptionDesk,
    playback: PlaybackDesk,
    live: Query<Entity, With<VoiceLine>>,
    players: Query<&PlayerName>,
) {
    if queue.current.is_some_and(|entity| live.contains(entity)) {
        return;
    }
    queue.current = None;
    // 静音模式：上一条字幕还没读够就先不换（有声音时这段永远是 0）
    if !playback.cfg.playback && desk.clock.0 > 0.0 {
        desk.clock.0 -= desk.time.delta_secs();
        return;
    }
    while let Some(cue) = queue.pending.pop_front() {
        let Ok(name) = players.get(cue.player) else {
            continue;
        };
        let Some(path) = voice_path(&name.0, cue.event) else {
            if playback.cfg.log {
                info!("audio: voice {} 无语音（静默）", name.0);
            }
            continue;
        };
        // 字幕跟**这一条声音**同时起：正文按语音目录名 + 事件键查表，
        // 查不到就空着（有声音没字幕，不编一句）
        let text = desk
            .dialogue
            .as_ref()
            .and_then(|lines| {
                crate::art::voice_key(&name.0).and_then(|key| lines.line(key, cue.event.file()))
            })
            .unwrap_or("");
        if playback.cfg.playback {
            queue.current = Some(
                commands
                    .spawn((
                        AudioPlayer::new(playback.server.load(path)),
                        PlaybackSettings::DESPAWN
                            .with_volume(Volume::Linear(VolumeBus::Voice.gain(&playback.settings))),
                        VoiceLine,
                    ))
                    .id(),
            );
            queue.started_at = desk.time.elapsed_secs();
            desk.caption.show(cue.player, text);
        } else if !text.is_empty() {
            desk.caption.show(cue.player, text);
            desk.clock.0 = caption_secs(text);
            queue.started_at = desk.time.elapsed_secs();
            if playback.cfg.log {
                info!("audio: caption {text} (silent {:.1}s)", desk.clock.0);
            }
            return;
        } else {
            // 静音且没有正文：这一条不占时间，接着找下一条有词的
        }
        if playback.cfg.log {
            info!(
                "audio: voice {} {} (queue {})",
                name.0,
                cue.event.file(),
                queue.pending.len()
            );
        }
        return;
    }
    // 队列空了、也没有人在念：气泡跟着声音一起消失
    desk.caption.clear();
    desk.clock.0 = 0.0;
}

/// 起播之后这一小段里不认「单击跳过」。启动某一屏的那一下点击会留下一条 `PointerClick`
/// （消息活两帧），而开局头几帧的帧长有上百毫秒 —— 开场白起播之后那条旧点击还会被读到，
/// 于是「点开始对局」这一下把自己的第一句掐掉了（P16d 实报：「第一个角色没有登场台词，
/// 后面都有」，日志是 `voice 姬百年 enter` 之后 13ms 的 `voice skip by click`）。
/// 0.3 秒只盖住这段余量，不改变「点一下跳过一句」的手感。
const SKIP_GRACE: f32 = 0.3;

/// 这一次点击该不该掐掉当前这句。判据是**这句已经念了多久**，不是点击自己的时间戳 ——
/// `PointerClick` 不带时刻，能读到的只有「它两帧之内还在」。抽成纯函数是为了让 13ms
/// 那条实报能被钉进单测（`now - started_at` 恰好是日志里那两个时刻之差）。
fn click_may_skip(started_at: f32, now: f32) -> bool {
    now - started_at >= SKIP_GRACE
}

/// 主键点一下打断一句。**一次点击只打断一条**：循环里 `break`，于是同一次点击
/// 不会顺着把下一条也掐掉（下一条由下一帧 `First` 里的 `advance_voice` 起出来）。
fn skip_voice_on_click(
    mut clicks: MessageReader<PointerClick>,
    mut commands: Commands,
    mut queue: ResMut<VoiceQueue>,
    time: Res<Time>,
    cfg: Res<AudioConfig>,
    live: Query<Entity, With<VoiceLine>>,
) {
    let Some(current) = queue.current else { return };
    if !click_may_skip(queue.started_at, time.elapsed_secs()) {
        return;
    }
    for click in clicks.read() {
        if click.button != PointerButton::Primary || !live.contains(current) {
            continue;
        }
        queue.current = None;
        commands.entity(current).despawn();
        if cfg.log {
            info!("audio: voice skip by click");
        }
        break;
    }
}

/// 回菜单：清队列、掐当前。与 UI 那批「跨局残留」同一条纪律 —— 上一局没念完的几句
/// 不该在新一局的开场白后面排队。
fn clear_voice(
    mut commands: Commands,
    mut queue: ResMut<VoiceQueue>,
    mut caption: ResMut<VoiceCaption>,
    mut clock: ResMut<CaptionClock>,
    live: Query<Entity, With<VoiceLine>>,
) {
    queue.pending.clear();
    queue.entered.clear();
    queue.started_at = 0.0;
    caption.clear();
    clock.0 = 0.0;
    if let Some(entity) = queue.current.take()
        && live.contains(entity)
    {
        commands.entity(entity).despawn();
    }
}

/// 盘上的十条通用音效（`game-sfx/synth_sfx.py` 的 `make_game` 那一批）与四条 UI 音。
/// 键名与文件名同字面量；`ui_hover_soft` 与 crisp 那套**刻意不在这张表里**（今晚不接）。
const SFX_FILES: [&str; 10] = [
    "dice_roll",
    "coin_gain",
    "coin_loss",
    "card_play",
    "item_place",
    // 摆放标志物（P16b）：买地/升级的过场里那一声「地契砸下来」
    "land_place",
    "jail",
    "fate",
    "monopoly",
    "bankrupt",
];
const UI_FILES: [&str; 4] = [
    "ui_click_soft",
    "ui_confirm_soft",
    "ui_cancel_soft",
    "ui_error_soft",
];

/// `NM_AUDIO_SELFTEST=1`：开局把整套素材各放一遍（十五句语音 + 十条音效 + 四条 UI），
/// 一句一行日志。给无人值守验收与将来试听用；只在 `Scene::Board` 生效 —— 菜单上
/// 还没有玩家，挑不出嗓子。
#[derive(Resource, Default)]
struct Selftest {
    on: bool,
    built: bool,
    next: usize,
    clock: f32,
    items: Vec<SelftestItem>,
}

/// 自检的一条。语音**走真正的队列**而不是另起一条路：这样点击打断才验得到
/// （`NM_AUDIO_SELFTEST` 的用处之一就是给那条手测一个稳定的靶子）。
#[derive(Clone)]
enum SelftestItem {
    Voice(Entity, VoiceEvent),
    Sound { key: String, path: String },
}

const SELFTEST_GAP: f32 = 0.8;

/// 自检要读的那几样。打包是房规（§六 坑 8）—— 这条 system 的职责只是「按节拍放下一首」。
#[derive(bevy::ecs::system::SystemParam)]
struct SelftestView<'w, 's> {
    time: Res<'w, Time>,
    scene: Res<'w, State<Scene>>,
    order: Option<Res<'w, TurnOrder>>,
    players: Query<'w, 's, &'static PlayerName>,
    cfg: Res<'w, AudioConfig>,
}

fn run_selftest(
    mut selftest: ResMut<Selftest>,
    view: SelftestView<'_, '_>,
    settings: Res<Settings>,
    mut queue: ResMut<VoiceQueue>,
    server: Res<AssetServer>,
    mut commands: Commands,
) {
    if !selftest.on || *view.scene.get() != Scene::Board {
        return;
    }
    if !selftest.built {
        // 挑一副嗓子：队里第一个有录音的人。自检的语音**走真正的队列**，所以必须绑在
        // 一个真玩家实体上 —— 路径由 `advance_voice` 按他自己的名字拼。
        let Some(player) = view.order.as_ref().and_then(|order| {
            order.order.iter().copied().find(|who| {
                view.players
                    .get(*who)
                    .is_ok_and(|name| crate::art::voice_key(&name.0).is_some())
            })
        }) else {
            return;
        };
        let name = view
            .players
            .get(player)
            .map_or_else(|_| "?".to_string(), |name| name.0.clone());
        for event in VoiceEvent::ALL {
            selftest.items.push(SelftestItem::Voice(player, event));
        }
        for file in SFX_FILES {
            selftest.items.push(SelftestItem::Sound {
                key: format!("sfx {file}"),
                path: format!("audio/sfx/{file}.ogg"),
            });
        }
        for file in UI_FILES {
            selftest.items.push(SelftestItem::Sound {
                key: format!("ui {file}"),
                path: format!("audio/sfx/{file}.ogg"),
            });
        }
        selftest.built = true;
        info!(
            "audio: selftest 开始，{name} 的嗓子，共 {} 条（每 {SELFTEST_GAP}s 一条）",
            selftest.items.len()
        );
        return;
    }
    // 语音那几条**等前一句念完再推下一条**：队列上限 8 会把它们丢掉，而自检的用处
    // 正是「逐条听一遍」。音效与 UI 没有队列，仍按 0.8s 的节拍走
    let next_is_voice = matches!(
        selftest.items.get(selftest.next),
        Some(SelftestItem::Voice(..))
    );
    if next_is_voice {
        if queue.current.is_some() || !queue.pending.is_empty() {
            return;
        }
    } else {
        selftest.clock += view.time.delta_secs();
        if selftest.clock < SELFTEST_GAP {
            return;
        }
        selftest.clock = 0.0;
    }
    let Some(item) = selftest.items.get(selftest.next).cloned() else {
        return;
    };
    selftest.next += 1;
    match item {
        SelftestItem::Voice(player, event) => {
            info!("audio: selftest voice {}", event.file());
            queue.pending.push_back(VoiceCue {
                player,
                event,
                seat: 0,
                sub: 0,
            });
        }
        SelftestItem::Sound { key, path } => {
            info!("audio: selftest {key}");
            if view.cfg.playback {
                commands.spawn((
                    AudioPlayer::new(server.load(path)),
                    PlaybackSettings::DESPAWN
                        .with_volume(Volume::Linear(VolumeBus::Sfx.gain(&settings))),
                ));
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
            .add_message::<UiSoundRequest>()
            .init_resource::<Bgm>()
            .init_resource::<VoiceQueue>()
            .init_resource::<VoiceCaption>()
            .init_resource::<CaptionClock>()
            .insert_resource(Selftest {
                on: env::var("NM_AUDIO_SELFTEST").is_ok_and(|value| value.trim() == "1"),
                ..Default::default()
            })
            .add_systems(OnEnter(Scene::Menu), clear_voice)
            // 严格串起来：意图 → 退役 → 起停 → 出声。并行元组里没有顺序可言
            // （§6.1 坑 36），而这一条链上每一步都读上一步写的那个意图
            .add_systems(
                Update,
                (aim_bgm, retire_finished_world, play_bgm, duck_bgm).chain(),
            )
            // 语音导演排在 **First**（P16b）：收事件 → 起下一句 → 写全局闸。
            // 闸必须赶在 `PreUpdate` 的 AI 与 `Update` 的掷骰/走步之前写好，否则第一条
            // 台词要晚一帧才按得住 —— 而那一帧正是「骰子还在翻、人已经走了」。
            // 这里读的消息是上一帧写的（消息活两帧），不会丢。
            .add_systems(
                First,
                (collect_voice, advance_voice, hold_for_voice).chain(),
            )
            // 单击打断留在 `Update`：它读的 `PointerClick` 是 UI 这一帧才写的。
            // 下一句由下一帧最前面的 `advance_voice` 起出来，闸门在那一帧重新判 ——
            // 于是「跳过」与「台词串行」不矛盾：跳过只是让当前这句立刻结束。
            .add_systems(Update, skip_voice_on_click)
            // 音效自己不排队：它只在这一帧读一遍、去重、放
            .add_systems(Update, collect_sfx)
            // 音量档位改了就把正在响的那几条按到新增益上（只在 `Settings` 变的那一帧动手）
            .add_systems(Update, apply_audio_volumes)
            .add_systems(Update, run_selftest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        ecs::system::RunSystemOnce,
        prelude::{AppExtStates, Messages, MinimalPlugins, NextState},
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
                hold: false,
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

    fn ids(count: usize) -> Vec<Entity> {
        let mut app = App::new();
        (0..count)
            .map(|_| app.world_mut().spawn_empty().id())
            .collect()
    }

    /// 收租这一对：两个人共用**付款那位**的回合序（否则「得钱者先说」会被座位号打乱），
    /// 子序把收款方排在前面。租金是显式接管，所以他们这一帧的通用现金语音一律不再出。
    #[test]
    fn the_rent_pair_pays_the_receiver_first_under_one_seat() {
        let who = ids(2);
        let (payer, owner) = (who[0], who[1]);
        let out = cash_voices(
            1.0,
            &[(payer, owner, 2400)],
            &[(payer, -2400), (owner, 2400)],
            &[],
            &std::collections::HashSet::new(),
        );
        assert_eq!(
            out,
            vec![
                (owner, VoiceEvent::CashBigGain, 0, payer),
                (payer, VoiceEvent::CashBigLoss, 1, payer),
            ]
        );
    }

    /// 门槛随物价；边界含等号。小租金一句不说 —— 但它照样接管了两个人的现金，
    /// 于是「500 的租」不会拐个弯从通用那条路上说出来。
    #[test]
    fn the_floor_scales_with_the_price_index_and_includes_the_edge() {
        assert!(loud_enough(2000, 1.0));
        assert!(!loud_enough(1999, 1.0));
        assert!(loud_enough(4000, 2.0), "物价翻倍，门槛也翻倍");
        assert!(!loud_enough(3999, 2.0));

        let who = ids(2);
        let (payer, owner) = (who[0], who[1]);
        let out = cash_voices(
            1.0,
            &[(payer, owner, 500)],
            &[(payer, -500), (owner, 500)],
            &[],
            &std::collections::HashSet::new(),
        );
        assert!(out.is_empty(), "小额租金不该开口：{out:?}");
    }

    /// 静默族（银行／股票／抵押／买地那一族）压过一切：那些钱在别处已经说过了。
    #[test]
    fn the_quiet_family_says_nothing_even_when_the_amount_is_big() {
        let who = ids(1);
        let mut silent = std::collections::HashSet::new();
        silent.insert(who[0]);
        let out = cash_voices(1.0, &[], &[(who[0], 9000)], &[], &silent);
        assert!(out.is_empty(), "{out:?}");
    }

    /// 命运那笔由 fate_* 接管；同一帧还有别的现金时也不再说 cash_big_*。
    #[test]
    fn fate_takes_the_cash_line_over() {
        let who = ids(1);
        let out = cash_voices(
            1.0,
            &[],
            &[(who[0], 5000)],
            &[(who[0], VoiceEvent::FateLoss)],
            &std::collections::HashSet::new(),
        );
        assert_eq!(out, vec![(who[0], VoiceEvent::FateLoss, 0, who[0])]);
    }

    /// 同一个人同帧几笔现金只取绝对值最大的那一笔；正的排在负的之前（同一子序里稳定排序）。
    #[test]
    fn one_player_speaks_once_with_the_loudest_amount() {
        let who = ids(1);
        let out = cash_voices(
            1.0,
            &[],
            &[(who[0], 3000), (who[0], -5000), (who[0], 4000)],
            &[],
            &std::collections::HashSet::new(),
        );
        assert_eq!(out, vec![(who[0], VoiceEvent::CashBigLoss, 0, who[0])]);
    }

    /// 排队的顺序键是 `(事件主语回合序, 子序)`，同键保持入列顺序（稳定排序）；
    /// 满 8 条丢的是**队尾**的后来者，队头那一句不许丢。
    #[test]
    fn the_queue_is_ordered_by_seat_and_drops_the_tail() {
        let who = ids(3);
        let mut cues = vec![
            VoiceCue {
                player: who[0],
                event: VoiceEvent::Enter,
                seat: 2,
                sub: 0,
            },
            VoiceCue {
                player: who[1],
                event: VoiceEvent::Framed,
                seat: 0,
                sub: 1,
            },
            VoiceCue {
                player: who[2],
                event: VoiceEvent::Enter,
                seat: 0,
                sub: 0,
            },
        ];
        let mut pending = std::collections::VecDeque::new();
        assert_eq!(line_up(&mut cues, &mut pending, MAX_QUEUE), 0);
        let order: Vec<Entity> = pending.iter().map(|cue| cue.player).collect();
        assert_eq!(order, vec![who[2], who[1], who[0]]);

        let mut many: Vec<VoiceCue> = (0..10)
            .map(|i| VoiceCue {
                player: who[0],
                event: VoiceEvent::Enter,
                seat: 0,
                sub: i as u8,
            })
            .collect();
        let mut pending = std::collections::VecDeque::new();
        assert_eq!(line_up(&mut many, &mut pending, MAX_QUEUE), 2);
        assert_eq!(pending.len(), MAX_QUEUE);
        assert_eq!(pending.front().map(|cue| cue.sub), Some(0), "队头被丢了");
        assert_eq!(pending.back().map(|cue| cue.sub), Some(7));
    }

    /// 把 `collect_voice` 一个人跑起来：二十来条消息少注册一条，`VoiceInbox` 就建不出来，
    /// 整条 system 静默不跑 —— 而那正是本仓库反复踩的那种「一条都不报」的失效。
    fn voice_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<VoiceQueue>()
            .init_resource::<VoiceCaption>()
            .init_resource::<CaptionClock>()
            .insert_resource(AudioConfig {
                playback: false,
                hold: false,
                log: false,
            })
            .add_message::<CashChanged>()
            .add_message::<RentDue>()
            .add_message::<CardDrawn>()
            .add_message::<CardPlayed>()
            .add_message::<ItemPlaced>()
            .add_message::<LandBought>()
            .add_message::<PlayerDetained>()
            .add_message::<PlayerBankrupt>()
            .add_message::<GameOver>()
            .add_message::<TurnStarted>()
            .add_message::<BankOpDone>()
            .add_message::<StockTraded>()
            .add_message::<MortgageDone>()
            .add_message::<FinancingDrawn>()
            .add_message::<FinancingRecalled>()
            .add_message::<LandUpgraded>()
            .add_message::<FacilityBuilt>()
            .add_message::<LotteryBought>()
            .add_message::<AuctionWon>()
            .add_message::<DiceRolled>()
            .add_message::<PlayerBailed>()
            .add_message::<UiSoundRequest>()
            .add_systems(Update, collect_voice);
        app
    }

    fn speaker(app: &mut App, name: &str, seat: u32) -> Entity {
        app.world_mut()
            .spawn((PlayerName(name.to_string()), PlayerId(seat)))
            .id()
    }

    /// 买地那一笔**不许出声**（静默族）。这一条是踩出来的：第一版 `collect_voice` 两处
    /// 各 `read()` 一遍 `LandBought`，前面那次只用来查聚街，静默那一半于是永远漏着 ——
    /// 每买一块地念一句「大额失钱」，而这正是 60 秒长跑里 22 句全是 loss 的来源。
    #[test]
    fn a_purchase_is_quiet_even_though_it_is_loud_money() {
        let mut app = voice_app();
        let buyer = speaker(&mut app, "姬百年", 0);
        app.world_mut()
            .resource_mut::<Messages<CashChanged>>()
            .write(CashChanged {
                player: buyer,
                delta: -16_500,
                balance: 183_500,
            });
        app.world_mut()
            .resource_mut::<Messages<LandBought>>()
            .write(LandBought {
                player: buyer,
                land: LandId(24),
                price: 16_500,
            });
        app.update();
        assert!(
            app.world().resource::<VoiceQueue>().pending.is_empty(),
            "买地这一笔现金不该开口：{:?}",
            app.world().resource::<VoiceQueue>().pending
        );
    }

    /// 正面的一半：一笔大额租金要真的进队，且收款方在前 —— 没有这一条，上面那条
    /// 「什么都不说」在「整条 system 根本没跑」时也是绿的。
    #[test]
    fn a_loud_rent_reaches_the_queue_receiver_first() {
        let mut app = voice_app();
        let payer = speaker(&mut app, "莱息", 1);
        let owner = speaker(&mut app, "范一文", 2);
        app.world_mut()
            .resource_mut::<Messages<RentDue>>()
            .write(RentDue {
                payer,
                owner,
                land: LandId(24),
                amount: 2_400,
            });
        for (who, delta) in [(payer, -2_400), (owner, 2_400)] {
            app.world_mut()
                .resource_mut::<Messages<CashChanged>>()
                .write(CashChanged {
                    player: who,
                    delta,
                    balance: 0,
                });
        }
        app.update();
        let queue = app.world().resource::<VoiceQueue>();
        let said: Vec<(Entity, VoiceEvent, u8)> = queue
            .pending
            .iter()
            .map(|cue| (cue.player, cue.event, cue.sub))
            .collect();
        assert_eq!(
            said,
            vec![
                (owner, VoiceEvent::CashBigGain, 0),
                (payer, VoiceEvent::CashBigLoss, 1),
            ],
            "收款方该先说，付款方后说"
        );
    }

    /// P16b 的接线：队列非空 → 闸门落下；念完 → 松开。
    ///
    /// 两半缺一不可：只断言「按下」的话，把闸门写成恒真也是绿的；而恒真的闸正好是
    /// 那种「整局静默停住、画面还在动」的失效。第三段钉长跑与静音那两条路 ——
    /// 它们也走这条 system，只是 `hold: false` 时队列里有东西也不许按。
    #[test]
    fn a_pending_line_holds_the_turn_machine_and_releases_it_when_done() {
        fn wired(hold: bool) -> App {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .init_resource::<VoiceQueue>()
                .init_resource::<PerformanceHold>()
                .init_resource::<Selftest>()
                .insert_resource(AudioConfig {
                    playback: true,
                    hold,
                    log: false,
                })
                .add_systems(First, hold_for_voice);
            app
        }
        let cue = VoiceCue {
            player: Entity::PLACEHOLDER,
            event: VoiceEvent::CashBigGain,
            seat: 0,
            sub: 0,
        };

        let mut app = wired(true);
        app.world_mut()
            .resource_mut::<VoiceQueue>()
            .pending
            .push_back(cue);
        app.update();
        assert!(
            app.world().resource::<PerformanceHold>().0,
            "还有一句要念，回合机该停住"
        );

        app.world_mut().resource_mut::<VoiceQueue>().pending.clear();
        app.update();
        assert!(
            !app.world().resource::<PerformanceHold>().0,
            "念完了就该松闸"
        );

        // NM_AUTOROLL / NM_AUDIO=0 那两条路：队列有东西也不按
        let mut app = wired(false);
        app.world_mut()
            .resource_mut::<VoiceQueue>()
            .pending
            .push_back(cue);
        app.update();
        assert!(
            !app.world().resource::<PerformanceHold>().0,
            "长跑与静音模式不该被台词闸住"
        );
    }

    /// 开场白的节奏（P16b 修正）：**每人自己的第一回合前念自己那句** ——
    /// 第一位回合开始只进队一句，第二位回合开始再进下一句；念过的不再念。
    /// 旧版在第一位回合前把全员连成串，这条测试的「恰好一句」就是那版的回归线。
    #[test]
    fn each_player_announces_on_their_own_first_turn() {
        let mut app = voice_app();
        let a = speaker(&mut app, "姬百年", 0);
        let b = speaker(&mut app, "莱息", 1);

        // 第一位上场：队列里只有他自己的那一句
        app.world_mut()
            .resource_mut::<Messages<TurnStarted>>()
            .write(TurnStarted { player: a });
        app.update();
        let said: Vec<Entity> = app
            .world()
            .resource::<VoiceQueue>()
            .pending
            .iter()
            .map(|cue| cue.player)
            .collect();
        assert_eq!(
            said,
            vec![a],
            "第一位回合前只该有他自己的上场台词，不该全员连串：{said:?}"
        );

        // 第一位行动完，第二位上场：追加的正是第二位那句
        app.world_mut().resource_mut::<VoiceQueue>().pending.clear();
        app.world_mut()
            .resource_mut::<Messages<TurnStarted>>()
            .write(TurnStarted { player: b });
        app.update();
        let said: Vec<Entity> = app
            .world()
            .resource::<VoiceQueue>()
            .pending
            .iter()
            .map(|cue| cue.player)
            .collect();
        assert_eq!(said, vec![b], "第二位上场前才轮到他那句：{said:?}");

        // 第二位又轮到一圈：念过的不再念
        app.world_mut().resource_mut::<VoiceQueue>().pending.clear();
        app.world_mut()
            .resource_mut::<Messages<TurnStarted>>()
            .write(TurnStarted { player: a });
        app.update();
        assert!(
            app.world()
                .resource::<VoiceQueue>()
                .pending
                .iter()
                .all(|cue| cue.player != a),
            "开场白一人只念一次"
        );

        // 回菜单清账（`clear_voice`）：新局的第一回合不记账。
        // `run_system_once` 缺资源时**不 panic 而是返回 Err** —— 写 `let _ =` 会让
        // 这条用例在「清账其实没跑」时绿着，所以这里要它必须跑成功。
        app.world_mut()
            .run_system_once(clear_voice)
            .expect("clear_voice 的参数必须齐全（VoiceCaption／CaptionClock 等）");
        app.world_mut().resource_mut::<VoiceQueue>().pending.clear();
        app.world_mut()
            .resource_mut::<Messages<TurnStarted>>()
            .write(TurnStarted { player: a });
        app.update();
        let said: Vec<Entity> = app
            .world()
            .resource::<VoiceQueue>()
            .pending
            .iter()
            .map(|cue| cue.player)
            .collect();
        assert_eq!(said, vec![a], "清账之后开场白要重新起头：{said:?}");
    }

    #[derive(Resource, Default)]
    struct Captured(Vec<&'static str>);

    fn capture_sfx(mut inbox: AudioInbox<'_, '_>, mut out: ResMut<Captured>) {
        out.0 = sfx_for_frame(&mut inbox);
    }

    /// 同一帧两掷只响一下；两笔进账也只响一下（同键合并），出账另算一条。
    /// 顺序就是表里的顺序（骰子在前）。
    #[test]
    fn a_dice_roll_sounds_once_and_coins_split_by_direction() {
        let mut app = voice_app();
        app.init_resource::<Captured>()
            .add_systems(Update, capture_sfx);
        let who = speaker(&mut app, "姬百年", 0);
        for _ in 0..2 {
            app.world_mut()
                .resource_mut::<Messages<DiceRolled>>()
                .write(DiceRolled {
                    player: who,
                    steps: 5,
                    dice: 1,
                    faces: [5, 0, 0],
                    from: game_core::prelude::TileIndex(0),
                    to: game_core::prelude::TileIndex(5),
                    tumble: true,
                });
        }
        for delta in [3000, 4000, -5000] {
            app.world_mut()
                .resource_mut::<Messages<CashChanged>>()
                .write(CashChanged {
                    player: who,
                    delta,
                    balance: 0,
                });
        }
        app.update();
        assert_eq!(
            app.world().resource::<Captured>().0,
            vec!["dice_roll", "coin_gain", "coin_loss"],
            "同帧同键该合并成一条"
        );
    }

    /// 买地那一笔同帧也有 CashChanged，但它不是「丢钱」，不给金币声（与语音同源）；
    /// P16b 起它给的是 `land_place` —— 标志物砸在自家地上那一声，与落下动画同步。
    #[test]
    fn a_purchase_makes_no_coin_noise() {
        let mut app = voice_app();
        app.init_resource::<Captured>()
            .add_systems(Update, capture_sfx);
        let buyer = speaker(&mut app, "姬百年", 0);
        app.world_mut()
            .resource_mut::<Messages<CashChanged>>()
            .write(CashChanged {
                player: buyer,
                delta: -16_500,
                balance: 0,
            });
        app.world_mut()
            .resource_mut::<Messages<LandBought>>()
            .write(LandBought {
                player: buyer,
                land: LandId(24),
                price: 16_500,
            });
        app.update();
        assert_eq!(
            app.world().resource::<Captured>().0,
            vec!["land_place"],
            "买地只该有摆放那一声，不该有金币声"
        );
    }

    /// UI 四声走消息：派发侧写什么，这一帧就放什么（没有按钮的「点击」也照放）。
    #[test]
    fn the_ui_requests_are_played_verbatim() {
        let mut app = voice_app();
        app.init_resource::<Captured>()
            .add_systems(Update, capture_sfx);
        for sound in [UiSound::Error, UiSound::Confirm] {
            app.world_mut()
                .resource_mut::<Messages<UiSoundRequest>>()
                .write(UiSoundRequest(sound));
        }
        app.update();
        assert_eq!(
            app.world().resource::<Captured>().0,
            vec!["ui_error_soft", "ui_confirm_soft"]
        );
    }

    /// 十五个语音事件各拼自己那一格；没有录音的角色（隐藏位／无名座位）一条都拼不出来 ——
    /// 静默是设计，不是把别人的嗓子借给他。
    #[test]
    fn every_voice_event_resolves_to_its_own_file() {
        let mut paths: Vec<String> = VoiceEvent::ALL
            .iter()
            .filter_map(|event| voice_path("姬百年", *event))
            .collect();
        assert_eq!(paths.len(), 15);
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), 15, "有两条事件指向同一个文件");
        assert_eq!(
            voice_path("姬百年", VoiceEvent::CashBigLoss).as_deref(),
            Some("audio/voice/ji-bainian/cash_big_loss.ogg")
        );
        assert_eq!(voice_path("沈万三", VoiceEvent::Enter), None);
        assert_eq!(voice_path("甲", VoiceEvent::Enter), None);
    }

    /// 字幕表的事件键与 `VoiceEvent::file()` 必须逐字相同。一边是 enum、一边是 JSON，
    /// 拼错一个字谁都不会报错 —— 症状只有「有声音、没字幕」。
    #[test]
    fn every_voice_event_has_a_dialogue_line() {
        let lines = crate::dialogue::DialogueLines::load();
        for event in VoiceEvent::ALL {
            assert!(
                lines.line("ji-bainian", event.file()).is_some(),
                "{} 在台词表里查不到，字幕会静默缺失",
                event.file()
            );
        }
    }

    /// 字幕的停留时长：短句有下限、长句有上限、中间跟着字数走。静音模式下这是
    /// 「字还没读完就换下一条」唯一挡板 —— 队列本身一帧就能走完。
    #[test]
    fn caption_dwell_scales_with_line_length_but_stays_bounded() {
        assert_eq!(caption_secs(""), 1.2, "空句也给一个下限");
        assert!(caption_secs(&"字".repeat(20)) > caption_secs("站住。"));
        assert_eq!(caption_secs(&"字".repeat(100)), 6.0, "长句封顶");
    }

    /// 起播那一小段里不认跳过：实报「开局第一个角色没有登场台词」就是启动那一屏的
    /// 点击在开场白起播之后才被读到（日志里两条相隔 13ms）。同一句被点一下之后
    /// （过了宽限期）仍然必须掐得掉，否则跳过就废了。
    #[test]
    fn a_click_that_was_already_in_flight_does_not_kill_the_first_line() {
        assert!(
            !click_may_skip(12.137, 12.150),
            "13ms 前起播的那一句，不该被启动那一屏的点击掐掉"
        );
        assert!(!click_may_skip(12.137, 12.3), "宽限期内仍不掐");
        assert!(click_may_skip(12.137, 12.6), "过了宽限期照常掐");
    }

    /// 上屏状态机：`show` 换人换词并递增 `seq`（UI 按它决定重写文字），空句与
    /// `clear` 都收起来；本来空着时 `clear` 不再动 `seq` —— 否则每帧空转都会让
    /// 气泡重排一次。
    #[test]
    fn caption_shows_and_clears_without_spinning_the_sequence() {
        let mut caption = VoiceCaption::default();
        let speaker = Entity::PLACEHOLDER;
        caption.show(speaker, "站住。");
        assert_eq!(caption.player, Some(speaker));
        assert_eq!(caption.seq, 1);

        caption.show(speaker, "  ");
        assert_eq!(caption.player, None, "空句等于没有台词");
        assert_eq!(caption.seq, 2);

        caption.clear();
        assert_eq!(caption.seq, 2, "本来就空着，不该再动 seq");
    }

    /// 每条正在响的 sink 归哪条总线。串线的症状是「拉满语音，背景音乐也变响」，
    /// 而两边都出得了声、都在该在的位置上 —— 耳朵分得出，测试分不出，除非这里钉住。
    #[test]
    fn a_sink_belongs_to_the_bus_that_started_it() {
        assert_eq!(
            bus_of(Some(&BgmTag::World(2)), None),
            Some(VolumeBus::Bgm),
            "世界列表那条该走 BGM 档"
        );
        assert_eq!(
            bus_of(Some(&BgmTag::Solo(Solo::Menu)), None),
            Some(VolumeBus::Bgm),
            "菜单曲也是 BGM 总线"
        );
        assert_eq!(
            bus_of(Some(&BgmTag::Event(EventMusic::Fiesta)), None),
            Some(VolumeBus::Bgm),
            "事件曲也是 BGM 总线"
        );
        assert_eq!(
            bus_of(None, Some(&VoiceLine)),
            Some(VolumeBus::Voice),
            "语音该走语音档"
        );
        assert_eq!(bus_of(None, None), None, "音效是一次性的，不在这条路上");
    }
}
