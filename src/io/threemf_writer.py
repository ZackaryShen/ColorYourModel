"""3MF 文件写入模块

生成符合 3MF Materials & Colors Extension 规范的彩色 3MF 文件。
3MF 本质 ZIP 包，内含 XML 格式的模型定义。

参考规范：
- 3MF Core Specification: https://3mf.io/spec_core/
- 3MF Materials and Colors Extension: https://3mf.io/spec_m_c/
"""

import zipfile
from pathlib import Path
from typing import Optional
from xml.etree import ElementTree as ET

import numpy as np
import trimesh

# 3MF XML 命名空间
NS_3MF = "http://schemas.microsoft.com/3dmanufacturing/2013/01"
NS_MAT = "http://schemas.microsoft.com/3dmanufacturing/material/2015/02"


class ThreeMFWriter:
    """3MF 文件写入器：将带颜色的网格导出为 3MF 格式"""

    def write(
        self,
        mesh: trimesh.Trimesh,
        face_colors: np.ndarray,
        output_path: str | Path,
    ) -> Path:
        """将带颜色的网格写入 3MF 文件

        Args:
            mesh: 三角网格对象
            face_colors: 每个面的 RGBA 颜色，shape=(n_faces, 4)，值域 [0, 1]
            output_path: 输出文件路径

        Returns:
            写入的文件路径
        """
        output_path = Path(output_path)
        output_path.parent.mkdir(parents=True, exist_ok=True)

        # 构建 XML 模型
        model_xml = self._build_model_xml(mesh, face_colors)

        # 写入 ZIP（3MF 格式）
        with zipfile.ZipFile(output_path, "w", zipfile.ZIP_DEFLATED) as zf:
            zf.writestr("3D/3dmodel.model", model_xml)
            zf.writestr("[Content_Types].xml", self._build_content_types())
            zf.writestr("_rels/.rels", self._build_rels())

        return output_path

    def _build_model_xml(self, mesh: trimesh.Trimesh, face_colors: np.ndarray) -> str:
        """构建 3MF 模型 XML 内容"""
        # 注册命名空间
        ET.register_namespace("", NS_3MF)
        ET.register_namespace("m", NS_MAT)

        model = ET.Element("model", {
            "unit": "millimeter",
            "xml:lang": "en-US",
        })

        resources = ET.SubElement(model, "resources")

        # 构建颜色组
        basematerials = ET.SubElement(resources, f"{{{NS_MAT}}}basematerials", id="1")
        unique_colors = {}
        for rgba in face_colors:
            color_key = tuple(rgba)
            if color_key not in unique_colors:
                idx = len(unique_colors)
                unique_colors[color_key] = idx
                r, g, b, a = rgba
                color_str = f"#{int(r*255):02X}{int(g*255):02X}{int(b*255):02X}"
                ET.SubElement(basematerials, f"{{{NS_MAT}}}base", {
                    "name": f"color_{idx}",
                    "displaycolor": color_str,
                })

        # 构建网格对象
        obj = ET.SubElement(resources, "object", {
            "id": "2",
            "type": "model",
            f"{{{NS_MAT}}}basematerials": "1",
        })
        mesh_elem = ET.SubElement(obj, "mesh")

        # 顶点
        vertices_elem = ET.SubElement(mesh_elem, "vertices")
        for vertex in mesh.vertices:
            ET.SubElement(vertices_elem, "vertex", {
                "x": f"{vertex[0]:.6f}",
                "y": f"{vertex[1]:.6f}",
                "z": f"{vertex[2]:.6f}",
            })

        # 三角面片（带颜色索引）
        triangles_elem = ET.SubElement(mesh_elem, "triangles")
        for i, face in enumerate(mesh.faces):
            color_idx = unique_colors[tuple(face_colors[i])]
            ET.SubElement(triangles_elem, "triangle", {
                "v1": str(face[0]),
                "v2": str(face[1]),
                "v3": str(face[2]),
                "pid": "1",
                "p1": str(color_idx),
            })

        # 构建项
        build = ET.SubElement(model, "build")
        ET.SubElement(build, "item", {"objectid": "2"})

        return ET.tostring(model, encoding="unicode", xml_declaration=True)

    @staticmethod
    def _build_content_types() -> str:
        return (
            '<?xml version="1.0" encoding="UTF-8"?>\n'
            '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">\n'
            '  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>\n'
            '  <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>\n'
            '</Types>'
        )

    @staticmethod
    def _build_rels() -> str:
        return (
            '<?xml version="1.0" encoding="UTF-8"?>\n'
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">\n'
            '  <Relationship Target="/3D/3dmodel.model" Id="rel0" '
            'Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>\n'
            '</Relationships>'
        )
