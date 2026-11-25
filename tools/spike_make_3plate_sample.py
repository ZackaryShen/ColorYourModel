#!/usr/bin/env python3
"""SPIKE (throwaway, not product code): writes examples/3plate_sample.3mf — a
3-plate package following docs/superpowers/specs/2026-11-15-3mf-project-format-design.md,
used to verify in Snapmaker Orca that our multi-plate + private-part layout is
readable. Regenerate with: python3 tools/spike_make_3plate_sample.py
"""
import json, math, struct, zipfile, collections

BASE = "/Users/zackary/selfDir/programing/ColorYourModel-worktrees/feature-020-p0-engineering/examples/paint_color_sample.3mf"
OUT = "/Users/zackary/selfDir/programing/ColorYourModel-worktrees/feature-020-p0-engineering/examples/3plate_sample.3mf"

PALETTE = ["#E03131", "#1971C2", "#2F9E44", "#F08C00", "#7048E8"]  # 5 slots

# ---------- paint_color encoder (mirror of src-tauri/src/export/paint_color.rs) ----------
def encode_paint_color(slot):
    assert 1 <= slot <= 16
    bits = []
    def push(v, n):
        for i in range(n): bits.append((v >> i) & 1)
    push(0, 2)                      # split_sides = 0 (leaf)
    if slot < 3:
        push(slot, 2)
    else:
        push(0b11, 2)
        rem = slot - 3
        while True:
            if rem < 0b1111:
                push(rem, 4); break
            push(0b1111, 4); rem -= 0b1111
    while len(bits) % 4: bits.append(0)
    nibbles = [c[0] | c[1] << 1 | c[2] << 2 | c[3] << 3 for c in (bits[i:i+4] for i in range(0, len(bits), 4))]
    nibbles.reverse()
    return "".join(f"{n:X}" for n in nibbles)

# ---------- meshes: (vertices, triangles, slot per triangle) ----------
def cube(s=20.0):
    h = s / 2
    v = [[-h,-h,0],[h,-h,0],[h,h,0],[-h,h,0],
         [-h,-h,s],[h,-h,s],[h,h,s],[-h,h,s]]
    f = [[0,2,1],[0,3,2],   # bottom
         [4,5,6],[4,6,7],   # top
         [0,1,5],[0,5,4],   # front
         [2,3,7],[2,7,6],   # back
         [3,0,4],[3,4,7],   # left
         [1,2,6],[1,6,5]]   # right
    slots = [2,2, 1,1, 3,3, 3,3, 3,3, 3,3]
    return v, f, slots

def tetra(r=14.0, h=22.0):
    v = [[r*math.cos(math.radians(a)), r*math.sin(math.radians(a)), 0] for a in (90, 210, 330)]
    v.append([0, 0, h])
    f = [[0,1,3],[1,2,3],[2,0,3],[0,2,1]]
    slots = [1,1,1,5]
    return v, f, slots

def hex_prism(r=9.0, h=30.0):
    ring = [[r*math.cos(math.radians(60*i)), r*math.sin(math.radians(60*i))] for i in range(6)]
    v = [[x,y,0] for x,y in ring] + [[x,y,h] for x,y in ring]
    f, slots = [], []
    for i in range(6):
        j = (i+1) % 6
        f += [[i, j, j+6], [i, j+6, i+6]]; slots += [4,4]
    for i in range(1, 5):                       # top fan
        f.append([6, 6+i, 6+i+1]); slots.append(5)
    for i in range(1, 5):                       # bottom fan
        f.append([0, i+1, i]); slots.append(2)
    return v, f, slots

# Orca assigns an object to the plate whose WORLD RECTANGLE contains it.
# Plate grid layout (OrcaSlicer PartPlate.hpp / PartPlate.cpp):
#   cols = round(sqrt(count)), +1 if sqrt(count) > round(sqrt(count))
#   row = index / cols, col = index % cols
#   origin = (col * W * 1.2, -row * D * 1.2)      # rows go towards -Y
#   pitch factor 1.2 = 1 + LOGICAL_PART_PLATE_GAP (1/5)
# Verified against ~/Downloads/model.3mf: object 4 at x=459.6 sits in plate 2's
# cell [324.6, 595.1] for a 270.5 bed.
def bed_size():
    with zipfile.ZipFile(BASE) as z:
        cfg = json.loads(z.read("Metadata/project_settings.config"))
    xs, ys = [], []
    for pt in cfg.get("printable_area", []):
        x, _, y = pt.partition("x")
        xs.append(float(x)); ys.append(float(y))
    return (max(xs) if xs else 220.0, max(ys) if ys else 220.0)

BED_W, BED_D = bed_size()

def plate_origin(index0):
    import math
    count = len(PLATES)
    root = math.sqrt(count)
    cols = int(root) if root == int(root) else int(root) + 1
    row, col = index0 // cols, index0 % cols
    return (col * BED_W * 1.2, -row * BED_D * 1.2)

LOCAL_X, LOCAL_Y = 60.0, 110.0   # inset from the plate's own origin

def plate_xy(index0):
    ox, oy = plate_origin(index0)
    return ox + LOCAL_X, oy + LOCAL_Y

# plate -> (object id, name, mesh, placement x,y) -- x,y filled below
PLATES = [
    (1, "Plate1-Cube",   cube()),
    (2, "Plate2-Tetra",  tetra()),
    (3, "Plate3-Prism",  hex_prism()),
]
PLATES = [(oid, name, mesh) + plate_xy(i) for i, (oid, name, mesh) in enumerate(PLATES)]

def fmt(x):
    return f"{x:.6f}".rstrip("0").rstrip(".") or "0"

def model_xml():
    parts = ['<?xml version="1.0" encoding="UTF-8"?>',
             '<model unit="millimeter" xml:lang="en-US" '
             'xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" '
             'xmlns:BambuStudio="http://schemas.bambulab.com/package/2021">',
             '<metadata name="Application">ColorYourModel-spike</metadata>',
             '<resources>']
    for oid, name, (verts, faces, slots), _, _ in PLATES:
        parts.append(f'<object id="{oid}" type="model"><mesh><vertices>')
        for x, y, z in verts:
            parts.append(f'<vertex x="{fmt(x)}" y="{fmt(y)}" z="{fmt(z)}"/>')
        parts.append('</vertices><triangles>')
        for tri, slot in zip(faces, slots):
            parts.append(f'<triangle v1="{tri[0]}" v2="{tri[1]}" v3="{tri[2]}" '
                         f'paint_color="{encode_paint_color(slot)}"/>')
        parts.append('</triangles></mesh></object>')
    parts.append('</resources><build>')
    for oid, _, _, px, py in PLATES:
        parts.append(f'<item objectid="{oid}" transform="1 0 0 0 1 0 0 0 1 {fmt(px)} {fmt(py)} 0" printable="1"/>')
    parts.append('</build></model>')
    return "".join(parts)

def model_settings_xml():
    p = ['<?xml version="1.0" encoding="UTF-8"?>', '<config>']
    for oid, name, _, _, _ in PLATES:
        p.append(f'<object id="{oid}"><metadata key="name" value="{name}"/>'
                 f'<metadata key="extruder" value="0"/>'
                 f'<part id="{oid}" subtype="normal_part"><metadata key="name" value="{name}"/>'
                 f'<metadata key="matrix" value="1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1"/>'
                 f'<metadata key="source_file" value="{name}.stl"/>'
                 f'<metadata key="source_object_id" value="0"/>'
                 f'<metadata key="source_volume_id" value="0"/>'
                 f'<metadata key="source_offset_x" value="135.0"/>'
                 f'<metadata key="source_offset_y" value="135.0"/>'
                 f'<metadata key="source_offset_z" value="0"/>'
                 f'<mesh_stat edges_fixed="0" degenerate_facets="0" facets_removed="0" '
                 f'facets_reversed="0" backwards_edges="0"/></part></object>')
    maps = " ".join(["1"] * len(PALETTE))
    for pid, (oid, name, _, _, _) in enumerate(PLATES, start=1):
        p.append(f'<plate><metadata key="plater_id" value="{pid}"/>'
                 f'<metadata key="plater_name" value="{name}"/>'
                 f'<metadata key="locked" value="false"/>'
                 f'<metadata key="filament_map_mode" value="Auto For Flush"/>'
                 f'<metadata key="filament_maps" value="{maps}"/>'
                 f'<model_instance><metadata key="object_id" value="{oid}"/>'
                 f'<metadata key="instance_id" value="0"/>'
                 f'<metadata key="identify_id" value="{100+pid}"/></model_instance></plate>')
    p.append('<assemble>')
    for oid, _, _, px, py in PLATES:
        p.append(f'<assemble_item object_id="{oid}" instance_id="0" '
                 f'transform="1 0 0 0 1 0 0 0 1 {fmt(px)} {fmt(py)} 0" offset="0 0 0" />')
    p.append('</assemble></config>')
    return "".join(p)

def project_settings():
    with zipfile.ZipFile(BASE) as z:
        base = json.loads(z.read("Metadata/project_settings.config"))
    slots = len(PALETTE)
    out = dict(base)
    for k, v in list(out.items()):
        if isinstance(v, list) and len(v) == 6:
            out[k] = (v * slots)[:slots]           # old sample was 6 slots
    out["filament_colour"] = PALETTE
    out["filament_settings_id"] = ["Generic PLA"] * slots
    out["filament_ids"] = ["GFSL991"] * slots
    out["inherits_group"] = [out["print_settings_id"]] + ["Generic PLA"] * slots + [out["printer_settings_id"]]
    return json.dumps(out, indent=1, ensure_ascii=False)

def cym_parts():
    """Private engineering parts: exactly what the design writes."""
    plates = []
    files = {}
    for pid, (oid, name, (verts, faces, slots), _, _) in enumerate(PLATES, start=1):
        labels = b"".join(struct.pack("<I", 0) for _ in faces)
        rgba = b"".join(bytes([0x8A, 0x8A, 0x8A, 0xFF]) for _ in faces)
        names = struct.pack("<I", 1) + struct.pack("<I", 100000) + struct.pack("<H", 5) + "盘区一".encode()
        binb = (b"CYMST1\x01" + struct.pack("<I", len(faces)) + labels + rgba
                + struct.pack("<I", 100001) + names)
        files[f"Metadata/cym/plate_{pid}.bin"] = binb
        files[f"Metadata/cym/history_{pid}.bin"] = struct.pack("<II", 0, 0)  # 0 undo, 0 redo
        plates.append({
            "plater_id": pid, "name": name, "object_ids": [oid],
            "state": f"plate_{pid}.bin", "history": f"history_{pid}.bin",
            "fingerprint": {"vertex_count": len(verts), "face_count": len(faces),
                            "hash": "fnv1a64:0000000000000000"},
        })
    files["Metadata/cym/project.json"] = json.dumps({
        "format_version": 1, "app": "ColorYourModel", "unit": "millimeter",
        "active_plate": 1, "plates": plates,
    }, indent=1, ensure_ascii=False).encode()
    return files

def content_types():
    return ('<?xml version="1.0" encoding="UTF-8"?>'
            '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            '<Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>'
            '<Default Extension="png" ContentType="image/png"/>'
            '<Default Extension="bin" ContentType="application/octet-stream"/>'
            '<Default Extension="json" ContentType="application/json"/>'
            '</Types>')

RELS = ('<?xml version="1.0" encoding="UTF-8"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Target="/3D/3dmodel.model" Id="rel-1" '
        'Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>'
        '</Relationships>')

def main():
    with zipfile.ZipFile(BASE) as z:
        machine = z.read("Metadata/machine_settings_1.config")
        process = z.read("Metadata/process_settings_1.config")
    entries = {
        "[Content_Types].xml": content_types().encode(),
        "_rels/.rels": RELS.encode(),
        "3D/3dmodel.model": model_xml().encode(),
        "Metadata/model_settings.config": model_settings_xml().encode(),
        "Metadata/project_settings.config": project_settings().encode(),
        "Metadata/machine_settings_1.config": machine,
        "Metadata/process_settings_1.config": process,
    }
    entries.update(cym_parts())
    with zipfile.ZipFile(OUT, "w", zipfile.ZIP_DEFLATED) as z:
        for name, body in entries.items():
            z.writestr(name, body)
    print("wrote", OUT)
    print("entries:", len(entries))
    for name in entries: print("  ", name)

if __name__ == "__main__":
    main()
