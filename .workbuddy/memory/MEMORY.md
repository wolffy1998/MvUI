# 项目长期记忆 — MvUI

## 这是什么
mamepgui 1.8.2（Qt/C++）用 Rust + egui 重写。**项目名 MvUI，已迁移到
`C:\Users\11921\Desktop\MvUI`（2026-10-04），并推送到 GitHub
`wolffy1998/MvUI`（SSH 443，`gh` CLI 未装，用纯 git）。**
**Rust 工程就是仓库根**（单 crate `mvui`）：`src/core/` = 纯逻辑层（不依赖 egui），
其余 `src/*.rs` = UI 层，产物 `target/release/mvui.exe`。
旧目录 `Desktop\mamepgui-rewrite\` **已于 2026-10-04 删除**（内容全部并入 MvUI，
只剩一个空目录壳）。项目记忆现在也住在这里：`.workbuddy/memory/`。

## 最高优先级约定
**一切行为以旧版 mamepgui 1.8.2 为准。** 参考源码在
`D:\Game\Tools\MAME\msys64\src\mamepgui-1.8.2`（`audit.cpp` / `gamelist.cpp` /
`prototype.cpp` / `mainwindow.cpp` / `utils.cpp` / `res/`）。
新功能或修 bug 前，先到旧版找对应实现，按它的语义改，再考虑"要不要更宽容"。
Rust 里的代码注释统一用 `origin: xxx` 标注对应的旧版函数/行号。

## 两份 README，各管一段（别搞混）
- 根目录 `README.md` = **旧版 1.8.2 的架构分析** → 行为基准（旧版怎么做，就怎么做）。
- `.workbuddy/docs/DESIGN.md` = **本项目的重构设计文档** → 实现基准
  （§6 业务 / §9 图标 / §12 并发线程 / §13 配置缓存 / §17 功能对照清单）。
  改动记录：`.workbuddy/docs/OPTIMIZATION.md`。旧版架构分析（旧版行为基准）
  在 `.workbuddy/docs/ANALYSIS-mamepgui-1.8.2.md`，图片/文档加载架构在
  `.workbuddy/docs/WORKSPACE.md`。

## 关键文件对照
| Rust | 旧版对应 |
| --- | --- |
| `src/core/audit.rs` | `audit.cpp` |
| `core/src/listxml.rs` | `MameDat` 的 listxml 解析 |
| `core/src/cache.rs` | `MameDat::save/load`（gamelist.cache） |
| `core/src/folders.rs` | `gamelist.cpp::initFolders` / `filterAcceptsRow` |
| `core/src/icons.rs` | `gamelist.cpp::loadIconWorkder`（机器图标：icons.zip/散装 .ico） |
| `core/src/options/mod.rs` | `mameopt.cpp` |
| `app/src/views.rs` | `GameListDelegate` + sort/filter proxy |
| `app/src/ui.rs` | `mainwindow.ui` + dock |
| `app/src/icons.rs` + `assets/icons/` | `res/`（build.rs 嵌进二进制） |

## 已知约定 / 坑
- 列宽默认值来自旧版 `res/mamepgui.ini` 的 `column_state`；见
  `app.rs::COL_DEFAULT_WIDTH` / `COL_MIN_WIDTH`。
- egui_extras：**只有末列**是 `Column::remainder()` 时才会每帧重算宽度；
  非末列的 resizable remainder 会被第一帧宽度永久冻结。
- rompath 一律取 `OptionCore` 里 `opts["rompath"].currvalue`
  （mame.ini 经选项链后的当前值），不要自己拼路径。
- 审计的 crc 匹配范围是"本游戏 + 其克隆集"，**不是全库**；全库 crc 索引
  会因 BIOS/设备 rom 被数千机种共用而爆炸。
- 7z 读归档头 `FileCRC`，不解压。
- 缓存 `gamelist.cache` 有 `audited` 标志：false=只解析过，true=审计完。
  换 mame.exe 会因 `mame_version` 串不同而整库重建（预期行为）。
- `GameLibrary.index` 是 `#[serde(skip)]`，反序列化后必须
  `rebuild_indexes()`。
- **工程没有 git 仓库**（`git status` 报 not a repository）。改坏文件只能手写补回，
  所以批量删除/替换后**必须立刻 `cargo check`**，别连删十几处再编译。
  另外「一次改多文件」的脚本里，两处同文案的 replace 要留意命中数。
- `build.rs` 的 `CARGO_MANIFEST_DIR` **就是仓库根**，引用 `assets/` 只需一层
  `assets/` 必须用 `CARGO_MANIFEST_DIR` 拼 `../../assets/...`；图标资源改动后
  可查 `target/*/build/mvui-*/out/icon_assets.rs`（应有 105 项）。
- source 级 ini 名按 **MAME 本体**算：`ini/source/<文件名去扩展名>.ini`
  （`mame-0.168/src/emu/emuopts.cpp::parse_standard_inis`，只取文件名、丢目录）。
  别照抄旧版 `sourcefile.replace(".c",".ini")`（对 `.cpp` 会产出 `pacman.inipp`）。
- `lc_desc` / `lc_mftr` 是**本地化名称**（旧版从 `.mmo` 二进制读，
  `gamelist.cpp:1830-1885`），移植版从未填充 → 「本地化游戏列表」开关目前空操作。
  别把它们当"小写副本"做 serde skip，会丢数据。
- 非 UTF-8 的 ini 统一走 `options::read_text_file`（BOM → UTF-8 → GB18030）；
  旧的 `read_to_string` 会让该级设置被静默忽略、随后保存按默认值覆盖用户配置。
- **左侧文件夹树是手画的行**，不是 `CollapsingState::show_header`：那个 API 只给
  可展开行画 toggler，叶子行的图标/文字会整体左移一个分支列。`ui.rs::folder_row`
  每行都分配分支列（宽度 = `spacing().indent`），有下级的才在里面点响应 + 画三角。
  二级缩进量取 `folder_row` 返回的**实测** label 文字起点，不要用样式常量推算。
  `visuals.indent_has_left_vline` 要关掉（旧版树没那条竖线）。
- `CollapsingState::toggle()` **不持久化**（旧代码靠 `show_body` 顺带 `store`）；
  自己画行后必须 `state.store(&ctx)`，且开着时每帧再 store，否则应用一空闲节点就合上。
- 截图/点击脚本要**按 exe 路径**筛窗口：旧版 1.8.2 的进程也叫 `mamegui.exe`，
  只按进程名会抓到旧版窗口。工具在 `.workbuddy/tools/`（`shot_tree.py` /
  `measure_rows.py`）。
- **`egui::Label` 默认 `Sense::hover()`**，会盖住行/单元格自己的命中矩形把 hover
  抢走 → `Response::context_menu`（依赖 `hovered()`）点得中却弹不出。树行、表格
  单元格的 `Label` 都要 `.sense(egui::Sense { click: false, drag: false,
  focusable: false })`，图标用 `icons::draw_passive`。egui 0.29 **没有**
  `Sense::empty()` / `Sense::nothing()`，只能写结构体字面量。
- 表头悬停/拖拽要**自己做命中测试**（`in_header(p, rect)` + 收集 `hdr_rects`），
  egui_extras 的 header 不产生可用命中。三条坑：拖拽必须用
  `pointer.latest_pos()`（按住时 `interact_pos()` 恒为按下点）；起点要跨帧记住
  （松手时 egui 已清 `press_origin`）→ 放 `app.header_drag`；重排要**松手时单次
  move**（`views.rs::move_column`），逐帧实时换位会级联。
- 列宽存在 `app.col_widths: [f32; COL_LAST]`，**不能放 egui temp data**：表 id 含
  列序，重排列序会重建 egui_extras 状态，temp 数据随之丢失。回写时跳过隐藏列的
  0 宽，否则隐藏列再显示只能回最小宽度。
- 右键菜单是自绘 `egui::Area` + 显式 `Frame`（状态 `app.ctx_menu:
  Option<(Pos2, usize)>`）。**不要用 `Response::context_menu`**：它只在被调用的
  那一帧绘制，而菜单需要 `&mut self`（只能在 table pass 之后组装）。
- sevenz-rust 的 `for_each_entries`：**同一 folder 的所有条目共享一条顺序解码流**，
  跳过条目时必须把 reader 排干（`io::copy(reader, &mut sink())`），否则后续条目
  全部从错位处解码（静默数据损坏）。`list_archive` 也要过滤 `name` 以 `/` 结尾的
  目录条目。
- 命令.dat 记号：`_2_1_4_1_2_3_6` / `_2_3_6_3_2_1_4` 里的 `4` 是**重复项**，原版
  重复后再走普通规则 → 渲染成**两个**图标（`Qdb`+`Hcf` / `Qdf`+`Hcb`），别塌缩成
  一个。`Qdf`（不是 `Qcf`）→ `dir-qdf.png`。多 tag 时每个 tag 各自包一对 `\0`
  分隔符。
- **ini 读写必须共用 `OptionCore::ini_file_for`**：写入侧（`windows.rs::apply_edit`）
  曾手拼 `<mame>/source/<sourcefile.replace(".c","")>.ini`，与读取侧不一致 →
  source 层改完下次读回来是空的（`pacman.cpp` 还会变成 `pacmanpp.ini`）。
  任何"按层写 ini"的新代码都调 `ini_file_for`，别另拼字符串。
- **选项的 GUI 覆盖只认 `guivisible="1"`**（模板里恰好 14 个，即注释里的
  "14 GUI keys"）。按名字匹配会误伤：GUI 设置里的 `language`(zh_CN) 会覆盖 MAME
  核心选项 `language`(English) 并被写进 mame.ini。
- **加锁顺序全局统一 `opts → lib`**（std Mutex 非重入）。UI 线程
  `ensure_chain` 是先 opts 后 lib，所以后台线程（`background::run_audit` 等）
  必须先取 opts 读完就释放、再取 lib，反过来会与"审计途中打开选项对话框"死锁。
- **一个手势 = 一个显式状态机**，别用 `else if` 链摆弄多个 `Option` 状态：
  表头那段曾因链里出现两次 `else if down` 而让第二个永不可达，落点从不更新，
  列重排彻底失效且松手误触发排序。现在是纯函数
  `views::header_step(...) -> HeaderStep` + `shift_slot()`，两者都有测试。
- **按需加载的资源都要"记住失败"**：`game_icons`/`snap_tex`/`dat_texts` 的值
  `None` 表示"确认没有"，并用 `*_requested` latch；判断可请求要同时看两者
  （`icon_needs_request()`），否则无资源的行会每帧重发请求。
  选中相关的加载统一走 `selection_settling()` 150ms 防抖（设计 §12）。
- **窗口背景图：图画在 `CentralPanel` 内部，绝不能用 `Order::Background` +
  `screen_rect()`**。这个坑排查了很久，三种错法都试过：
  (1) 只把 `extreme_bg_color` 半透明 → 那是 egui 的 void 色，**面板不变、图根本看不见**；
  (2) 把 `window_fill` 半透明（那才是 `egui_dock` 的 `TabBodyStyle::bg_fill`
  来源，style.rs:704）→ 槽位对了，但图铺满**整个窗口**把不透明菜单栏/工具栏
  （都用 `panel_fill`）一起埋掉，界面全没；
  (3) 换到 `Order::PanelResizeLine` → 没用，图仍在面板之上。
  正解是 `ui::draw_background(&mut ui)`：拿 `ui.painter_at(ui.max_rect())` 在
  **中央面板自己的 frame 里**画图，绘制顺序交给 egui，图天然在所有 dock 之下。
  然后 `apply_theme_with_bg` 里 `v.window_fill` 半透明 128（= 旧版
  `setTransparentBg` 的 `QPalette::Base`），`panel_fill` 保持不透明。
  另：表头拖拽幽灵底色要用 `panel_fill` 不能用 `window_fill`（浮层要实体底）。
- **DAT 有字节偏移索引**（`core/src/datindex.rs`，设计 §3.2/§3.3）：`$info=标签`
  → 记录字节区间，mtime 失效，实测 **22.998ms → 0.067ms（340x）**。三条铁律：
  (1) **只做性能层**：miss / 过期 / zip 内 DAT 一律回落 `get_history` 线性扫描，
      9 项测试逐字节 pin 两条路径一致；
  (2) **记录范围是「贪婪」的** —— rec_data 只被「不含该 tag 的 `$info=`」清除，
      同一 tag 出现在两条记录里时会吞掉中间那条记录。按位置切块会返回更少
      文本，**行为就变了**；
  (3) 缓存里的 `DatIndex` 必须用 **`Arc`** —— `clone()` 会深拷贝 5 万个 tag 的
      HashMap（约 20ms/次），足以抹掉全部收益（第一版实测就是 0.9x）。
  另外 `history_indexed` 里 cloneof 回退会**递归调自己**，锁作用域必须窄于递归
  路径，否则 `std::sync::Mutex` 不可重入直接死锁（cargo test 会 SIGTERM）。
  基准：`cargo run --release --example datindex_bench`（自造 18MB DAT 测等价+速度）。
- **内容目录一律锚定 exe 目录**（`core/paths.rs`，2026-10-04 起，不再是 mame 目录）：
  `snap/ flyers/ cabinets/ marquees/ titles/ cpanel/ pcb/ dats/ folders/ bkground/`
  和 `mame_cn.lst` 默认全在 `<mvui.exe 所在目录>` 下，目录不存在会创建。
  解析顺序：配置绝对路径原样用 → 相对路径按 **exe 目录** → 没配置用内置默认。
  **`rompath` 是唯一例外**（天生属于 MAME），仍走 `opt_resolved_dirs()`；
  artwork/dat 一律走 `content_image_dirs()` / `content_dat_file()` /
  `content_folders_dir()` / `content_localized_list()` / `content_background_dir()`
  （都经 `content_setting()`：GUI 设置优先，option 链兜底）。
  **改这些默认值只改 `core/paths.rs` 一处**——目录弹窗的空值是
  `gui.remove(key)` 而不是存空串，正是为了守住这个单点。
- **本地化游戏列表读 `mame_cn.lst`**（`core/lst.rs`），不是 1.8.2 的二进制 `.moo`
  （那个从未实现，所以开关一直是空操作）。格式：**3 列 tab 分隔、无表头**
  `set名 \t 描述 \t 厂商`（实测第 2/3 列内容相同）。**文件实际是 GB18030 不是
  UTF-8**，必须走 `options::read_text_file`（BOM→UTF-8→GB18030）。
  在 `LibraryReady` 时应用（不在审计阶段），所以改完按刷新即生效。
- **`window_fill` 必须保持不透明**：egui 从这一个槽位派生了 dock leaf 的
  `TabBodyStyle::bg_fill`（egui_dock style.rs:704）、`Frame::menu`
  （frame.rs:123）和所有 popup。把它设半透明会让**菜单和弹窗一起变透明**。
  壁纸的 veil 只写进 `ui::dock_style(ctx, wallpaper, dark)` 的
  `style.tab.tab_body.bg_fill`（唯一只被 dock leaf 读的槽位），
  `faint_bg_color` 跟着 veil（否则半透明面板上出现不透明亮带）。
  7 个 `egui::Window` 全部加 `windows::opaque_frame(ctx)`。这正是 1.8.2
  `setTransparentBg` 只换**一个**画刷（`QPalette::Base`）的做法。
- **选项对话框不用 ComboBox**：枚举（kind=3）直接平铺 `selectable_label`，
  超过 6 项分两列。模板最宽的枚举 `scale_effect` 只有 18 项，平铺成本远低于
  一次点击；ComboBox 展开时还盖住下面的行且继承半透明底色。
- **背景目录按旧版语义**：`background_directory` 选项 + 内置默认 `bkground`
  （沿用 1.8.2 的拼写），但基准目录已从 mame 目录改为 exe 目录。
- **背景纹理必须降采样**：用户那张是 5888x3312 = 19.5 Mpx，直接 `load_texture`
  吃 ~78MB 显存。`BG_MAX_EDGE = 4096` + `fit_within()`（保持宽高比、只缩不放）。
  `background_is_dark` 同样先缩到 64 再求均值，否则 UI 线程要卡几百毫秒。
- **别再用 `ImageGrab(all_screens=True).crop(rect)` 验证界面**：窗口会跑到副屏、
  别的窗口会盖在上面，裁出来的是桌面/别的程序——曾因此误判"背景图把界面全挡住"
  折腾半天。用 `.workbuddy/tools/win_shot.py`（`PrintWindow` + `PW_RENDERFULLCONTENT`）
  或先把窗口 `SetWindowPos` 到主屏置顶再抓。旧 `shot_tree.py` 会最大化，
  尺寸对不上，别混用。
- 维护基线：`cargo check --workspace` **0 warning**、`cargo test --workspace`
  **46 项**通过（core 40 + app 6）、`cargo audit` **0 漏洞**。
  依赖：`encoding_rs`（GBK）、`winresource`（build-dep，exe 图标，需 windres）、
  `quick-xml 0.41`（从 0.36 升上去修 RUSTSEC-2026-0194/0195）。
- **安全边界（2026-10-04 审计）**：
  - `archive::extract_mame_file` 走 `sanitized_join()`，拒绝 traversal / 绝对路径 /
    盘符相对(`C:foo`) / 反斜杠分隔，并做词法归一化复核（5 个测试）。**任何新增的
    "从归档/dat 名拼路径写盘"都必须走它**，别直接 `join`。
  - `icons.rs` 不再按归档头声明的 size 预分配（`read_capped` + `MAX_ICON_BYTES`），
    防恶意 icons.zip 的分配 DoS。
  - sevenz-rust 的 RUSTSEC-2026-0245（decompress_impl 路径穿越）**不适用**：
    我们只用 `SevenZReader::open` + `for_each_entries`。依据留档在
    `.cargo/audit.toml`（注意是 `.cargo/audit.toml`，
    cargo-audit 0.22 的固定路径，`--file` 是指定 lockfile 不是配置）。
  - MAME 一律 `Command::arg` 启动，**不经 shell**，无命令注入面。
- `optiontemplate.xml` **只有一份**，在仓库根 `assets/`；
  core 用 `include_str!("../../../../assets/optiontemplate.xml")`，
  曾经在 `crates/mamegui-core/assets/` 还有一份副本（已删）——别再复制。
- **表头浮动幽灵**（MxUI 式拖动）：`app.header_drag_x: f32` 在按下时锁存
  `pointer.x`（松手 egui 会清 `press_origin`），拖动中用
  `egui::Area("header_drag_ghost", order=Foreground, interactable(false))` 画在
  `src_rect.left() + (pointer.x - header_drag_x)`，内容=源列表头标题+
  `window_fill` 底+selection 色描边。与 `header_drag_insert` 插入指示线并存
  （幽灵=手里拿着什么，指示线=会落在哪）。该 Area 必须画在 `.body()` 之后
  ——`hdr_rects`/`table_out` 那时才拿得到。
- **构建前先杀进程**：产物已改名 **`target/release/mvui.exe`**（不再有
  `mamegui.exe`）。`cargo clean` / `cargo build --release` 报
  `os error 5 拒绝访问` 就是有实例在跑锁住了 exe →
  `taskkill /IM mvui.exe /F`（旧名 `mamegui.exe` 也补一条）。

## 构建
```
cargo check --workspace
cargo test --workspace
cargo build --release   # 产物 target/release/mvui.exe（约 9.6MB）
```
构建 exe 图标需要 MinGW 的 `windres` 在 PATH
（`%USERPROFILE%\scoop\apps\mingw\current\bin`）；缺失时只打 `cargo:warning` 不失败。
全量 `cargo clean` 后重编 release 约 **9 分钟**；`target` 稳定在 ~548MB。
