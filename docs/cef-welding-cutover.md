# CEF + welding 单实现迁移（Cutover）

分支 `probe/cef-welding-osr`。前置：`docs/cef-welding-feasibility.md`、
`docs/cef-welding-production-readiness.md`、`docs/cef-welding-migration-spec.md`。

## 1. 结论

**单实现迁移已在真实 IntCAD 上完成并运行：WRY 及其 GTK/WebKitGTK 子窗口路径已删除，
Assistant 只剩 CEF 一套实现。** Linux 原生 Wayland 与 X11（XWayland）均通过；真实 CAD
并行性能 p95 回归 **+1.0%**（阈值 10%）。原生 Xorg、Windows、macOS 仍 UNTESTED，
因此这是**迁移完成、跨平台验收未完成**，不得合并为生产发布。

## 2. 改了什么

| 项 | 变更 |
|---|---|
| 新增 `apps/solidcraft/src/assistant_cef.rs` | CEF 运行时、页面服务、纹理、输入队列、IME、剪贴板桥、popup、状态推送 |
| 新增 `apps/solidcraft/src/assistant_page.rs` | 页面内嵌文件与 CSP（从 WRY 模块搬出，页面本身未改） |
| 删除 `apps/solidcraft/src/assistant_web.rs` | WRY 视图、GTK 重挂、X11 键盘交接、位置同步、GTK turn 节流（约 1100 行） |
| `apps/solidcraft/src/main.rs` | `assistant_web` 字段与分支改为单一 CEF；`main()` 首行加 CEF 子进程分派；`wgpu_options()` 改用 grafting 共享设备 + `WgpuSetup::Existing` |
| `apps/solidcraft/Cargo.toml` | 删除 `wry`（Windows/macOS/Linux 三处）、`gtk`、`gdkx11`；新增 `welding 0.15.0`（`cef-runtime`）、`dpi` |
| `Cargo.lock` | `wry`、`gtk`、`webkit2gtk`、`gdkx11` 条目为 **0**（`grep -c` 验证） |
| `assets/locales/pending/assistant-web-host.json` | “页面打不开”改为 CEF/GPU 互操作说明（8 种语言）；删除过时的“原生 Wayland 需要 X11 子窗口”文案 |
| `packaging/notices/WEB-VIEW-PLATFORM-NOTICES.txt` | 整篇替换为 CEF/Chromium（BSD-3）+ welding MPL-2.0 说明 |
| `packaging/notices/CEF-CREDITS.html`（新） | CEF 生成的完整第三方清单（8.3 MiB），三个平台的包脚本都会安装 |
| `packaging/{linux/package.sh,macos/package.sh,windows/package.ps1}`、`assets/platforms/windows/solidcraft.wxs` | 安装/打包 CEF-CREDITS.html；MSI 组件补上该文件 |
| `packaging/linux/{nfpm.yaml,AppRun}` | 删除 libgtk-3 / libwebkit2gtk 依赖与“缺 WebKitGTK”提示，改为 CEF 自带的通用缺失库提示 |
| `NOTICE`、`docs/web-view-licences.md`、`crates/ui-egui/src/help.rs` | 桌面 WebView 段落改写为 CEF/Chromium；帮助页嵌入的也是新通告 |

页面传输（保持既有 IPC 契约）：

- 宿主在 `127.0.0.1:<随机端口>` 服务内嵌页面（`assistant_page.rs`），因为 CEF/welding 没有
  custom-scheme 接口，而 ES module 需要真实 origin。
- 页面 → 宿主：`/ipc.js` 定义 `window.ipc.postMessage` → `POST /ipc` → `web::act`（与 WRY
  完全相同的动作白名单）。
- 宿主 → 页面：`window.__assistantHost.state(<json>)`，经 `execute_script`；`ready` 与页面
  画布色同样经 `/ipc` 回来（决定 `webView.page`）。
- 内联 shim 会被页面自带 CSP 拒绝，所以 shim 作为同源文件 `/ipc.js` 提供，CSP 只放开
  `connect-src 'self'`。

## 3. 验收（Linux）

| 项目 | 结果 | 证据 |
|---|---|---|
| 真实 Assistant 页面作为 egui 纹理 | **PASS** | `cutover/x11.png`、`cutover/wayland.png`（`ui.screenshot` 现在包含页面，证明它是应用自己画的） |
| 原生 Wayland（无 DISPLAY） | **PASS** | `webView {host:true,page:true}`，CEF 首帧、页面加载、截图 |
| X11 / XWayland | **PASS** | 同上 + 输入脚本（复制、中文粘贴、焦点、滚轮、popup、右键） |
| 共享 device / 无 CPU readback | **PASS** | 设备由 grafting 建，`WgpuSetup::Existing`；帧以 DMABUF 导入 |
| 首帧输入队列 | **PASS** | 探针 soak：`queue_typed=true`，对照按键丢失 |
| 剪贴板（CEF 原生不可用） | **PASS（桥接）** | 页面→系统、系统→页面（中文） |
| 中文 IME 组合/取消/连续 | **PASS** | 探针序列；应用内由 egui IME 事件转发 |
| popup / 右键菜单 | **PASS** | 探针 `POPUP rect`、`ContextMenuRequested` |
| resize / HiDPI | **PASS** | 面板矩形变化触发 CEF resize；1.25→2.0 缩放切换 |
| 100 次创建/销毁、崩溃恢复 | **PASS** | soak 100/100、`terminated→recovered` |
| 沙箱 | **PASS** | `CefSandboxMode::Sandboxed` 默认；10/10 soak |
| GPU 互操作缺失时的行为 | **PASS** | `SOLIDCRAFT_ASSISTANT_CEF_NO_INTEROP=1`：面板给出说明，CAD 正常 |
| 应用内剪贴板：页面→系统 | **PASS** | `app-input.py`：`typed_copied_out="clip-abc"`、`copied_out="你好，世界"`（X 剪贴板读回） |
| 应用内 popup | **PASS** | 实页注入原生 `<select>` 后，应用日志 `popup surface 150x138 at 10,43` |
| 仓库检查 | **PASS** | `cargo xtask lists`（目录/消息排序、pending locale 正常折叠）、`cargo xtask layers`（25 crates 无越层）、`cargo check -p solidcraft --tests`（WRY 删除后测试目标可编译） |
| 应用内 fcitx5 中文输入（提交/取消） | **PASS** | `scripts/app-ime.sh`：X11+XIM 下输入 `nihao` 提交为 `你哈哦`（页面读回、应用日志 `IME commit`）；再组合 `zhong` 后 Esc 取消，字段不变 |
| 候选窗/preedit 显示 | **PASS（由 IME 自绘）** | fcitx5 自带候选与 preedit，客户端只收到空 preedit + 提交文本（egui-winit 记录的 Linux+fcitx5 行为）；候选窗不属于应用窗口 |
| 原生 Xorg | **UNTESTED** | 本机无 root/VT，`Xorg` 在 `parse_vt_settings` 失败 |

### 3.1 本轮修掉的集成缺口

egui 只在自身 TextEdit 获得焦点时才让 winit 打开输入法；页面编辑器在 CEF 里，所以
fcitx5 的按键最初全部以拉丁字符直达页面（提交出 `nihao`）。现在页面获得焦点时宿主发送
`ViewportCommand::IMEAllowed(true)`（失焦/隐藏/关闭时关闭），fcitx5 才真正接管输入法，
同一次按键序列提交出中文。这是“CEF 嵌入后必须由宿主显式开启 IME”的硬约束。

## 4. 真实 CAD 性能（同机、同模型、同窗口协议）

X11/XWayland，AMD RX 9060 XT，`ui.inspect.frame_gaps_ms`，4 s 连续拖拽：

| 指标 | WRY（迁移前同分支） | CEF（迁移后） | 变化 |
|---|---:|---:|---:|
| 拖拽 p50 | 1.63 ms | 1.39–1.50 ms | 更好 |
| 拖拽 p95 | **30.19 ms** | **30.48 ms** | **+1.0%** |
| 拖拽 p99 | 30.61 ms | 30.61 ms | 0 |
| >50 ms 卡顿 | 0 | 0 | — |
| 期间 CPU | 12.9% | 12.2% | 持平 |
| 页面就绪 | 0.59 s | 0.51 s | 更快 |
| 空闲节奏（尾部 20 帧） | 84.3 ms（≈11.9 fps） | **184.7 ms（≈5.4 fps）** | 更少无意义重绘 |
| RSS（含页面） | 315–682 MB | 897–1129 MB | +约 400 MB |
| 进程数 | 3 | 9 | Chromium 多进程 |

门槛“CAD p95 回归 ≤10%”满足。空闲不再有 PR #111 的 60 FPS 空转；`stats` 显示页面静止时
`cef_frames=0`、宿主 ~5.5 fps。

## 5. GPU 能力不足

`interop_setup()` 在设备缺少 `VULKAN_EXTERNAL_MEMORY_DMA_BUF`（或测试开关强制关闭）时返回
`None`：eframe 用自己的设备，`CefAssistant::new` 拒绝创建页面，`web::Problem::Failed` 让面板
显示“页面无法启动 + 检查图形设置 + 重试”，CAD 视口完全可用（`nointerop.png`）。没有
CPU readback，也没有黑屏或崩溃。

## 6. 体积、资源与 CI

| 项 | 迁移前（WRY） | 迁移后（CEF） |
|---|---|---|
| Assistant 运行时依赖 | 系统 GTK3 + WebKitGTK（LGPL，需系统库例外） | 随包 `libcef.so` + `Resources` |
| 运行时体积（未压缩） | 0（系统库） | 1394 MiB；`strip` 后 260 MiB；+pak/icu ≈ 300 MiB |
| 下载体积 | 0 | CEF minimal 311 MiB（构建期） |
| 许可 | WebKitGTK LGPL + GTK | CEF/Chromium BSD-3 + 第三方；welding MPL-2.0 |
| CI | 无 CEF 下载 | 每次干净构建 +311 MiB 下载、cmake/ninja、`libcef_dll_wrapper`（约 2 分钟）；建议缓存 |
| 内存 | 315–682 MB | 897–1129 MB |

**许可已完成**：`WEB-VIEW-PLATFORM-NOTICES.txt` 换成 CEF/Chromium 通告，`CEF-CREDITS.html`
随包安装（Linux/macOS/Windows/MSI），`NOTICE`、帮助页与 `docs/web-view-licences.md` 同步更新；
`strings target/release/intcad` 中 “WebKitGTK 4.1” 出现 **0** 次、“Chromium Embedded
Framework” 15 次。仍需人工法律审查（非工程阻断）。

## 7. 跨平台

| 平台 | 状态 | 备注 |
|---|---|---|
| Linux Wayland | **PASS** | 截图 + 页面 ready + 首帧 |
| Linux X11/XWayland | **PASS** | 同上 + 输入 |
| Linux 原生 Xorg | **UNTESTED** | 环境限制 |
| Windows | **UNTESTED** | 需 D3D12 与 `bootstrap.exe` 沙箱打包；VM 无 GPU |
| macOS | **UNTESTED** | 需 framework/helper 打包与公证；无设备 |

## 7.1 验收交接（未测平台）

| 平台 | 负责人 | 步骤 | 通过标准 |
|---|---|---|---|
| Linux 原生 Xorg | Linux 维护者 | 在带 AMD/Intel 的 Xorg 会话构建本分支，`XDG_SESSION_TYPE=x11 DISPLAY=:N`、`SOLIDCRAFT_WAYLAND=0`，跑 `scripts/app-perf.py` 与 `scripts/app-input.py` | `webView.page=true`；拖拽 p95 与 Wayland 同量级；剪贴板/IME/popup 与应用内一致 |
| Windows 11 | Windows 维护者 | MSVC + cmake 构建，CEF 154 DLL/`bootstrap.exe` 打进 MSI，务必 `CefSandboxMode::Sandboxed`；跑同一组脚本（`app-perf.py` 的拖拽换成 UI 自动化） | D3D12 共享纹理导入、页面就绪、输入、生命周期、沙箱、安装包体积 |
| macOS（Apple Silicon + Intel） | macOS 维护者 | framework + helper app 打包、`try_run_subprocess`、公证；同一组脚本 | IOSurface/Metal 导入、输入法（NSTextInputClient）、打包与公证 |
| OS 输入法（Linux） | Linux 维护者 | ~~未验证~~ 已在本机 fcitx5（X11/XIM）完成：提交 `你哈哦`、Esc 取消不改字段；Wayland text-input 路径交由维护者在 niri 会话复核 | 中文上屏、取消；候选窗由 IME 自绘 |

## 7.2 构建与检查（本轮）

| 检查 | 结果 |
|---|---|
| `cargo build -p solidcraft --release`（Linux，CEF） | **PASS** |
| `cargo check -p solidcraft --tests` | **PASS**（仅 2 个未用告警） |
| `cargo test -p solidcraft-ui-egui --lib -- help` | **PASS**（25 passed, 0 failed；4997 filtered） |
| `cargo xtask lists` / `cargo xtask layers` | **PASS**（25 crates，无越层；pending locale 正常折叠） |
| `cargo check -p solidcraft --target x86_64-pc-windows-msvc` | **BLOCKED（工具链/依赖，不是本模块）**：rustup 1.99.1 + windows-msvc std（FHS 环境内）已就绪，检查可把依赖图编译到 Windows 目标，但在 `ring` 的原生归档步骤失败（宿主机没有 MSVC 的 `lib.exe`/`llvm-lib`），尚未到达应用 crate；即使补上，`cef-dll-sys` 仍需 Windows CEF 与 MSVC 才能生成 wrapper。因此 Windows 构建验收必须在 Windows runner 上做 |
| `cargo check -p solidcraft --target aarch64-apple-darwin` | **未运行（缺 Apple SDK）**：依赖的原生 C 与 CEF framework 都需要 macOS SDK/工具链；交给 macOS runner |

`assistant_cef.rs` 已改为平台无关：Linux 只用 `cfg(target_os = "linux")` 包住 DMA-BUF 设备创建与
`Vulkan` 能力检查，Windows/macOS 走 `welding::windows_cef` / `macos_cef` 的 producer（Windows 的
producer 在创建时需要宿主 device，已按签名传入），纹理导入统一由 `WgpuTextureImporter` 按后端处理。
因此 Linux 之外不再引用 `linux_cef` 或 `build_dmabuf_capable_device`。

Windows 沙箱的已知缺口写在 `sandbox()`：CEF 的 Windows 沙箱需要 `bootstrap.exe` 创建上下文，
`execute_process_from` 拒绝 `Sandboxed`，在 MSI 提供该形态之前，Windows 运行会显示“页面不可用”
而不是以无沙箱方式启动 Chromium。

## 8. 回滚

- 迁移提交在独立分支；`git revert` 迁移提交即可回到 WRY（`assistant_web.rs` 与依赖在历史中）。
- 不保留运行时双实现：回滚是 Git 层面的，不是产品开关。
- 数据/配置：`SOLIDCRAFT_*_DIR` 下新增 `cef-assistant` 缓存目录，删除不影响设计文件。

## 9. 迁移 PR 草案

> **Assistant: draw the conversation with CEF instead of a native child window**
>
> The Assistant's page is now rendered by CEF into a GPU texture the app's own wgpu device
> samples and egui draws in the panel. Linux no longer needs an X11 child window, GTK or
> WebKitGTK: native Wayland and X11 both work, and the page is part of the app's frame.
> WRY and its dependencies are removed; the page, the IPC contract and the chat logic are
> unchanged.
>
> Measured on the real viewport: p95 frame time +1.0% (threshold 10%), page ready 0.51 s,
> idle 5.4 fps instead of 11.9, memory +0.4 GB, CEF runtime 260 MiB stripped.
>
> Remaining before release: replace the WebKitGTK notices with CEF's, accept the +260 MiB
> runtime, and pass native Xorg, Windows and macOS acceptance on real GPUs.

## 10. 复现

```sh
# 构建（CEF 的 wrapper 需要 cmake/ninja/C++）
CEF_PATH=/path/to/cef_binary_154 ... cargo build -p solidcraft --release

# 运行：Linux 上需要 CEF 的库路径（NixOS 用 nix/runtime.nix 的包装器）
CEF_PATH=... LD_LIBRARY_PATH=$CEF_PATH:... ./target/release/intcad --control 18095

# 验收截图与性能
python3 experiments/cef-osr/scripts/app-shot.py 18095 shot.png
nix shell nixpkgs#xdotool -c python3 experiments/cef-osr/scripts/app-perf.py \
  target/release/intcad cef --seconds 4 --out /tmp/perf
```
