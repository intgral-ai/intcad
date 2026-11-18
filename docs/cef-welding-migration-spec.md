# CEF + welding 迁移架构 Spec（草案）

状态：草案，供评审。**不实施**；本文件只定义接口、边界与验收，供后续任务拆分。

依据：`docs/cef-welding-feasibility.md`（第一轮）、
`docs/cef-welding-production-readiness.md`（第二轮）。
实验代码：`experiments/cef-osr/`。

## 0. 不变量

- 保留现有 Assistant 前端（`packages/assistant-web`）与业务 IPC 语义；不重写 CAD Renderer；
  不引入新的原生 Web 子窗口或跨进程窗口定位机制。
- 继续使用 IntCAD 现有 `wgpu` 设备与 egui 渲染循环；不新增第二条呈现路径。
- WRY 路径在迁移期间保留，可用环境变量/偏好回退。

## 1. 组件

### 1.1 `GpuDeviceFactory`（新，`apps/solidcraft` 内）

职责：产出一个 **既能给 egui 绘制、又能导入 CEF DMA-BUF** 的 `wgpu::Device`。

```rust
pub struct InteropDevice {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub target_format: wgpu::TextureFormat,
}

pub enum InteropError {
    NoVulkanAdapter(String),
    ExtensionMissing { missing: Vec<&'static str> },
    DeviceOpen(String),
    SurfaceFormat(String),
}

/// Linux: `grafting::vulkan_dmabuf::create_dmabuf_host_context`（等价于
/// `welding::build_dmabuf_capable_device`）——它会用 `open_with_callback`
/// 在设备创建时加上 `VK_EXT_queue_family_foreign` 等扩展。
/// Windows/macOS: 普通 `request_device`，由 D3D12/Metal 导入路径决定。
pub fn create_interop_device(adapter: &wgpu::Adapter, desc: &wgpu::DeviceDescriptor)
    -> Result<InteropDevice, InteropError>;
```

接入点：`apps/solidcraft/src/main.rs` 的 `wgpu_options()`。当 CEF 表面启用时：

1. 先枚举并选择 adapter（沿用现有 `startup::configure_adapter` 的品牌偏好 / `WGPU_ADAPTER_NAME`）；
2. 用 `create_interop_device` 建 device；
3. `options.wgpu_options.wgpu_setup = WgpuSetup::Existing(WgpuSetupExisting { instance, adapter, device, queue })`；
4. 目标格式固定为 `Bgra8UnormSrgb`，若表面不支持则报 `SurfaceFormat` 并回退 WRY。

失败处理：**不得**降级到普通设备后"假装成功"；导入失败必须显式报错（见 1.4）。

### 1.2 `BrowserRuntime`（新）

职责：CEF 进程生命周期与子进程分派。

```rust
pub struct BrowserRuntime { /* CefRuntime */ }

pub enum RuntimeConfig {
    sandbox: CefSandboxMode,          // 生产必须 Sandboxed
    cache_root: PathBuf,
    switches: Vec<(String, Option<String>)>,
}

impl BrowserRuntime {
    /// 必须在 main 的第一行调用；命中 CEF 子进程则返回退出码。
    pub fn execute_subprocess() -> Option<i32>;
    pub fn initialize(config: RuntimeConfig) -> Result<Self, RuntimeError>;
    pub fn pump(&self);                 // do_message_loop_work
}

impl Drop for BrowserRuntime { /* cef::shutdown */ }
```

约定：

- `main()` 顺序：`execute_subprocess()` → 环境/日志 → eframe；任何线程池之前。
- 生产 `CefSandboxMode::Sandboxed`；`UnsandboxedTrustedContent` 只允许测试构建，并通过
  编译期/运行期断言禁止出现在 release。
- Linux 下根据**实际**窗口协议设置 `XDG_SESSION_TYPE` 与 Ozone 开关：
  原生 Wayland → `--ozone-platform=wayland`；X11/XWayland → `--ozone-platform=x11`。
  两者都不可用（无显示连接）→ 不启动 CEF，面板显示"无可用显示协议"。
- 初始化失败是一次性失败：不重试、不静默换后端；记录一次性诊断并让 Assistant 退回内置文本。

### 1.3 `WebSurface`（新）

职责：一个 CEF 浏览器实例的渲染、输入、布局、生命周期。

```rust
pub struct WebSurface { /* LinuxCefProducer + InputQueue + clipboard bridge */ }

pub struct SurfaceConfig {
    pub url: String,
    pub size: PhysicalSize<u32>,   // 物理像素
    pub scale: f32,
    pub visible: bool,
}

impl WebSurface {
    pub fn create(runtime: &BrowserRuntime, config: SurfaceConfig) -> Result<Self, SurfaceError>;
    pub fn pump(&mut self) -> Vec<SurfaceEvent>;      // 导航/标题/控制台/脚本结果/权限
    pub fn acquire(&mut self, host: &HostWgpuContext) -> Option<ImportedTexture>;
    pub fn acquire_popup(&mut self, host: &HostWgpuContext) -> Option<PopupSurface>;
    pub fn resize(&mut self, physical: PhysicalSize<u32>) -> Result<(), SurfaceError>;
    pub fn set_scale(&mut self, scale: f32);
    pub fn set_visible(&mut self, visible: bool);
    pub fn input(&mut self, event: InputEvent);       // 见 1.3.1
    pub fn close(self);
}
```

#### 1.3.1 输入与首帧队列

- 所有输入先入 `InputQueue`；首帧到达前只缓存，到达后按序回放。
- 指针移动合并（仅保留最后位置），键盘/IME/按键不得合并、不得丢弃。
- 队列上限（建议 256 个关键事件）；溢出计数并上报，不静默丢弃。
- 失焦、隐藏、关闭、初始化失败：清空队列。
- 剪贴板（因为 cef#4341）：
  - 复制/剪切：宿主读取活动输入框选区或 `window.getSelection()`，写系统剪贴板；
  - 粘贴：系统文本以 IME commit 注入焦点控件；
  - 图片粘贴明确不支持，给出可理解提示。
- 修饰键映射：`meta = mac_cmd only`（不得把非 macOS 的 `command` 当 Meta）。

### 1.4 `AssistantSurface`（集成层）

职责：把 `WebSurface` 嵌入现有 Assistant 面板，保持 IPC 与 UX。

- 绘制：`egui_wgpu::Renderer::register_native_texture` + 在面板矩形内 `painter.image`；
  每帧替换纹理并在替换时 `free_texture`。面板隐藏/遮挡时 `set_visible(false)`。
- 尺寸：使用 ui-egui 已有的 `web::placement` 矩形；矩形变化才 resize，避免每帧重排。
- 焦点：点击面板 → `move_focus(Forward)` + 宿主焦点状态；点击 CAD 区域 → 清队列并交还键盘。
- 事件：`pump()` 的导航事件继续走现有 `web::act` 通道；标题/控制台只用于诊断。
- 页面传输：复用现有 loopback HTTP（`page_address(true)`）；`window.ipc.postMessage`
  用一个 shim 接到 CEF message router（或 CDP evaluate），shim 注入在页面加载前完成。
- 能力检测：启动时探测 `VULKAN_EXTERNAL_MEMORY_DMA_BUF`；缺失则不创建 CEF 表面，
  面板显示与今天原生 Wayland 相同的诊断，并提供回退到 WRY/内置对话的入口。

## 2. 平台选择

| 平台 | 渲染 | 设备创建 | 运行时 | 输入法 |
|---|---|---|---|---|
| Linux Wayland | Vulkan + DMABUF（modifier 显式/隐式） | grafting `create_dmabuf_host_context` | CEF + `--ozone-platform=wayland` | 宿主 IME → CEF IME API |
| Linux X11/XWayland | Vulkan + DMABUF（implicit modifier） | 同上 | CEF + `--ozone-platform=x11` | 同上 |
| Windows | D3D12 shared texture | 普通 device | CEF + `libcef.dll`、沙箱 `bootstrap.exe` | TSF → CEF IME API |
| macOS | Metal / IOSurface | 普通 device | CEF framework + helper apps | NSTextInputClient → CEF IME API |

无显示连接（Linux headless）：不启动 CEF（`--ozone-platform=headless` 无加速帧）。

## 3. 失败与降级

| 失败 | 行为 |
|---|---|
| 无 Vulkan / 扩展缺失 | 面板诊断 + 回退 WRY（X11/Wayland 各自可用时）或内置对话；不上 CPU readback |
| 导入连续失败 N 次 | 关闭该表面、上报一次、回退；不无限重试 |
| 渲染进程崩溃 | 监听 `ContentProcessTerminated`，导航回首页 + `request_repaint`；OOM 不重试 |
| CEF 初始化失败 | 进程内一次性失败，回退内置对话 |
| 输入队列溢出 | 计数上报；不静默丢键盘/IME |

## 4. 回退（WRY）

- 保留 `assistant_web.rs` 与 WRY 依赖一个发布周期以上。
- 开关：偏好 `assistant.engine = "cef" | "wry" | "builtin"`，默认 `wry` 直到 Phase 3 达标。
- 迁移按组件落地（先 device + surface，再面板），任何一步失败可单独回退。

## 5. 验收门槛（与就绪报告一致）

1. 真实 CAD 并行：p95 回归 ≤10%（Wayland 与 X11 分开），无 GPU 阻塞/死锁。
2. 原生 Xorg + GPU：GPU OSR、输入、IME、剪贴板桥、popup 全部通过。
3. Windows（D3D12）与 macOS（Metal）各自 GPU 导入与生命周期通过。
4. 沙箱在三个平台均为默认，且无 `--no-sandbox` 回退。
5. 安装包体积与 CI 时间有预算并被测量。
