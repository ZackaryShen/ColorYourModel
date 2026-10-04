<p align="center">
  <img src="assets/icon.png" alt="ColorYourModel 标志" width="128">
</p>

# ColorYourModel

[English](README.md) | [简体中文](README.zh-CN.md)

> 把 3D 打印白模 STL 变成基于分区的彩色 3MF，直接多耗材彩色打印。

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)
[![Release](https://img.shields.io/badge/release-v0.1.1-blue)](https://github.com/ZackaryShen/ColorYourModel/releases/tag/v0.1.1)
![Built with Tauri 2](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)
![React 18](https://img.shields.io/badge/React-18-61DAFB?logo=react&logoColor=black)

**ColorYourModel（CYM）** 是一款桌面应用（Tauri 2 + React + Three.js 前端，Rust 后端），解决一个实际的 3D 打印问题：白模 STL 只是一堆无结构的三角面片——没有"分区"概念，也没有颜色。高细节模型在切片软件里逐面片涂色基本不可行。

CYM 自动把网格分割成语义化区域（头盔、皮肤、底座……），提供分区感知的上色工具做微调，最终导出符合规范的**带逐分区颜色的 3MF**，在 Snapmaker Orca / OrcaSlicer 中导入即为真实的耗材分色——已在真机端到端验证（2026-08，Snapmaker U1）。

```mermaid
flowchart TD
    A["STL 白模导入"] --> B["分割 —— 种子面板<br/>自动融合（平面 / 多视角 / 二面角 / 眼睛四通道投票）<br/>或手动种子生长 · 眼睛检测"]
    B --> C["微调<br/>合并 / 拆分 / 重命名 / 重分割"]
    D --> E["上色<br/>画笔 · 喷漆 · 智能笔 · 填充 · 橡皮 · 吸管<br/>统一撤销/重做"]
    E --> F["导出<br/>3MF（颜色 + 机型预设）· OBJ（逐面颜色）"]
    F --> G["Snapmaker Orca / OrcaSlicer<br/>切片打印"]
```

> **第一次用？** 先看**[使用手册](docs/bak/user-guide/manual.md)**（[English](docs/bak/user-guide/manual.en.md)）——每个按钮、每个快捷键、相机技巧、分区工作流、导出步骤和当前局限，附截图，零基础友好。

## 截图

| | |
|---|---|
| <img src="samples/sanji-diorama/example02.png" alt="187 万面手办分割为 37 个区域" width="480"> | <img src="samples/liangsheng/example01.png" alt="胸像上色为 11 个区域" width="480"> |
| 187 万面手办 → 37 区，可逐部件上色 | 上色胸像——3MF 端到端验证用的就是这个模型 |

十三个真实案例——手办、AI 生成模型、招牌、建筑件——见[**示例图库**](docs/bak/cases/examples.md)。

## 下载安装

到 [**Releases**](https://github.com/ZackaryShen/ColorYourModel/releases) 页下载即用安装包——当前版本 [v0.1.1](https://github.com/ZackaryShen/ColorYourModel/releases/tag/v0.1.1)。

| 平台 | 安装包 |
|------|--------|
| Windows | [`x64-setup.exe`](https://github.com/ZackaryShen/ColorYourModel/releases/download/v0.1.1/ColorYourModel_0.1.1_x64-setup.exe)（推荐）· [`x64_en-US.msi`](https://github.com/ZackaryShen/ColorYourModel/releases/download/v0.1.1/ColorYourModel_0.1.1_x64_en-US.msi) |
| Linux | [`amd64.deb`](https://github.com/ZackaryShen/ColorYourModel/releases/download/v0.1.1/ColorYourModel_0.1.1_amd64.deb) · [`x86_64.rpm`](https://github.com/ZackaryShen/ColorYourModel/releases/download/v0.1.1/ColorYourModel-0.1.1-1.x86_64.rpm) · [`AppImage`](https://github.com/ZackaryShen/ColorYourModel/releases/download/v0.1.1/ColorYourModel_0.1.1_amd64.AppImage)（自带 WebKit，约 80 MB） |
| macOS | [`universal.dmg`](https://github.com/ZackaryShen/ColorYourModel/releases/download/v0.1.1/ColorYourModel_0.1.1_universal.dmg)（Apple Silicon + Intel，未签名） |

> macOS 包未签名（无 Apple Developer 账号），首启被 Gatekeeper 拦截时执行 `xattr -cr /Applications/ColorYourModel.app`。界面语言默认跟随系统（中文环境中文、其他环境英文），可随时在 视图 → 语言 切换。源码构建见[快速开始](#快速开始)。

## 演示视频

| 演示 | 内容 |
|------|------|
| [分区与导出](https://github.com/ZackaryShen/ColorYourModel/releases/download/media/partition-and-export-demo.mp4) | 种子融合分割 → 按区 3MF 导出全流程 |
| [绘制工具](https://github.com/ZackaryShen/ColorYourModel/releases/download/media/painting-tools-demo.mp4) | 画笔 / 喷涂 / 填充等上色工具 |
| [3MF 成果呈现](https://github.com/ZackaryShen/ColorYourModel/releases/download/media/3mf-showcase-demo.mp4) | 导出的多色 3MF 载入 Snapmaker Orca，分区映射为耗材分配 |

> 视频以 [Release 资产](https://github.com/ZackaryShen/ColorYourModel/releases/tag/media)方式托管——点击即播放，不会增大 git clone 体积。

## 功能特性

### 当前可用 ✅

**导入与分割**

- **STL 导入**——二进制 + ASCII，1.5M 面级模型约 4 秒加载，带进度事件
- **分割**——种子面板一键**融合**（平面 / 多视角 / 二面角 / 眼睛四通道投票）或手动种子生长；8 种可互换算法收于同一后端接口，经分区面板的「重分割」按区域选用。导入**不做**自动分割——分区生成是显式步骤
- **种子分区**——推荐种子（平面 / 多视角 / 截面 / 显著性）、逐点手动生长、自动融合 + 小分区合并
- **眼睛区域检测**——手办模型一键识别，全局检测（无需 ROI）
- **分区管理**——合并 / 拆分 / 重命名分区，单区域重分割

**上色**

- **上色工具**——画笔、喷漆、智能笔（分区边界感知）、填充、橡皮、吸管
- **标签权威填充**——填充严格作用于点击面所属分区，两个分区可安全共用同一颜色（有回归测试保护）
- **统一撤销 / 重做**——一条时间线覆盖画笔、填充、橡皮与手动编辑

**导出**

- **3MF 导出**——已端到端验证（2026-08）：多耗材颜色正确导入 Snapmaker Orca；内嵌机型预设让切片器开箱即显示 *Snapmaker U1 (0.4 nozzle)*。导出对话框：机型 → 喷嘴 → 工艺 → 耗材槽位 → 目标切片器，支持多分区选择 + 一键套用
- **OBJ 导出**——经 MTL 材质分组实现逐面颜色，最多量化到 256 色

**视口与交互**

- **分区视图**——分区彩色可视化 + 边界描边；hover 高亮为 GPU 着色器方案（逐顶点标签属性，O(1) 切换，按需渲染）
- **BVH 面拾取**——`three-mesh-bvh` 加速的 CPU 射线求交，1.5M 面模型稳定命中
- **交互细节**——3D 画笔光标环、Space 平移、进度条、崩溃诊断（JS 错误桥接）
- **i18n**——语言默认跟随系统（中文环境中文、其他环境英文），选择持久化

### 计划中 🗓

- [ ] 调色板预设 + 颜色历史
- [ ] 同类模型批量处理
- [ ] OrcaSlicer 插件形态集成

## 技术栈

| 层级 | 技术 | 说明 |
|------|------|------|
| 前端 | React 18 + TypeScript | 声明式 UI |
| 3D | Three.js + @react-three/fiber + three-mesh-bvh | 渲染 + 加速拾取 |
| 状态 | Zustand（persist） | 应用状态 + 偏好持久化 |
| 桌面 | Tauri 2 | 原生窗口 + Rust 后端 |
| 后端 | Rust | 网格 I/O、分割、上色、导出 |
| Rust 依赖 | nalgebra · parry3d · petgraph · kiddo · stl_io · quick-xml · zip | 几何、图、空间索引、解析 |
| 构建/测试 | Vite 6 · Vitest + Testing Library · cargo test | `tsc --noEmit` / `vitest run` / `cargo test --lib` |

### 架构示意

```mermaid
flowchart LR
    subgraph FE["React + TypeScript"]
        UI["Toolbar · Viewport<br/>SeedPanel · ExportDialog<br/>SegmentsPanel · …"]
        STORE["Zustand store<br/>（持久化偏好）"]
        UI <--> STORE
    end
    subgraph BE["Rust 后端（Tauri 2）"]
        CMD["commands/<br/>mesh · segment · paint<br/>history · export · js_bridge"]
        SEG["segment/<br/>8 算法 + 种子/融合/eye 检测"]
        CORE["mesh/ · paint/ · export/"]
        CMD --> SEG
        CMD --> CORE
    end
    UI -- "Tauri invoke（IPC）" --> CMD
```

## 项目结构

```
ColorYourModel/
├── src/                          # React 前端 (TypeScript)
│   ├── components/
│   │   ├── Toolbar/              # 工具选择 + 参数
│   │   ├── Viewport/             # 3D 视口（BVH 拾取、着色器高亮）
│   │   ├── BrushSettings/        # 画笔参数
│   │   ├── ColorPanel/           # 颜色选择
│   │   ├── SegmentsPanel/        # 分区管理
│   │   ├── StatusBar/            # 状态栏 + HUD 诊断
│   │   ├── ExportDialog/         # 3MF 导出向导（预设库）
│   │   ├── SeedPanel.tsx         # 种子分区：Auto（融合）/ Manual（生长）
│   │   ├── IntelligentSegmentPanel.tsx  # 自动分割算法 + 参数
│   │   └── DebugLogViewer.tsx    # 应用内崩溃诊断
│   ├── hooks/                    # useMesh · usePaintTool · useTauriCommand · useHistory
│   ├── store/appStore.ts         # Zustand 全局状态（持久化）
│   ├── types/                    # mesh · segment · export 类型
│   ├── utils/                    # 日志 · 分区调色板
│   ├── i18n.ts · i18nDict.ts     # 中英双语词典 + useT hook
│   ├── App.tsx · main.tsx
├── src-tauri/                    # Rust 后端
│   ├── src/
│   │   ├── commands/             # Tauri IPC 层（mesh/segment/paint/history/export）
│   │   ├── mesh/                 # STL 加载 · 模型 + 邻接图 · kdtree
│   │   ├── segment/              # 分割算法、种子、融合、eye 检测
│   │   ├── paint/                # brush · spray · fill · smart snap · eraser
│   │   ├── export/               # 3MF · OBJ · 切片器预设 · 量化
│   │   └── lib.rs                # Tauri Builder + 命令注册
│   ├── resources/presets/        # 生成的切片器预设（源自 OrcaSlicer vendor 树）
│   └── Cargo.toml
├── docs/                         # HTML 文档（可编辑 md 源在 docs/bak/）
├── tools/                        # 预设萃取与 3MF/fill 验证脚本
├── examples/                     # 示例产物（paint_color_sample.3mf）
└── CHANGELOG.md
```

## 快速开始

### 环境要求

- [Node.js](https://nodejs.org/) ≥ 18
- [Rust](https://www.rust-lang.org/tools/install) ≥ 1.70 + Cargo
- [Tauri 2 系统依赖](https://v2.tauri.app/start/prerequisites/)（WebView2 等）

### 运行

```bash
git clone https://github.com/ZackaryShen/ColorYourModel.git
cd ColorYourModel

npm install
npm run tauri dev      # 首次启动编译 Rust 后端（约 2-3 分钟）
```

### 测试与检查

```bash
npx tsc --noEmit               # TypeScript
npx vitest run                 # 前端单测
cd src-tauri && cargo test --lib   # 后端单测 + 回归测试
```

### 调试日志

```bash
# Windows PowerShell
$env:RUST_LOG="debug"; npm run tauri dev

# Linux / macOS
RUST_LOG=debug npm run tauri dev
```

## 文档

📖 **在线文档：<https://zackaryshen.github.io/ColorYourModel/>** —— 用户指南、算法、技术文档、案例图库。

Markdown 源文件在 [`docs/bak/`](docs/bak/README.md)（与 docs 树镜像）；由它生成的 HTML 存档是正式的可读文档。简版：

- **[使用手册](docs/bak/user-guide/manual.md)**（[English](docs/bak/user-guide/manual.en.md)）—— 完整操作说明，附截图。**从这里开始。**
- [docs/algorithms/](docs/bak/algorithms/) —— 分割与检测算法（数学原理）
- [docs/technical/](docs/bak/technical/) —— 工程机制（拾取、高亮、撤销/重做、导出管线）
- [docs/cases/](docs/bak/cases/) —— 案例展示与验证记录
- `docs/bak/01…10-*.md` —— 中文工作文档（PRD、架构、缺陷清单、路线图、研究笔记）
- [CHANGELOG.md](CHANGELOG.md) —— 逐迭代变更记录

本项目采用对抗式自循环开发（`PLAN → REFUTE → REVISE → IMPLEMENT → TEST → RETROSPECT`），各轮记录见 CHANGELOG。

## 路线图

> [v0.1.0](https://github.com/ZackaryShen/ColorYourModel/releases/tag/v0.1.0) 已发布。状态以 PRD 验收标准衡量，不以"代码写完"衡量。

- [x] **v0.1 核心闭环** —— STL 导入 → 分割 → 上色 → 已验证的 3MF 导出（2026-08 完成，2026-09 发布为 v0.1.0）
  - 3MF 已在 Snapmaker Orca (U1) 端到端验证；fill 路由有回归测试保护
  - 待办：fill 簇真机复验（见 [docs/04](docs/bak/04-缺陷与遗留问题清单.md)）
- [ ] **v0.2** —— 调色板预设 + 颜色历史、体验打磨
- [ ] **v1.0** —— 批量处理、OrcaSlicer 插件形态集成

## 社区

- 💬 [**讨论区 Discussions**](https://github.com/ZackaryShen/ColorYourModel/discussions) —— Q&A 求助、Show and tell 晒作品、Ideas 提想法。中文 / English 均可。
- 🐛 [Issues](https://github.com/ZackaryShen/ColorYourModel/issues) —— 报 bug 与功能请求（有模板：面数 / 系统 / 版本）
- 🤝 [CONTRIBUTING.md](CONTRIBUTING.md) —— 构建、测试命令与文档流程。请遵循 [Conventional Commits](https://www.conventionalcommits.org/)；提交即表示同意贡献内容按 AGPL-3.0 授权。

## 许可证

本项目基于 [GNU AGPL-3.0](LICENSE) 授权。

Copyleft 保证衍生作品——包括网络服务形态——同样开源，同时任何人（含公司）都可自由使用、学习、修改和再分发。注：内置切片器预设萃取自 OrcaSlicer（AGPL-3.0）vendor 树，按 AGPL 派生数据处理。如需替代授权，请联系维护者。
