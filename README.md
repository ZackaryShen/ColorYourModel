# ColorYourModel 🎨

> 为 3D 打印白模 STL 提供智能分块上色解决方案，一键导出 3MF 彩色模型

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Python 3.10+](https://img.shields.io/badge/python-3.10+-blue.svg)](https://www.python.org/downloads/)

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
自动网格分割（Mesh Segmentation）
    ↓  ← 手动微调选区
交互式上色
    ↓
导出 3MF（带颜色）
    ↓
OrcaSlicer 切片打印
```

## 功能特性

- [ ] **智能网格分割**：基于曲率分析与锐边检测，自动将模型划分为逻辑区域
- [ ] **手动选区调整**：支持选区合并、拆分、边界微调
- [ ] **交互式上色**：为每个区域分配颜色，实时 3D 预览
- [ ] **颜色管理**：预设调色板、颜色命名、历史记录
- [ ] **3MF 导出**：输出符合 3MF Materials & Colors 扩展规范的彩色模型
- [ ] **批量处理**：对同类模型批量应用分割和上色规则

## 技术栈

| 模块 | 技术 | 说明 |
|------|------|------|
| 网格处理 | [trimesh](https://github.com/mikedh/trimesh) / [open3d](http://www.open3d.org/) | STL 解析、网格分割算法 |
| 3MF I/O | [lib3mf](https://github.com/3MFConsortium/lib3mf) | 3MF 格式读写 |
| 3D 可视化 | [pyvista](https://github.com/pyvista/pyvista) / [pyqtgraph](https://pyqtgraph.readthedocs.io/) | 交互式 3D 预览 |
| GUI 框架 | [PyQt6](https://www.riverbankcomputing.com/software/pyqt/) | 桌面界面 |
| 数值计算 | [numpy](https://numpy.org/) / [scipy](https://scipy.org/) | 向量运算、曲率分析 |

## 项目结构

```
ColorYourModel/
├── src/
│   ├── core/              # 核心算法
│   │   ├── __init__.py
│   │   ├── mesh_loader.py     # STL/OBJ 加载与预处理
│   │   ├── segmentation.py    # 网格分割算法
│   │   └── color_manager.py   # 颜色管理
│   ├── io/                # 文件 I/O
│   │   ├── __init__.py
│   │   ├── stl_reader.py      # STL 读取
│   │   └── threemf_writer.py  # 3MF 写入（带颜色）
│   ├── ui/                # 用户界面
│   │   ├── __init__.py
│   │   ├── main_window.py     # 主窗口
│   │   ├── viewer_3d.py       # 3D 视图控件
│   │   └── color_panel.py     # 上色面板
│   └── __init__.py
├── tests/                 # 测试
│   └── __init__.py
├── examples/              # 示例模型（小文件）
├── docs/                  # 文档
├── requirements.txt
├── setup.py
├── pyproject.toml
├── .gitignore
├── LICENSE
└── README.md
```

## 快速开始

```bash
# 克隆仓库
git clone https://github.com/yourname/ColorYourModel.git
cd ColorYourModel

# 创建虚拟环境
python -m venv .venv
.venv\Scripts\activate   # Windows
# source .venv/bin/activate  # Linux / macOS

# 安装依赖
pip install -r requirements.txt

# 运行
python -m src.ui.main_window
```

## 3MF 颜色规范

3MF（3D Manufacturing Format）是 3MF Consortium 制定的开放标准，其 **Materials & Colors Extension** 支持：

- 逐面片（per-triangle）颜色指定
- 基于顶点的颜色插值
- 标准 sRGB 色彩空间

本项目输出的 3MF 文件遵循该扩展规范，确保与主流切片软件（OrcaSlicer、PrusaSlicer、BambuStudio）兼容。

## 相关工具参考

| 工具 | 特点 | 适用场景 |
|------|------|----------|
| [BambuStudio](https://github.com/bambulab/BambuStudio) | 内置 STL 上色功能（按高度/角度） | 简单分色 |
| [Paint3D (Windows)](https://apps.microsoft.com/detail/9NBLGGH5FV99) | 3D 模型绘画 | 艺术创作 |
| [Meshmixer](https://www.meshmixer.com/) | 网格分割 + 区域上色 | 专业建模 |
| [Nomad Sculpt](https://nomadsculpt.com/) | iPad 上的雕刻与上色 | 移动端 |
| [Polychromatic](https://github.com/nicolai-wachenschwan/polychromatic) | 自动网格分割着色 | 3D打印上色 |

## 开发指南

### 提交规范

遵循 [Conventional Commits](https://www.conventionalcommits.org/)：

```
feat:     新功能
fix:      Bug 修复
docs:     文档更新
refactor: 代码重构（不改变行为）
test:     添加/修改测试
chore:    构建、CI 等杂项
```

### 运行测试

```bash
pytest tests/ -v
```

## 路线图

- **v0.1** — 基础 STL 导入 + 手动网格分割 + 单色填充 + 3MF 导出
- **v0.2** — 自动曲率分割 + 交互式选区调整
- **v0.3** — 完整 GUI（3D 预览 + 上色面板 + 调色板）
- **v1.0** — 批量处理 + 插件接口（OrcaSlicer 集成）

## 许可证

本项目采用 [MIT 许可证](LICENSE)。

## 贡献

欢迎提交 Issue 和 Pull Request！
