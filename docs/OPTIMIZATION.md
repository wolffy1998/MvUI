# mamegui-rs 优化记录与逻辑预览

对照两份文档核对实现后的一次修正 + 补全：

- `../README.md` —— 原版 mamepgui 1.8.2 的技术架构（**行为基准**：一切以旧版为准）
- `../README-OG.md` —— 本项目的重构设计文档（**实现基准**：§6 业务、§9 图标、§12 并发、§13 配置）

基线（改动前）：`cargo check --workspace` 0 warning，`cargo test --workspace` 9 项通过。
现状：`cargo check` 0 warning，`cargo test` **22 项**通过，`cargo build --release` 产出 `target/release/mamegui.exe`。

---

## 一、修掉的行为偏差

### 1. 列表列拖拽重排完全失效（新增测试 4 项）

**问题**：`views.rs::draw_table` 的表头指针处理是一条 `else if` 链，其中出现两次
`else if down`；第二个永远不可达（`down == true` 时已被前一个分支吃掉）。
后果链：拖拽过程中从不记录落点 → `header_drag` 恒为 `(from, from, false)` →
`moved` 永远 false → `move_column` 成为死代码，且**松手被当成普通点击，反而把
那一列排序了**（双击场景下还会误重置列宽）。

**修法**：抽成纯状态机 `header_step(pressed, down, resize_active, drag_active, sep, slot)`
返回 `HeaderStep::{Idle, BeginResize, BeginDrag, UpdateResize, UpdateDragTo, EndResize, EndDrag}`，
调用处只做副作用；列位移算法抽成 `shift_slot()`（`QHeaderView::moveSection` 语义：
整段平移，不是两两交换）。两段逻辑都是纯函数，因此可以直接测。

### 2. 驱动源（source）级选项保存到了错误的文件

**问题**：`windows.rs::apply_edit` 手拼 `<mame>/source/<sourcefile 去掉 ".c">.ini`，
而读取侧 `OptionCore::ini_file_for` 用的是 `<inidir>/ini/source/<文件名去扩展名>.ini`
（README 4.4 + mame-0.168 `parse_standard_inis`）。两侧不一致意味着
**改完 source 层、下次启动读回来是空的**；且对 `pacman.cpp` 这种现代驱动，
`.replace(".c","")` 会产出 `pacmanpp.ini`。

**修法**：写入侧改用同一个 `ini_file_for`（BIOS / Cloneof / Game 三层一并统一），
写失败不再静默而是进日志。加 2 项回归测试锁死路径规则。

### 3. GUI 自身设置污染 MAME 选项

**问题**：`options/mod.rs::load_ini` 的 "GUI-overlap" 分支按**名字**匹配
`pGuiSettings` 的键。而 `language` 既是 GUI 设置（值如 `zh_CN`）又是 MAME 的
真实选项（值如 `English`），于是核心选项被覆盖，随后又会被 `save_ini_file`
写进 `mame.ini` 变成非法的 `language zh_CN`。

**修法**：按设计文档 §6.4-4「GUI 层设置不进 ini 体系」改为只认
`guivisible="1"` 的选项——模板里恰好 14 个，与代码注释里的 "14 GUI keys" 吻合。

### 4. 审计线程与选项对话框可能互锁

**问题**：`background::run_audit` 是 `lib → opts` 的加锁顺序，而 UI 线程的
`ensure_chain` 是 `opts → lib`。审计跑着的时候打开选项对话框即可死锁。

**修法**：审计线程改为先取 opts 读完路径释放，再取 lib 做快照，全局统一为 `opts → lib`。

---

## 二、按设计文档补全

### 5. 游戏图标管线（设计文档 §9.3，旧版 README §6.1③）

之前行首只有状态色方块，图标库（icons.zip / 散装 .ico）完全没接。现在：

| 环节 | 实现 |
| --- | --- |
| 查找 | `core::icons::read_game_icon(dirs, game)`：`icons.zip`（中央目录直查 + 大小写回退）→ `icons.7z`（走 `iterate_mame_file` 的顺序流）→ 散文件（目录内与 `icons/` 子目录） |
| 继承 | `icon_candidates()`：克隆回退父集、MESS 软体回退主机；列表侧再沿 cloneof 链向上走 |
| 传输 | 后台 `background::load_icon` → `AppEvent::IconReady` → 主线程解码上传纹理 |
| 缓存 | `game_icons` 带 LRU（512 张）；`None` 表示"确认无图标"，避免每帧重复请求 |
| 渲染 | 有图标画 16px（大图标视图 48px），否则仍是状态方块（`icons::draw_passive`，不抢行的命中区） |

取"按需加载 + LRU"而非设计文档写的"一次性扫出全量字节"：现代图标包是几万个文件、
上百 MB，而屏幕上同时只有几十行，全量读入没有收益。

### 6. 选中项防抖（设计文档 §12）

新增 `sel_changed_at` + `selection_settling()`（150ms）；`request_preview / request_dat /
request_game_icon` 在防抖窗口内不发任务。快速滚动 4 万行时不再为掠过的每一台机器
排队加载截图 / DAT / 图标。

### 7. 主题与背景模式持久化（设计文档 §11、§13）

`dark_bg / bg_stretch / bg_tile` 之前可切换但从不写盘，每次启动又回到深色。
现已纳入 `mamepgui.ini`，启动时读回（默认仍为深色）。

### 8. 唯一的资源副本

`optiontemplate.xml` 有两份（workspace 根 + `crates/mamegui-core/assets/`）内容完全相同，
一份喂 `include_str!`、一份没人用，改了其中一份对程序毫无影响。
现在统一指向 workspace 根，删除重复目录。

---

## 三、优化后的逻辑预览

### 模块职责

| 层 | 模块 | 职责 | 本次变化 |
| --- | --- | --- | --- |
| core | `listxml` | `mame -listxml` 流式解析 → `GameMeta` | — |
| core | `library` | 游戏集合 + 名称索引、`complete_data` 派生字段 | — |
| core | `cache` | `gamelist.cache`（magic + 版本 + `audited` 标志） | — |
| core | `folders` | 30 维分类树、自定义文件夹 ini | — |
| core | `audit` | zip/7z/CHD 审计、软体扫描、fixdat 导出 | — |
| core | `archive` | zip/7z 统一扫描（INFO/READ/EXTRACT） | — |
| core | `options` | 六层 ini 继承链、模板、写回 | **修 source 路径、GUI 覆盖** |
| core | `icons` | **新增**：机器图标查找与继承链 | 新增 |
| core | `launcher` / `mameproc` / `settings` / `dat` / `model` | 启动参数、子进程、配置、DAT 解析、数据模型 | — |
| app | `app` | 单一可写状态机 + 事件泵 + 缓存 | 图标缓存、防抖、设置 |
| app | `views` | 列表渲染、过滤排序、启动、右键菜单 | **表头状态机、图标行** |
| app | `ui` | 菜单/工具栏/停靠树/目录树/状态栏 | 防抖打戳 |
| app | `windows` | 选项对话框、目录、播放、命令行 | **source 路径** |
| app | `background` | 后台任务：boot / audit / 预览 / DAT / 图标 / verify | 图标任务、锁序 |
| app | `icons` / `i18n` / `fonts` | 内嵌 PNG 注册表、翻译表、CJK 字体 | — |

### 关键路径（优化后）

```
启动   GuiSettings → 探测 mame(-help) → 缓存命中?
         命中(已审计) ────────────────► 发布列表
         否则 -listxml 流式解析 → -showconfig 取默认 ini
              → 选项链 Global 层 → 先落缓存(audited=false)
              → 发布列表（UI 立刻可用）
              → 后台审计 → 目录树重建 → 缓存(audited=true)
选中   行点击 → refilter(可见 id 序列) → 防抖 150ms
          → 截图 ×1 / DAT ×5 / 图标 ×N 各自 latch 后起后台任务
          → 事件回主线程 → 纹理/文本缓存（DAT 200 条、图标 512 张 LRU）
选项   层级切换 → ensure_chain(opts→lib 顺序) → 渲染差异高亮
         编辑 → apply_edit → ini_file_for(level) → save_ini_file(只写改动项)
启动   build_args：附加参数 + 系统名 + 设备挂载 + 语言 + 命令行差异
         → spawn → watcher 线程 → MameExited → 清理临时 rom
```

### 表头手势状态机

```
按下 ─ 在分隔线 ±4px ─► 按住跟踪列宽 ─► 松手：写回列宽（绝不排序）
     └ 在列内      ─► 按住记录目标列 ─► 松手：跨列则重排，否则排序
                                              （双击：恢复默认列宽）
```

---

## 四、设计文档条款核对（仍未对齐的部分，未擅自改）

| 条款 | 现状 | 说明 |
| --- | --- | --- |
| §6.1-2 fixdat 导入（`load_fixdat`） | 未实现 | 只有导出。需要 Logiqx 解析 + 按 rom 名/sha1 合并 + UI 入口，改动面较大 |
| §6.1-3 MESS 软体扫描 | 部分 | `audit_console` 已生成 ext rom，但只在审计时跑，不是独立入口 |
| §8 字体自带（Noto Sans SC 嵌入） | 未实现 | 现在读系统字体（simhei / msyh / 苹方 / meiryo），缺字体时日志告警。egui 默认 Body 12.5px ≈ 9.4pt，与旧版 9pt 相当，字号无需调整 |
| §10 十国语言 | 3 种 | zh_CN / zh_TW / en_US，回退链 zh_TW→zh_CN→en 已按设计实现 |
| §7.2 键盘导航、字母前缀跳转 | 未实现 | 鼠标交互齐全；键盘仅 Ctrl+F / F5 |
| §13 单实例互斥锁、托盘 | 未实现 | 无 `tray-icon` 依赖 |
| §6.1 缓存文件名 `library.bin` | 现为 `gamelist.cache` | 沿用旧版文件名，避免老用户缓存失效；如需与设计一致可改，但会丢弃现有缓存 |
| §5.1 `lc_desc` / `lc_mftr` | 从未填充 | 「本地化游戏列表」开关目前是空操作：旧版从 `.mmo` 二进制读取，格式未确证 |
| IPS / M1 | 按设计不实现 | `Cargo.toml` 已标注 scope |

---

## 五、验证

```
cargo check  --workspace   0 warning
cargo test   --workspace   22 passed（原 9 + 新增 13）
cargo build  --release     target/release/mamegui.exe
```

新增测试分布：表头状态机 3、列位移 1、source ini 路径 1、各级 ini 路径 1、
GUI 覆盖过滤 1、图标查找与继承 6。
