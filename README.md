# Chatterbox Desktop

Chatterbox Desktop 是 [Chatterbox](https://github.com/raphael-alex/chatterbox) 的桌面客户端，基于 **Tauri 2.x** + **Leptos 0.7** 构建。它通过本地 Python IPC 子进程与 Chatterbox 核心服务通信，提供与 Luna 的文本/语音交互体验。

## 项目架构

```
chatterbox-desktop/          # 本仓库
├── src/                     # Leptos 前端 (WASM)
├── src-tauri/               # Tauri Rust 后端
├── python_ipc_server.py     # Python IPC 子进程入口
└── ...

chatterbox/                  # Chatterbox 核心服务 (同级目录)
├── chatterbox/              # Python 包
├── config.yaml              # 配置文件
└── requirements.txt         # Python 依赖
```

> **目录要求**：`chatterbox-desktop` 与 `chatterbox` 必须在**同级目录**下，因为 `python_ipc_server.py` 通过相对路径导入 `chatterbox` 核心模块。

## 前置依赖

### 1. 系统工具

- [Rust](https://rustup.rs/) (最新稳定版)
- [Tauri CLI](https://v2.tauri.app/reference/cli/): `cargo install tauri-cli`
- [Trunk](https://trunkrs.dev/): `cargo install trunk`
- Python 3.10+ (chatterbox 核心使用了 Python 3.10+ 的 Union Type 语法)

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

配置 API Key：

```bash
# 复制配置文件并根据你的需求修改
cp config.yaml config.yaml
```

在 `config.yaml` 中设置对应的 API Key，或通过环境变量注入：

```bash
export OPENAI_API_KEY="your-openai-key"
export DEEPSEEK_API_KEY="your-deepseek-key"
```

> 详见 [chatterbox README](https://github.com/raphael-alex/chatterbox) 了解完整的配置说明。

### 3. 设置 Python 路径 (可选)

如果系统默认的 `python3` 版本低于 3.10，可以通过环境变量指定：

```bash
export CHATTERBOX_PYTHON="/usr/local/bin/python3"  # 或你的 pyenv/shims 路径
```

## 开发启动

```bash
cd chatterbox-desktop
cargo tauri dev
```

此命令会：

1. 启动 `trunk serve` 构建并服务前端 (<http://localhost:1420>)
2. 编译并启动 Tauri Rust 后端
3. 自动唤起桌面窗口
4. 在窗口加载完成后，Rust 后端会自动启动 `python_ipc_server.py` 子进程
5. Python IPC 进程会加载同级目录的 `chatterbox` 核心并初始化 Luna 会话

### 首次启动检查清单

- [ ] `chatterbox` 核心已克隆到同级目录
- [ ] `chatterbox/requirements.txt` 依赖已安装
- [ ] `chatterbox/config.yaml` 已正确配置（含 API Key）
- [ ] Python 版本 >= 3.10
- [ ] Rust + Tauri CLI + Trunk 已安装

## 构建生产包

```bash
cargo tauri build
```

构建产物位于 `src-tauri/target/release/bundle/`。

## 技术栈

| 层级     | 技术                                                     |
| -------- | -------------------------------------------------------- |
| 前端框架 | Leptos 0.7 (CSR)                                         |
| 前端构建 | Trunk                                                    |
| 桌面框架 | Tauri 2.x                                                |
| 后端语言 | Rust                                                     |
| IPC 进程 | Python 3.10+                                             |
| 核心服务 | [chatterbox](https://github.com/raphael-alex/chatterbox) |

## 常见问题

**Q: 启动后卡在 "Connecting to Luna..."**

A: 打开 DevTools (右键 → Inspect → Console) 查看前端日志，同时检查终端中 Rust/Python 的输出。常见原因：

- `chatterbox` 不在同级目录 → `No module named 'chatterbox'`
- Python 版本过低 → `unsupported operand type(s) for \|`
- 缺少 API Key → 配置校验失败
- `config.yaml` 路径错误 → Rust 会自动将子进程 CWD 设为 `chatterbox/` 目录

**Q: 如何切换 LLM 引擎？**

A: 修改 `chatterbox/config.yaml` 中的 `llm.engine` 字段（支持 `openai` / `deepseek`），无需重启桌面客户端，下次新建会话时生效。
