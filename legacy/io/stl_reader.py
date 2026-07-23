"""STL 文件读取模块

封装 trimesh 的 STL 读取功能，提供标准化的网格加载接口。
"""

from pathlib import Path
from typing import Optional

import trimesh


class STLReader:
    """STL 文件读取器"""

    @staticmethod
    def read(filepath: str | Path) -> trimesh.Trimesh:
        """读取 STL 文件

        Args:
            filepath: STL 文件路径

        Returns:
            三角网格对象
        """
        filepath = Path(filepath)
        if not filepath.suffix.lower() == ".stl":
            raise ValueError(f"不支持的文件格式: {filepath.suffix}，仅支持 .stl")
        if not filepath.exists():
            raise FileNotFoundError(f"文件不存在: {filepath}")

        mesh = trimesh.load(str(filepath), file_type="stl", force="mesh")
        return mesh

    @staticmethod
    def get_info(filepath: str | Path) -> dict:
        """获取 STL 文件基本信息（不加载完整网格）

        Args:
            filepath: STL 文件路径

        Returns:
            包含文件大小、面数等信息的字典
        """
        filepath = Path(filepath)
        file_size = filepath.stat().st_size if filepath.exists() else 0
        return {
            "filepath": str(filepath),
            "file_size_mb": round(file_size / (1024 * 1024), 2),
            "format": "STL",
        }
