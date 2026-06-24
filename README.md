# Chatterbox Desktop

Chatterbox Desktop 是 [Chatterbox](https://github.com/raphael-alex/chatterbox) 的桌面客户端，基于 **Tauri 2.x** + **Dioxus 0.6** 构建。它通过本地 Python IPC 子进程与 Chatterbox 核心服务通信，提供与 Luna 的文本/语音交互体验。

## 项目架构

```
chatterbox-desktop/          # 本仓库
├── src/                     # Dioxus 前端 (WASM)
├── src-tauri/               # Tauri Rust 后端
├── python_ipc_server.py     # Python IPC 子进程入口
├── .env                     # API Key 配置 (不提交到 git)
└── ...

chatterbox/                  # Chatterbox 核心服务 (同级目录)
├── chatterbox/              # Python 包
├── config.yaml              # 配置文件
└── requirements.txt         # Python 依赖
```

> **目录要求**：开发模式下 `chatterbox-desktop` 与 `chatterbox` 必须在**同级目录**下。生产构建时 chatterbox 会被打包进应用资源。

## 前置依赖

### 1. 系统工具

- [Rust](https://rustup.rs/) (最新稳定版)
- [Tauri CLI](https://v2.tauri.app/reference/cli/): `cargo install tauri-cli`
- [Trunk](https://trunkrs.dev/): `cargo install trunk`
- Python 3.10+ (chatterbox 核心使用了 Python 3.10+ 的 Union Type 语法)
- ffmpeg (语音模式需要): `brew install ffmpeg`

### 2. Chatterbox 核心服务

克隆核心项目到**同级目录**：

```bash
cd ..
git clone https://github.com/raphael-alex/chatterbox.git
cd chatterbox
```

安装 Python 依赖：

```bash
pip install -r requirements.txt
```

### 3. 配置 API Key

在项目根目录创建 `.env` 文件：

```bash
cp .env.example .env  # 如果有示例文件
# 或手动创建
```

编辑 `.env` 填入你的 API Key：

```
DEEPSEEK_API_KEY=your-deepseek-key
OPENAI_API_KEY=your-openai-key
```

> **安全提示**：`.env` 文件包含敏感信息，已加入 `.gitignore`，不要提交到代码仓库。

也可通过环境变量注入（优先级低于 `.env`）：

```bash
export DEEPSEEK_API_KEY="your-deepseek-key"
```

## 开发启动

```bash
cd chatterbox-desktop
cargo tauri dev
```

此命令会：

1. 启动 `trunk serve` 构建并服务前端 (localhost:1420)
2. 编译并启动 Tauri Rust 后端
3. 自动唤起桌面窗口
4. Rust 后端自动启动 `python_ipc_server.py` 子进程
5. Python IPC 进程加载 chatterbox 核心并初始化 Luna 会话

## 构建生产包

```bash
cargo tauri build
```

构建产物位于 `target/release/bundle/`。

## 技术栈

| 层级     | 技术                                                     |
| -------- | -------------------------------------------------------- |
| 前端框架 | Dioxus 0.6 (WASM in Tauri webview)                      |
| 前端构建 | Trunk                                                    |
| 桌面框架 | Tauri 2.x                                                |
| 后端语言 | Rust + Tokio (异步 IPC)                                   |
| IPC 进程 | Python 3.10+ (stdin/stdout JSON-RPC)                     |
| 核心服务 | [chatterbox](https://github.com/raphael-alex/chatterbox) |

## 常见问题

**Q: 启动后卡在 "Connecting to Luna..."**

A: 打开 DevTools (右键 → Inspect → Console) 查看前端日志，同时检查终端中 Rust/Python 的输出。常见原因：

- `chatterbox` 不在同级目录 (开发模式) → `No module named 'chatterbox'`
- Python 版本过低 → `unsupported operand type(s) for |`
- 缺少 API Key → 配置校验失败
- ffmpeg 未安装 → 语音模式不可用

**Q: 如何切换 LLM 引擎？**

A: 修改 `chatterbox/config.yaml` 中的 `llm.engine` 字段（支持 `openai` / `deepseek`），无需重启桌面客户端，下次新建会话时生效。

**Q: API Key 如何配置？**

A: 在项目根目录创建 `.env` 文件，填入 `DEEPSEEK_API_KEY` 或 `OPENAI_API_KEY`。应用启动时会自动读取。
