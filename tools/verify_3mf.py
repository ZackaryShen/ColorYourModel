import zipfile, json, sys

path = 'examples/paint_color_sample.3mf'
z = zipfile.ZipFile(path)
names = z.namelist()
print('entries:', len(names))
for n in sorted(names):
    print(' ', n)

proj = json.loads(z.read('Metadata/project_settings.config'))
print('--- project_settings.config keys:', len(proj))
print('printer_settings_id:', proj.get('printer_settings_id'))
print('print_settings_id:', proj.get('print_settings_id'))
print('filament_settings_id:', proj.get('filament_settings_id'))
g = proj.get('inherits_group', [])
print('inherits_group len:', len(g))
print('inherits_group:', g)
print('filament_colour:', proj.get('filament_colour'))

m = json.loads(z.read('Metadata/machine_settings_1.config'))
print('--- machine keys:', len(m))
print('printer_settings_id:', m.get('printer_settings_id'))
print('printer_model:', m.get('printer_model'))
print('printer_variant:', m.get('printer_variant'))
print('has machine_start_gcode:', 'machine_start_gcode' in m)
print('nozzle_diameter:', m.get('nozzle_diameter'))
print('gcode_flavor:', m.get('gcode_flavor'))
print('default_bed_type:', m.get('default_bed_type'))

p = json.loads(z.read('Metadata/process_settings_1.config'))
print('--- process keys:', len(p))
print('print_settings_id:', p.get('print_settings_id'))
print('compatible_printers:', p.get('compatible_printers'))
print('layer_height:', p.get('layer_height'))
