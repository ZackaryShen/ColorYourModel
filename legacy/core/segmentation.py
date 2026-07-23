"""网格分割算法模块

支持基于曲率、锐边检测和图割的自动网格分割，
将连续的三角面片聚合为具有语义意义的逻辑区域。
"""

from typing import Optional

import numpy as np
import trimesh
from scipy.sparse import csr_matrix
from scipy.sparse.csgraph import connected_components


class MeshSegmenter:
    """网格分割器：将三角网格划分为可独立上色的逻辑区域"""

    def __init__(self, mesh: trimesh.Trimesh) -> None:
        self._mesh = mesh
        self._labels: Optional[np.ndarray] = None  # 每个面的区域标签
        self._n_segments: int = 0

    def segment_by_dihedral_angle(self, angle_threshold: float = 30.0) -> np.ndarray:
        """基于二面角（相邻面法线夹角）进行分割

        当两个相邻三角形面片之间的二面角超过阈值时，认为它们属于不同区域。
        适用于有明显棱角的模型（如机械零件、建筑模型）。

        Args:
            angle_threshold: 二面角阈值（度），超过此角度的边被视为分割边界

        Returns:
            每个面对应的区域标签数组，shape=(n_faces,)
        """
        mesh = self._mesh
        n_faces = len(mesh.faces)

        # 构建面邻接图
        adjacency = mesh.face_adjacency
        normals = mesh.face_normals

        # 计算每条邻接边的二面角
        adj_normals = normals[adjacency]
        dot_products = np.sum(adj_normals[:, 0] * adj_normals[:, 1], axis=1)
        dot_products = np.clip(dot_products, -1.0, 1.0)
        dihedral_angles = np.degrees(np.arccos(dot_products))

        # 超过阈值的边断开连接
        connected_mask = dihedral_angles < angle_threshold
        rows = np.concatenate([adjacency[connected_mask, 0], adjacency[connected_mask, 1]])
        cols = np.concatenate([adjacency[connected_mask, 1], adjacency[connected_mask, 0]])
        data = np.ones(len(rows), dtype=np.int32)
        graph = csr_matrix((data, (rows, cols)), shape=(n_faces, n_faces))

        n_components, labels = connected_components(graph, directed=False)
        self._labels = labels
        self._n_segments = n_components
        return labels

    def segment_by_curvature(self, curvature_threshold: float = 0.5) -> np.ndarray:
        """基于面曲率进行分割（待实现）

        利用顶点曲率估算每个面片的弯曲程度，
        将高曲率区域（如细节、装饰）与平坦区域分离。

        Args:
            curvature_threshold: 曲率阈值

        Returns:
            每个面对应的区域标签数组
        """
        raise NotImplementedError("基于曲率的分割算法尚未实现")

    def segment_by_height(self, n_bins: int = 5) -> np.ndarray:
        """按高度分层分割（简单基线方法）

        将模型沿 Z 轴等分为 n_bins 个区域。
        适用于简单的按层上色需求。

        Args:
            n_bins: 高度分层数量

        Returns:
            每个面对应的区域标签数组
        """
        mesh = self._mesh
        face_centers = mesh.triangles_center  # (n_faces, 3)
        z_coords = face_centers[:, 2]
        z_min, z_max = z_coords.min(), z_coords.max()

        if z_max - z_min < 1e-8:
            labels = np.zeros(len(mesh.faces), dtype=np.int32)
        else:
            bins = np.linspace(z_min, z_max + 1e-8, n_bins + 1)
            labels = np.digitize(z_coords, bins[1:-1]).astype(np.int32)

        self._labels = labels
        self._n_segments = int(labels.max()) + 1
        return labels

    def merge_segments(self, label_a: int, label_b: int) -> np.ndarray:
        """合并两个区域

        Args:
            label_a: 区域 A 的标签
            label_b: 区域 B 的标签

        Returns:
            更新后的标签数组
        """
        if self._labels is None:
            raise RuntimeError("请先调用分割方法")
        self._labels[self._labels == label_b] = label_a
        self._n_segments = len(np.unique(self._labels))
        return self._labels

    @property
    def labels(self) -> Optional[np.ndarray]:
        return self._labels

    @property
    def n_segments(self) -> int:
        return self._n_segments
