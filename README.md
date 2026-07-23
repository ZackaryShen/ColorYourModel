# ColorYourModel 🎨

> 为 3D 打印白模 STL 提供智能分块上色解决方案，一键导出 3MF 彩色模型

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

## 背景与问题

在 3D 打印领域，许多精美的模型（如手办、建筑、机械零件）以白模 STL 格式提供。STL 格式本质上是一堆**独立的三角形面片**，没有"零件"或"区域"的概念。

当用户在 OrcaSlicer 等切片软件中尝试为模型上色时，面临以下痛点：

- 🔺 **面片散落**：每个三角形都是独立的，无法按"逻辑区域"（如盔甲、皮肤、底座）批量选区
- 🖌️ **手动上色低效**：只能逐面片或逐层涂色，对高细节模型几乎不可行
- 📦 **格式限制**：STL 不支持颜色信息，需要导出为 3MF 才能携带颜色数据

**ColorYourModel** 旨在解决这个问题：自动识别模型中的逻辑区域，提供交互式上色界面，最终导出带颜色的 3MF 文件，直接送入切片软件打印。

## 工作流程

```
STL 白模导入
    ↓
自动网格分割（二面角阈值 + 法线一致性合并 + 小分区吸收）
    ↓  ← 分区画笔手动微调
交互式上色（画笔 / 喷漆 / 智能笔 / 填充 / 橡皮）
    ↓
导出 3MF（带颜色）
    ↓
OrcaSlicer / BambuStudio 切片打印
```

## 功能特性

### 已完成 ✅

- [x] **STL 导入**：支持二进制 + ASCII STL，1.5M 面级别模型 4 秒内加载
- [x] **自动网格分割**：三阶段算法（二面角分裂 → 法线一致性合并 → 小分区吸收），最终生成语义化区域
- [x] **分区画笔**：拖拽涂选面，松开鼠标创建新分区（手动微调自动分割结果）
- [x] **GPU 面拾取**：颜色编码 ID 渲染到 RenderTarget，O(1) 像素读取，支持 1.5M 面
- [x] **多种画笔工具**：普通笔 / 喷漆 / 智能笔（分区边界感知） / 填充 / 橡皮 / 吸管
- [x] **分区视图**：一键切换分区彩色可视化，黄色边框高亮分区边界
- [x] **Space 平移**：按住 Space 键切换为平移模式，松开恢复旋转
- [x] **画笔光标**：3D 环形预览跟随鼠标，显示画笔半径
- [x] **中英文 i18n**：界面全双语支持
- [x] **3MF 导出**：输出符合 Materials & Colors 扩展规范的彩色模型
- [x] **进度条**：加载模型和自动分割时显示进度事件

### 计划中 🗓

- [ ] 选区合并 / 拆分 / 边界微调
- [ ] 颜色调色板预设 + 历史记录
- [ ] 批量处理同类模型
- [ ] OrcaSlicer 插件集成

## 技术栈

| 层级 | 技术 | 说明 |
|------|------|------|
| **前端框架** | [React 18](https://react.dev/) + [TypeScript](https://www.typescriptlang.org/) | 声明式 UI，类型安全 |
| **3D 渲染** | [Three.js](https://threejs.org/) + [@react-three/fiber](https://github.com/pmndrs/react-three-fiber) | WebGL 渲染 + React 集成 |
| **桌面框架** | [Tauri 2](https://tauri.app/) | 原生窗口 + Rust 后端，替代 Electron |
| **后端算法** | [Rust](https://www.rust-lang.org/) | STL 解析、网格分割、面拾取 |
| **状态管理** | [Zustand](https://github.com/pmndrs/zustand) | 轻量级 React 状态管理 |
| **构建工具** | [Vite 6](https://vitejs.dev/) | 快速开发服务器 + HMR |
| **图形算法** | [petgraph](https://github.com/petgraph/petgraph) | 面邻接图（UnGraph）、BFS、图分割 |
| **KD-Tree** | [kiddo](https://github.com/sdd/kiddo) | 面空间索引 |
| **STL 解析** | [stl_io](https://crates.io/crates/stl_io) | 二进制 + ASCII STL 读取 |
| **3MF 导出** | [quick-xml](https://github.com/tafia/quick-xml) + [zip](https://github.com/zip-rs/zip2) | XML 生成 + ZIP 打包 |

### 架构示意

```
┌─────────────────────────────────────────────┐
│  React (TypeScript)                         │
│  ┌─────────┐ ┌──────────┐ ┌──────────────┐ │
│  │ Toolbar │ │ Viewport │ │SegmentsPanel │ │
│  └────┬────┘ └────┬─────┘ └──────┬───────┘ │
│       │           │              │          │
│  ┌────┴───────────┴──────────────┴────┐     │
│  │  Zustand Store (appStore.ts)       │     │
│  └────────────────┬───────────────────┘     │
│                   │ Tauri invoke            │
├───────────────────┼─────────────────────────┤
│  Rust Backend     │                         │
│  ┌────────────────┴───────────────────┐     │
│  │  commands/  (mesh, segment, paint) │     │
│  ├────────────────────────────────────┤     │
│  │  mesh/     (loader, model, colors) │     │
│  │  segment/  (dihedral, flood_fill)  │     │
│  │  paint/    (brush, spray, fill)    │     │
│  │  export/   (threemf)               │     │
│  └────────────────────────────────────┘     │
└─────────────────────────────────────────────┘
```

## 项目结构

```
ColorYourModel/
├── src/                           # React 前端 (TypeScript)
│   ├── components/
│   │   ├── Toolbar/               # 工具栏（画笔选择 + 参数）
│   │   ├── Viewport/              # 3D 视口（Three.js 渲染 + GPU 拾取）
│   │   ├── BrushSettings/         # 画笔参数面板
│   │   ├── ColorPanel/            # 颜色选择
│   │   ├── SegmentsPanel/         # 分区管理面板
│   │   └── StatusBar/             # 状态栏
│   ├── hooks/
│   │   ├── useMesh.ts             # 几何体构建 + 颜色更新
│   │   ├── usePaintTool.ts        # 画笔工具调用封装
│   │   └── useTauriCommand.ts     # Tauri 命令封装层
│   ├── store/
│   │   └── appStore.ts            # Zustand 全局状态
│   ├── types/
│   │   └── mesh.ts                # 类型定义（MeshData, PaintTool, Segment）
│   ├── utils/
│   │   └── logger.ts              # 日志工具
│   ├── i18n.ts                    # 中英文国际化
│   ├── App.tsx                    # 根组件
│   └── main.tsx                   # 入口
├── src-tauri/                     # Rust 后端
│   ├── src/
│   │   ├── commands/              # Tauri 命令（IPC 接口层）
│   │   │   ├── mesh.rs            # load_model, get_face_color
│   │   │   ├── segment.rs         # auto_segment, paint_segment_face, finalize_segment
│   │   │   ├── paint.rs           # brush_paint, spray_paint, fill_paint, ...
│   │   │   └── mod.rs             # 模块注册
│   │   ├── mesh/
│   │   │   ├── loader.rs          # STL 加载（binary + ASCII）
│   │   │   ├── model.rs           # MeshModel 结构体 + 面邻接图构建
│   │   │   └── face_colors.rs     # 颜色计算（衰减函数等）
│   │   ├── segment/
│   │   │   ├── dihedral.rs        # 三阶段自动分割算法
│   │   │   └── flood_fill.rs      # BFS 泛洪填充
│   │   ├── paint/
│   │   │   ├── brush.rs           # 画笔 + 喷漆 + 智能笔
│   │   │   ├── fill.rs            # 区域填充 + 分区填充
│   │   │   └── eraser.rs          # 橡皮擦
│   │   ├── export/
│   │   │   └── threemf.rs         # 3MF 格式导出
│   │   └── lib.rs                 # Tauri Builder + 命令注册
│   └── Cargo.toml
├── legacy/                        # 旧版 Python 实现（已废弃）
├── index.html
├── package.json
├── vite.config.ts
├── tsconfig.json
└── README.md
```

## 快速开始

### 环境要求

- [Node.js](https://nodejs.org/) ≥ 18
- [Rust](https://www.rust-lang.org/tools/install) ≥ 1.70 + Cargo
- [Tauri 2 Prerequisites](https://v2.tauri.app/start/prerequisites/)（WebView2 等系统依赖）

### 安装与运行

```bash
# 克隆仓库
git clone https://github.com/yourname/ColorYourModel.git
cd ColorYourModel

# 安装前端依赖
npm install

# 启动开发模式（前端 HMR + Rust 后端编译）
npm run tauri dev
```

首次启动会自动编译 Rust 后端（约 2-3 分钟），之后增量编译通常在 5 秒内完成。

### 启用调试日志

```bash
# Windows PowerShell
$env:RUST_LOG="debug"; npm run tauri dev

# Linux / macOS
RUST_LOG=debug npm run tauri dev
```

## 核心算法

### 自动网格分割（三阶段）

1. **Phase 1 — 二面角分裂**：遍历所有共享边，二面角 > 阈值（默认 30°）处断开，生成初始碎片区域
2. **Phase 3 — 法线一致性合并**：迭代合并法线方向相近的相邻区域，直到收敛
3. **Phase 4 — 小分区吸收**：面数 < MIN_REGION_CAP（30）的小分区合并到最大相邻区域

### GPU 面拾取

每个面分配唯一 ID → 编码为 RGB 颜色 → 渲染到离屏 RenderTarget → 鼠标位置像素读取 → 解码为 face ID。避免 CPU 端 raycast 对大模型的 O(n) 开销。

### 分区画笔（手动分区）

用户拖拽时逐面标记（`paint_segment_face`），前端维护 `Set<faceId>` 去重，松开鼠标时调用 `finalize_segment` 重建所有分区的元数据（面数、名称、颜色）。手动标签起始偏移 100,000，与自动分区标签空间隔离。

## 3MF 颜色规范

3MF（3D Manufacturing Format）是 3MF Consortium 制定的开放标准，其 **Materials & Colors Extension** 支持：

- 逐面片（per-triangle）颜色指定
- 基于顶点的颜色插值
- 标准 sRGB 色彩空间

本项目输出的 3MF 文件遵循该扩展规范，确保与主流切片软件（OrcaSlicer、PrusaSlicer、BambuStudio）兼容。

## 开发指南

### 提交规范

遵循 [Conventional Commits](https://www.conventionalcommits.org/)：

```
feat:     新功能
fix:      Bug 修复
docs:     文档更新
refactor: 代码重构（不改变行为）
test:     添加/修改测试
perf:     性能优化
chore:    构建、CI 等杂项
```

### 类型检查

```bash
# TypeScript（前端）
npx tsc --noEmit

# Rust（后端）
cd src-tauri && cargo check
```

### 对抗式开发流程

本项目采用对抗式自循环开发（Adversarial Development Loop），每轮迭代经过：

```
PLAN（找理论缺口）→ REFUTE（对抗证伪）→ REVISE（逐条修订）→ IMPLEMENT（最小改动）→ TEST（验证）→ RETROSPECT（复盘）
```

详见 [CHANGELOG.md](CHANGELOG.md) 中每轮迭代的详细记录。

## 路线图

- [x] **v0.1** — STL 导入 + 自动分割 + 基础画笔 + 3MF 导出
- [x] **v0.2** — GPU 拾取 + 多种画笔 + 分区视图 + i18n
- [ ] **v0.3** — 分区合并/拆分 + 调色板 + 颜色历史
- [ ] **v1.0** — 批量处理 + OrcaSlicer 插件集成

## 相关工具参考

| 工具 | 特点 | 适用场景 |
|------|------|----------|
| [BambuStudio](https://github.com/bambulab/BambuStudio) | 内置 STL 上色功能（按高度/角度） | 简单分色 |
| [Paint3D (Windows)](https://apps.microsoft.com/detail/9NBLGGH5FV99) | 3D 模型绘画 | 艺术创作 |
| [Meshmixer](https://www.meshmixer.com/) | 网格分割 + 区域上色 | 专业建模 |
| [Nomad Sculpt](https://nomadsculpt.com/) | iPad 上的雕刻与上色 | 移动端 |
| [Polychromatic](https://github.com/nicolai-wachenschwan/polychromatic) | 自动网格分割着色 | 3D打印上色 |

## 许可证

本项目采用 [MIT 许可证](LICENSE)。

## 贡献

欢迎提交 Issue 和 Pull Request！
