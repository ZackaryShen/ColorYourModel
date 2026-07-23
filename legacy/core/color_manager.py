"""颜色管理模块

管理区域颜色分配、调色板、颜色历史记录。
"""

from dataclasses import dataclass, field
from typing import Optional

import numpy as np


@dataclass
class ColorEntry:
    """颜色条目：记录某个区域的颜色信息"""

    segment_id: int
    color: tuple[int, int, int]  # RGB (0-255)
    name: str = ""
    alpha: int = 255


# 3D 打印常用调色板（基于 Bambu Lab AMS 常用色系）
DEFAULT_PALETTE: dict[str, tuple[int, int, int]] = {
    "白色": (255, 255, 255),
    "黑色": (30, 30, 30),
    "红色": (200, 30, 30),
    "橙色": (230, 120, 20),
    "黄色": (240, 210, 40),
    "绿色": (40, 170, 60),
    "蓝色": (30, 80, 200),
    "紫色": (130, 40, 170),
    "粉色": (230, 140, 160),
    "灰色": (150, 150, 150),
    "棕色": (140, 80, 40),
    "肤色": (240, 200, 170),
    "金属银": (192, 192, 192),
    "金属金": (212, 175, 55),
    "透明": (255, 255, 255),
}


class ColorManager:
    """颜色管理器：为网格分割区域分配和管理颜色"""

    def __init__(self) -> None:
        self._assignments: dict[int, ColorEntry] = {}
        self._palette: dict[str, tuple[int, int, int]] = dict(DEFAULT_PALETTE)
        self._history: list[ColorEntry] = []

    def assign_color(
        self,
        segment_id: int,
        color: tuple[int, int, int],
        name: str = "",
        alpha: int = 255,
    ) -> ColorEntry:
        """为指定区域分配颜色

        Args:
            segment_id: 区域 ID（来自 MeshSegmenter 的标签值）
            color: RGB 颜色元组 (0-255)
            name: 颜色名称（可选）
            alpha: 不透明度 (0-255)

        Returns:
            创建的 ColorEntry 对象
        """
        entry = ColorEntry(segment_id=segment_id, color=color, name=name, alpha=alpha)
        self._assignments[segment_id] = entry
        self._history.append(entry)
        return entry

    def get_color(self, segment_id: int) -> Optional[tuple[int, int, int]]:
        """获取指定区域的颜色

        Returns:
            RGB 元组，若未分配则返回 None
        """
        entry = self._assignments.get(segment_id)
        return entry.color if entry else None

    def get_face_colors(self, labels: np.ndarray) -> np.ndarray:
        """为所有面片生成 RGBA 颜色数组（用于 3D 可视化渲染）

        Args:
            labels: 面片标签数组，shape=(n_faces,)

        Returns:
            RGBA 颜色数组，shape=(n_faces, 4)，值域 [0, 1]
        """
        n_faces = len(labels)
        colors = np.ones((n_faces, 4), dtype=np.float32)  # 默认白色

        for seg_id, entry in self._assignments.items():
            mask = labels == seg_id
            r, g, b = entry.color
            colors[mask] = [r / 255.0, g / 255.0, b / 255.0, entry.alpha / 255.0]

        return colors

    def auto_assign(self, n_segments: int) -> None:
        """自动为 n_segments 个区域分配不同颜色（轮选调色板）

        Args:
            n_segments: 区域数量
        """
        palette_colors = list(self._palette.values())
        for i in range(n_segments):
            color = palette_colors[i % len(palette_colors)]
            self.assign_color(i, color, name=list(self._palette.keys())[i % len(palette_colors)])

    @property
    def assignments(self) -> dict[int, ColorEntry]:
        return dict(self._assignments)

    @property
    def palette(self) -> dict[str, tuple[int, int, int]]:
        return dict(self._palette)

    def clear(self) -> None:
        self._assignments.clear()
        self._history.clear()
