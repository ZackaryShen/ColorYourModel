"""网格加载与预处理模块"""

from pathlib import Path
from typing import Optional

import numpy as np
import trimesh


class MeshLoader:
    """STL/OBJ 网格加载器，支持文件读取与基础预处理"""

    def __init__(self) -> None:
        self._mesh: Optional[trimesh.Trimesh] = None
        self._filepath: Optional[Path] = None

    def load(self, filepath: str | Path) -> trimesh.Trimesh:
        """加载网格文件（支持 STL、OBJ、PLY 等 trimesh 支持的格式）

        Args:
            filepath: 模型文件路径

        Returns:
            加载后的 trimesh.Trimesh 对象

        Raises:
            FileNotFoundError: 文件不存在
            ValueError: 文件格式不支持或文件损坏
        """
        filepath = Path(filepath)
        if not filepath.exists():
            raise FileNotFoundError(f"文件不存在: {filepath}")

        try:
            mesh = trimesh.load(str(filepath), force="mesh")
        except Exception as e:
            raise ValueError(f"无法加载文件 {filepath}: {e}") from e

        if not isinstance(mesh, trimesh.Trimesh):
            raise ValueError(f"文件 {filepath} 未能解析为三角网格")

        self._mesh = mesh
        self._filepath = filepath
        return mesh

    def preprocess(
        self,
        mesh: trimesh.Trimesh,
        merge_vertices: bool = True,
        remove_degenerate: bool = True,
    ) -> trimesh.Trimesh:
        """预处理网格：合并重复顶点、移除退化三角形

        Args:
            mesh: 待处理的网格
            merge_vertices: 是否合并重合顶点
            remove_degenerate: 是否移除面积为零的三角形

        Returns:
            处理后的网格
        """
        if merge_vertices:
            mesh.merge_vertices()
        if remove_degenerate:
            mesh.remove_degenerate_faces()
        mesh.fix_normals()
        return mesh

    @property
    def mesh(self) -> Optional[trimesh.Trimesh]:
        return self._mesh

    @property
    def filepath(self) -> Optional[Path]:
        return self._filepath
