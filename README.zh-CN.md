<h1 align="center">
  <img src="assets/app-icon/png/128.png" alt="yttt 应用图标" width="64" valign="middle"> yttt
</h1>

<p align="center">macOS · Windows · Linux</p>

<p align="center"><sub><a href="README.zh-CN.md">中文</a> · <a href="README.md">English</a></sub></p>

<p align="center">
  <strong>项目优先、终端优先的 Agent 工作台。</strong><br>
  在一个桌面窗口中管理项目、终端、文件与 Agent 会话。
</p>

<h3 align="center"><a href="https://github.com/yuWorm/yttt/releases">下载 yttt</a></h3>

<p align="center">
  <img src="docs/images/readme-workbench.png" alt="yttt 最大化工作台，显示五个演示项目、Agent 状态和分屏终端" width="960">
</p>
<p align="center"><sub>最大化窗口截图；项目与 Agent 状态为模拟数据，终端文字为合成演示内容，未运行真实 Agent CLI。</sub></p>

## 核心功能

<table>
<tr>
<td width="50%" valign="top">
<h3>多项目与 Agent</h3>
<p>在同一窗口切换项目；每个项目保留自己的标签、布局和 Agent 状态。</p>
<a href="docs/usage.md#product-model">使用文档 →</a>
</td>
<td width="50%" valign="top">
<h3>可分屏的终端</h3>
<p>在标签页内拆分终端，把多个 CLI 工作流放在同一视图。</p>
<a href="docs/usage.md#first-launch">使用文档 →</a>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<h3>项目文件与编辑器</h3>
<p>浏览项目文件、编辑文本、预览图片；终端与文件共用标签栏。</p>
<a href="docs/usage.md#project-files-and-editor">使用文档 →</a>
</td>
<td width="50%" valign="top">
<h3>本地与远端工作区</h3>
<p>连接现有桌面 Host，或通过 SSH 打开远端项目；工作区资源由所属 Host 管理。</p>
<a href="docs/usage.md#connect-to-an-existing-desktop-host">桌面 Host →</a> · <a href="docs/usage.md#ssh-projects">SSH →</a>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<h3>键位与界面</h3>
<p>配置快捷键、全局 Vim、主题和窗口栏；设备偏好与 Host 配置分开。</p>
<a href="docs/usage.md#keybindings">快捷键 →</a> · <a href="docs/usage.md#configuration-targets">配置 →</a>
</td>
<td width="50%" valign="top">
<h3>桌面命令行</h3>
<p>使用 <code>yttt ctl</code> 查看项目与 Agent 状态，管理标签、窗格和终端输入。</p>
<a href="docs/cli-control.md">CLI 文档 →</a>
</td>
</tr>
</table>

## Agent 工作流

你可以在终端窗格中使用已有的 CLI Agent。yttt 可在项目侧栏显示受支持的 Agent 生命周期状态；
普通终端命令不等同于已接入状态检测的 Agent。详情见
[Agent 状态说明](docs/usage.md#agent-status)。

## 下载与安装

从 [GitHub Releases](https://github.com/yuWorm/yttt/releases) 下载对应平台的桌面安装包：

- **macOS Apple Silicon：**下载 `.dmg`，将 yttt 拖入“应用程序”。
- **Windows x86_64：**运行 `.exe` 安装程序。
- **Linux x86_64：**解压 `.tar.gz`，运行其中的 `bin/yttt`。

请核对随安装包提供的 SHA-256 校验和。macOS 安装包采用 ad-hoc 签名，未经过 Developer ID 公证。
桌面端、Host 与远端 Server 需要使用匹配的版本。

<details>
<summary>从旧版 1.0.0 升级</summary>

项目在发布 v1.0.0 后回到了 pre-1.0 版本路线。请手动下载安装后续 pre-1.0 版本；
按 SemVer 比较的更新检查不会将其视为从 1.0.0 升级。

</details>

## 开始使用

启动应用后选择语言和终端字体，再打开本地项目。在标签页中添加终端或 Agent、拆分窗格，
并从右侧文件树打开项目文件。远端连接与会话恢复的步骤见[完整使用文档](docs/usage.md)。

要通过脚本操作已启动的桌面，请从 `yttt ctl --help` 或
[CLI 示例](docs/cli-control.md)开始。

## 文档

- [使用指南](docs/usage.md) — 界面、远端连接、会话恢复和配置。
- [CLI 控制](docs/cli-control.md) — 桌面命令与示例。
- [Host/Client 架构](docs/host-client-architecture.md) — 进程与资源归属。
- [双语更新日志](CHANGELOG.md) — 版本变更与升级说明。

## 开发

使用 Rust 从源码构建并启动：

```bash
cargo run
cargo run -- /path/to/project
```

在 macOS 上，`scripts/run-dev-app.sh` 可为 GPUI 界面检查创建开发版 `.app`。
运行 `scripts/run-dev-app.sh --fixture readme` 可以打开与上图对应的五个**模拟项目**；
此场景不会启动真实 Agent CLI。

<details>
<summary>发布维护</summary>

在对应原生平台上运行 `scripts/build-macos-dmg.sh`、
`scripts/build-windows-installer.ps1` 或 `scripts/build-linux-tar.sh` 构建安装包。
发布前运行 `python3 scripts/prepare_release.py <version>`；修复已有版本章节时使用
`--repair-current`，然后运行 `python3 scripts/test_release_tools.py`。
创建标签前检查清单、锁文件、更新日志与 README，并等待发布提交上的必需验证通过。
标签工作流会再次验证该标签对应的提交，再构建并发布安装包、Server 程序、更新元数据与校验和。

</details>
