#!/usr/bin/env python3
"""Check the U1 tool assignment on the four sides of the CYM test cuboid."""

import argparse
from collections import Counter
import json
from pathlib import Path
import re
from zipfile import ZipFile


def side_at(start, end, bounds):
    xmin, ymin, xmax, ymax = bounds
    x, y = ((before + after) / 2 for before, after in zip(start, end))
    dx, dy = (abs(before - after) for before, after in zip(start, end))
    # Test straight, central wall spans. The toolpath runs inside the mesh
    # by about half a line width, and the corners contain mixed edge paths.
    if xmin + 4 < x < xmax - 4 and dx > 1 and dy < .02:
        if .09 < y - ymin < .65:
            return "y-min"
        if .09 < ymax - y < .65:
            return "y-max"
    if ymin + 4 < y < ymax - 4 and dy > 1 and dx < .02:
        if .09 < x - xmin < .65:
            return "x-min"
        if .09 < xmax - x < .65:
            return "x-max"
    return None


def verify(archive_path):
    expected = {"y-max": 0, "y-min": 1, "x-min": 2, "x-max": 3}
    counts = {side: Counter() for side in expected}
    layers = {side: set() for side in expected}
    with ZipFile(archive_path) as archive:
        meta = json.loads(archive.read("Metadata/plate_1.json"))
        gcode = archive.read("Metadata/plate_1.gcode").decode()
    if meta["filament_ids"] != [0, 1, 2, 3]:
        raise ValueError("The sliced fixture did not contain the four U1 tools")
    bounds = meta["bbox_objects"][0]["bbox"]
    if abs(bounds[2] - bounds[0] - 20) > .01 or abs(bounds[3] - bounds[1] - 16) > .01:
        raise ValueError("Unexpected object orientation or size")

    height = 0
    tool = None
    role = None
    xy = [0., 0.]
    relative = False
    relative_extrusion = False
    on_object = False
    for line in gcode.splitlines():
        if line.startswith(";Z:"):
            height = float(line.removeprefix(";Z:"))
        elif line.startswith(";TYPE:"):
            role = line.removeprefix(";TYPE:")
        elif line.startswith("; printing object "):
            on_object = True
        elif line.startswith("; stop printing object "):
            on_object = False
        code = line.split(";", 1)[0].split()
        if not code:
            continue
        command = code[0]
        if re.fullmatch(r"T[0-9]+", command):
            tool = int(command[1:])
        if command in ("G90", "G91"):
            relative = command == "G91"
        if command in ("M82", "M83"):
            relative_extrusion = command == "M83"
        if command not in ("G0", "G1", "G2", "G3"):
            continue
        fields = {arg[0]: float(arg[1:]) for arg in code[1:] if arg[0] in "XYE"}
        previous = xy[:]
        for axis, name in enumerate("XY"):
            if name in fields:
                xy[axis] = fields[name] + previous[axis] if relative else fields[name]
        if (relative_extrusion and on_object and command == "G1" and
                3 < height < 21 and role == "Outer wall" and fields.get("E", 0) > 0):
            side = side_at(previous, xy, bounds)
            if side:
                counts[side][tool] += 1
                layers[side].add(height)

    for side, wanted in expected.items():
        print(f"{side}: {dict(counts[side])} across {len(layers[side])} layers (expected T{wanted})")
        if set(counts[side]) != {wanted} or len(layers[side]) < 20:
            raise ValueError(f"Unexpected outer wall tool or insufficient samples on {side}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("sliced_archive", type=Path)
    verify(parser.parse_args().sliced_archive)


if __name__ == "__main__":
    main()
