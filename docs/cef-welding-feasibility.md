# CEF + welding GPU OSR：可行性验证报告

**结论：CONDITIONAL GO。** 在 Linux 上，CEF 加速离屏渲染（`OnAcceleratedPaint` → DMA-BUF →
宿主 wgpu 纹理）**确实可以完全不依赖 XWayland** 地跑在原生 Wayland 上，并且能作为 egui
纹理画进同一个原生窗口；这一点有像素级证据，不是文档推断。但在把它当成三平台统一嵌入
架构之前，有三个必须先解决的前置条件（自有 wgpu device 创建、页面 IPC 桥、发布体积与
沙箱），所以不是无条件 GO，也不是 NO-GO。

实验代码与证据：`experiments/cef-osr/`（独立 workspace，未改动生产 WRY 路径与
`Cargo.lock`），原始日志在 `experiments/cef-osr/evidence/2026-11-18/`。

---

## 1. 现有 IntCAD 架构与兼容性

| 项 | 现状（develop `96a247ccb`） |
|---|---|
| UI / 窗口 | `eframe` 0.37 + `egui` 0.37 + `winit` 0.30.13 |
| CAD 渲染 | **wgpu 30.0.1**（Linux Vulkan、Windows D3D12、macOS Metal）；`egui_glow` 仅在锁文件中，应用未使用 |
| Assistant | `wry` 0.57.2：Linux 为 WebKitGTK 4.1 + GTK3 的 **X11 子窗口**（`build_as_child` + `gdk_x11_window_foreign_new_for_display`），Windows 为 WebView2，macOS 为 WKWebView |
| 页面传输 | `with_custom_protocol` 自定义协议 + `with_ipc_handler`；同一模块里已有 loopback HTTP 变体（`page_address(over_http)`） |
| Linux 输入 | GTK 窗口重挂 + 直接 `XSetInputFocus` 交接键盘 + GTK `draw` 信号驱动的「按需 turn」 |
| 已知限制 | `docs/native-wayland.md`：原生 Wayland 没有 X11 子窗口可给，面板显示诊断；PR #110 XWayland 为生产基线，PR #111 GTK3 原生 Wayland Host 因 readback / 空闲刷新 / 事件循环问题冻结 |
| 基线性能 | `docs/linux-webview-performance.md`：真实桌面 XWayland 下空闲约 11.8 fps（85 ms/帧）、拖拽 99.9 fps；空闲 CPU 未单独记录 |

**兼容性结论（实测，不是推断）**

- welding 0.15.0 默认就是 **wgpu 30**，与 IntCAD 的 30.0.1 同属一个 crate 版本，所以
  `ImportedTexture.texture/view` 可以直接交给 `egui_wgpu::Renderer::register_native_texture`，
  **不需要**把 CAD Renderer 从 wgpu 换成别的，也不需要 `egui_glow`。
- 真正的缺口不在渲染器，而在**设备创建**：DMA-BUF 导入要求 Vulkan device 在
  `VkDeviceCreateInfo` 里启用 `VK_EXT_queue_family_foreign`，这只能通过
  `hal_adapter.open_with_callback` + `create_device_from_hal` 做到；eframe 默认走
  `adapter.request_device`，即使把 `VULKAN_EXTERNAL_MEMORY_DMA_BUF` 加进
  `required_features` 也不行。本实验先复现了这个失败（`VulkanForeignQueueExtensionNotEnabled`），
  然后用 grafting 的 `create_dmabuf_host_context` 建 device、手工构造
  `egui_wgpu::RenderState` 并以 `WgpuSetup::Existing` 交给 eframe，面板随即正常显示。
  这是 **必须写进迁移方案的一处改动**，不是可选项。

## 2. 版本与 commit

| 组件 | 版本 / 来源 |
|---|---|
| IntCAD 基线 | `origin/develop` = `96a247ccb`（PR #179 合并后），实验分支 `probe/cef-welding-osr` |
| welding | 0.15.0（crates.io，MPL-2.0），默认 features + `cef-runtime` |
| CEF | `154.5.0+154.0.34`（Chromium `154.0.8037.98`），`cef_binary_154.0.34+g14c5a08+chromium-154.0.8037.98_linux64_minimal` |
| wgpu / wgpu-hal | 30.0.1（与 IntCAD 锁文件一致） |
| egui / eframe | 0.37.0（与 IntCAD 一致） |
| 宿主 | NixOS 26.11，Rust 1.98.1，Mesa 26.3.0，Vulkan 1.4.363 |
| GPU | AMD Radeon RX 9060 XT（RADV GFX1200）、Intel BMG G31（ANV/xe）、AMD Raphael iGPU |
| 会话 | niri（原生 Wayland）+ XWayland `:0`；另用 headless sway 1.12（wlroots，无 X server）与 weston 16 做纯 Wayland 验证 |
| 官方 demo | `github.com/merely-made/wgpu-weld` HEAD（welding 0.15 对应源码），`demo-weld-linux` |

## 3. Phase 1–5 结论矩阵

| 阶段 | 项目 | 结论 | 证据 |
|---|---|---|---|
| 1 | 官方 demo + 真实 CEF 启动、加载页面 | **PASS** | `official-demo-wayland.log`：CEF 初始化、页面 LoadEnd |
| 1 | 官方 demo 触发 `OnAcceleratedPaint` 并导入 4 帧 | **PASS** | 同文件：`imported frame #1..#4 (1280x800 Bgra8UnormSrgb)`，像素校验 exit 0 |
| 1 | 无 CPU readback（CEF→宿主路径） | **PASS** | 帧以 DMABUF fd 交付；readback 仅出现在探针的校验代码里 |
| 1 | 纹理生命周期 / resize | **PASS** | 每帧 owned fd；面板 resize 后纹理 1144×884 → 644×604 |
| 1 | 无隐藏软件回退 | **PASS** | 全部日志无 SwiftShader；`--ozone-platform=headless` 时**一帧都没有**，说明加速路径不是软件兜底 |
| 2 | A. 仅 `WAYLAND_DISPLAY`、无 `DISPLAY` | **PASS** | 02/03/06/07/10：各导入 1 帧，中心像素 `#1e90ff` 精确匹配 |
| 2 | B. 不连 X11 socket | **PASS** | `DISPLAY=:99`（假值）+ strace：`X11-unix` 连接尝试 **0 次** |
| 2 | C. 完全没有 X server 的纯 Wayland | **PASS** | headless sway（无 XWayland 进程、`DISPLAY` 未设置）下官方 demo 与面板均出帧 |
| 2 | D. Ozone 参数对比 | **PASS（有边界）** | `wayland` 与「自动」均工作；`x11` 无 `DISPLAY` 时初始化失败（`Missing X server or $DISPLAY`）；`headless` 不产生加速帧（默认 ANGLE 下直接 SIGTRAP） |
| 2 | E. DMA-BUF 实际共享路径 | **PASS** | 见 §5：1 buffer/1 plane、stride 4096、modifier X11 `0x00ffffffffffffff` / Wayland `0x0` |
| 3 | egui + 页面同窗口、无子窗口 | **PASS** | `panel-fixture.png` |
| 3 | 面板 resize / HiDPI | **PASS** | resize 截图与纹理尺寸；X11 1.25 倍缩放下页面 `dpr 1.25`、CSS 384×464 |
| 3 | 鼠标 / 滚轮 / 键盘 / 焦点 | **PASS** | 页面回传 `mousedown 150,98`、`wheel -20/-40`、`key h…`；面板 `INPUT` 轨迹 |
| 3 | 中文 IME 组合与提交 | **PASS（CEF 侧）** | `compositionstart` → `compositionupdate "ni hao"` → `compositionupdate "你好"` → `compositionend "你好"`，X11 与 Wayland 各一次 |
| 3 | Web popup（`<select>`） | **PASS** | `popup rect {x:408,y:108,width:67,height:98}` |
| 3 | 剪贴板 | **UNTESTED** | 需要真实 seat 上的剪贴板所有者；headless sway 的 seat 无键盘能力 |
| 3 | 面板隐藏 / 恢复 | **UNTESTED** | 控件已接 `set_visible`，未做自动化 |
| 3 | 事件循环重入、长时间泄漏 | **PARTIAL** | 约 40 次启停无崩溃；未做 ≥1 小时 RSS 曲线 |
| 4 | 空闲 CPU/内存 | **PASS** | 探针 0.72% / 585 MiB / 9 进程；面板 0.75% / 734 MiB / 8 进程 |
| 4 | 空闲实际帧率 | **PASS** | 静态页 40 s 内仅 1 帧；面板 10 s 内 6 帧（交互期 p50 32 ms） |
| 4 | 首帧时间 | **PASS** | 63–150 ms（Wayland 65–89 ms；一次冷启动 1.1 s，需更大样本） |
| 4 | Resize 压力（连续） | **PARTIAL** | 单次 1144×884→644×604 验证；未做 100 次连续 resize |
| 4 | 多次开关资源释放 | **UNTESTED** | 每次独立进程；未在同一进程内反复 create/destroy |
| 4 | CAD 视口性能影响 | **UNTESTED** | 未接入 CAD Renderer（按要求不迁移） |
| 4 | AMD / Intel 双 GPU | **PASS（功能）** | Intel 宿主 + AMD CEF（跨 GPU）、Intel+Intel、AMD+AMD 均像素精确；跨 GPU 性能未测 |
| 5 | Linux 沙箱 | **PASS（本机）** | `PROBE_SANDBOX=sandboxed` 下仍导入帧、像素匹配 |
| 5 | Windows / macOS 运行验证 | **UNTESTED** | 无可用 GPU 设备；仅做静态分析 |
| 5 | 发布体积 / 许可 | **PASS（有代价）** | 见 §7：`libcef.so` 1388 MiB，整目录 1.5 GB；CEF BSD-3 + MPL-2.0 welding |

## 4. Wayland 是否真正摆脱 XWayland：**是（PASS）**

判定依据是三条独立的证据，而不是「在 Wayland 会话里能跑」：

1. **进程环境**：`DISPLAY` 未设置，只给 `WAYLAND_DISPLAY`（`wayland-1`），CEF 仍产出
   加速帧；同时 `DISPLAY=:99`（一个不存在的 X server）也能正常出帧，说明它根本没去用 X。
2. **系统调用**：同一场景下 `strace -f -e trace=connect` 记录里，`/tmp/.X11-unix` 的连接
   尝试为 **0**。
3. **无 X server 的合成器**：headless sway 里没有 XWayland 进程，官方 demo 的像素校验与
   本报告的 egui 面板都在其上通过。

必须写清楚的边界：

- CEF 仍需**某种显示连接**。`--ozone-platform=headless` 默认 ANGLE 直接崩溃
  （`Could not open the default X display` → SIGTRAP），加 `use-angle=gl-egl` 后能加载
  页面但**没有**加速帧。也就是说：无显示服务器的 CI 不能靠 headless 拿 GPU OSR，必须
  起一个 headless Wayland 合成器（本实验用 sway）。
- Ozone 平台必须与实际会话一致。把 `XDG_SESSION_TYPE=wayland` 但 `WAYLAND_DISPLAY`
  去掉，CEF 会在 `CefInitialize` 阶段失败；`XDG_SESSION_TYPE=x11` 而无 `DISPLAY`
  同样失败。生产集成时这两个变量要由宿主显式设定，不能依赖继承。
- 本实验里 CEF 154 在 Wayland 下的**默认**参数就能出 DMABUF（case 02），不需要额外
  `--use-angle=gl-egl`；这与 CEF issue #3953 的修复（2025-07）一致，但该修复是 cefclient
  侧的默认值，宿主库不能假设它永远存在，建议显式传参。

## 5. GPU 纹理导入路径与性能证据

导入链（实测）：

```
CEF GPU 进程 (ANGLE/EGL + GBM)
  └─ OnAcceleratedPaint: dmabuf fd(s) + offset/stride/modifier + fourcc   (Bgra8UnormSrgb, 1024×720)
       └─ welding 在回调内 dup(2) 并持有 OwnedFd（回调返回后 CEF 会回收资源）
            └─ grafting: VkImportMemoryFdInfoKHR + VK_EXT_image_drm_format_modifier
                 └─ wgpu-hal 注册外部内存（需 VK_EXT_queue_family_foreign 在设备上启用）
                      └─ wgpu::Texture → TextureView → egui_wgpu 注册为 TextureId
```

实测数据（`07-wayland-gl-egl.log`、`01-x11-wayland-unset.log`）：

- `size 1024x720`、`format Bgra8UnormSrgb`、`drm_format 0x00000000 (ARGB8888)`、
  `buffers 1`、`planes 1`、`offset 0`、`stride 4096`、`plane size 2949120`。
- modifier 在 Wayland 下为 `0x0`（显式 linear），在 X11 下为 `0x00ffffffffffffff`
  （`DRM_FORMAT_MOD_INVALID`，隐式）；welding 把隐式按 linear 处理，RADV 上整幅纹理
  读回验证通过（不是只数非零字节）。
- 像素校验：中心像素 BGRA `[255,144,30,255]` = RGB `#1e90ff`，与页面背景完全一致。
- 首帧：Wayland 65–89 ms、X11 73–150 ms（进程启动 → 第一次导入）；面板里交互期帧间隔
  p50 ≈ 32 ms（约 31 fps 的 CEF 出帧上限，不是宿主轮询上限）。
- 空闲：探针 40 s 只有 1 帧、CPU 0.72%；面板 12 s 采样 0.75% CPU、734 MiB RSS（8 进程）。
  这直接避开了 PR #111 的「空闲 60 FPS / 高 CPU」问题：CEF 加速 OSR 是**变化驱动**的。
- 对照（同一台机器，生产 `solidcraft` 0.1.1 打包二进制 + Assistant 打开，Xvfb/llvmpipe
  呈现）：空闲 20 s **53.7% CPU / 942 MiB / 3 进程**（`wry-baseline.out`）。这**不是**
  同后端对比（一边 llvmpipe 呈现，一边 GPU 呈现），只能说明方向一致，不能当作定量结论。

## 6. 剩余风险

**技术**

1. **设备创建必须自有**（已复现并绕过）：eframe 默认路径拿不到
   `VK_EXT_queue_family_foreign`。选项：(a) 只手写 `RenderState`（本 PoC 的做法，约
   100 行，需要假定 `Bgra8UnormSrgb` 表面格式）；(b) 给 egui-wgpu 提一个
   `device_creator` 钩子（更干净，但要上游接受）。
2. **首帧前输入被丢弃**：在第一次 `OnAcceleratedPaint` 之前发送的鼠标/键盘/IME 不会到
   页面。宿主必须把输入排队到第一帧之后（本探针在加 `frame_count > 0` 门控前，点击与
   IME 全部静默丢失）。
3. **页面传输要换**：welding 0.15 没有 `with_custom_protocol` 等价物，`window.ipc.postMessage`
   也不存在。真实 `packages/assistant-web/dist` 用 `file://` 打开时模块脚本被 CORS 拦；
   用 loopback HTTP 打开时脚本能跑，但因为没有 host 注入状态而显示空白
   （`panel-real-assistant-page.png`）。需要：复用 IntCAD 已有的 loopback HTTP
   (`page_address(true)`)，再把 `window.ipc` 用一个 5–10 行的 shim 接到 CEF 的
   message router（`post_web_message` / `WebMessage`）或 CDP `Runtime.evaluate`。
4. **CEF 上游缺陷**：原生 DevTools 窗口在 151 起在 Windows/Linux 崩溃（CEF 已知，使用 CDP
   代替）；Linux 上渲染进程崩溃事件不投递（welding 文档注明的平台差异）。
5. **无显示服务器即无 GPU OSR**（见 §4），CI 需要 headless 合成器。
6. **多 GPU**：跨 GPU（Intel 宿主 + AMD CEF）功能可用且像素正确，但性能与稳定性未测；
   生产应固定单一 GPU（`--gpu-preferences` / 设备选择）。
7. **内存与启动**：CEF 常驻 585–734 MiB、8–9 个进程；比 WebKitGTK（942 MiB / 3 进程）
   进程多但总内存更低。冷启动首帧 0.1–1.1 s 波动大，需要更大样本。
8. **NixOS 运行时**：`libcef.so` 没有 RPATH，必须显式 `LD_LIBRARY_PATH`
   （`nix/runtime.nix` 已验证）；发行版打包要么 patchelf，要么 wrapper。

**安全**

- 沙箱不是可选项，但**可用**：`PROBE_SANDBOX=sandboxed` 在 NixOS（非特权 user
  namespace）下正常出帧。生产必须在三平台分别证明：Linux 需要 `chrome-sandbox`
  或 userns 可用；Windows 走 CEF 154 的 `bootstrap.exe` + `CefWindowsSandboxContext`；
  macOS 需要 `libcef_sandbox.dylib` 与 helper bundle。**不能**把
  `UnsandboxedTrustedContent` 当交付形态。
- 页面内容必须保持「只加载自带资源」：现有 CSP（`default-src 'none'`）与导航拦截要继续
  生效；CEF 侧要用 `with_navigation_handler` 等价物（welding 的 `NavigationEvent`）拦掉
  外链，禁用 DevTools 原生窗口，权限请求默认拒绝（welding 默认就是 deny）。

**许可证**

- CEF/Chromium：BSD-3-Clause + 大量第三方（`CREDITS.html` 8.7 MB），允许闭源分发，但
  必须随包附 notices，并保留 Chromium 的许可证文本。
- welding / grafting：MPL-2.0（文件级 copyleft）。链接不传染，但修改过的 MPL 文件要
  公开。对本项目**比现状更宽松**：现在 Linux 依赖 LGPL 的 WebKitGTK，需要
  「系统库、不改动、动态链接」的例外；CEF 不需要这个例外。`skills/solidcraft-ground-rules`
  的 GPL/LGPL 限制不受影响。
- 需要新增 `ATTRIBUTION.md`/notices 条目，并过一遍 `cargo xtask licences`（CEF 本体不是
  Cargo 依赖，需单独登记）。

## 7. Windows / macOS / Linux 维护与发布成本

| 平台 | 构建依赖 | 运行/打包 | 体积 | 现状 |
|---|---|---|---|---|
| Linux | cmake + C++ 编译器（`cef-dll-sys` 编 `libcef_dll_wrapper`），CEF 压缩包 ~311 MB | `libcef.so` + `Resources`/`locales`/`*.pak`/`icudtl.dat`/`v8_context_snapshot.bin`；Nix/发行版需要 rpath 或 wrapper | 目录 1.5 GB；`libcef.so` 1388 MiB（未 strip），`locales` 49 MB（可裁到 en-US/zh-CN） | 实测通过 |
| Windows | MSVC + cmake；CEF 154 的 `libcef.dll`、`bootstrap.exe` | 沙箱上下文必须在 client DLL 里；签名、安装器、崩溃处理 | 与 Linux 同量级（未实测） | 未验证（无 GPU VM） |
| macOS | Xcode + cmake；`Chromium Embedded Framework.framework` + helper apps | 每个 helper 必须调用 `try_run_subprocess`；公证、`libcef_sandbox.dylib` | framework 通常 300–500 MB | 未验证（无设备） |

维护成本主要是 **Chromium 版本节拍**：welding 0.15 绑定 CEF 154，升级 Chromium 需要
welding/cef crate 同步升级 + 三平台回归；参考上游的 parity battery（每平台一次，含
IME/popup/剪贴板/CDP）。建议每季度一次，预留约 1 周。

## 8. 若迁移：涉及模块、工作量与顺序

| 模块 | 改动 | 量级 |
|---|---|---|
| `apps/solidcraft/src/main.rs` | 用 grafting `create_dmabuf_host_context` 建 device，构造 `RenderState` 并以 `WgpuSetup::Existing` 交给 eframe（或改为上游钩子） | 80–150 行 |
| `apps/solidcraft/src/assistant_web.rs`（1046 行） | 删除 wry/GTK/X11 子窗口/reparent/键盘交接/`at_work` turn；换成 CEF producer：创建、`acquire_native_frame`、resize、`set_visible`、导航策略、生命周期 | 新 ~500–700 行，删 ~700 行 |
| `crates/ui-egui/src/agent_chat/panel.rs` | 保留面板矩形；注册 native texture 并绘制；页面 z-order 退化为 egui 图层（菜单/对话框遮挡问题消失） | 100–200 行 |
| 输入 | egui 事件 → `send_mouse_input` / `send_keyboard_input` / `ime_*` / `send_drag_input`；首帧前排队；焦点进入/退出 | 200–300 行 + 测试 |
| 页面传输 | 自定义协议 → 复用 loopback HTTP；`window.ipc.postMessage` → CEF message router shim | 150–250 行（含前端 shim） |
| 打包 | `packaging/{linux,windows,macos}`：CEF 运行时、notices、Nix wrapper、CI 缓存 | 2–4 周 |
| 测试 | 现有 E2E 的 Assistant 步骤改为「纹理非空 + 页面事件」；沙箱回归 | 1–2 周 |

顺序建议：

1. **先行 spike**：在 `assistant_web.rs` 之外新建 `assistant_cef` 模块，保留 WRY 路径与
   XWayland fallback，用 feature flag 切换。
2. 打通 device + 纹理 + resize + 输入（本 PoC 已验证）。
3. 接真实页面（HTTP + IPC shim），跑 `packaging/assistant-check.py` 同等验收。
4. 三平台打包与沙箱证明；再决定是否删除 WRY 路径。
5. 期间持续用 `docs/linux-webview-performance.md` 的 `frame_gaps_ms` 与空闲 CPU 对照。

粗估：Linux 端到端 **6–9 周**（含测试与打包），Windows +2–3 周，macOS +2–3 周；
不含 Chromium 升级维护。

## 9. 建议：CONDITIONAL GO

**GO 的部分**：CEF + welding 在 Linux 上完成了本任务最难的验收——原生 Wayland、
GPU 加速、无 XWayland、无 CPU 回读、纹理直接进现有 wgpu/egui 渲染器；并且空闲行为优于
当前 WebKitGTK 路径（变化驱动，无 60 FPS 空转）。

**条件（必须先满足再谈「统一架构」）**：

1. 决定 device 创建方案（自有 `RenderState` 或上游 egui-wgpu 钩子），并写进架构文档。
2. 用真实 Assistant 页面打通 HTTP + IPC shim，取得一次端到端截图与一轮交互验收。
3. Linux 沙箱在目标发行版（含禁用 userns 的系统）上给出通过证据；Windows/macOS 各自
   完成沙箱与打包证明。
4. 接受发布体积（每平台约 +1.4 GB 未裁剪，裁剪后仍需数百 MB）与季度级 Chromium 升级。
5. 在完成 1–4 之前，**保留 WRY + XWayland 为生产路径**，不删除 fallback，不合并迁移 PR。

**最小复现**（阻断点、不是结论）：把 `egui_wgpu` 默认设备直接拿去 `WgpuTextureImporter::import`
会得到 `Vulkan external memory import failed: unsupported interop path:
VulkanForeignQueueExtensionNotEnabled`；`experiments/cef-osr/poc/src/bin/panel.rs` 的
`build_dmabuf_capable_device` + `RenderState` 段落是绕过它的最小改动。
