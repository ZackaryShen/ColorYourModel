"""核心算法模块：网格加载、分割与颜色管理"""

from src.core.mesh_loader import MeshLoader
from src.core.segmentation import MeshSegmenter
from src.core.color_manager import ColorManager

__all__ = ["MeshLoader", "MeshSegmenter", "ColorManager"]
