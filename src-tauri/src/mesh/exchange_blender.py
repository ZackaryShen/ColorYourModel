"""Convert Blender's static surfaces with an indexed triangle/RGBA stream."""

import struct
import sys

import bpy
import numpy as np


MAGIC = b"CYMMESH1"
DISPLAY_COLOR = "displayColor"
DISPLAY_OPACITY = "displayOpacity"


def run(operator, **options):
    if "FINISHED" not in operator(**options):
        raise RuntimeError(f"{operator.idname()} did not finish")


def material_color(material, default_color):
    if material is None:
        return default_color
    if material.node_tree is not None:
        shader = next(
            (node for node in material.node_tree.nodes if node.type == "BSDF_PRINCIPLED"),
            None,
        )
        if shader is not None and not shader.inputs["Base Color"].is_linked:
            inputs = shader.inputs
            return (*inputs["Base Color"].default_value[:3], inputs["Alpha"].default_value)
    return tuple(material.diffuse_color)


def property_array(collection, name, dtype, width=1):
    values = np.empty(len(collection) * width, dtype=dtype)
    collection.foreach_get(name, values)
    return values.reshape((-1, width)) if width != 1 else values


def triangle_colors(obj, triangles, default_color):
    mesh = obj.data
    attrs = mesh.color_attributes
    color = attrs.get(DISPLAY_COLOR) or attrs.active_color or next(iter(attrs), None)
    opacity = mesh.attributes.get(DISPLAY_OPACITY)
    if opacity is not None and (
        opacity.data_type != "FLOAT" or opacity.domain not in {"POINT", "CORNER", "FACE"}
    ):
        opacity = None

    # Blender stores scene-linear color. Reduce varying corner/vertex colors
    # to one color per triangle before encoding sRGB, matching CYM face colors.
    if (color is not None and color.domain == "CORNER") or (
        opacity is not None and opacity.domain == "CORNER"
    ):
        corners = property_array(mesh.loop_triangles, "loops", np.int32, 3)
    if color is None or (opacity is not None and opacity.domain == "FACE"):
        polygons = property_array(mesh.loop_triangles, "polygon_index", np.int32)
    if color is None:
        palette = np.asarray(
            [material_color(slot.material, default_color) for slot in obj.material_slots]
            + [default_color], dtype=np.float32,
        )
        indices = property_array(mesh.polygons, "material_index", np.int32)
        rgba = palette[np.minimum(indices[polygons], len(palette) - 1)]
    else:
        values = property_array(color.data, "color", np.float32, 4)
        rgba = values[corners if color.domain == "CORNER" else triangles].mean(axis=1)
    if opacity is not None:
        values = property_array(opacity.data, "value", np.float32)
        if opacity.domain == "FACE":
            rgba[:, 3] = values[polygons]
        else:
            indices = corners if opacity.domain == "CORNER" else triangles
            rgba[:, 3] = values[indices].mean(axis=1)
    rgba = np.clip(np.nan_to_num(rgba, copy=False), 0, 1)
    rgb = rgba[:, :3]
    rgba[:, :3] = np.where(rgb <= 0.0031308, rgb * 12.92, rgb ** (1 / 2.4) * 1.055 - 0.055)
    return np.floor(rgba * 255 + 0.5).astype(np.uint8)


def write_mesh(path, default_color, max_faces):
    depsgraph = bpy.context.evaluated_depsgraph_get()
    face_count = 0
    with open(path, "wb") as output:
        output.write(MAGIC)
        for instance in depsgraph.object_instances:
            obj = instance.object
            if obj.type != "MESH" or not obj.data.polygons:
                continue
            mesh = obj.data
            mesh.calc_loop_triangles()
            count = len(mesh.loop_triangles)
            if count == 0:
                continue
            face_count += count
            if face_count > max_faces or len(mesh.vertices) > max_faces * 3:
                raise RuntimeError("Blender mesh exceeds the exchange size limit")

            triangles = property_array(mesh.loop_triangles, "vertices", np.int32, 3)
            rgba = triangle_colors(obj, triangles, default_color)
            points = property_array(mesh.vertices, "co", np.float32, 3)
            used = np.zeros(len(points), dtype=bool)
            used[triangles] = True
            if not used.all():
                # Drop loose vertices while retaining coincident but distinct indices.
                triangles = np.cumsum(used, dtype=np.uint32)[triangles] - 1
                points = points[used]

            matrix = instance.matrix_world
            world = np.asarray(matrix, dtype=np.float64)
            points = 1000 * (points @ world[:3, :3].T + world[:3, 3])
            if matrix.to_3x3().determinant() < 0:
                triangles = triangles[:, [0, 2, 1]]
            output.write(struct.pack("<II", len(points), count))
            points.astype("<f4", copy=False).tofile(output)
            triangles.astype("<u4", copy=False).tofile(output)
            rgba.tofile(output)
        output.write(struct.pack("<II", 0, 0))
    if face_count == 0:
        raise RuntimeError("The file contains no mesh faces")


def read_mesh(path, max_faces):
    # Rust stages exactly one block of Z-up millimetres, indices, and face RGBA.
    with open(path, "rb") as source:
        if source.read(len(MAGIC)) != MAGIC:
            raise RuntimeError("Invalid Blender mesh data header")
        point_count, face_count = struct.unpack("<II", source.read(8))
        if not 0 < point_count <= max_faces * 3 or not 0 < face_count <= max_faces:
            raise RuntimeError("Blender mesh exceeds the exchange size limit")
        points = np.fromfile(source, dtype="<f4", count=point_count * 3) / 1000
        indices = np.fromfile(source, dtype="<i4", count=face_count * 3)
        rgba = np.fromfile(source, dtype=np.uint8, count=face_count * 4).reshape((-1, 4))
        if len(points) != point_count * 3 or len(indices) != face_count * 3 or len(rgba) != face_count:
            raise RuntimeError("Incomplete Blender mesh data")

    mesh = bpy.data.meshes.new("ColorYourModel")
    mesh.vertices.add(point_count)
    mesh.vertices.foreach_set("co", points)
    mesh.loops.add(face_count * 3)
    mesh.loops.foreach_set("vertex_index", indices)
    mesh.polygons.add(face_count)
    mesh.polygons.foreach_set("loop_start", np.arange(0, face_count * 3, 3, dtype=np.int32))
    mesh.update(calc_edges=True)
    bpy.context.scene.collection.objects.link(bpy.data.objects.new(mesh.name, mesh))

    color = mesh.color_attributes.new(name=DISPLAY_COLOR, type="BYTE_COLOR", domain="CORNER")
    corner_rgba = np.repeat(rgba.astype(np.float32) / 255, 3, axis=0)
    color.data.foreach_set("color_srgb", corner_rgba.ravel())
    mesh.color_attributes.active_color = color
    mesh.color_attributes.render_color_index = mesh.color_attributes.find(color.name)
    return mesh, rgba[:, 3]


def import_exchange(source, target, extension, default_color, max_faces):
    if extension == "fbx":
        run(bpy.ops.import_scene.fbx, filepath=source, use_anim=False,
            use_image_search=False, use_custom_props=False, use_custom_normals=False)
    else:
        run(bpy.ops.wm.usd_import, filepath=source, import_cameras=False,
            import_curves=False, import_lights=False, import_volumes=False,
            import_skeletons=False, import_blendshapes=False, import_points=False,
            apply_unit_conversion_scale=True, import_textures_mode="IMPORT_NONE",
            read_mesh_uvs=False, set_frame_range=False, create_world_material=False)
    write_mesh(target, default_color, max_faces)


def export_exchange(source, target, extension, max_faces):
    mesh, opacity = read_mesh(source, max_faces)
    if extension == "fbx":
        run(bpy.ops.export_scene.fbx, filepath=target, object_types={"MESH"},
            check_existing=False, bake_anim=False, use_triangles=True,
            apply_unit_scale=True, apply_scale_options="FBX_SCALE_ALL",
            colors_type="SRGB", prioritize_active_color=True)
    else:
        # USD's standard displayColor has only RGB. Preserve alpha as displayOpacity.
        if np.any(opacity < 255):
            attribute = mesh.attributes.new(name=DISPLAY_OPACITY, type="FLOAT", domain="FACE")
            attribute.data.foreach_set("value", opacity.astype(np.float32) / 255)
        run(bpy.ops.wm.usd_export, filepath=target, convert_scene_units="METERS",
            check_existing=False, export_mesh_colors=True, export_materials=False,
            export_uvmaps=False, export_custom_properties=False)


def main():
    if bpy.app.version < (4, 5, 0):
        raise RuntimeError("Blender 4.5 or newer is required for FBX and USD exchange")
    operation, source, target, extension, gray, limit = sys.argv[sys.argv.index("--") + 1:]
    bpy.ops.wm.read_factory_settings(use_empty=True)
    units = bpy.context.scene.unit_settings
    units.system = "METRIC"
    units.scale_length = 1.0
    if operation == "import":
        default_color = (float(gray), float(gray), float(gray), 1.0)
        import_exchange(source, target, extension, default_color, int(limit))
    else:
        export_exchange(source, target, extension, int(limit))


if __name__ == "__main__":
    main()
