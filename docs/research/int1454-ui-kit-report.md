# INT-1454 — 独立 UI 与上游同步决策报告

日期：2026-11-16  
范围：SolidCraft v0.41.0 首次真实验证；不合并产品分支，不改 #145 的 draft 状态。  
结论类型：**路线 A 部分可行，路线 B 有具体阻塞；推荐先做路线 A 的定向回移。**

## 0. 先回答三个决策问题

### 0.1 能否在来源清楚的旧 UI 上继续开发，并支持未来闭源商业发行？

**可以，前提是继续使用 IntCAD develop 现在的 `crates/ui-egui`，不把 `crates/ui-kit`、`look`、`theme`、`assets/branding` 从品牌期上游导入。**

证据：

- `origin/develop`（`02a786d47`）的 `crates/ui-egui/src/ui_kit/**` 是 66 个文件、34,203 行，和上游 fork 点 `06749ac4`（0.40.0-rc.2，MIT/Apache）的对应文件相比只有 15 行 IntCAD 改动。
- IntCAD develop 全树没有 `Brand and Interface` 文本；没有 `solidcraft_theme`、`solidcraft_look` 或 `crates/look` 依赖。
- 图标 66 个 SVG、25 个主题 JSON、字体在 develop 与 `06749ac4` 字节级一致；这些是 MIT/Apache 或 OFL 资产，已有 ATTRIBUTION 行。
- 未来闭源发行需要保留 MIT 版权/许可文本、Apache `NOTICE`、OFL 字体文本，并说明修改；Apache-2.0 §6 不授予商标权。IntCAD 自己的 DFM、Assistant、Review 代码是 IntCAD 原创，可按自己的条款发行。

未决条件：

1. 不要从 `312454f89` 之后的任何提交导入品牌路径；品牌期新增的 `toast.rs`、`confirm_tests.rs`、菜单/偏好/图标小部件不能直接复制。
2. 若要使用品牌期之前的独立 `crates/ui-kit` 快照（路线 B），需要法律确认 `LICENSE-BRAND` §5 的“published”是否覆盖公开但未打 tag 的 commit `a3475683`，以及 §1(a)/(c) 的“design”主张是否越过 MIT 授权。路线 A 不需要这个确认。
3. IntCAD 的 `crates/theme`/`look` 替换分支目前仍带有品牌 `LICENSE` 文本、25 个品牌期主题和 98% 相同的调色板；若走路线 B，发行前必须清理（见 §2.3）。

### 0.2 为吸收 v0.41.0 的全部开放改进，实际要维护哪几处接口？

开放边界是 `crates/{doc,format,engine,io,kernel,geom,sketch,smlib,render,gpu,viewport*,control*,plugins,mcp*}` 和 `solidcore`；受限边界只有 `crates/{ui-kit,look,theme}`、`assets/{branding,icons/app}`，约 30,021 行，占 `solidcore/` 以外净变更的约 6.0%。

必须长期维护的接口：

| 接口 | 为什么 | 现状 |
|---|---|---|
| 文档格式 `51`→`55` | develop 的最后一个 folded 目录是 `v50`，另有 5 个 pending 文件，写出的 `Formats::current()` 是 51；v0.41.0 已 fold 到 `v55` 且 `pending/` 为空。develop 的 5 个 pending 文件与上游 `v51/` 中 5 个文件字节一致，但上游 `v51` 还包含另外 10 个变更。 | **必须先做格式决策**，否则任何 `crates/doc` 回移都会把 51 重新解释。 |
| `solidcore` / `solidcore-record` rev | 上游把内核作为两个 pin 的依赖推进；IntCAD 的 pin `8ddce901` 不在当前 clone，delta 未测。 | 通过 rev bump 吸收，不在树内 merge 2,800 个 vendored commit。 |
| engine/command/schema | 360 个 commit，最大的开放面；大多数命令可逐条吸收。 | 可直接吸收，但新 UI 入口要单独接。 |
| I/O（STEP/GLB/IGES） | STEP 导出 inside-out、锥/球 seam、GLB assembly nodes 等。 | 多数无品牌依赖；STEP export 的 `solid_shells` 在 develop 不存在。 |
| viewport/render/GPU/control | 上游把 viewport 拆成 `viewport`/`-gpu`/`-egui`，把 control 拆成独立 crate。 | 这是架构选择，不是补丁；单个修复可先回移。 |
| UI 菜单/偏好/图标钩子 | 上游把“功能入口”放在品牌 `ui-kit` 的 menu/prefs/icon 文件里；开放侧只有功能本体。 | 在 IntCAD 内部 `ui_kit` 重新实现小入口，不能 cherry-pick 整个 commit。 |
| 生成的列表/本地化 | `docs/features.md`、`THIRD-PARTY-LICENSES.txt`、`assets/locales/pending/*`、`attribution/pending/*`、`xtask/mesh-decisions.txt` | 每次同步后跑 `cargo xtask lists`、`assets`、`i18n`、`mesh-decisions`。 |

**最大初次投入**：文档格式 51→55 + 内核 rev + 144 个 UI 编译错误的接口适配。**最大长期成本**：每周把开放 commit 分类、拆混合 commit、在 IntCAD 内部 UI 重做品牌侧菜单/偏好入口；不是持续重写整个 UI。

### 0.3 A、B 的人日、推荐路线和下一步

| 路线 | 首次可运行（人日） | 可发布（人日） | 并行人数 | 关键路径 |
|---|---|---|---|---|
| **A 定向回移**（保留 develop UI，按需吸收单个开放改进） | 0.5 / 1 / 2 | 2 / 3 / 5（每个改进） | 1–2 | 选 commit → 拆品牌尾 → 回移 → 构建 → E2E |
| **A 全量 v0.41.0 开放同步**（保留 develop UI） | 8 / 12 / 18 | 22 / 30 / 42 | 2–3 | 文档格式 + 内核 rev + UI 接口 + E2E |
| **B #145 新 SolveApp + 独立 ui-kit** | 12 / 16 / 22 | 35 / 50 / 65 | 3–4 | 法律确认 + 预品牌 kit 适配 + DFM/右栏切片移植 + 视觉/跨平台 |

（乐观 / 最可能 / 保守；人日按 8 小时。数字来源见 §4。）

**推荐：路线 A 的定向回移，先投 2 人 × 10 个工作日（20 人日）。**

下一步的可验收里程碑：

1. 在最新 develop 上回移 3 个开放改进：重复圆 profile（已做）、orbit pivot（已做）、一个 I/O 或 GLB/STEP 修复（待做）。
2. 完成文档格式 51→55 的 3–5 日 spike，给出“能否用 v51–v55 + 本地 v56”的结论。
3. 修复 DFM 旧报告在模型编辑后仍可定位的问题（见 §6.5）。
4. 建立每周同步脚本：按路径分类开放/受限、拆混合 commit、生成待办；用 `v0.40.1..v0.41.0` 回放。
5. 验收：当前 UI 构建 + DFM 双向选择 + 旧报告失效 + 3 个开放改进 + 无品牌 crate 依赖。

达到后再决定是否做全量 v0.41.0 开放同步。**不建议现在投路线 B**，除非法律书面确认预品牌快照可复用，且产品明确需要新版 SolveApp 的 Ribbon/停靠/窗口布局；这些主要是品牌设计，不是开放功能。

## 1. 固定基线

| 引用 | SHA | 说明 |
|---|---|---|
| origin/develop（调研开始时） | `02a786d47180ffa44a99f0508749c1c11b383c0d` | 2026-11-16 11:42；含 #165/#168 |
| IntCAD v0.1.0 tag | `3b9a975299ed6fc6813593e10036f8a00f6a4d8c` | 旧发布，仅用于来源 |
| #145 base | `ac560eec548746872a6502f6df3a296cd08fea6d` | develop at fork |
| #145 上游合并提交 | `d1315f0e8606b4d5b91b9b75a67b302ff717dcc2` | parents `ac560eec` + `3c8efdb5` |
| #145 head | `f45d977481614fad850c7f9ca67e48e6e7956e5f` | 仍为 draft |
| SolidCraft v0.41.0 | `3c8efdb51952ed9a5f3ebc722c2bad1454fe6d80` | 固定目标 |
| 共同祖先 | `06749ac464e16357d0375b78e6b3cac214066493` | |
| 品牌许可首次生效 | `312454f899a371f6dac52348749bc4dcb94d94f0` | 加 `LICENSE-BRAND`、`NOTICE`、`ui-kit/LICENSE` |
| 品牌前最后快照 | `a34756830185f82da3a0e3ddd1ea25ef21111558` | 0.40.0-rc.3，MIT OR Apache-2.0 |

DFM 链路：`#131` `18366bbf0`、`#157` `183cf7836`、`#158` `4eb71a7a6`，均已在 develop。

## 2. 许可与来源

### 2.1 许可时间线

| 版本/提交 | `crates/ui-kit` | `LICENSE-BRAND` | 许可 |
|---|---|---|---|
| 上游 fork 点 `06749ac4`（0.40.0-rc.2） | 无独立 crate；`ui-egui/src/ui_kit/**` | 无 | MIT/Apache |
| `a3475683`（0.40.0-rc.3，品牌前最后 commit） | 103 文件 / 50,748 行；Cargo `license.workspace = true` | 无 | MIT/Apache |
| `312454f89` | 仍是同一批文件，加 `LICENSE` | **首次加入** | 品牌 |
| v0.41.0 `3c8efdb5` | 78 个非测试文件 | 有 | 品牌 |
| IntCAD develop `02a786d47` | 无独立 crate；66 文件内部 `ui_kit` | 无 | MIT/Apache |

`LICENSE-BRAND` §5 原文：

> **5. Earlier versions**  
> Versions of SolidCraft published before this licence took effect stay under the licences they were published with.

### 2.2 可复核来源表

| 路径 | 来源 SHA | 当时许可 | 再发行义务 | 置信度 |
|---|---|---|---|---|
| `crates/ui-egui/src/ui_kit/**`（66 文件，34,203 行） | `06749ac4` + 15 行 IntCAD | MIT/Apache | 保留 MIT 文本、Apache NOTICE、说明修改 | 高 |
| `crates/ui-egui/src/{toolbar,titlebar,timeline,prefs,home,icons,theme,fonts}.rs` | 上游 fork 前 | MIT/Apache | 同上 | 高 |
| `crates/ui-egui/src/{dfm_*,right_rail,project*,agent_chat/**,control*}`（新增 36 文件） | IntCAD 原创 | IntCAD 自己的条款 | 无上游义务 | 高 |
| `assets/icons/ui/**`（66 SVG + design/*.toml） | `06749ac4`…develop 字节一致 | MIT/Apache | 保留 ATTRIBUTION 行 163–166 等 | 高 |
| `assets/styles/ui/themes/*.json`（25） | `06749ac4` | MIT/Apache | ATTRIBUTION 行 207–211+ | 高 |
| `assets/fonts/**` | IBM Plex 等 | OFL 1.1 / 各自许可 | 随字体保留 OFL 文本 | 高 |
| 预品牌独立 `crates/ui-kit`（`a3475683`） | `a3475683` | MIT/Apache（commit，非 tag） | 保留 MIT/Apache、NOTICE、说明修改 | 中（§5 法律解释） |
| v0.41.0 `crates/ui-kit` | `3c8efdb5` | 品牌 | 不可在 IntCAD 分发（无额外授权） | 高 |
| `crates/look`（#145 替换） | 6/8 src 文件是 MIT 文件移动；`glyphs.rs` +35/−9、`pointer.rs` +3/−2、`compass.rs` +6/−151、`icons.rs` +280/−240 | 混合 | 需逐文件 PROVENANCE；主题/图标要回到 `a3475683` | 中 |
| `crates/theme`（#145 替换） | `a3475683` 已有 MIT `theme`；品牌期改 4+/1−（palettes）、18+/0−（lib） | 混合 | 从 `a3475683` 重新生成，不要用品牌期文件 | 中 |
| `assets/branding`、`assets/icons/app` | 上游品牌 | 品牌 | 不得使用 | 高 |

品牌期 UI 增量：`toast.rs` 363 行是唯一没有品牌前来源的非测试文件；其余品牌期编辑约 `+1,073/−558`（`menu` +210、`theme_files` +76、`app_bar` +73、`icons/svg` +55、`confirm` +49、`choice` +38、`actions` +37 等）。这就是路线 B 真正需要独立实现/重写的 UI 量，而不是此前“约 15k 行、20 文件”。

### 2.3 三个具体场景

**(a) 在旧实现上新增 DFM 控件。**  
许可文本：MIT/Apache 允许修改和闭源发行，条件是保留版权/许可文本、传播 Apache NOTICE、说明修改；新代码属于 IntCAD。工程判断：在 `crates/ui-egui/src/ui_kit/` 新建文件，使用 IntCAD 的 theme token；不要以品牌期 `ui-kit` 文件为模板；加 per-file provenance 注释。法律待确认：控件外观若刻意接近品牌期设计，是否落入 §1(a) 的“design”；§3(c) 是否适用于没有 SolidCraft 对应物的 DFM 控件。

**(b) 把后续开放提交的改进移入旧 UI。**  
许可文本：`312454f89` 之后对品牌路径的提交不是 MIT；§5 只保护之前“published”的版本，不保护之后的新 commit；开放路径仍然是 MIT/Apache。工程判断：只 merge/rebase 开放路径；把品牌侧 diff 当 bug 报告，在 IntCAD 的 MIT 文件里重写；不要 `git cherry-pick` 整个 commit，也不要把上游 merge 进来（merge-base 仍是 `06749ac4`，会一次带入所有品牌文件）。法律待确认：品牌期对 MIT 文件的一行修复是否本身受品牌许可（很可能是新版权作品）；仅位于品牌目录但只改开放代码的 commit 是否按路径算品牌材料。

**(c) 从受限目录迁入开放文件的代码。**  
许可文本：许可附着在收到的副本上，不附着在当前路径。MIT 代码移入 `look` 仍是 MIT；品牌代码移入 `ui-egui` 仍是品牌。工程判断：按 `(path@sha, blob, licence)` 追踪，不按目录；混合文件先拆分并在 `PROVENANCE.md` 记录品牌前 SHA；路线 B 的 `look/themes` 回到 `a3475683` 文本，`theme/palettes.rs` 重新生成，删除/替换遗留品牌 `LICENSE`。法律待确认：MIT 文本 + 少量品牌新增行的混合文件是否属于品牌材料的衍生；把 MIT 代码移入品牌目录是否构成 §3(b) 的“distribution other than as part of SolidCraft”（若代码本来已是 MIT，通常不是，但应书面确认）。

### 2.4 需要权利人或律师确认

1. §5 的“published version”是否覆盖公开但未打 tag 的 `a3475683`，还是只覆盖 tag（rc.2/rc.3）。
2. §1(a) 的“the design”和 §1(c) 的“design tokens, colour palettes”是否主张一种越过 MIT 授权的设计权。
3. §3(c) 的“look that could be taken for SolidCraft”是否会影响一个使用普通 CAD 控件惯例的独立应用。
4. 品牌期对 MIT 文件所做修改的许可归属。
5. 维护者是否会按 §4 把贡献重新许可为 MIT/Apache（一个答复可消除路线 B 的大部分风险）。

## 3. 上游 v0.41.0 覆盖与周同步

### 3.1 净变更分组

`06749ac..3c8efdb5` 共 **4,009 个 commit**；按路径分组（commit 数有重叠）：

| 组 | commit（全量 / v0.40.1→v0.41.0） | 文件 | 行 | 对 IntCAD 的判断 |
|---|---:|---:|---:|---|
| kernel/geometry + vendored solidcore | 330 + 2,800 vendored / 74 | 3,743 | 1,384,697 | **按依赖吸收**：移动 `solidcore`/`solidcore-record` rev；delta 未测 |
| doc-format（v51–v55） | 165 / 38 | 158 | 23,459 | **适配 + 冲突**：先解决 51 的双重含义 |
| engine/commands/schema | 360 / 51 | 423 | 114,571 | **逐命令直接吸收**（不需要新 UI 的命令优先） |
| I/O / STEP / GLB | 63 / 15 | 73 | 27,235 | **混合**：GLB export 缺失可吸收；AP242 是架构不匹配 |
| viewport/render/GPU/control | 38 / 7 | 203 | 64,264 | **适配**：crate split 是接口选择；单个修复可先回移 |
| plugins | 1 / 1 | 13 | 2,147 | **可适配**：`craft-plugin` 开放；wasmtime 带 73 个 lock 包 |
| UI open（`ui-egui`） | 343 / 53 | 439 | 143,376 | **冲突**：develop 有 14+ 个后 fork UI commit；上游把调用点移入品牌 kit |
| UI restricted | 27 / 10 | 556 | 30,021 | **受限**：路线 A 不导入；路线 B 用预品牌快照 |
| Assistant/MCP/WebView | 20 / 3 | 16 | 4,009 | **直接吸收** MCP 侧；control 是 crate 移动 |
| build/packaging/CI | 146 / 38 | 166 | 21,070 | **逐文件适配**；不要照搬 solidcore mirror workflows |
| tests/corpus | 133 / 26 | 1,824 | 7,473,853 | **可直接吸收**；6.5M 行 fixture 要过磁盘预算 |
| docs/planning | 2,142 / 59 | 2,667 | 231,493 | **大多跳过**；solidcore/plan 是上游自己的计划 |

### 3.2 混合 commit 的规律

4,009 个 commit 中只有 **25 个**同时改开放和受限路径（v0.40.1→v0.41.0 窗口 203 个中 9 个）。典型：

| commit | 受限 / 开放文件 | 主题 | 真正受限的部分 |
|---|---:|---|---|
| `a30b34707` | 9 / 140 | 统一 Design/Render 停靠 | 9 个 `ui-kit` 停靠文件 |
| `53a76b23f` | 2 / 68 | 沙箱 WASM 插件宿主 | Plugins 偏好页 |
| `2b936bd7b` | 2 / 6 | GLB 导出 | `app_bar.rs` 一个菜单项 |
| `7e0e63504` | 1 / 47 | 样条切点手柄 | `menu.rs` +88 行 |
| `7cc64dbb3` | 2 / 17 | Constrain 对话框 | 2 个对话框文件 |
| `95dec905c` | 2 / 16 | 按选择构造 Plane/Axis/Point | 一个图标（且预品牌已有） |
| `7081f99b1` | 5 / 3 | View toast | 组件本体（品牌期新增） |
| `865497c70` | 28 / 1 | isoparm 颜色 | 28 个 token/调色板文件 |

规律：**受限的一半通常是“让功能可见”的 widget、菜单项、偏好行或图标；功能本体在开放侧。** 因此路线 A 可以吸收功能，再在 IntCAD 内部 UI 重做入口；路线 B 则要处理品牌 kit。

### 3.3 开放代码是否依赖受限 crate

- v0.41.0 的 36 个 workspace 成员中，只有 `ui-egui`、`apps/solidcraft`、`apps/solidcraft-web` 直接依赖 `ui-kit`/`look`/`theme`；**没有开放 crate 嵌入受限路径的文件**。
- 方向是“受限 kit 消费开放 widget”，不是“开放功能依赖品牌”。
- 源码包和安装包最终会携带 `ui-kit`/`look`/`theme` 的代码和 `assets/branding`/`assets/icons/app`；IntCAD 不能把 v0.41.0 的包直接分发。

### 3.4 历史周同步回放（v0.40.1 → v0.41.0）

窗口：`v0.40.1` `0c5292285`（2026-11-11）→ v0.41.0，**203 commit、892 文件、+67,333/−16,111**。这是品牌边界之后的一天，适合测试稳态。

| 组 | commit | 备注 |
|---|---:|---|
| kernel/geometry | 74 | 14 个 `mesh-decisions.txt` commit 中 11 个 |
| engine/commands | 51 | |
| doc-format | 38 | |
| ui-open | 53 | |
| build/CI | 38 | |
| docs | 59 | |
| tests/corpus | 26 | |
| assets | 22 | |
| I/O | 15 | |
| ui-restricted | 10 | 9 个是混合 commit |
| viewport | 7 | |
| Assistant/MCP | 3 | |
| plugins | 1 | `53a76b23f`，+73 lock 包 |

拟议周同步流程与回放结果：

1. **按 SHA 取 stable tag 到临时分支**；按路径分类开放/受限。
2. **拆混合 commit**：只取开放 hunk；品牌侧入口登记为“需在 IntCAD 内部 UI 重做”。
3. **处理文件移动**：全量范围有 496 个跨边界 rename；周窗口没有新 rename，但首次同步必须处理。`git cherry-pick 95dec905c` 会想要 `crates/look/icons/.../plane_auto.svg`，IntCAD 没有这个目录；开放侧会引用缺失 icon id。
4. **检查新受限依赖**：不能只 diff manifest。`ui-egui → ui-kit` 和 `look → ui-kit, theme` 是 move commit 引入的，manifest diff 看不到。
5. **文档格式**：先跑 `crates/doc/src/format/pending/` 对比；当前 5 个 pending 与上游 `v51/` 5 个文件字节一致，但 v51 还有 10 个变更。任何 doc cherry-pick 前必须先决定 51→55。
6. **生成文件/列表**：`docs/features.md`、`THIRD-PARTY-LICENSES.txt`、14 个 locale pending、5 个 attribution pending、`xtask/mesh-decisions.txt`；每次同步后跑 `lists`、`assets`、`i18n`、`mesh-decisions`。

工具会漏掉的具体点：

- rename 到受限路径（398 个 icon、69 个 `ui_kit/*.rs`）。
- module → crate 间接层：上游改 `crates/ui-kit/src/menu.rs`，IntCAD 对应 `crates/ui-egui/src/ui_kit/menu.rs`，路径永远不匹配。
- move commit 新增的依赖边。
- `v51` 与 develop 的 51 含义冲突。
- 生成文件和 pending locale/attribution 的 fold/sort。

### 3.5 最值得先吸收的开放改进（排序）

1. **Mesh decision → exact geometry**：18 个 commit，14 个在最后一周；develop 仍有 21 条 allowlist 行在 v0.41.0 已删除。无品牌尾，价值/风险最高。
2. **GLB export**：`2b936bd7b`、`114bdff4e`；开放侧 `crates/io`，品牌尾仅一个菜单项。
3. **STEP 稳健性**：`566c1027f`、`eee6c7ae5`、`9fc8caf83`、`3453963cf`；无品牌尾。
4. **内核 rev bump**：blend curve、trim/cut、sweep sharp corner、tangent booleans；无品牌尾。
5. **插件宿主**：`53a76b23f`、`2167af4d6`、`ae346a7b2`；开放，但带 73 个 lock 包和 wasmtime，需要 feature gate。
6. **Sheet metal、sketch snap、MCP cancel/compact、viewport 修复**：按需。

## 4. 路线估算

### 4.1 路线对比

| 维度 | A：保留 develop UI，吸收开放变更 | B：#145 新 SolveApp + 独立 ui-kit |
|---|---|---|
| 复用范围 | 当前 UI 100% 保留；内部 `ui_kit` 66 文件；DFM/Assistant/Review 全保留 | 预品牌 `crates/ui-kit` 103 文件/50,748 行；77/78 非测试文件；44 个 ui-egui 适配点；但 #145 缺少整套 DFM/右栏切片 |
| 首次可运行 | 全量 8 / 12 / 18 人日；定向单改进 0.5 / 1 / 2 人日 | 12 / 16 / 22 人日 |
| 可发布 | 全量 22 / 30 / 42 人日 | 35 / 50 / 65 人日 |
| 合理并行 | 2–3 人 | 3–4 人 |
| 关键路径 | 文档格式 51→55 → 内核 rev → UI 接口 144 错 → E2E | 法律确认 → 预品牌 kit 适配 → DFM/右栏切片移植 → 视觉/跨平台 |
| 预计日历 | 2–3 周（2–3 人） | 4–6 周（3–4 人） |
| 许可风险 | 低（只使用 MIT/Apache 旧 UI；不导入品牌路径） | 中（§5 “published” 与 §1(a) 设计主张待确认） |
| 用户价值 | 保留 IntCAD 自己的 DFM/Assistant/Review；吸收几何/IO/插件/内核 | 获得 #145 的新 SolveApp/Ribbon/停靠，但这些主要是品牌设计，且需重做 |
| 长期同步 | 每周开放路径 1–3 人日；混合 commit 拆品牌尾 | 每周/每 release 2–5 人日；品牌期 UI 无法吸收，只能重写 |

区间拉大的主因：

- **A**：文档格式 51→55 是否必须整体迁移；`solidcore` rev delta 未测；144 个 UI 编译错误里有多少是机械 import、有多少是行为变化；Assistant/右栏后 fork 改动的回归。
- **B**：法律对预品牌快照的解释；#145 缺少 DFM 切片的移植量；品牌期 UI 增量（toast/menu/app_bar/prefs）重写；跨平台 WebView/面板边界。

### 4.2 组件成本

单位为独立工作量的乐观 / 最可能 / 保守人日；路线总价按关键路径组合，不直接相加（避免共享的 engine/doc/kernel 重复计算）。

| 组件 | 一次性移植 | 跨平台/发布验收 | 以后每次同步 | 备注 |
|---|---|---|---|---|
| 旧 UI 内部 `ui_kit` 保留（A） | 0 | 0 | 1–3 人日/周 | 已存在，只需回归 |
| 预品牌 `ui-kit` 导入 + 44 适配（B） | 5–8 / 7 / 10 | 3–5 | 2–3 人日/release | 实测：kit `--all-targets` 1 个测试修正后 exit 0；ui-egui 44 errors/13 文件 |
| 品牌期 UI 增量重写（toast/menu/app_bar/prefs）（B） | 3–6 | 2–4 | 1–3 人日/release | `toast.rs` 363 行；其余 +1,073/−558 |
| DFM/右栏/Assistant 切片移植到 #145（B） | 8–15 | 3–5 | 2–4 人日/release | #145 完全没有 `engine/src/dfm/`、`dfm_overlay`、`dfm_review`、`right_rail` 等 |
| 文档格式 51→55（A/B 全量） | 5–10 | 2–4 | 1–2 人日/release | 5 个 pending 与 v51 5 个文件字节一致，但 v51 还有 10 个变更 |
| 内核/solidcore rev（A/B 全量） | 3–5 | 2–4 | 0.5–2 人日/release | rev delta 未测；树内 merge 不建议 |
| engine/viewport/control 接口（A/B 全量） | 3–6 | 2–4 | 1–3 人日/release | viewport/control crate split |
| UI 接口 144 编译错误（A 全量） | 4–8 | 2–4 | 0.5–2 人日/release | 实测 144 errors（egui 0.36）；多数是 `engine::render` → viewport/render/gpu 的 import |
| E2E/跨平台（两条） | 2–4 | 4–8 | 1–2 人日/release | Linux VM 已跑；Windows/macOS 另计 |
| 许可/来源审计（B 或发行前） | 1–3 + 外部法律 | — | 0.5 人日/release | §5/§1(a)/§3(c) |

### 4.3 估算的证据等级

- **实测**：构建时间、测试失败/通过、E2E 截图、144/209 编译错误、预品牌 kit 编译、44 个消费者错误、DFM 报告内容、orbit 漂移。
- **类推**：未实际修复的 144 个错误的人日、全量 v0.41.0 的周同步、路线 B 的 DFM 切片移植。
- **低置信度**：内核 rev delta；文档格式迁移的完整回归；法律结论；Windows/macOS 的 WebView/原生宿主。

## 5. 实验

### 5.1 实验一：重复圆 profile（开放几何修复，成功）

- 基线：`02a786d47`。测试 `a_circle_drawn_twice_is_one_hole` **失败**：环 profile 的 `hole_curves` 是 `[["c1"],["c2"]]`，期望 1 个孔。
- 回移：`3c93d21ca` 的 `crates/sketch/src/profiles.rs`，34 行，`git apply --3way` **clean**；测试 **通过**（1 passed）。
- 当前 UI（真实桌面 VM）：
  - 基线：同一环形 profile 拉伸 → **331.83 mm³、3 面、无孔**（Body1）。
  - 修复后：**251.41 mm³、4 面（2 圆柱 + 2 平面）、一个孔**（Body3）。
  - 期望环体积：π(3.25²−1.6²)×10 = **251.406 mm³**。
- 截图：`docs/screenshots/int1454-v2/exp1-ring-before.png`、`exp1-ring-after.png`。
- 跨 crate：只有 `crates/sketch`；无格式变化；无品牌代码。
- 计时（实测）：测试前 16 s、测试后 10 s；本机 `cargo build -p solidcraft` 255 s；E2E 构建 459 s；UI 交互约 5 min。编码/调试约 10 min，编译等待单列。
- 证明：路线 A 可以吸收一个具体开放几何修复，并保持当前 UI。不能证明：所有几何修复都能无冲突回移（本例是 34 行、单文件）。

### 5.2 实验二：orbit pivot（开放 UI/视口修复，成功）

- 回移：`b1ebc2c2d` + `2534c2853`，7 文件、259 insertions/39 deletions。唯一冲突在 `viewport.rs` 的 `hover_joint`/`orbit_pivot` 字段；保留 `hover_joint` + 新注释后解决。
- 单元测试：`orbit_after_home_turns_the_model_in_place_beside_the_browser` 在修复版 **通过**（1 passed；构建 347 s）。
- 当前 UI（真实桌面 VM，Browser 打开）：
  - 基线：Home 后模型中心约 x=805；orbit 后约 x=900（右移约 95 px）。
  - 修复版：Home 后 x≈805；orbit 后 x≈805（保持）。
- 截图：`exp2-orbit-before-home.png` / `exp2-orbit-before-rotated.png`；`exp2-orbit-after-home.png` / `exp2-orbit-after-rotated.png`。
- DFM 回归：同一修复版上重跑 DFM 报告，`review:finding:sheet-1/F5` 与 F6 行选择均更新标签；截图 `exp2-dfm-row-f5.png`。
- 计时：测试构建 347 s；E2E 构建 441 s；冲突解决约 25 min；编译等待单列。
- 证明：一个经过 UI/视口边界的开放改进可以接入当前 UI，且 DFM 选择不回归。不能证明：所有 UI/viewport 改动都只需 259 行；大改动仍可能碰到 crate split。

### 5.3 DFM 真实回归（最新 develop）

构造：60×40×2 mm 原生 sheet，两个 Ø2 孔；factory profile `critical_shop.json` + `project.settings_set(process=press brake, material=5052-H32, thickness=2)`；`dfm.check`。

结果：3 个 failed finding：

- `sheet-1/F5` `sm.hole_min_diameter`，hole 1，measured 2 / required 3，anchor `[2,20,1]`
- `sheet-1/F8` `sm.hole_to_edge`，hole 1，measured 1 / required 3，anchor `[2,20,1]`
- `sheet-1/F6` `sm.hole_min_diameter`，hole 2，measured 2 / required 3，anchor `[30,20,1]`

**同一张面/同一个孔有两个 finding（F5、F8）**，F6 是另一个孔，避免“选中同一张面”掩盖 identity。

已验证：

- Review 行 F5 → 标签 F5 蓝框、行 F5 蓝底（`dfm-row-f5.png`）。
- Review 行 F6 → 标签 F6、行 F6（`dfm-row-f6.png`）。
- 标签 F5（屏幕坐标约 405,480）→ 行 F5。
- 旋转/缩放后标签跟随孔位。

**具体阻塞（未满足“修改模型后旧报告不再定位”）**：

- 在模型上再加第三个 Ø2 孔后，`inspect.dfm` 返回 `freshness: "unknown"`，不是 `staleModel`；旧报告、旧标签、旧标记仍在，仍可点击定位。
- 代码证据：`dfm_review.rs:245` 只按 `document_id` 缓存；`ViewportState::set_dfm_freshness` 没有生产调用者；`dfm_overlay.rs:265/285` 只在 freshness 非 current 时隐藏/拒绝，但编辑后没有人更新 freshness。
- 这是 DFM 产品行为缺陷，不是 ui-kit 替换成本；A/B 都需要修。
- 截图：`dfm-stale-after-edit.png`（模型已有第三个孔，旧 F5/F6/F8 标签仍在）。

**同面标签的待核实风险**：F5/F8 标签相邻约 33 px；VM 坐标下点击 F8 标签附近曾在一次运行中选中 F6。行选择本身是准确的。建议发布 `dfm:label:<finding_id>` 手柄并加一个“同面两 finding”的自动化用例，再判断是坐标偏移还是 hit-test 缺陷。

**Assistant / WebView**：

- `ui.agent_chat {"open":true,"side":"right"}` 打开原生 WebView；页面实际显示 “Set up an assistant / No agent found”、安装项和输入框（`assistant-webview.png`），`webView.host: true`；交互后 `page: true`。
- **已验证拖拽**：在存有 DFM 项目报告的设计上，把 Review 报告卡拖入 Assistant，`ui.agent_chat` 的 `attachments` 变为 `["DFM report run-..."]`，WebView `page: true`（`assistant-report-drop.png`）。这证明宿主侧的卡片拖拽和附件桥接。
- **已验证 Assistant 双向选择**：Assistant 的 DFM 检查卡由宿主 `agent_chat::web::run_check` 创建（探针分支让控制通道的 `send` 像页面一样识别“check the sheet metal part”，`probe/int1454-exp2` `c2631046a`）。在真实 WebView 中：
  - 点击 Assistant 行 F5（`sm.hole_min_diameter`，hole 1）→ Review 行 F5 选中（`review-after-assistant-f5.png`），即视口选中同一 `(run_id, finding_id)`；
  - 点击 Assistant 行 F8（`sm.hole_to_edge`，hole 1）→ 视口标签 F8 选中（`assistant-row-f8.png`），F6 保持未选；
  - 点击视口标签 F6 → Assistant 行 F6（`sm.hole_min_diameter`，hole 2）选中（`assistant-label-f6-row.png`）。
  - 检查卡截图：`assistant-check-card.png`。
- **未验证**：把附件发送给 agent 的完整 round-trip。当前 VM 没有安装 agent；附件只证明宿主侧桥接，不证明模型实际读取报告。

### 5.4 #145 的 CI 分类

#145 head `f45d9774`，CI run `43682880705`。失败项：

| 失败 | 类别 | 说明 |
|---|---|---|
| Format and repository checks | 候选分支合并集成 | rustfmt diff（agent_chat/tests、control、titlebar 等） |
| Native Linux Clippy | 候选分支合并集成 | 106 个 dead-code/unused，主要在 `control_params.rs`、`agent_chat/*`、`panel_runtime/*` |
| Compile WebAssembly | 候选分支合并集成 | `solidcraft_ui_egui::ControlRequest` vs `solidcraft_control::ControlRequest` 类型冲突 |
| Windows app startup / full-baseline | 候选分支合并集成 | `graphics_restart_*` 未解析 |
| Reference tests / selected regression | 候选分支合并集成 | `solidcraft_engine::render::png` 不存在 |
| Defaults line check | 管理/PR 文本 | `Changes a default:` 行格式 |

**没有一项能归因于“替换 ui-kit 引入”**；全部是 #145 的合并/接口集成问题。`unattributed` 为空。这个分类很重要：不要把 #145 的现有失败计入 ui-kit 重写成本。

## 6. 路线 B 的具体阻塞

已有证据（预品牌 `a3475683`）：

- `cargo check -p solidcraft-ui-kit`：仅 1 个测试辅助 `Tokens` destructuring 错误；补 `..` 后 **exit 0**；`--all-targets` **exit 0**。
- 保持 #145 调用点不变，`cargo check -p solidcraft-ui-egui`：**44 errors / 13 files**：`browser.rs` 21、`viewport.rs` 4、`toolbar.rs` 3、`timeline.rs` 3、`dialogs_assembly.rs` 3 等；对应品牌期约 8 个上游提交（toast、`Row::{rename,handle}`、`Segments::glyphs`、`Bar::reorder`、`Handles::{controls,grip,plugins}`、`BarAction::Plugins`、`Action::Seek`、`set_dialog_mnemonics`、`compact_number_field`、`menu::scroll_rows`、`backing_on_hover_text_beside`、`prefs_window::Host::confirm`、`LayoutPreview::history_in_browser`）。
- 计时：环境准备 9 min；kit 编译 249 s（含 target lock 等待）；ui-egui 37 s；调试 3 min；target +2.26 GB。
- #145 缺少整套 DFM/右栏/Assistant 切片：`crates/engine/src/dfm/`、`cmd/dfm_check.rs`、`schema/dfm.rs`（3,352 行）、`dfm_overlay.rs`（667）、`dfm_chat.rs`（340）、`dfm_review.rs`（512）、`right_rail.rs`（335）及其测试都不在 #145。路线 B 必须从 develop 再移植一遍。
- #145 真实桌面 E2E（此前的独立运行）：应用打开、Assistant 打开、视口点选面成功（`F2:side:l1`，Plate，544 mm²），但侧栏拖宽 5 次均未改变 `agent_chat_width`（保持 380）；原生 GTK/WebKit 子窗口覆盖在左侧 Browser 上，右侧另有一个空 egui Assistant 卡片。这是有状态面板/宿主边界的具体阻塞证据。
- 许可：预品牌 `a3475683` 是 MIT/Apache commit，但从未作为独立 `ui-kit`/`theme` crate 进入 tag；§5 的“published”与 §1(a) 的“design”需要法律确认。`crates/look` 没有品牌前独立快照；`look/themes` 25/26 与品牌期字节一致、每份只多一行 `isoparm`；`theme/palettes.rs` 与品牌期 98% 相同；#145 的 `crates/theme/LICENSE` 仍是品牌文本。路线 B 发行前必须清理这些。

## 7. 结论与建议

### 7.1 已验证可行

- 当前 develop UI（66 文件内部 `ui_kit`）是 MIT/Apache 来源，可以继续开发并闭源发行，前提是不导入品牌路径并保留 notices。
- 开放几何修复（重复圆）可以 34 行回移，当前 UI 得到正确体积/孔。
- 开放 UI/视口修复（orbit pivot）可以 259 行回移，当前 UI 行为改善，DFM 双向选择不回归。
- DFM 行↔标记双向选择在最新 develop 上工作；同面两个 finding 的身份可以区分（F5/F8 vs F6）。
- Assistant WebView 在真实桌面 VM 中实际渲染页面，不只是空宿主；Assistant 检查卡的行→视口、视口标签→Assistant 行均已在真实 WebView 中点击验证（F5/F8/F6）。

### 7.2 存在具体阻塞

- **DFM 旧报告失效**：编辑模型后 freshness 变 `unknown`，旧报告/标记仍可定位；需要按 `document_id + revision` 刷新或在编辑时调用 `set_dfm_freshness`。
- **路线 B 的 DFM 切片**：#145 完全没有 `dfm/`、`dfm_overlay`、`dfm_review`、`right_rail`、`dfm_chat`；这不是 kit 适配能解决的。
- **路线 B 的法律边界**：预品牌快照是 commit 而非 tag；§5/§1(a) 需要书面确认；`look/theme` 的遗留品牌文本/主题需要清理。
- **文档格式 51 的双重含义**：在迁移到 v51–v55 前，任何 `crates/doc` 回移都可能写错格式。

### 7.3 仍待验证

- 144 个全量路线 A 编译错误实际修复时间（目前只统计，未修）。
- `solidcore` pin 从 `8ddce901` 到 v0.41.0 的实际 delta。
- 同面标签的 33 px 自动化风险：双向点击已验证，但行展开会改变 WebView 行坐标，且标签没有发布 handle；需要 `dfm:label:<finding_id>` 和固定行 handle 才能稳定回归。
- Assistant 报告卡发送给 agent 的 round-trip（宿主侧拖拽、附件桥接和双向选择已验证；当前环境无 agent，未验证模型读取）。
- Windows/macOS 的 WebView、面板和跨平台验收。
- 律师对 §5/§1(a)/§3(c) 的结论。

### 7.4 本次能拿到的用户价值 / 遗漏

**能拿到**：当前 IntCAD UI 内的几何修复（重复圆）、orbit 行为、DFM 双向选择、Assistant WebView；开放的内核/几何、I/O（GLB、STEP）、engine 命令、插件宿主、MCP/control、build/CI。它们不需要品牌 crate。

**会遗漏/延后**：新版 SolveApp 的 Ribbon、统一停靠、窗口 chrome、start page、prefs window、view toast、isoparm 主题色，以及品牌期新增的菜单/偏好/图标外观。这些是品牌设计或品牌期代码，不能直接吸收；路线 A 只按用户价值重做小入口。

**限制来源**：许可是硬边界（品牌路径）；架构是第二边界（doc format、solidcore pin、viewport/control split）；实施成本是第三边界（#145 的 DFM/右栏切片、全量 144 错、跨平台 E2E）。

## 8. 证据索引

详细证据文件（本分支 `docs/research/int1454-evidence/`）：

- `licence.md` — 许可时间线、逐路径来源表、三种场景、待法律确认问题
- `upstream-coverage.md` — 4,009 commit 分组、混合 commit、开放/受限依赖、v0.40.1→v0.41.0 回放、文档格式冲突
- `dfm-boundary.md` — 两条桌面调用链、DFM 数据/选中/投影/命中/WebView 归属、E2E 方法
- `commands.md` — 全部实验的准确命令、SHA、结果和截图对应

早期工作材料（关键结论已并入本报告；路径仅作追溯）：

- `/tmp/opencode/int1454-v2/baseline.md`
- `/tmp/opencode/int1454-uikit/reuse-map.md`
- `/tmp/opencode/int1454-uikit/call-audit.md`
- `/tmp/opencode/int1454-uikit/prototype.md`

实验分支（不合并）：

- `probe/int1454-exp1` `eda5fbd3e` — 重复圆修复 + 测试；工作树 `worktrees/int1454-exp1`
- `probe/int1454-exp2` `c2631046a` — orbit 修复（7 文件 259/39）+ 控制通道复用 DFM 检查短语；工作树 `worktrees/int1454-exp2`
- `probe/int1454-uikit` — 预品牌 kit 替换原型；工作树 `worktrees/int1454`
- `probe/int1454-route-a` `d9eae79a7` — develop UI + v0.41.0 后端的全量探测；144 errors
- `docs/int1454-ui-kit-study` — 本报告和截图

截图（本报告同目录 `docs/screenshots/int1454-v2/`）：

| 文件 | 内容 |
|---|---|
| `exp1-ring-before.png` | 基线：环形 profile 拉伸为实心，331.83 mm³，3 面 |
| `exp1-ring-after.png` | 修复：一个孔，251.41 mm³，4 面 |
| `exp2-orbit-before-home.png` / `-rotated.png` | 基线 Home 后 orbit，模型右移约 95 px |
| `exp2-orbit-after-home.png` / `-rotated.png` | 修复版 Home 后 orbit，模型保持 |
| `dfm-review.png` | 最新 develop 的 DFM 报告：F5/F8 同孔、F6 另一孔 |
| `dfm-row-f5.png` / `dfm-row-f6.png` | 行选择不同 finding |
| `dfm-stale-after-edit.png` | 加第三个孔后旧报告/标记仍在（阻塞） |
| `assistant-webview.png` | Assistant WebView 实际显示 |
| `assistant-report-drop.png` | Review 报告卡拖入 Assistant 后生成 DFM 报告附件 |
| `assistant-check-card.png` | Assistant 中的真实 DFM 检查卡（含 failed 行） |
| `assistant-row-f5.png` | Assistant 行 F5 选中/展开 |
| `review-after-assistant-f5.png` | Assistant 行 F5 选中后 Review 行 F5 同选 |
| `assistant-row-f8.png` | Assistant 行 F8 选中，视口标签 F8 同选 |
| `assistant-label-f6-row.png` | 点击视口标签 F6 后 Assistant 行 F6 选中 |
| `exp2-dfm-row-f5.png` | orbit 修复版上的 DFM 回归 |

## 9. 建议的下一步（可排期）

**投入：2 人 × 10 个工作日（20 人日），先做路线 A。**

| 工作 | 人日 | 验收 |
|---|---:|---|
| 重复圆回移 + 测试 + E2E（已做，收尾） | 1 | 当前 UI 体积 251.41 mm³，一孔 |
| orbit 回移 + 测试 + E2E（已做，收尾） | 1 | Home 后 orbit 不漂移；DFM 不回归 |
| 一个 I/O/GLB/STEP 开放修复 | 3–5 | 同一文件前后对照；当前 UI 可见 |
| 文档格式 51→55 spike | 3–5 | 能读/写 v51–v55，保留本地 v56；或明确放弃 |
| DFM 旧报告失效修复 | 0.5–1 | 编辑后 freshness 变 staleModel，标记隐藏、定位拒绝 |
| 周同步脚本 + v0.40.1→v0.41.0 回放 | 2–3 | 输出开放/受限/混合 commit 清单和待办 |
| 回归/E2E/报告 | 2–3 | Linux VM 通过；Windows/macOS 交给 CI |

**里程碑**：当前 develop UI + 3 个开放改进 + 文档格式结论 + DFM stale 修复 + 周同步脚本；无品牌 crate 依赖。达到后再评估是否做全量 v0.41.0 开放同步；路线 B 只在法律确认预品牌快照且产品明确需要新版 SolveApp 外观时重新立项。
