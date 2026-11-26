# CYM 几何交换格式调研

## 桌面工具覆盖

此处以创作、扫描与打印工具的官方格式列表交叉核对，判断各格式对应的工作流。列表反映软件支持情况，市场份额需另有统计数据。

- Blender 提供 OBJ、PLY、STL、FBX、USD 等交换格式，并提供 glTF 2.0 导入导出。Maya Creative 同时支持 OBJ、FBX 和 USD，说明 FBX 与 USD 属于设计师常遇到的场景格式。[Blender 导入导出手册](https://docs.blender.org/manual/en/latest/files/import_export/index.html) · [Blender glTF 2.0](https://docs.blender.org/manual/en/5.2/addons/scene_gltf2.html) · [Maya Creative 支持列表](https://help.autodesk.com/cloudhelp/ENU/MayaCRE-ManagingScenes/files/GUID-3A6190CA-E296-4A10-9287-5AEE156DBA9D.htm)
- 扫描建模工具 Metashape 可交换 OBJ、PLY、STL、FBX 和 GLB。PrusaSlicer 支持 3MF、STL、OBJ、STEP 和 AMF，将 STEP 三角化，并推荐使用 3MF 代替 AMF。PLY 因此应覆盖网格扫描文件，同时检测没有面的点云。[Metashape 官方手册](https://www.agisoft.com/pdf/metashape_2_3_en.pdf) · [PrusaSlicer 支持列表](https://help.prusa3d.com/article/supported-file-formats_1772)

## CYM 转换规则

CYM 的交换目标是三角网格、毫米坐标和逐面 RGBA。现有渲染、分区和涂色依赖同一份三角拓扑。3MF 工程用于还原板、耗材及编辑状态，其余格式以当前网格和能表达的外观作为交付目标。导入端可把场景实例的变换烘入顶点，并在镜像变换后恢复正面绕序。导出端分开处理颜色保真、格式限制造成的转换及工程状态保存。[CYM 网格](../../../src-tauri/src/mesh/model.rs) · [CYM 纹理入口](../../../src-tauri/src/mesh/texture.rs) · [3MF 核心及扩展](https://3mf.io/spec/)

| 格式 | 优先级与转换边界 | 独立验证 |
| --- | --- | --- |
| STL | 打印基础网格。读取 ASCII 和二进制，写标准二进制。无标准面色与单位字段，沿用 CYM 毫米惯例。 | Blender 导出的输入，CYM 产物由 Blender 读取。另核对二进制三角计数、尺寸及绕序。 |
| OBJ + MTL | 设计工具通用交换。支持正负面索引和凹多边形三角化。优先采用 MTL 漫反射颜色和透明度，否则对顶点颜色求均值。导出最多保留 256 个 RGBA 材质。 | Blender 双向交换和 Rust 回归覆盖色值、绕序、特殊文件名和旧版 CYM 材质。 |
| PLY | 扫描和网格着色。覆盖 ASCII、大小端二进制及带颜色的网格；按属性名读取，点云应给出明确反馈。逐面颜色可用独立三角角点携带常见顶点色，回读时重建网络邻接。 | Blender 可验证顶点色交换；手工二进制 fixture 补足大小端、属性顺序、面色属性和错误索引。 |
| glTF 2.0 / GLB | 设计与预览交换。复用 GLB 纹理流程的尺寸与坐标约定 `[x, -z, y] × 1000`；另处理无贴图网格、材质色、`COLOR_0` 以及 `.gltf` 的本地依赖。glTF 颜色乘数采用线性色，转换后再写 sRGB 字节。完整的贴图预览与烘焙需独立保持。 | Blender 生成带父子变换、负缩放和相邻异色面的资产。用 Khronos Validator 检查输出，Blender 复读并比较尺寸与色值。 |
| 3MF | 保留现有工程及切片器联动。几何交换需区分业务工程 3MF 和带材质扩展的通用 3MF。已有工程/切片读回优先做回归。 | OrcaSlicer / PrusaSlicer 打开并比较对象、尺度、面色及耗材槽，对已保存工程另验证 CYM 重开。 |
| FBX | 常见 DCC 交换格式。需要静态网格、实例变换、单位、轴与颜色适配。Assimp 文档列有 ASCII/二进制 FBX 导出，适合试验统一原生转换；本机 Blender 同时可作为外部转换器或独立验证器。 | 用 Blender/Autodesk 导出 FBX 输入，覆盖嵌套镜像与随面绑定的色数据。若生产依赖采用 Blender，外部验证优先用另一套读取器。 |
| OpenUSD | 常见 DCC 场景交换格式。USD/USDA/USDC/USDZ 需要处理组合、实例、米制比例、坐标轴及材质；网格逐面颜色可用 `uniform` 频率的 `displayColor`，也要验证主流消费者实际读到的颜色。原生集成优先评估 OpenUSD，或明确标明可选 Blender 转换依赖。 | Blender 产物和 `usdchecker`/`usdcat` 交叉验证实际 `UsdGeomMesh` 的点、面、`metersPerUnit`、`upAxis`、颜色及 `subdivisionScheme`。 |
| STEP / IGES | 机械 CAD 曲面与实体，导入需 CAD 内核按公差离散；网格写成三角平面集合无法恢复原始曲面与设计参数。可作为单独的 CAD 入口评估。 | 用机械 CAD 导出的有单位圆柱、曲面、组件测误差、尺寸和变换。 |
| AMF、DAE、3DS、Alembic | 覆盖特定存量和专业场景。依照用户实际文件来源和需求排序。 | 每增加一个格式都需要对端回读和明确的颜色、面片及单位边界。 |

格式定义和工具能力见各自的原始资料。[Wavefront OBJ 原规格](https://paulbourke.org/dataformats/obj/obj_spec.pdf) · [Greg Turk PLY 原规格](https://gamma-web.iacs.umd.edu/POWERPLANT/papers/ply.pdf) · [Khronos glTF 2.0](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html) · [Khronos glTF Validator](https://github.com/KhronosGroup/glTF-Validator) · [Assimp 支持的写出格式](https://github.com/assimp/assimp-docs/blob/master/source/usage/use_the_lib.rst) · [OpenUSD Mesh](https://openusd.org/release/api/class_usd_geom_mesh.html) · [OpenUSD Primvars](https://openusd.org/release/user_guides/primvars.html) · [OCCT STEP 翻译](https://github.com/Open-Cascade-SAS/OCCT/wiki/step)

## 验收判据

- 用独立工具生成非对称、至少包含两种颜色且中心偏移的网格。逐格式明确期望的范围、面数、有向三角、颜色及格式特有信息。导入后在 CYM 检查分区和涂色，保存或导出后用独立读取器比对同一组几何与颜色指标。应用与标准共有的误差来源应设定可解释的容差。
- 自身往返可以发现回归，同时可能掩盖一对互相抵消的读写错误。每个已支持方向分别用独立输入和独立读回覆盖。额外检查缺失配套文件、恶意索引、非法变换、混合内容、半途失败和加载既有工作区状态。

## 实测结果

本轮运行环境为 macOS arm64、Blender 5.2.2 LTS。FBX 和 USD 使用独立安装的 Blender 4.5 或更新版本，经过有版本标记的私有二进制流传递顶点索引与逐面 RGBA。保留两个位置重合但拓扑独立的壳体，同时保留异色共享边。

| 格式 | 样本和验证 |
| --- | --- |
| STL、OBJ | Blender 独立生成 ASCII STL 与 OBJ/MTL。CYM 导入后核对三个轴的边界、三角面数量、绕序和 MTL 颜色。Blender 复读 CYM 的二进制 STL 和 OBJ/MTL，再比对有向三角和颜色。macOS 桌面应用通过标准文件选择器导入双材质 OBJ，显示 2 个面和两种预期颜色。 |
| PLY | Blender 生成 ASCII 和二进制 PLY，回读磁盘颜色；覆盖大端、逐面色和凹面。Stanford Bunny 独立复读 69,451 个有向三角和 208,353 个角点的 RGBA，均匹配。[Stanford 公开样本](https://graphics.stanford.edu/data/3Dscanrep/) |
| glTF、GLB | Blender 生成外部 buffer、父子变换和镜像实例。CYM 核对绕序、尺寸、色值和原始索引。Khronos Validator 检查两个导出零问题。Blender 复读 0.25 × 0.125 × 0.5 m，两个面的 RGBA 精确。验证三种 alphaMode，真实 Duck GLB 仍进入贴图路径。[Khronos glTF Validator](https://github.com/KhronosGroup/glTF-Validator) |
| FBX | Blender 生成带厘米单位和镜像变换的输入。桥接导入后的有向三角和 RGBA 匹配。ufbx 独立复读最终输出的 4 个有向面、8 个原始索引点及 RGBA，最大坐标误差约 0.0000041 mm。[ufbx](https://ufbx.github.io/) |
| USD、USDA、USDC、USDZ | 独立编写的 Y-up、厘米制、带镜像 USDA 达到预期边界、绕序和 uniform 颜色。4 种后缀全部通过 Blender 实际往返。OpenUSD 校验器通过 USDZ，`usdcat` 回读 4 个有向面、8 个索引点、米制 Z-up 和准确颜色及透明度。 |
| 3MF | 现有多盘、编辑历史、输出失败保全及渐变相关 Rust 回归通过。本轮沿用既有打印路径。 |

OBJ 线性颜色编码、相邻异色与重合断开拓扑、透明度解释、非法索引及输出后缀均有回归覆盖。桌面实测覆盖 OBJ 原生文件选择与呈现，导出通过应用代码、前端流程测试和独立读取器验证。macOS 在读取 Downloads 中的 MTL 时会弹出系统授权，本次桌面实测使用非受保护测试目录。Windows 和 Linux 未运行验证。

## 提交前独立复验

Astra 在 macOS 桌面版打开 8 面 5 色的 OBJ，实际修改 1 个面并保存工程。正式 writer 从该工程导出 PLY 和 GLB，Blender 复读的 8 个有向三角面、40 × 26 × 24 mm 尺寸和逐面 RGB 均匹配。原生文件选择器重开 PLY 后显示笔画的实际颜色 `#FB130C`。后台控制受系统限制，网格导出菜单和保存对话框未直接操作。另验证了单材质 GLB 贴图预览和多材质 GLB 的几何导入入口。

Astra 复现了两处已有的 OBJ 写盘风险。导出会覆盖同名的现有 MTL，后端也会接受 3MF 文件名。修复后 OBJ 拒绝错误后缀，现有 MTL 保持原字节，新 OBJ 指向独占创建的带编号 MTL。独立复验覆盖 12 份输入、7 种输出、4 次并发导出和落盘失败，5 项验收全部通过。

本机具备 Blender 时，可在仓库根目录运行两个外部工具测试。

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib blender_independently_reads_and_writes_stl_and_obj -- --ignored
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib roundtrips_coincident_shells_and_shared_color_seams -- --ignored
```
