
---

## 优化后的架构提案

### 1. 命令设计（保持不变，但增强选项）

```bash
fb-gen install                     # 交互式：检测缺失 → 推荐安装
fb-gen install toolchain           # 仅工具链
fb-gen install toolchain --arch xtensa
fb-gen install sdk --mcu stm32f1
fb-gen install middleware --name FreeRTOS
fb-gen install --list              # 列出可安装的包（支持筛选：--type toolchain/sdk/middleware）
fb-gen install --dry-run           # 仅检查，不实际安装
fb-gen install --upgrade <pkg>     # 升级已安装的包
fb-gen install --uninstall <pkg>   # 卸载
fb-gen install --list-installed    # 查看已安装内容及其版本/路径
```

### 2. 文件结构（职责更细分）

```
src/
  install/
    mod.rs                    # InstallManager: 对外接口，聚合各子模块
    catalogue/
      mod.rs                  # 包目录入口（支持本地硬编码 + 远程动态合并）
      embedded.rs             # 内置硬编码包列表（fallback）
      remote.rs               # 远程 manifest 拉取与缓存（优先使用）
      schema.rs               # 包描述结构体定义（统一 Toolchain/Sdk/Middleware）
    downloader/
      mod.rs                  # HTTP 客户端（支持重试、断点续传、进度条）
      checksum.rs             # SHA256 / SHA512 校验
      extractor.rs            # 解压（tar.gz/zip/7z），支持符号链接保留
    installer/
      mod.rs                  # 通用安装流程（下载 → 校验 → 解压 → 后处理）
      toolchain.rs            # 工具链专用后处理：PATH 配置、符号链接、ldconfig
      sdk.rs                  # SDK 安装：全局缓存或项目本地（支持 --scope）
      middleware.rs           # 中间件：处理依赖 SDK 的自动安装
    environment/
      mod.rs                  # 环境变量管理（shell profile, 用户级 env file）
      path_manager.rs         # PATH 追加/移除，处理 Windows 注册表（可选）
      version_manager.rs      # 多版本共存：`fb-gen use toolchain xtensa-1.2.3`
    resolver/
      mod.rs                  # 依赖解析器（检测冲突、缺失、推荐安装）
      scanner.rs              # 复用现有 scanner 检测已安装内容
    config.rs                 # 用户配置（安装根目录、镜像源、代理、并行下载数等）
```

### 3. 包目录设计（可扩展 + 远程 fallback）

#### 3.1 统一包描述（使用枚举区分类型，便于通用处理）

```rust
pub struct Package {
    pub id: String,                 // "xtensa-esp32-elf-gcc-8.4.0"
    pub name: String,               // 显示名
    pub kind: PackageKind,          // Toolchain / Sdk / Middleware
    pub version: String,
    pub targets: Vec<TargetTriple>, // 支持的宿主平台（x86_64-unknown-linux-gnu 等）
    pub downloads: PlatformUrls,
    pub sha256: String,
    pub verify: Option<VerifySpec>, // 例如 { command: "gcc --version", expect: "xtensa" }
    pub dependencies: Vec<String>,  // 依赖的包 ID
    pub conflicts: Vec<String>,     // 冲突的包 ID
    pub scope: InstallScope,        // Global / LocalProject
}

pub struct PlatformUrls {
    pub linux: Option<String>,
    pub macos: Option<String>,
    pub windows: Option<String>,
}
```

#### 3.2 混合目录源（优先远程，降级硬编码）

- 离线或网络错误时使用内置 `embedded.rs` 中的硬编码包列表（保证基础可用）。
- 支持用户自定义镜像源（`fb-gen config set manifest.url ...`）。

### 4. 安装流程（增强）

```
fb-gen install toolchain --arch xtensa
      │
      ▼
1. Resolve: 从 catalogue 匹配最新稳定版本（支持 `--version` 指定）
      │
      ▼
2. Dependency check: 递归检查依赖包（如 SDK 依赖工具链）
      │
      ▼
3. Dry-run: 若 --dry-run，输出计划并退出
      │
      ▼
4. Download: 异步并行下载（限制并发数），支持断点续传，显示进度条
      │
      ▼
5. Verify: SHA256 + 可选 GPG 签名（敏感包）
      │
      ▼
6. Extract: 原子写入（先解压到临时目录，再 mv 到最终路径）
      │  路径：~/.fb-gen/toolchains/xtensa-esp32-elf/8.4.0/
      │  支持 --prefix 自定义根目录
      │
7. Configure: 写入环境配置（~/.fb-gen/env.d/xtensa.sh）
      │  并在 ~/.fb-gen/current/ 中设置软链接指向最新版本
      │
8. Verify: 执行 verify 命令确认可用性
      │
9. Record: 记录安装清单到 ~/.fb-gen/installed.json（含安装时间、版本、文件列表）
      │
10. Post-install hook: 可选提示用户执行 `source ~/.fb-gen/env` 或重启 shell
```

### 5. 阶段规划（优先级调整）

| 阶段 | 内容 | 包数量 | 核心关注点 |
|------|------|--------|-------------|
| **Phase 1** | 核心框架：下载、校验、解压、环境配置、多版本共存 | — | 基础设施稳固，跨平台测试（Windows/macOS/Linux） |
| **Phase 2** | L1 工具链（5 架构 × 3 平台） + 远程 manifest 服务 | ~15 | 实现 catalogue 远程拉取，验证端到端流程 |
| **Phase 3** | L2 MCU SDK（STM32、nRF、ESP 等） | ~25 | 依赖解析（SDK 可能依赖特定工具链版本），scope 支持 |
| **Phase 4** | L3 中间件（FreeRTOS、lwIP、mbedTLS 等） | ~10 | 处理复杂依赖（中间件需要 SDK 头文件路径注入） |
| **Phase 5** | 高级特性：升级、卸载、回滚、并行安装、交互式 TUI | — | 完善用户体验 |

### 6. 关键优化点说明

#### 6.1 多版本共存与版本切换

- 每个包安装到版本隔离目录：`~/.fb-gen/toolchains/xtensa/1.2.3/`
- 使用 `current` 软链接指向活跃版本：`~/.fb-gen/toolchains/xtensa/current/`
- 用户可通过 `fb-gen use toolchain xtensa@1.2.3` 切换
- 环境变量文件中写入 `PATH=~/.fb-gen/toolchains/xtensa/current/bin:$PATH`

#### 6.2 依赖解析与冲突检测

- 引入简单 SAT 求解器（如 `rust-sat`）或拓扑排序处理依赖树
- 检测冲突：比如两个工具链可能同时修改 `CC` 环境变量，提示用户选择
- 依赖缺失时自动推荐安装（交互式确认）

#### 6.3 远程包目录（避免二进制膨胀）

- 硬编码包列表仅包含 5~10 个最常用工具链作为离线兜底
- 正式使用时从远程 JSON 获取完整包信息，支持动态新增包而不需重新编译 fb-gen
- JSON schema 版本化，确保向后兼容

#### 6.4 进度与用户体验

- 使用 `indicatif` 库显示下载进度条、解压 spinner
- 彩色输出（`colored`）：成功绿色，警告黄色，错误红色
- 交互式提示（`dialoguer`）：当检测到多个候选版本时让用户选择

#### 6.5 跨平台细节

- Windows 上使用 `%USERPROFILE%\.fb-gen\`，PATH 通过 `setx` 或注册表修改（需管理员？可提示）
- Windows 上解压 `.zip` 为主，`.tar.gz` 需额外依赖（内置 `tar` 或 `libarchive`）
- macOS 上注意 ARM64 与 x86_64 工具链区分

#### 6.6 安全性增强

- 所有下载必须校验 SHA256（清单中提供）
- 可选 GPG 签名验证（对官方工具链，`verify` 字段增加 `signature_url`）
- 安装前检查文件是否已被修改（防覆盖）

#### 6.7 配置可扩展

支持用户配置文件 `~/.config/fb-gen/config.toml`：

```toml
[install]
root = "/opt/fb-gen"                # 默认 ~/.fb-gen
parallel_downloads = 4
proxy = "http://proxy.example.com:8080"

[manifest]
url = "https://mirror.fb-gen.com/manifest.json"
cache_ttl_hours = 48

[env]
auto_source = true                  # 自动在 .bashrc/.zshrc 添加 source 行
```
安装的包文件夹使用cmake连接进项目
### 7. 测试策略

- 单元测试：每个子模块（downloader、checksum、extractor、resolver）
- 集成测试：模拟 HTTP 服务器，安装假包到临时目录，验证环境文件生成
- 跨平台测试：在 GitHub Actions 上跑 Linux/macOS/Windows 三平台
- 端到端测试：真实下载一个小型工具链（如 RISC-V 工具链），验证 gcc 能运行

---

