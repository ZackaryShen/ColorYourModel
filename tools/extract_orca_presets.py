#!/usr/bin/env python3
"""Distil an OrcaSlicer vendor profile tree into the compact preset library CYM ships.

Why this exists
---------------
OrcaSlicer / Snapmaker Orca store printer, process and filament presets as a
graph of JSON files linked by an ``inherits`` field, spread over hundreds of
files, and interleaved with abandoned ``... copy.json`` / ``..._old.json``
leftovers that are NOT registered in the vendor index. Hand-copying values out
of that tree is how you end up shipping a fork's stale temperature or, worse, a
developer's ``"MyToolChanger 0.4 nozzle - Copy"`` settings id.

So the rule is: the vendor index (``profiles/<Vendor>.json``) is the only
whitelist. A preset that is not reachable from it does not exist.

Usage
-----
    python tools/extract_orca_presets.py \
        --profiles-dir "C:/path/to/OrcaSlicer/resources/profiles" \
        --vendor Snapmaker \
        --machine-model "Snapmaker U1" \
        --out src-tauri/resources/presets/snapmaker_u1.json

The output is committed to the repo and embedded at compile time via
``include_str!``; this script is a one-shot developer tool, never run at
runtime.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

# ---------------------------------------------------------------------------
# Field whitelists
# ---------------------------------------------------------------------------
# We deliberately carry a subset rather than the fully flattened config. The
# 3mf we emit is a *colour hand-off*, not a slice-ready project: Orca re-derives
# everything else from the preset it resolves. Carrying the whole flattened
# config would balloon the binary by megabytes and, worse, freeze values that
# should track the user's installed profile.

# REFUTE-6: the embedded machine preset is what Orca selects and shows in the
# dropdown, and when its `inherits` chain cannot resolve (user has not
# installed the vendor profile) the loader falls back to the DEFAULT printer
# config. Missing keys therefore silently become generic values, so the
# embedded preset must be a COMPLETE config, not a diff. These whitelists
# exist only to keep the JSON small; every key below is one Orca actually
# reads when the preset is selected. (bed_model / bed_texture / thumbnails
# are deliberately excluded: they reference files inside the vendor profile
# directory that would not exist next to our embedded preset.)

MACHINE_FIELDS = [
    "printer_model",
    "printer_variant",
    "printer_technology",
    "printer_structure",
    "gcode_flavor",
    "nozzle_diameter",
    "nozzle_type",
    "nozzle_volume",
    "printable_area",
    "printable_height",
    "extruder_type",
    "extruder_offset",
    "extruder_colour",
    "max_layer_height",
    "min_layer_height",
    "default_bed_type",
    "single_extruder_multi_material",
    "machine_tool_change_time",
    "retract_length_toolchange",
    # motion limits — arrays indexed by extruder / speed limit group
    "machine_max_acceleration_e",
    "machine_max_acceleration_extruding",
    "machine_max_acceleration_retracting",
    "machine_max_acceleration_travel",
    "machine_max_acceleration_x",
    "machine_max_acceleration_y",
    "machine_max_acceleration_z",
    "machine_max_jerk_e",
    "machine_max_jerk_x",
    "machine_max_jerk_y",
    "machine_max_jerk_z",
    "machine_max_speed_e",
    "machine_max_speed_x",
    "machine_max_speed_y",
    "machine_max_speed_z",
    "machine_min_extruding_rate",
    "machine_min_travel_rate",
    # retraction / z-hop
    "deretraction_speed",
    "retract_before_wipe",
    "retract_when_changing_layer",
    "retraction_length",
    "retraction_minimum_travel",
    "retraction_speed",
    "retract_restart_extra",
    "retract_restart_extra_toolchange",
    "retract_lift_above",
    "retract_lift_below",
    "retract_lift_enforce",
    "retraction_distances_when_cut",
    "long_retractions_when_cut",
    "z_hop",
    "z_hop_types",
    "z_hop_when_prime",
    "travel_slope",
    "wipe",
    "wipe_distance",
    # gcode macros — the machine profile's soul; dropping these makes the
    # printer print with the default printer's start/end gcode
    "machine_start_gcode",
    "machine_end_gcode",
    "change_filament_gcode",
    "layer_change_gcode",
    "before_layer_change_gcode",
    "machine_pause_gcode",
    # misc machine behaviour
    "host_type",
    "silent_mode",
    "auxiliary_fan",
    "purge_in_prime_tower",
    "scan_first_layer",
    "printer_notes",
    "nozzle_volume",
    "ramming_pressure_advance_value",
    "tool_change_temprature_wait",
    "machine_load_filament_time",
    "machine_unload_filament_time",
    "enable_filament_ramming",
    "extruder_clearance_radius",
    "extruder_clearance_height_to_rod",
    "extruder_clearance_height_to_lid",
    "bed_exclude_area",
]

PROCESS_FIELDS = [
    "layer_height",
    "initial_layer_print_height",
    "line_width",
    "initial_layer_line_width",
    "inner_wall_line_width",
    "outer_wall_line_width",
    "top_surface_line_width",
    "sparse_infill_line_width",
    "internal_solid_infill_line_width",
    "support_line_width",
    "wall_loops",
    "sparse_infill_density",
    "print_sequence",
    # the embedded process preset needs compatible_printers so Orca can link
    # it to the embedded machine preset it was selected with
    "compatible_printers",
]

FILAMENT_FIELDS = [
    "filament_type",
    "filament_vendor",
    "filament_density",
    "filament_diameter",
    "filament_cost",
    "filament_flow_ratio",
    "filament_max_volumetric_speed",
    "filament_soluble",
    "filament_is_support",
    "nozzle_temperature",
    "nozzle_temperature_initial_layer",
    "nozzle_temperature_range_low",
    "nozzle_temperature_range_high",
    "hot_plate_temp",
    "hot_plate_temp_initial_layer",
    "cool_plate_temp",
    "cool_plate_temp_initial_layer",
    "textured_plate_temp",
    "textured_plate_temp_initial_layer",
    "eng_plate_temp",
    "eng_plate_temp_initial_layer",
    "compatible_printers",
]

# Keys that Config.cpp:792 routes into `key_values` instead of the config.
# They are metadata about the preset, not print settings.
META_FIELDS = ["name", "type", "setting_id", "filament_id", "instantiation", "inherits", "from"]


class PresetIndex:
    """Name -> raw JSON, for one preset category, built from the vendor index."""

    def __init__(self, profiles_dir: Path, entries: list[dict[str, str]]) -> None:
        self.raw: dict[str, dict[str, Any]] = {}
        self.missing: list[str] = []
        for entry in entries:
            path = profiles_dir / entry["sub_path"]
            if not path.is_file():
                self.missing.append(entry["sub_path"])
                continue
            with path.open(encoding="utf-8") as fh:
                data = json.load(fh)
            # The index name is authoritative; a few files disagree with their
            # own "name" field and Orca keys off the index.
            self.raw[entry["name"]] = data

    def resolve(self, name: str, _seen: frozenset[str] = frozenset()) -> dict[str, Any]:
        """Flatten a preset against its ``inherits`` chain (child wins)."""
        if name in _seen:
            raise ValueError(f"inherits cycle through {name!r}")
        node = self.raw.get(name)
        if node is None:
            return {}
        parent_name = node.get("inherits")
        merged: dict[str, Any] = {}
        if parent_name:
            merged.update(self.resolve(parent_name, _seen | {name}))
        for key, value in node.items():
            if key == "inherits":
                continue
            merged[key] = value
        return merged

    def instantiable(self) -> list[str]:
        """Presets the UI actually offers. ``instantiation`` is a string."""
        out = []
        for name, node in self.raw.items():
            if str(node.get("instantiation", "true")).lower() == "true":
                out.append(name)
        return out


def pick(flat: dict[str, Any], fields: list[str]) -> dict[str, Any]:
    return {k: flat[k] for k in fields if k in flat}


def nozzle_of(compatible: list[str] | None, variant_names: dict[str, str]) -> list[str]:
    """Map ``compatible_printers`` entries back to nozzle diameters."""
    if not compatible:
        return []
    return sorted({variant_names[c] for c in compatible if c in variant_names})


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--profiles-dir", required=True, type=Path)
    ap.add_argument("--vendor", required=True)
    ap.add_argument("--machine-model", required=True)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--model-id-override", default=None,
                    help="Second slicer target's model_id, e.g. the upstream numeric id")
    args = ap.parse_args()

    vendor_file = args.profiles_dir / f"{args.vendor}.json"
    with vendor_file.open(encoding="utf-8") as fh:
        vendor = json.load(fh)

    # `sub_path` in the index is relative to the vendor's own sub-directory,
    # not to `profiles/`.
    root = args.profiles_dir / args.vendor

    machines = PresetIndex(root, vendor.get("machine_list", []))
    processes = PresetIndex(root, vendor.get("process_list", []))
    filaments = PresetIndex(root, vendor.get("filament_list", []))

    # --- machine model registration -------------------------------------
    model_entry = next(
        (m for m in vendor.get("machine_model_list", []) if m["name"] == args.machine_model),
        None,
    )
    if model_entry is None:
        print(f"machine model {args.machine_model!r} is not in {vendor_file}", file=sys.stderr)
        return 1
    with (root / model_entry["sub_path"]).open(encoding="utf-8") as fh:
        model = json.load(fh)

    # --- machine variants ------------------------------------------------
    variants = []
    variant_names: dict[str, str] = {}  # preset name -> nozzle diameter
    for name in sorted(machines.instantiable()):
        flat = machines.resolve(name)
        if flat.get("printer_model") != args.machine_model:
            continue
        nozzle = str(flat.get("printer_variant", ""))
        variant_names[name] = nozzle
        variants.append(
            {
                "nozzle_diameter": nozzle,
                "preset_name": name,
                "setting_id": machines.raw[name].get("setting_id"),
                "config": pick(flat, MACHINE_FIELDS),
            }
        )
    variants.sort(key=lambda v: float(v["nozzle_diameter"] or 0))

    # --- processes -------------------------------------------------------
    out_processes = []
    for name in sorted(processes.instantiable()):
        flat = processes.resolve(name)
        nozzles = nozzle_of(flat.get("compatible_printers"), variant_names)
        if not nozzles:
            continue
        out_processes.append(
            {
                "name": name,
                "setting_id": processes.raw[name].get("setting_id"),
                "nozzles": nozzles,
                "layer_height": flat.get("layer_height"),
                "config": pick(flat, PROCESS_FIELDS),
            }
        )
    out_processes.sort(key=lambda p: (p["nozzles"], float(p.get("layer_height") or 0)))

    # --- filaments -------------------------------------------------------
    out_filaments = []
    for name in sorted(filaments.instantiable()):
        flat = filaments.resolve(name)
        nozzles = nozzle_of(flat.get("compatible_printers"), variant_names)
        if not nozzles:
            continue
        cfg = pick(flat, FILAMENT_FIELDS)
        out_filaments.append(
            {
                "name": name,
                "setting_id": filaments.raw[name].get("setting_id"),
                "filament_id": filaments.raw[name].get("filament_id"),
                "nozzles": nozzles,
                "type": (cfg.get("filament_type") or [""])[0]
                if isinstance(cfg.get("filament_type"), list)
                else cfg.get("filament_type"),
                "vendor": (cfg.get("filament_vendor") or [""])[0]
                if isinstance(cfg.get("filament_vendor"), list)
                else cfg.get("filament_vendor"),
                "config": cfg,
            }
        )
    out_filaments.sort(key=lambda f: (f["nozzles"], f["vendor"] or "", f["name"]))

    model_ids = {"snapmaker_orca": model.get("model_id")}
    if args.model_id_override:
        model_ids["orcaslicer"] = args.model_id_override

    payload = {
        "schema_version": 1,
        "source": f"{args.vendor}.json ({vendor.get('version')})",
        "id": args.machine_model.lower().replace(" ", "_"),
        "vendor": args.vendor,
        "name": args.machine_model,
        "family": model.get("family"),
        "model_ids": model_ids,
        "nozzle_diameters_declared": model.get("nozzle_diameter"),
        "variants": variants,
        "processes": out_processes,
        "filaments": out_filaments,
    }

    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w", encoding="utf-8", newline="\n") as fh:
        json.dump(payload, fh, indent=2, ensure_ascii=False)
        fh.write("\n")

    print(f"wrote {args.out} ({args.out.stat().st_size} bytes)")
    print(f"  variants  : {len(variants)} -> {[v['nozzle_diameter'] for v in variants]}")
    print(f"  processes : {len(out_processes)}")
    print(f"  filaments : {len(out_filaments)}")
    for idx, label in ((machines, "machine"), (processes, "process"), (filaments, "filament")):
        if idx.missing:
            print(f"  WARNING {label}: {len(idx.missing)} indexed files missing on disk")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
