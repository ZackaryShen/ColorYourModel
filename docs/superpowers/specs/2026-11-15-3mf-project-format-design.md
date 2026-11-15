# 设计：以 3MF 为工程文件 —— 多盘导入/导出、工程状态内嵌、Tab 内存换出

日期：2026-11-15 · 分支：feature_020_p0_engineering · 状态：待批准（方案 A 已由用户拍板）

## 背景与意图

feature_020_p2_paint 曾设计 `.cym`（ZIP：`project.json` + `model.bin`）作为工程文件，完整保存几何、逐面分区标签、逐面涂装颜色、区域名与手工标签高水位。现在不打算维护第二个容器格式：3MF 本身就是 ZIP/OPC 包，除了切片器要读的那部分，还能携带私有 part。**一个 `.3mf` 同时作为切片交付物与工程文件。**

同时补齐今天完全缺失的一环：3MF 导入（现在 `load_model` 只分派 `stl`，`commands/mesh.rs:66`）。导入要重建几何、继承原模型颜色，并支持多盘模型——每盘一个 Tab、盘内多对象合并成一个网格（与 Orca 的“盘内对象合并”心智一致），不丢盘。

性能上：活跃 Tab 占内存优先，不活跃 Tab 落盘换出（借用硬盘）；撤销历史随 Tab 一起落盘，切换/换出都不丢。

## 目标

1. 导出的 3MF 在 Snapmaker Orca / OrcaSlicer / BambuStudio 中颜色、盘、预设不退化（沿用现有 `export_3mf` 的已验证通道）。
2. 重新导入 CYM 导出的 3MF 时，无损还原：逐面精确 RGBA、逐面分区标签、区域名、手工标签高水位、撤销/重做栈、盘划分与 Tab 名。
3. 没有工程字段的 3MF 走**基础导入**：几何 + `paint_color` 颜色 + 盘划分，绝不因缺少工程字段而失败。
4. 多盘 → 多 Tab；Tab 之间数据隔离；超预算时非活跃 Tab 落盘换出，切回无数据丢失（含撤销历史）。
5. 单一文件既是切片交付物又是工程文件，不再有 `.cym`。

## 非目标

- 不做盘内部件（part）身份保留：盘内多对象合并为一个网格（用户已确认）。导出的 3MF 每盘只有一个 object。
- 不导入外部 3MF 的 `basematerials` / `displaycolor` 材质色，不做纹理/UV（v1 已知限制，见回退矩阵）。
- 不做 `.cym` 迁移（该格式从未发布）。
- 不做自动保存、崩溃恢复、多窗口。
- 不做自动化「过一遍切片器再存回来」后仍还原工程状态的努力——那属于回退矩阵里的正常降级。

## 反目标（即使技术可行也算失败）

- 因为工程 part 解析失败而拒绝打开一个几何合法的 3MF。
- 面身份不确定时套用逐面数组——那会**静默错色**，比丢颜色更糟。
- 为了“符合标准”把逐面数据写进 XML 三角自定义属性（1.88M 面量级 ≈ 50MB+ 文本，解析与写出都不可接受）。
- 为了省事让换出变成“撤销历史被清空”（用户明确要求历史落盘）。

## 约束与已验证事实

**3MF 容器结构（实测 `梁圣.3mf`，Snapmaker_Orca-2.4.1 产出，13.6MB）**

- `_rels/.rels` 指向主模型 part（`3D/3dmodel.model`）；主模型可只有 `<component p:path="…">`，真实几何在**独立 part** `3D/Objects/梁圣.stl_1.model`（13.6MB），引用关系在 `3D/_rels/3dmodel.model.rels`，主模型 `requiredextensions="p"`。读取器必须走 OPC 关系 + component 递归，不能假设几何都在主模型里。
- 盘划分只存在于 `Metadata/model_settings.config`：`<plate>`（`plater_id`、`plater_name`、`locked`、`filament_map_mode`、`filament_maps`）+ `<model_instance>`（`object_id` / `instance_id` / `identify_id`）；另有 `<object id>` 块（`name` / `extruder` / `<part>` 及其 `matrix` / `source_*` / `mesh_stat`）与 `<assemble>`（`assemble_item`）。实物位置在 `build/item@transform`（3MF 3×4 行向量矩阵，末 3 个为平移）。
- **双盘实测**（`model.3mf`，同版本 Orca 产出，5.5MB）：2 个 `<plate>`、2 个 `build/item`、2 个外部 part；盘 1 的对象是 148,460 面**全叶子**涂装（槽 1–7 都真实用到），盘 2 的对象是 993,568 面（含细分）。两盘共用**一套** 7 色 `filament_colour`，`filament_maps` 长度 = 色槽数。
- **`<assemble>` 不是打印位置**：同一文件里 `assemble_item` 的 transform（对象 4：缩放 0.701、平移到 0,0,75）与 `build/item` 的（缩放 0.701、平移到 459.6,122.2,52.6）**数值不同**，前者是排布装配视图的元数据。几何/打印位置只信 `build × component`。
- **盘归属按世界坐标判定，不是按 `model_instance` 列表**（两轮真机实测 + 源码 `PartPlateList::compute_origin` / `compute_colum_count`）：盘在世界空间里排成**网格**，`model_settings` 的 `<plate>` 只提供盘的**存在与命名**。对象落在哪个盘的矩形里就属于哪个盘。
  - 网格公式（OrcaSlicer `PartPlate.hpp` / `PartPlate.cpp`，与 Snapmaker Orca 2.4.1 实测一致）：
    ```text
    count = 盘数
    cols  = round(sqrt(count))，若 sqrt(count) > round(sqrt(count)) 则 +1   # 即向上取整
    row   = (plater_id-1) / cols      col = (plater_id-1) % cols
    origin = (col * 床宽 * 1.2, -(row * 床深 * 1.2))      # 行方向朝 -Y
    1.2 = 1 + LOGICAL_PART_PLATE_GAP (1/5)
    ```
  - 实测对照 1：`model.3mf` 对象 4 在 x=459.6，床宽 270.5 → 盘 2 单元 `[324.6, 595.1]`，落其中 ✓（2 盘时 cols=2，盘 2 在同一行）。
  - 实测对照 2：自造三盘样本第一版把三个对象放在 x=60/135/210（都在盘 1 单元）→ 三个对象全被归到盘 1。
  - 实测对照 3：第二版按“一行”排布（x=60/384.6/709.2）→ 3 盘时 cols=2，盘 3 在 `row=1, col=0`（**-Y 方向**），第三个对象因此落在所有盘之外，Orca 用红框标成越界。
- 由上述规则推出的工程约定：**导入时把每盘几何归一化到盘局部坐标**（减去该盘 `origin`），**导出时再按盘加回 `origin`**。这样 Tab 内视图/包围盒始终正常，且写回后归属不变。床宽/床深取所选机型 `printable_area` 的 max x / max y；盘数 = 本次导出的 Tab 数。对象在盘内的局部位置保持至少 20mm 内缩，给常量误差留余量。
- `<part><metadata key="matrix">` 与 component transform 在单盘样本里数值相同（平移都是 3.81e-06），双盘样本里差到 1e-6 量级——**不叠加应用**。
- 单位在 `model@unit`（该文件 `millimeter`），导入需按 unit 换算。

**颜色通道（沿用现有导出结论）**

- 逐面颜色只有 `triangle@paint_color` 这一条切片器会读的通道，值不是 RGB 而是**1-based 挤出机槽位**；RGB 由 `Metadata/project_settings.config` 的 `filament_colour[]` 决定（该文件为 `["#FFFFFF","#FFFF00","#00C1AE","#F4E2C1"]`）。`basematerials`/`displayid` 在 Orca/Bambu 的读取器里无读者（见 `export/threemf.rs` 模块注释与 `paint_color.rs`）。
- 槽位上限取两套切片器交集 16（`paint_color::MAX_EXTRUDER_SLOT`），导出前经 `export/quantize.rs` 量化到 ≤16 槽。

**paint_color 状态机（本轮实测验证）**

`paint_color` 是 libslic3r `TriangleSelector` 的序列化位流：

- 每节点：2 bits `split_sides` + state；state < 3 用 2 bits 直写，≥3 用 `0b11` escape + 4-bit 小端块（块值 15 表示续块）。
- `split_sides = 0` 是叶子，state 即槽位（0 = 未上色）。
- `split_sides ≠ 0` 时该三角形被细分，**子三角形数 = split_sides + 1**（1→2、2→3、3→4），递归存储。
- 位序：每个 nibble 内 LSB-first；写入端把完成的 nibble **逆序**放到字符串里（`0C` 在盘上是 nibble `[0x0, 0xC]`，解码前要反转）。
- 实测校验：`梁圣.3mf` 的 8,693 个不同 `paint_color` 取值按上述文法**全部干净解析、无剩余比特**（脚本见验证记录）。13,364 个上色三角形里只有 4,419 个叶子，8,945 个带细分。
- **尚未确定（实现时必须 port 源码）**：2 bits 是“分裂边数”，具体**哪几条边**被分裂、以及子三角形的顶点顺序，由父级细分模式决定，必须照搬 libslic3r `TriangleSelector` 的 `deserialize` + `split_triangle` 划分规则。位语法已实测确认，几何划分未确认；实现后要用真实文件做颜色连通性校验。

**细分代价（实测）**

148,460 个基础三角形 → **993,568 个叶子面**（6.7×），单个上色三角形最多裂成 970 个叶子。颜色分布：槽 1（含未上色与 state 0）551,051、槽 2 131,275、槽 3 132,501、槽 4 178,741。

**本仓库模型侧约束**

- `MeshModel` 是**单网格**：无对象树、无 transform、无盘概念（`mesh/model.rs:87`）。所以盘内多对象必须在导入时烘焙 transform 后合并，导出时按盘写一个 object。
- 撤销栈是逐面增量、条目自逆、depth 50 / 256MB 预算（`mesh/history.rs`，`DEFAULT_MAX_DEPTH` / `DEFAULT_BYTE_BUDGET`）。条目存 `(face, 旧颜色)` 与 `(face, 旧标签)`。
- `MeshDataDto` 已经带上 `segment_labels` 与 `segments`（含区域名），导入还原可直接复用这条 DTO 通路（`mesh/model.rs:171`）。
- 前端 `MeshData` 是纯 `number[]`（`src/types/mesh.ts:11`），1M 面量级约 190MB/模型，另加 three.js `BufferGeometry` 与边表 —— **换出的内存大头在前端**。
- `AppState` 只有一份 `Mutex<Option<MeshModel>>` 与全局 `hidden_labels`（`commands/mesh.rs:9`），多 Tab 要拆。

**Orca 会重写文件**：用户把 CYM 导出的 3MF 在切片器里打开再保存，私有 part 会丢、面序会变、paint_color 可能新增细分。工程状态必须靠指纹校验，不匹配就降级。

## 方案（已选 A）

| 方案 | 工程状态放哪 | 结论 |
|---|---|---|
| **A（已选）** | 私有 OPC part（`Metadata/cym/*`），几何用 3MF 原生结构 | 隔离、可回退、逐字节无损；新增内容类型声明即可 |
| B | 混进 `Metadata/model_settings.config` 的自定义段 | 否决：与切片器元数据同居一个文件，Orca 保存时整份重写，语义脏 |
| C | 编码进三角自定义属性（`cym_label=…`） | 否决：XML 膨胀（1.88M 面 ≈ 50MB+），解析/写出慢 |

**面身份对齐**：选几何指纹整体校验（顶点/面数与字节哈希），不符则整体忽略工程 part。备选的“逐三角坐标键匹配”能容忍重排，但重叠几何有歧义，v1 不做。

## 设计

### 1. 文件布局（导出）

```text
[Content_Types].xml                  新增 Default: bin / json（OPC 要求每个 part 有类型）
_rels/.rels                          不变
3D/3dmodel.model                     每盘一个 <object type="model">（盘内对象已合并、transform 已烘焙）
                                     <build> 每盘一个 <item objectid=…>
Metadata/model_settings.config       盘归属：`<object>` 块 + `<plate>` + `<assemble>`（单盘也写，保证 round-trip 稳定）
                                     每盘必须：`plater_id`、`plater_name`、`locked`、`filament_map_mode`、
                                     `filament_maps`（长度 = 色槽数）；`model_instance` 带 `identify_id`。
                                     缩略图键（`thumbnail_file`…）v1 不写，避免指向不存在的 PNG（待真机确认 Orca 是否容忍）
Metadata/project_settings.config     现有：filament_colour = 量化调色板
Metadata/machine_settings_1.config   现有（选中机型路径）
Metadata/process_settings_1.config   现有
Metadata/filament_settings_<n>.config 现有（每槽一份）
Metadata/cym/project.json            工程头（见 2）
Metadata/cym/plate_<i>.bin           每盘逐面工程状态（见 2）
Metadata/cym/history_<i>.bin         每盘撤销/重做栈（见 3）
```

逐面数组不塞 JSON、每盘独立成 part：便于按盘懒加载、按盘校验、按盘失败降级。`.config` 现有导出未在 `[Content_Types].xml` 声明而 Orca 容忍；**新增的 part 按 OPC 声明类型**，不给严格读取器留把柄。

### 2. 工程状态（cym 该记住的东西）

`project.json`：

```json
{
  "format_version": 1,
  "app": "ColorYourModel",
  "unit": "millimeter",
  "active_plate": 1,
  "plates": [
    {
      "plater_id": 1,
      "name": "",
      "object_ids": [3],
      "state": "plate_1.bin",
      "history": "history_1.bin",
      "fingerprint": { "vertex_count": 74226, "face_count": 993568, "hash": "fnv1a64:…" }
    }
  ]
}
```

`plate_<i>.bin`（小端，魔数 `CYMST1\x01`）：

| 段 | 编码 |
|---|---|
| face_count | u32（与指纹冗余，自校验） |
| segment_labels | u32 × F |
| face_rgba | u8 × 4F（**量化前**的精确颜色） |
| next_manual_label | u32 |
| 区域名表 | u32 count，然后 (u32 label, u16 len, utf8 bytes) × count |

**不重复存几何**：3MF 的 mesh 是权威几何。指纹 = FNV-1a 64（哈希**规范化坐标**的 f32 LE 字节 ∥ `faces u32 LE`，不引入新依赖）+ 顶点/面计数。

**哈希域是「写进文件的世界坐标」**（导入侧在减去盘原点**之前**取值，导出侧在加上盘原点**之后**取值），不是盘局部坐标。原因：盘原点是 264mm 量级，`f32` 在该量级的 ulp ≈ 3e-5 mm，比 `{:.6}` 的 1e-6 网格粗 15 倍；若写侧先加原点、读侧再减回来，`fl(fl(L+O)−O)` 与 `L` 会差到 1.5e-5，指纹必然失配并静默降级为基础导入。改用世界坐标后，两侧取的是同一串十进制数字解析出的同一批 f32，往返无损。P0 只有单盘（原点 0），两种域等价；P1 必须用世界坐标。

**规范化哈希（builder 审查发现的坑）**：现有 writer 用 `{:.6}` 定点输出顶点（`threemf.rs:212-214`），f32 的位模式经「格式化→解析」后会变——实测 `model.3mf` 的 222,678 个坐标分量里 2,873 个（1.29%）落在这个精度歧义带。若直接哈希内存里的 f32 位，自家文件重开就会判「几何被改过」而丢掉全部工程状态。修法：保存时把每个坐标先 `format!("{:.6}", v)` 再 `parse::<f32>()` 得到**读回时必然得到的那个值**，哈希它的字节。读取端解析同一字符串用的是同一个 `str::parse::<f32>`，逐位一致。不改变 writer 输出格式，不冒切片器兼容风险。

**附带要求**：加一条单测——随机 f32 采样，断言 `hash(canonical(v))` 与「写出字符串再解析」得到的字节哈希相同。

**为什么保留精确 RGBA**：3MF 只有 ≤16 槽量化色，重开再导出会二次量化、逐次丢色。精确 RGBA 是无损 round-trip 的唯一途径；回退路径才用 `paint_color + filament_colour`。

`paint_sources`（issue #29 的涂装来源分组）v1 不持久化，列为已知损失。

### 3. 撤销历史持久化（用户要求）

每盘 `history_<i>.bin`：`undo` 栈（旧→新）与 `redo` 栈各带长度，条目编码：

```text
kind: u8 (0=paint 1=segmentPaint 2=region 3=merge 4=split 5=transform)
stroke_id: u8 flag + u64(有)
colors: u32 count + (u32 face, u8[4] rgba) × count
labels: u32 count + (u32 face, u32 label) × count
transform: u8 flag + f32[16]（仅 kind=5；行主序 4×4）
```

kind 5 = p2 分支的 `OpKind::Transform`：整模型变换条目不携带颜色/标签载荷，只有矩阵。**即使 p2 尚未合入，v1 也预留给它**——否则任何用过变换 gizmo 的模型保存后会因为「未知 kind」丢掉整段历史（见下条）。

- 恢复时重建 `History`，`bytes` 按实际 `len` 重算（`History::bytes` 用 capacity，恢复后 capacity≈len，语义正确）。`open_stroke` 不持久化——一次拖拽不跨保存。
- **持久化预算 64MB（原始字节），与内存 256MB 预算分离**：超出时从最旧 undo 条目开始丢，保留最近的（用户最关心最近操作），历史面板标“更早的步骤未保存”。
- 未知 `kind` 字节 → 整段历史丢弃并提示，不阻断打开。
- 3MF 是 DEFLATE，face id 升序、颜色成片重复，实测同类数据压缩率通常 5–10×。

### 4. 导入（reader）

分阶段，任一可选 part 失败都降级，只有几何本身解析失败才报错：

1. 打开 ZIP，读 `_rels/.rels` 找主模型；缺 rels 时回退 `3D/3dmodel.model`。
2. 解析主模型：`metadata` / `resources`（`object` 为 `mesh` 或 `components`）/ `build`。
3. 递归展开 `components`（同 part 或 `p:path` 外部 part），累乘 transform。
4. 盘划分：有 `model_settings.config` → 按 `<plate>` 的 `model_instance` 分组 build item；没有 → 全部合成单盘。
5. 每盘：按 `build/item@transform` × `component@transform` 烘焙顶点 → 多对象合并为单一 indexed mesh（顶点按位模式去重，同 `loader.rs` 的 STL 合并）→ **减去该盘世界原点 `(plater_id-1) × 床宽 × 1.2`，归一化到盘局部坐标**。
   **`model_settings.config` 里的 `<part><metadata key="matrix">` 与 `source_offset_*` 不参与烘焙**：实测 `梁圣.3mf` 中 part matrix（4×4 行主序）与 component transform（3MF 3×4）是**同一个变换的两种写法**（平移都是 3.81e-06），两者都应用会双重变换。几何位置只信 3MF core 的 `build × component`。
6. 颜色：解码 `paint_color`（含细分）→ 槽 → `filament_colour[slot-1]`；**无 `paint_color` 与 state 0 都归槽 1 颜色**（该文件为白）；无 `project_settings.config` 时归 `DEFAULT_FACE_COLOR`。**整对象上色**走另一条通道：`model_settings.config` 里 `<object>`/`<part>` 的 `<metadata key="extruder" value="0">`（0-based 槽索引），没有逐三角 `paint_color` 的对象/部件用它的颜色作基色——`model_settings.config` 反正要解析，不认它会让 Orca 里看着有色的盘导进来变成一片灰。
7. 细分重建：中点按「顶点对 (min,max) 索引」缓存复用 —— 共享边两侧必须得到逐位相同的中点，否则网格沿边开裂。子三角形顶点顺序按 libslic3r `TriangleSelector` 划分规则（见约束里的未确认项）。
8. 工程 part：读 `project.json` → 主版本不等则忽略工程 part（**不拒开文件**）；按盘校验指纹，一致则套精确 labels/RGBA/names/高水位/历史；不一致则只用原生颜色，状态栏提示“工程数据与几何不匹配，已按普通 3MF 导入”。
9. 派生状态重建（normals → bbox → segments → kdtree → vertex kdtree → adjacency），顺序同 `loader.rs` / p2 `load_cym`。
10. 返回 DTO + Tab 列表（每盘一项，含名称、面数、是否含工程状态）。

### 5. 多 Tab 与内存换出

- 后端 `AppState`：`Mutex<Option<MeshModel>>` → `Mutex<Tabs>`，`Tabs { tabs: Vec<Tab>, active: TabId }`。`Tab` 三态：
  - `Resident(MeshModel)` —— 在内存（含 `History`、`hidden_labels`）
  - `Source { path, plate_index }` —— **干净** Tab，真源就是磁盘上的源文件，随时按盘重读，**不需要换出文件**
  - `Swapped(path)` —— **脏** Tab 被换出后的私有交换文件（源文件不含未保存修改，只能靠它）
  `hidden_labels` 从全局移进 Tab（视图状态按模型隔离）。
- **命令层显式传 `tab_id`**（builder 审查发现：`state.mesh.lock` 有 46 处调用点、~37 个命令；若靠“隐式活跃 Tab”，一次指针涂抹期间的 Tab 点击会让排队中的 paint IPC 落到新 Tab 上，污染另一个模型的标签/颜色，且它自己的撤销栈救不回来）。前端在 Tab 创建时拿到 `tab_id` 并随每次调用传回；后端校验 Tab 存在且 `Resident`，否则返回明确错误。前端 paint 队列按 Tab 隔离，切 Tab 时清空未发送队列。
- **loader 从第一天就按盘分批**：`load_plates(path) -> Vec<PlateRef>`（只解析盘划分与元数据）与 `load_plate(path, plate_index) -> MeshModel` 两个入口。P1 只实例化活跃盘，其余是 `Source`，切 Tab 时按需读——这样 P2 加交换文件不需要重写 loader 形状。
- 常驻预算估算（单 Tab 字节）：`verts*12 + faces*12 + colors*4 + labels*4 + normals*12 + faces*常数(kdtree/adjacency) + history.bytes()`；系数以基准测试校准并写进注释。
- 换出：脏 Tab 写 `<app_cache_dir>/cym-tabs/<tab_id>.cym2`（`project.json` + `model.bin` + `history.bin` 的二进制容器，**不走 XML**——换出文件不是交付物，速度优先）→ 状态转 `Swapped`。状态编码与工程 part **共用同一套 struct 与编码函数**。干净 Tab 在 P1 就可以直接丢弃为 `Source`，不写任何文件。
- 换入：`Swapped` → 读回 → 重建派生状态 → 返回 DTO；`Source` → 读源文件对应盘。
- 前端：Tab 栏；切换时旧 Tab 释放 `meshData` 引用与 three.js 资源，只留轻量元数据（id/名称/面数/bbox/颜色直方图/脏标记）；切回时后端返回 DTO。
- `paintDirty` 由全局布尔改为按 Tab；退出确认覆盖所有脏 Tab。
- 关闭 Tab：脏 Tab 先确认；关闭即丢弃该 Tab 的交换文件与内存副本。
- **STL 导入语义**：导入一个 STL = 新建一个 Tab（等价于“加一个盘”），不再替换当前模型。**打开 3MF = 替换全部 Tab**（它是一份工程），有脏 Tab 时先确认。
- Tab 名来源：工程 part 的 `plates[].name` → `model_settings.config` 的 plate/对象名 → 文件名，依次回退。

### 6. 保存 / 导出 UX

- 现有「导出 3MF」对话框：写**全部 Tab**（每盘一个 object）+ 工程 part + 撤销历史。一次操作既交付切片器又是工程文件。
- **调色板与导出选择是工程级、不是 Tab 级**：`project_settings.config` 只有一个 `filament_colour` 数组，`paint_color` 是它的下标（`export/project_config.rs` 模块注释：槽数由该数组长度决定），所以全文件必须共用一套 ≤16 槽调色板与一套机型/工艺/耗材选择。做法：对所有 Tab 的颜色直方图做**一次联合中位切分**（`quantize.rs` 现有算法），导出对话框展示联合调色板与该文件的机型/工艺。因此每个 Tab 要持久化一份**颜色直方图**（颜色→面数，交换出内存后仍能参与预览与量化），上限取 4096 个颜色条目并保留最多的若干，超出部分并入最近色。
- 导入入口（📂 对话框与拖拽共用 `DROPPABLE_EXTENSIONS`，`src/utils/stlDrop.ts`）扩展为 `stl` + `3mf`，移除 `cym`。
- p2 的「打开工程…/保存工程」菜单项不再需要独立格式；「打开」走同一 3MF 导入，「保存」= 导出。

### 7. 回退矩阵

| 输入 | 行为 |
|---|---|
| CYM 本轮导出的 3MF | 全量还原（几何/精确色/标签/名称/高水位/历史/盘） |
| CYM 导出但几何被外部改过（指纹不符） | 基础导入（原生颜色）+ 状态栏提示 |
| Orca/Bambu 保存过的文件（私有 part 丢失） | 基础导入：几何 + `paint_color` 颜色 + 盘 |
| 其他工具 3MF（无 `paint_color`，只有 `basematerials`） | 几何导入，颜色为默认灰（v1 已知限制） |
| 无 `model_settings.config` 的多对象 3MF | 全部合并为单盘 |
| `model_settings` 的盘归属与该对象所处世界坐标矛盾 | **以位置为准**（与切片器显示一致）+ 日志 warn |
| `unit` 非毫米 | 按 3MF unit 换算到毫米（inch/meter/micron/centimeter） |
| 只有 `3D/3dmodel.model`、无 rels | 直接按主模型解析 |

### 8. 性能预算

- 导入 `梁圣.3mf`（13.6MB XML，148k 三角 → 993k 面）：目标 < 5s。
- 导出 1M 面（全部 Tab）：目标 < 5s。
- 换出/换入 1M 面：目标 < 2s（二进制，无 XML）。换出文件体积 ≈ 几何 24MB + 状态 8MB + 历史，DEFLATE 后更小。
- 内存：常驻预算默认按 2 个 1M 面模型（约 1.5GB 估算）设定，可在偏好里调；超出即 LRU 换出。

### 9. 测试

**fixture 策略**（沿用仓库既有先例 `loader.rs` 的 `CYM_PROBE_STL` / `CYM_BENCH_STL`）：

- 真实文件（`梁圣.3mf` 3MB 单盘、`model.3mf` 5.5MB 双盘）**不进 git**：相关用例 `#[ignore]` + 环境变量 `CYM_REAL_3MF`（单盘）/ `CYM_REAL_3MF_MULTI`（双盘）指向本地路径，换机器时按需跑，缺失时 skip 而非 fail。
- 合成 fixture **进 git**（都是几 KB）：单盘未上色、单盘上色（含一个细分三角形）、双盘各一个单位立方体、坏 `project.json`、指纹不符、无 `model_settings.config`、inch 单位、外部 part + component 嵌套。
- 真实文件的“8,693 个取值全解”用例读 `CYM_REAL_3MF`。

后端单元测试：

1. `paint_color` 解码：真实文件 8,693 个取值全部解析；中点缓存一致性（同一共享边只产生一个中点）；子三角形顺序与真实文件的颜色连通性校验。
2. 指纹：一致/不一致两条分支；指纹只差一个字节也判不一致。
3. 多盘写读 round-trip：合成 2 盘文件 → 读回 → 盘点/对象/transform/颜色一致。
4. 真实双盘导入（`CYM_REAL_3MF_MULTI`）：Tab 数 = 2；盘 1 = 148,460 面（全叶子，槽 1–7）、盘 2 = 993,568 面（含细分）；两盘共用 7 色 `filament_colour`。
5. 历史 round-trip：paint + merge + transform 混合 → 保存 → 重开 → `undo` 逐条回退结果与保存前一致；redo 栈同样；`kind=5` 的矩阵逐位相等。
6. 回退矩阵逐行构造最小 3MF 断言（含未知 `kind`、坏 `project.json`、缺 part）。
7. 换出/换入 round-trip：几何/颜色/labels/名称/历史逐字段相等。
8. 单位换算与 transform 烘焙（组件嵌套 + 外部 part），并断言 **part matrix 不被重复应用**（构造 component transform 与 part matrix 相同的文件，导入后顶点只被平移一次）。
9. 指纹规范化：随机 f32 → 写出字符串 → 解析 → 哈希一致（含 `{:.6}` 精度歧义带内取值，如超过 6 位小数、denormal、负零）。
10. 联合调色板：两个 Tab 颜色集合不同，导出后 `filament_colour` 覆盖两者、每面槽位指向正确颜色。

前端测试：

11. Tab 切换状态隔离（每个 Tab 的 meshData/segments/脏标记互不影响）、切回重新拉 DTO。
12. 导入入口扩展名分派（stl/3mf；cym 拒绝）。

真机/集成（**由用户执行**，我产出样本文件与逐条清单）：

13. `梁圣.3mf` 导入后颜色分区与 Orca 显示对照。
14. 合成 2 盘文件在 Snapmaker Orca 中显示两个盘。
15. 导出 → Orca 打开 → 颜色/盘/预设正确；再保存回来 → 按回退矩阵降级且不崩。

### 10. 分期

- **P0（本迭代）**：reader 基础（单盘 + 外部 part + component 递归 + 颜色细分重建）+ 回退矩阵 + writer 写工程 part/指纹 + 指纹不符回退 + 单盘历史持久化（含 kind 5 预留）。
- **P1**：多盘导入（loader 按盘分批，只实例化活跃盘）+ 多盘导出 + `model_settings.config` 写出 + Tab 栏 + per-tab 状态、脏标记、`tab_id` 命令层。
- **P2**：脏 Tab 换出/换入（预算 + LRU + 交换文件含历史）；干净 Tab 已经是 `Source`，无需额外机制。
- **P3（后续）**：`basematerials` 颜色、自动保存、部件身份保留。

## 我已按推荐定下的决策（评审时可推翻）

1. 未涂装面（无 `paint_color` / state 0）→ 槽 1 颜色，不归 CYM 默认灰。理由：再导出不换色。
2. 不实现 `.cym` 读取；p2 `mesh/project.rs` 的二进制编码（含其分节长度预检与面索引校验）**吸收为交换容器格式**，不是复制第二套 codec。
3. 交换文件用私有二进制容器（与工程 part 共用状态编码，但不是 3MF）。
4. 历史持久化预算 64MB，超出丢最旧。
5. 单盘也写 `model_settings.config`。
6. `paint_sources` 不持久化。
7. 指纹用**规范化坐标哈希**（`{:.6}` 格式化后再解析得到的 f32 字节），不用内存原值。
8. 历史条目格式 v1 就预留 `kind=5`（transform + 4×4 矩阵）：p2 的模型变换能力**不并入**，但该功能会按本 spec 重新实现，预留免得届时升版本或丢整段历史。
9. `model_settings.config` 的 part matrix / `source_offset_*` **不参与几何烘焙**（与 component transform 是同一变换的两种写法/数值近等，叠加会双重变换）。
10. 命令层显式 `tab_id`，不靠隐式活跃 Tab；前端 paint 队列按 Tab 隔离并在切换时清空。
11. loader 从第一天按盘分批（`load_plates` / `load_plate`），P1 只实例化活跃盘，干净 Tab 用 `Source` 态从源文件重读，不写交换文件。
12. 调色板与机型/工艺选择是工程级；所有 Tab 的颜色做一次联合中位切分，每 Tab 持久化颜色直方图供预览与量化。
13. 打开 3MF = 替换全部 Tab（脏则确认）；导入 STL = 追加一个 Tab；多文件拖入 v1 仍拒绝。
14. 对象/部件的 `extruder` 元数据（0-based）作为无 `paint_color` 时的基色。
15. 真实大文件不进 git（`#[ignore]` + `CYM_REAL_3MF` / `CYM_REAL_3MF_MULTI` 环境变量），合成小 fixture 进 git。
16. **不合并 `feature_020_p2_paint`**：从该分支只取需要的组件（`mesh/project.rs` 的分节长度预检与面索引校验写法、`OpKind::Transform` 的条目语义），在本分支按本 spec 重新实现；`.cym` 格式本身不实现。
17. 真机（Snapmaker Orca）验证由用户本人做，我负责产出可验证的样本文件与逐条验证清单。

## 已确认的关键输入

1. **p2 分支不并**（用户 2026-11-15 确认）：只取组件，`project.rs` 的硬化校验与 `Transform` 语义作为参考，格式另立。
2. **双盘真实样本**：`~/Downloads/model.3mf`（Orca 2.4.1，2 盘、2 外部 part、7 色共用调色板），已据此校准 `model_settings.config` 写法与 transform 语义。真机验证由用户执行。

## Builder 审查记录（2026-11-15）

独立 subagent（不可对话）复审了本文档与仓库源码，确认：3MF 结构事实、`paint_color` 的 8,693/8,693 解码、993,568 叶子面、`MeshModel` 单网格、`MeshDataDto` 字段、`AppState` 单模型、历史 50 条/256MB、`load_model` 只认 stl、`DROPPABLE_EXTENSIONS` 内容均与文档一致；libslic3r 源码未 vendored 但可联网取，故“port TriangleSelector”不阻塞。

它提出 9 个问题，其中 7 个已由决策 7–15 解决（指纹精度、transform 历史、part matrix、`tab_id`、按盘分批、联合调色板、extruder 基色），2 个已由用户回答并写入「已确认的关键输入」。它同时指出一处文档漂移已修正：`.cym` 后端与「打开/保存工程」菜单只存在于未合入的 p2 分支，本分支没有，因此 §6 的“移除菜单项”表述已改为“不再需要”。

## 风险

1. **多盘写出仍需真机复验**：盘归属的机制已经实测清楚（按世界坐标 + 列间距 1.2×床宽），样本已按此修正，但“修正后的三盘样本在 Orca 里每盘各一个对象”尚未确认。→ 用户复验 `examples/3plate_sample.3mf`。
2. **子三角形几何划分规则**：位语法已用真实文件 8,693/8,693 逐比特验证；`special_side` 与子节点逆序已从 OrcaSlicer `TriangleSelector.cpp` 源码确认，剩余风险仅在“边到子三角形的顶点顺序”实现细节，靠颜色连通性与真机对照兜住。
3. **1M 面 XML 写出/解析性能**：需要基准，否则大模型导出会卡。
4. **Orca 重写会丢私有 part**：用户可能以为“保存了工程”，实际只剩原生颜色 → 界面需明示“该文件已被切片器重写”。
5. **盘原点常量（1.2 倍床宽）来自源码而非文档**：若 Snapmaker 分支改动该常量，对象仍会落在正确盘内（只要内缩量大于误差），但贴着盘边放的对象可能落到相邻盘。写出时保持至少 20mm 内缩。

## 验证记录（本轮只读 spike，未写任何产品代码）

**单盘 `梁圣.3mf`**：OPC 结构、外部 part、`model_settings.config` 盘结构、`build` transform、`filament_colour` 4 色调色板、`paint_color` 状态机解码（8,693/8,693 通过）、细分规模（148,460 → 993,568 面，6.7×，单三角最多 970 叶子）与逐色面数。

**双盘 `model.3mf`**：2 个 `<plate>` / 2 个 `build/item` / 2 个外部 part；盘 1 = 148,460 面全叶子涂装（槽 1–7 真实用到）、盘 2 = 993,568 面含细分；两盘共用 7 色 `filament_colour`（`#8A8A8A #FFFFFF #000000 #D2B48C #00B0F0 #C99D65 #949262`）；`filament_maps` 长度 = 7；`assemble_item` 与 `build/item` 的 transform 数值不同（证实 assemble 不是打印位置）；`<part>` matrix 与 component transform 数值近等（证实不叠加）。

**三盘样本真机验证（两轮失败，规则已定死）**：
1. 第一轮：3 个盘都建出来了（盘名/对象名/5 色耗材/彩色对象都正确），但**三个对象全在盘 1**。→ 盘归属按世界坐标判定，不是 `model_instance` 列表。
2. 第二轮：按“盘排成一行”把对象放到 x=60/384.6/709.2 → 盘 1/2 各一个对象，**第三个对象落在所有盘之外**（Orca 红框标越界）。→ 盘是**网格**：3 盘时 `cols=2`，盘 3 在 `row=1, col=0`，原点 y 为负。
3. 第三轮：按 `cols = ceil(sqrt(count))`、`origin = (col·W·1.2, -row·D·1.2)` 重新生成（对象位于 (60,110)、(384.6,110)、(60,-215.2)）→ **真机通过**：三个盘各一个对象，位置/颜色/对象名/5 色耗材全部正确。多盘写出路径由此得到端到端验证。

由此确立“导入归一化到盘局部、导出按盘加回 origin”的工程约定（见「约束」），并把网格公式固定为兼容契约。
