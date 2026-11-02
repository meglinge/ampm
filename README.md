# AMPM

用 Rust 编写的 Amp Runner 桌面管理器：Windows 后台常驻、系统托盘、居中弹出的独立 GUI。暖白与橙色的中文界面嵌入可执行文件，不需要 Node.js、前端构建工具或外部 CDN。

## 安装与更新（Windows x64）

在 PowerShell 中执行下面这一行，无需管理员权限。安装脚本来自本项目的 [GitHub Releases](https://github.com/meglinge/ampm/releases/latest)；请先确认你信任此仓库。

```powershell
& { $ErrorActionPreference='Stop'; $p=Join-Path $env:TEMP ('ampm-'+[guid]::NewGuid()+'.ps1'); iwr 'https://github.com/meglinge/ampm/releases/latest/download/install.ps1' -UseBasicParsing -OutFile $p; try { powershell.exe -NoProfile -ExecutionPolicy Bypass -File $p } finally { Remove-Item $p } }
```

- 安装到当前用户的 `%LOCALAPPDATA%\Programs\AMPM`，创建开始菜单入口与卸载项。缺少 WebView2 时自动安装微软运行时；不安装 Amp CLI、不修改开机自启。
- **设置 → 检查更新**：检查稳定版本，有新版时可“安装并重启”。任何 Runner 仍在运行时拒绝安装，不强制停止任务。
- **自动更新**默认开启：启动 30 秒后检查，之后每 6 小时检查；仅当窗口收起、没有打开的编辑表单、托管 Runner 全部停止时安装并重启。可在设置中关闭，选择保存在用户数据目录的 `updates.json`。
- 下载走 HTTPS，校验 SHA-256 和 Windows x64 文件头；失败保留原程序，配置与项目目录不变。SHA-256 防止下载损坏，**不等同于独立签名**；更新信任所配置的 GitHub 仓库。当前没有 Windows 代码签名证书，首次启动可能遇到 SmartScreen 提示。
- 可在 Windows“已安装的应用”卸载 AMPM；卸载保留 Runner 配置和项目文件。再次运行安装脚本前，需从托盘退出当前安装。

只有发布构建启用更新。自行编译时，需要先设置 `$env:AMPM_UPDATE_REPOSITORY='meglinge/ampm'`；Fork 请改为自己的公开仓库。未配置更新源和 `--headless` 模式不会启动后台更新。当前实现使用公开 Release，无需也不内置 GitHub Token；不支持私有仓库的用户分发。

### 发布维护

`.github/workflows/release.yml` 在推送 `v*` 标签时执行测试、编译 x64 程序，再生成安装脚本及校验文件。三个文件全部上传到草稿 Release 后才公开，防止用户获取不完整版本。发布构建会将当前仓库写入更新源。

发布时先提升 `Cargo.toml` 的版本并更新 `Cargo.lock`，提交后创建匹配的标签（如 `v0.1.1`）并推送。不要覆盖已发布标签或替换其文件；修复应发布新的递增版本。手动运行工作流也必须选择匹配的版本标签。

本地可用 `scripts\package.ps1 -Repository meglinge/ampm -Tag v0.1.1` 生成 `target\dist` 中的三个发布文件，不会上传；打包前须以相同更新源编译 Release。

## 运行

构建需要 Rust 1.90 或更新版本。使用需要已安装、已登录的 Amp CLI，以及 Microsoft Edge WebView2 Runtime（多数 Windows 10/11 已安装）。Windows 使用原生 `amp.exe`；也可在每个 Runner 中选择 WSL。

Windows 中默认的 `amp` / `amp.exe` 优先从 PATH 查找，找不到时使用官方安装目录 `%AMP_HOME%\bin`（未设置时为用户目录下的 `.amp\bin`）。检测、启动和目录命令使用相同规则，避免桌面启动时旧 PATH 导致找不到程序。手动指定的路径不会被替换；仍找不到时，可用 `where.exe amp` 查出完整路径后填入“编辑 → Amp 可执行文件”。WSL 中的 Linux 路径不受此规则影响。

```powershell
git clone https://github.com/meglinge/ampm.git
cd ampm
cargo build --release
.\target\release\ampm.exe
```

也可以直接双击 `ampm.exe`。Release 版本没有控制台黑窗口，不会打开浏览器；启动后只在系统托盘显示橙色图标（可能在任务栏的隐藏图标区域）。首次运行无预置 Runner，不会自动启动 Runner 或开放远程终端。

- **单击托盘图标**，或右键选择“打开管理器”：在当前鼠标所在屏幕的工作区中心打开独立 GUI。
- **关闭窗口**：仅隐藏到托盘，后台服务、日志采集与托管 Runner 继续运行。
- **托盘右键 → 退出 AMPM…**：确认后等待 API 操作结束，停止托管 Runner 并退出后台。取消退出不会中断 Runner。
- WebView 在首次打开界面时才创建；收起与恢复复用同一窗口，不重复启动服务或 Runner。
- 文档与“打开 Amp”链接在默认浏览器打开；管理界面本身始终留在独立窗口中。

如需启动后直接展示 GUI，使用 `--open`（不再表示打开浏览器）：

```powershell
.\target\release\ampm.exe --open
```

这是当前登录用户会话内的后台应用，不是 Windows SCM 系统服务。无需管理员权限，登录自启默认关闭，不保证注销后继续运行。

### 登录自启与 Runner 恢复

- **设置 → 启动与恢复 → 登录 Windows 后启动 AMPM**：写入当前用户的 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，登录后静默进入托盘，不弹出窗口。保留开启时的程序位置、数据目录和端口；移动程序后需重新开启。一个用户只有一个 AMPM 自启入口，开启会改为当前实例。卸载仅移除指向本次安装的入口。
- **Runner → 编辑 → 随 AMPM 启动**：仅自动启动勾选的 Runner；每次启动 AMPM 生效，包括手动打开、登录自启和 AMPM 更新后的重新启动。沿用该 Runner 的目录、远程终端和共享权限，默认关闭。
- **异常退出自动重启次数**：每个 Runner 独立设置 0–20 次，0 为关闭。非零退出或启动失败时，等待 5 秒后重试；自动重启成功不会清零计数，达到上限后停止。手动启动、手动重启或 AMPM 重新启动会重置计数；退出码 0 不触发重试。
- **停止**（包括批量停止、退出 AMPM）会取消待重启；正常关闭窗口仅收起，不取消恢复。详情页显示“待重启”和已用次数，日志保留各次退出与重试信息。待重启也视为托管中，修改启动配置、删除或安装更新前需先停止。
- 不提供 AMPM 自身崩溃后的自动恢复，也不是未登录时运行的系统服务。WSL 的恢复依据 `wsl.exe` 的退出码，无法保证检测 WSL 内部残留或脱离托管的进程。

默认配置保存在系统用户的本地数据目录（Windows 为 `%LOCALAPPDATA%\ampm\data\config.json`，以“设置”中的实际路径为准）。可覆盖数据目录与端口：

```powershell
$env:AMPM_PORT = '19381'
$env:AMPM_DATA_DIR = 'D:\MyData\ampm'
.\target\release\ampm.exe --open
```

同一数据目录只允许一个管理器实例运行。配置使用临时文件原子替换保存；遇到损坏或未知版本的配置会报错退出，绝不静默覆盖。WebView2 的缓存存放在同一数据目录下的 `webview` 子目录，不使用个人浏览器配置。

开发和自动化测试可用 `cargo run -- --headless` 仅启动后台 API，不创建托盘或 GUI，通过 `http://127.0.0.1:19380` 访问、Ctrl+C 退出。非 Windows 平台目前仅提供此无界面模式。默认桌面模式内部也使用回环 API，但无需用户手动打开网址。

## 功能

- 固定侧栏、Runner 列表与详情分栏；勾选用于批量操作，点击名称切换详情。
- Runner 新增、改名、编辑、删除；按名称、ID、目录搜索和状态筛选。
- 单个或批量启动、停止、重启，独立报告每项成功或失败。
- 文件夹集中视图、本机磁盘目录浏览器、批量添加目录、解除关联。
- 每个 Runner 只有一份平等的目录列表，不区分主目录或附加目录；每项使用相同的管理操作。
- 稳定 Runner ID、重复 `--dir`、`--discover-dirs`、远程终端与工作区共享（`--share`）开关。
- 每个 Runner 独立设置 Amp 可执行路径，以及原生或 WSL 启动方式。
- 标准输出、错误输出与管理器日志，保留最近 300 条，单行上限约 8 KiB。
- Amp CLI 当前版本、稳定版检查与手动更新，支持本机默认和各 Runner 的原生 / WSL 环境。
- `amp runner list` 原始输出的只读发现。
- `N` 新建，`/` 搜索；窄屏布局与键盘可操作的对话框。

在“设置 → Amp CLI”选择安装环境，点击“检查更新”，有新版时点击“更新 Amp”。检查只读取版本与官方稳定版信息；安装调用所选 CLI 的 `amp update --porcelain`，随后重新读取版本确认结果，不修改 Runner 配置或自动重启。更新前须停止所有托管 Runner，并自行结束其他 Amp 会话；更新期间禁止启动 Runner、修改配置及同时更新 AMPM。旧版 CLI 不支持更新或权限不足时会显示命令错误，不回退到其他安装路径。此功能不会替未安装的环境安装 Amp，也不改变 Amp 自身的自动更新设置。

## Amp 的边界

启动实际调用 `amp --no-tui --runner-id …`。进程存在不代表已连接 Amp 云端；需要在日志和 Amp 网页中核对登录、连接与目录。

目录列表至少保留一项。Amp CLI 必须有进程启动位置，AMPM 内部使用列表第一项，其余项通过 `--dir` 传入；这不是目录优先级，也不限制首项在停止后被编辑或移除。自动发现扫描第一项下两级 Git 项目；调整首项会改变扫描范围及 Amp 动态目录记忆的上下文。

旧版配置的 `cwd` 和 `directories` 会按原顺序合并，读取时不改写文件。下一次保存配置时写入版本 2，仅存储统一的 `directories`；旧版 AMPM 不支持读取版本 2。

| 目录来源 | 管理方式 |
| --- | --- |
| 启动时已配置的目录 | 所有项使用相同规则：移除需先停止 Runner |
| 运行中新增目录 | 调用 `amp runner dirs add PATH --runner-id ID`，可在本次运行中调用 `remove` 移除 |
| 自动发现、Amp 已记忆的目录 | 详情页用 `amp runner dirs list --runner-id ID` 读取实际输出 |

**配置列表不是权限白名单，也不是文件系统沙箱。** Amp 会记忆动态新增目录，且记忆与启动工作目录有关。从 AMPM 的已停止配置中删掉路径，不会修改 Amp 的历史目录存储；重启时历史目录仍可能存在。需要撤销历史目录时，按官方文档在运行中的 Runner 上执行 `amp runner dirs remove PATH --runner-id ID`，再读取实际目录核对。自动发现的目录应关闭发现后重启。此工具从不删除或移动磁盘文件，也不直接编辑 Amp 的内部状态文件。

`amp runner list` 在本机验证版本中可用，但较老版本可能没有该子命令，且没有已验证的 JSON 输出格式。因此“本机发现”保留人类可读输出，**不接管、不停止外部 Runner**。

Amp 的自动更新可能替换自身进程。AMPM 不接管替换后的进程；是否重试取决于退出码和该 Runner 的异常重启次数，退出码 0 不重启。原生 Windows 托管进程树会随原进程退出一起清理。需要稳定托管时，可自行在 Amp 设置中关闭 `amp.runner.autoUpdate.enabled`；AMPM 不覆盖用户的 Amp 配置。

### WSL

填写 Linux 绝对路径，例如 `/home/me/projects`，不要填写 `D:\Projects`。发行版可留空使用默认值；可执行路径建议填写 `/home/me/.amp/bin/amp`。目标发行版需要 `setsid`、`sh`、`kill`，并在该环境独立完成 `amp login`。

启动使用固定 shell 脚本和独立参数，不把路径拼入脚本。停止只针对管理器启动的 Linux 进程组，绝不执行 `wsl --terminate`。**强制结束管理器时，WSL 进程可能遗留**，需要手动检查；WSL 并不等同于 Windows Job Object 托管。

## 安全

- 只绑定 `127.0.0.1`，没有局域网访问开关。
- 检查 Host 和 Origin，写接口使用随机会话令牌，防止跨站请求与 DNS rebinding。
- 前端无第三方脚本，启用 CSP；用户名称、目录和日志按文本转义。
- 原生进程通过参数数组启动；Windows 使用 Job Object 关闭托管进程树，Unix 使用独立进程组。
- 远程终端默认显式关闭，不受继承的 `AMP_REMOTE_CONTROL_TERMINAL=1` 影响；启用和启动前有风险提示。
- 工作区共享默认关闭；开启后启动时附加 `--share`，工作区成员可在 ampcode.com 的 Shared Runners 中使用该 Runner。成员会以你的身份在本机运行代码，使用你的文件、凭据和登录状态，并可能同时操作相同目录，仅共享给你信任其拥有本机 Shell 的人。工作区管理员可在 Member Settings 中禁用共享。
- 这是给可信本机用户使用的管理工具，不是多用户认证系统。Runner 继承当前用户的文件、凭据、网络与工具权限；停止/重启可能中断任务。

## 开发与验证

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
node --check web\app.js
powershell.exe -NoProfile -File tests\install.Tests.ps1
```

Rust 测试覆盖 ID/路径校验、参数边界、配置持久化及损坏保护、跨站请求保护、目录浏览、日志上限，以及真实子进程的启停、重复启动保护、批量部分失败和动态目录失败不写入配置。

测试还覆盖无 GUI 轮询时的后台进程监控、服务退出时停止子进程和释放端口，以及 WebView 外链协议与域名限制。

更新测试覆盖稳定版本比较、文件校验、限长下载及截断响应、更新互斥与安装条件，以及临时 Windows 进程的真实 EXE 替换。安装脚本测试在临时目录执行首次安装、重装、损坏下载、文件占用、卸载；网络、注册表、快捷方式和 WebView2 安装使用替身，不修改真实安装。

Amp CLI 更新测试覆盖构建时间戳比较、只读检查、所选路径与 WSL 参数、更新后版本核对、命令失败、版本未变、避免降级及更新期间的状态查询和操作互斥。

`tests\fixtures\fake_amp.rs` 是不联网的 CLI 测试替身，测试会用 `rustc` 编译到临时目录，不会启动真实 Amp Runner 或使用账号额度。GUI 验证同样应使用独立的 `AMPM_DATA_DIR` 和这个替身，避免对用户现有 Runner 产生影响。测试时可在启动前设置 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9224`，通过 CDP 检查真实 WebView2；产品默认不启用调试端口。

参考：[One runner is now enough](https://ampcode.com/news/one-runner-is-now-enough) · [Shared runners](https://ampcode.com/news/shared-runners) · [Runner 文档](https://ampcode.com/docs/cli/runners)
