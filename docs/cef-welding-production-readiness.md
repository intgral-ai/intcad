# CEF + welding 生产就绪度验证（第二轮）

分支 `probe/cef-welding-osr`，基线 `40f727437`（第一轮），本轮提交见文末。
实验代码：`experiments/cef-osr/`；本轮证据：`experiments/cef-osr/evidence/2026-11-18-round2/`。

## 结论：**CONDITIONAL GO**

> **2026-11-19 修复波状态说明：** 本文件记录的是第二轮历史验证，截图和数字保持原样，不因本轮
> 源代码修复而重新认定为当前通过。当前修复分支的 Cargo/原生/GPU/VM/视觉与四倍率、
> 全部主题验收仍为 **BLOCKED / NOT_RUN**；本轮只完成源码、Python 离线自检和静态检查。
> 详情和精确缺口见 `.superpowers/sdd/2026-11-19-cef-integration-repair/repair-report.md`。


- Linux 原生 Wayland 与 X11（经 XWayland）的 GPU OSR、输入、生命周期、GPU 资源管理、
  Linux 沙箱均通过；剪贴板在 CEF 原生路径上**不可用**（上游 cef#4341），但主机侧桥接已验证
  可用。
- **Phase 3（真实 CAD 并行性能）本轮未做集成验证**，按本任务自己的门槛不能 PASS；
  **真实 Xorg 会话**因本机无 root / 无 VT 无法启动（`Cannot open /dev/tty0`），标 UNTESTED；
  Windows / macOS 无 GPU 设备，标 UNTESTED。
- 因此不满足 ADOPT 的四项前置条件，结论为 CONDITIONAL GO：架构成立，但不得宣布生产就绪，
  不得合并生产迁移。

---

## 1. 环境与版本

| 项 | 值 |
|---|---|
| 宿主 | NixOS 26.11，kernel 7.3.4，Rust 1.98.1，Mesa 26.3.0，Vulkan 1.4.363 |
| 会话 | niri（原生 Wayland）+ XWayland `:0`；headless sway 1.12（无 X server） |
| GPU | AMD RX 9060 XT（RADV GFX1200）、Intel BMG G31（ANV）、AMD Raphael iGPU；无 NVIDIA |
| CEF | `154.5.0+154.0.34`（Chromium 154.0.8037.98），Linux x64 minimal，311 MiB tar.bz2 |
| welding | 0.15.0（MPL-2.0），默认 wgpu 30 |
| IntCAD 基线 | `origin/develop` = `96a247ccb`，未修改；生产 WRY 路径未触碰 |
| 真实 Xorg | **无法启动**：`parse_vt_settings: Cannot open /dev/tty0 (Permission denied)`，`sudo` 需要密码 |

## 2. 结论矩阵

### Phase 1 输入与生命周期（P0）

| 项目 | 结论 | 证据 |
|---|---|---|
| 首帧有界输入队列（顺序、合并移动、上限、清理） | **PASS** | 3 个单元测试 + soak：`queue_held=9 / sent=9 / typed=true`；对照 `queue_control_X_delivered=false`（不经队列的预首帧按键确实丢失） |
| 剪贴板 Web→系统（ASCII） | **PASS** | `copy_out: PASS (clipboard-test)`，`CLIPBOARD copy_out 14 byte(s)` |
| 剪贴板 系统→Web（中文） | **PASS** | `paste_cn: PASS`，`CLIPBOARD paste_in 15 byte(s)`，页面 `input "你好，世界"` |
| CEF 原生剪贴板 | **FAIL（上游）** | cef#4341（2026-11-02，open）：OSR 下不调用 `wl_data_device.set_selection`，系统剪贴板不更新；本轮实测原生路径 copy 不产生 `copy` 事件 |
| 焦点：页面 ↔ CAD ↔ 系统 | **PASS** | 点页面后键盘进页面；点侧栏后页面不再收到按键；再点页面恢复：`focus_*: PASS` |
| 中文 IME 连续输入 / 组合 / 取消 | **PASS（CEF 侧）** | `zhong` → 取消（`compositionend ""`）；`ni hao`→`你好`→commit；再连续 `世界`→`你好世界` |
| OS 输入法引擎（fcitx5 候选窗） | **UNTESTED** | 需要真实 seat 的 IME 会话；本轮 IME 走 CEF API 序列，未驱动 fcitx5 |
| `<select>` popup / 右键菜单 | **PASS** | `POPUP rect x=510 y=135 w=83 h=123`；`ContextMenuRequested` 1 次 |
| 隐藏 / 恢复 / 最小化 / 零尺寸 | **PARTIAL** | soak 覆盖 `set_visible(false/true)` 与 `resize(0,0)`；窗口最小化未单独验收 |
| HiDPI 1.0 → 1.25 → 2.0 | **PASS** | soak 每次循环切换；面板实测 `dpr 1.25` |
| 同进程创建/销毁 100 次 | **PASS** | 100/100 首帧，0 失败，8.5 s；RSS 280.2→282.2 MB，fd 119→116，VRAM 21 MiB 恒定 |
| resize + 输入压力 | **PARTIAL** | 每次循环 8 次随机尺寸 resize + 零尺寸；未做 1000 次连续压力 |
| 渲染进程崩溃与恢复 | **PASS** | `terminated_event=true, recovered_frame=true` |
| 关闭与资源释放 | **PASS** | 100 次 close/drop 无 fd/RSS 增长 |

### Phase 2 Linux 显示协议（P0）

| 项目 | 结论 | 证据 |
|---|---|---|
| 原生 Wayland：无 DISPLAY、仅 WAYLAND_DISPLAY | **PASS** | 第一轮 + 本轮；DMABUF 导入、像素 `#1e90ff` 精确 |
| 原生 Wayland：无 X11 socket | **PASS** | `DISPLAY=:99` + strace：X11 连接 **0** 次 |
| 禁用 XWayland 的纯 Wayland | **PASS** | headless sway（无 X server）下 demo 像素校验 exit 0、soak 100 次通过 |
| 原生 X11（真实 Xorg 会话） | **UNTESTED** | 环境无法启动 Xorg（无 root/VT）；见 §1 |
| X11 协议 + GPU（XWayland） | **PASS** | iozone=x11 + ANGLE gl-egl，DMABUF modifier `0x00ffffffffffffff`，像素精确 |
| XWayland 过渡兼容 | **PASS** | 与上一行同一路径；生产当前用户即运行在此 |
| 无 X11/Wayland 连接（`--ozone-platform=headless`） | **FAIL（预期）** | 默认 ANGLE 直接 SIGTRAP；`use-angle=gl-egl` 可加载页面但 **0** 加速帧 |
| GPU 能力不足（缺 `VK_EXT_queue_family_foreign`） | **PASS（行为正确）** | 明确报 `VulkanForeignQueueExtensionNotEnabled`，`imported=0`，exit 1，无崩溃、无 CPU 回退 |
| 软件渲染回退 | **未发生** | 全部日志无 SwiftShader；无 `cpu-paint-fallback` feature |

协议矩阵：

| 显示协议 | GPU 加速 OSR | DMA-BUF 导入 | 输入/IME | 剪贴板（原生） |
|---|---|---|---|---|
| 原生 Wayland（niri / headless sway） | PASS | PASS（modifier 0） | PASS | FAIL（cef#4341） |
| XWayland（niri 上的 X11 客户端） | PASS | PASS（modifier implicit） | PASS | FAIL（需桥接） |
| 原生 Xorg | UNTESTED | UNTESTED | UNTESTED | UNTESTED |
| 无显示（headless ozone） | FAIL | — | — | — |

### Phase 3 真实 CAD 并行性能（P0）

| 项目 | 结论 | 说明 |
|---|---|---|
| 真实 CAD 视口 + CEF 同进程帧耗时对照 | **UNTESTED** | 本轮未把 CEF 表面接入 `apps/solidcraft`。按任务门槛，未集成不得 PASS |
| Phase 1 的合成开销上界 | 参考 | 空闲 0.72% CPU / 585 MiB；交互帧间隔 p50 32 ms；静态页 40 s 仅 1 帧 |

> 这是本轮唯一未完成的 P0，也是 ADOPT 的主要缺口。实施顺序见 §7 与迁移 Spec。

### Phase 4 GPU 与跨平台（P0）

| 项目 | 结论 |
|---|---|
| AMD（RADV）：device 创建、导入、像素 | **PASS** |
| Intel（ANV/BMG）：同上 | **PASS** |
| 跨 GPU（Intel 宿主 + AMD 渲染） | **PASS（功能）**；性能未测 |
| 多 GPU 选择 | `PROBE_ADAPTER`（宿主）+ `MESA_VK_DEVICE_SELECT`（CEF）已验证 |
| NVIDIA | **UNTESTED**；上游已知 dmabuf/GBM 兼容问题（cef#3687、#4237） |
| 最低硬件 | Vulkan 设备需 `VK_EXT_external_memory_dma_buf` + `VK_EXT_image_drm_format_modifier` + `VK_EXT_queue_family_foreign`；Mesa AMD/Intel 满足 |
| Windows（E2E VM） | **UNTESTED**：现有 VM 无 GPU；D3D12 shared texture 路径仅上游 receipt |
| macOS | **UNTESTED**：Metal/IOSurface 路径仅静态审阅 |

### Phase 5 Release、沙箱与许可（P1）

| 项目 | 结论 | 数据 |
|---|---|---|
| Release 构建运行 | **PASS** | 全部实验二进制 `--release` |
| Chromium sandbox 真正启用 | **PASS** | `SOAK_SANDBOX=sandboxed` 10/10 循环通过（NixOS 非特权 user namespace）；无隐式降级日志 |
| CEF 子进程 / runtime / 资源打包 | **PASS（清单见下）** | `libcef.so`、`*.pak`、`icudtl.dat`、`v8_context_snapshot.bin`、`locales/`、`libvulkan.so.1`、`libvk_swiftshader.so`（可裁）、`chrome-sandbox` |
| 原始体积 | 1.5 GB 目录 | `libcef.so` 1387.6 MiB |
| strip 后 | **259.6 MiB**（-81%） | 运行完整性由 soak/面板复测通过 |
| 压缩下载 | 311 MiB（tar.bz2） | 官方 minimal |
| locales | 49 MiB / 220 个 | 可裁到 en-US + zh-CN |
| 许可 | CEF/Chromium：BSD-3 + 第三方；welding/grafting：MPL-2.0 | 必须随包附 `CREDITS.html`（8.3 MiB）与 Chromium 许可文本；MPL 只约束 welding 文件本身 |
| CI 成本 | 每次干净构建 +311 MiB 下载、cmake/ninja、`libcef_dll_wrapper`（约 2 分钟） | 建议缓存 CEF 与 wrapper |

## 3. 本轮发现的两个架构级问题

1. **CEF OSR 的剪贴板不可用**（上游 cef#4341，open）。Chromium 在无窗口 OSR 下创建了
   Wayland data source 却没有 `set_selection`（没有 serial），系统剪贴板不更新，反向粘贴也
   读到过期内容。宿主必须实现剪贴板桥（本 PoC 已实现并通过）：复制时用脚本读取当前
   选区/活动输入框，经 `egui` 写入系统剪贴板；粘贴时把系统文本以 IME commit 注入焦点控件。
   生产实现还需覆盖剪切、富文本降级、图片（明确不支持）。
2. **Ctrl 被当成 Ctrl+Super**：把 `Modifiers::command` 同时写入 CEF 的 meta 位会让所有
   Ctrl 快捷键变成 Ctrl+Super，页面看到普通按键。生产映射必须 `meta = mac_cmd only`。
   这个错误在 PoC 里直接导致复制/全选失效，属于集成时必须固化的约定。

另外确认：**预首帧输入确实会被静默丢弃**（对照实验 `queue_control_X_delivered=false`），
有界队列是必需项，不是优化项。

## 4. 性能与资源（本轮实测）

| 场景 | CPU | RSS | 进程 | VRAM | 帧行为 |
|---|---|---|---|---|---|
| soak 100 次创建/销毁（Wayland） | — | 280 MB（+2 MB/100 次） | 8–9 | 21 MiB 恒定 | 每次首帧 <1 s |
| soak 沙箱模式 10 次 | — | 264 MB | 8–9 | 21 MiB | 0 失败 |
| 探针空闲 40 s（第一轮） | 0.72% | 585 MB | 9 | — | 1 帧 |
| 面板空闲（第一轮） | 0.75% | 734 MB | 8 | — | 6 帧/10 s（宿主 200 ms 轮询） |
| WRY + XWayland 基线（Xvfb/llvmpipe） | 53.7% | 942 MB | 3 | — | 文档口径空闲 ~11.8 fps |

跨后端注记：WRY 数字是 llvmpipe 呈现，CEF 数字是 GPU 呈现，不能当作定量对比；Phase 3 的
公平对照必须在真实桌面上用同一模型完成。

## 5. 停止理由与未完成项

- **Phase 3 未做**：需要把 CEF 表面接进 `apps/solidcraft`（设备钩子 + 面板绘制 + IPC），
  属于生产迁移的第一步，本轮按"只验证、不迁移"的边界未执行。因此不得宣称生产就绪。
- **真实 Xorg 不可得**：无 root/VT 权限，`Xorg` 在 `parse_vt_settings` 失败。XWayland 提供
  了真实 X11 协议 + 真实 GPU 的等价证据，但按任务定义不能替代原生 Xorg 验收。
- **Windows/macOS**：无 GPU 设备，全部 UNTESTED。

## 6. ADOPT 前置条件（逐项、可验收）

| # | 条件 | 验收方式 | 阻塞级别 |
|---|---|---|---|
| 1 | 真实 CAD 并行性能 | 接入后同机同模型，CAD p95 回归 ≤10%，无 GPU 阻塞；Wayland 与 X11 分开报告 | P0 |
| 2 | 原生 Xorg GPU OSR | 在带 AMD/Intel 的 Xorg 会话跑 `matrix.sh` 的 x11 用例 + 面板输入验收 | P0 |
| 3 | Windows GPU 导入与生命周期 | 有 GPU 的 Windows 机器跑同一批 receipt（D3D12 路径） | P0 |
| 4 | macOS Metal/IOSurface 与打包 | Apple Silicon + Intel Mac 各一次 parity | P0 |
| 5 | 剪贴板生产桥接 | 复制/粘贴/剪切、中文、跨应用，纳入 E2E | P1（PoC 已通过） |
| 6 | 发布体积与 CI | 采用 strip + locale 裁剪后的安装包体积与流水线时间 | P1 |

## 7. 迁移实施顺序（与 Spec 对应）

1. `GpuDeviceFactory`：用 grafting 建共享 device，eframe 走 `WgpuSetup::Existing`（PoC 已验证）。
2. `BrowserRuntime`：CEF 早期初始化、`execute_process_from`、沙箱模式、退出。
3. `WebSurface`：producer 生命周期、resize/scale/visibility、有界输入队列、剪贴板桥。
4. `AssistantSurface`：在 `agent_chat` 面板内注册纹理并转发输入；保留现有 IPC（改为 HTTP +
   message-router shim）。
5. Phase 3 性能对照 → 达标后再删除 WRY 路径；在此之前 feature flag 并行存在。

## 8. 最终判定

**CONDITIONAL GO。** 主要技术路径成立且 Linux 输入/生命周期/GPU 资源管理可靠；但 Phase 3
与原生 Xorg、Windows、macOS 均为 UNTESTED，不满足 ADOPT 的四项 P0。下一步应只做"迁移
Spec 已定义的第一阶段"（设备工厂 + 面板嵌入 + 性能对照），取得 CAD p95 证据后再申请 ADOPT。
