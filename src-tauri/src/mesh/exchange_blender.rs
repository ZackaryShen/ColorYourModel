//! Optional Blender bridge for static FBX and USD mesh geometry.
//!
//! The bridge uses indexed triangles and one RGBA value per triangle. Keeping
//! indices separate from colors preserves coincident shells and painted seams.

use std::{
    env,
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use super::{
    face_colors::srgb_to_linear,
    model::{MeshModel, DEFAULT_FACE_COLOR},
};

const SCRIPT: &str = include_str!("exchange_blender.py");
const OVERRIDE: &str = "CYM_BLENDER";
const LOG_BYTES: u64 = 4096;
const CONVERSION_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAGIC: &[u8; 8] = b"CYMMESH1";
const MAX_FACES: usize = 2_000_000;
const MAX_VERTICES: usize = MAX_FACES * 3;

type MeshData = (Vec<[f32; 3]>, Vec<[u32; 3]>, Vec<[u8; 4]>);

pub(crate) fn import(path: &Path) -> Result<MeshData, String> {
    let blender = executable()?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let source = std::path::absolute(path).map_err(|err| format!("Cannot open input: {err}"))?;
    let work = tempfile::tempdir().map_err(|err| err.to_string())?;
    let target = work.path().join("mesh.bin");
    convert(
        &blender,
        work.path(),
        "import",
        &source,
        &target,
        &extension,
    )?;
    read_mesh(&target)
}

pub(crate) fn export(mesh: &MeshModel, path: &Path, extension: &str) -> Result<(), String> {
    let blender = executable()?;
    let work = tempfile::tempdir().map_err(|err| err.to_string())?;
    let source = work.path().join("mesh.bin");
    let target = work.path().join("converted").with_extension(extension);
    write_mesh(mesh, &source)?;
    convert(&blender, work.path(), "export", &source, &target, extension)?;
    fs::copy(target, path).map_err(|err| format!("Cannot write {}: {err}", path.display()))?;
    Ok(())
}

// Each block has u32 counts, then f32 XYZ millimetres, u32 triangle indices,
// and RGBA bytes. Values are little-endian. Each block starts its own indices.
fn read_mesh(path: &Path) -> Result<MeshData, String> {
    let mut input = BufReader::new(File::open(path).map_err(mesh_error)?);
    let mut magic = [0; MAGIC.len()];
    input.read_exact(&mut magic).map_err(mesh_error)?;
    if &magic != MAGIC {
        return Err("Invalid Blender mesh data header".into());
    }

    let mut vertices = Vec::new();
    let mut faces = Vec::new();
    let mut colors = Vec::new();
    loop {
        let point_count = read_u32(&mut input)? as usize;
        let face_count = read_u32(&mut input)? as usize;
        if point_count == 0 && face_count == 0 {
            break;
        }
        if point_count == 0
            || face_count == 0
            || point_count > MAX_VERTICES - vertices.len()
            || face_count > MAX_FACES - faces.len()
        {
            return Err("Blender mesh exceeds the exchange size limit".into());
        }

        let base_index = vertices.len() as u32;
        vertices.reserve(point_count);
        faces.reserve(face_count);
        colors.reserve(face_count);
        for _ in 0..point_count {
            let point = [
                f32::from_bits(read_u32(&mut input)?),
                f32::from_bits(read_u32(&mut input)?),
                f32::from_bits(read_u32(&mut input)?),
            ];
            if !point.iter().all(|value| value.is_finite()) {
                return Err("Blender mesh contains a non-finite position".into());
            }
            vertices.push(point);
        }
        for _ in 0..face_count {
            let triangle = [
                read_u32(&mut input)?,
                read_u32(&mut input)?,
                read_u32(&mut input)?,
            ];
            if triangle.iter().any(|index| *index as usize >= point_count) {
                return Err("Blender mesh contains an invalid triangle index".into());
            }
            faces.push(triangle.map(|index| base_index + index));
        }
        for _ in 0..face_count {
            let mut rgba = [0; 4];
            input.read_exact(&mut rgba).map_err(mesh_error)?;
            colors.push(rgba);
        }
    }
    if faces.is_empty() {
        return Err("The file contains no mesh faces".into());
    }
    Ok((vertices, faces, colors))
}

fn write_mesh(mesh: &MeshModel, path: &Path) -> Result<(), String> {
    if mesh.vertices.is_empty()
        || mesh.faces.is_empty()
        || mesh.vertices.len() > MAX_VERTICES
        || mesh.faces.len() > MAX_FACES
    {
        return Err("Blender exchange requires 1 to 2 million triangles".into());
    }
    let mut output = BufWriter::new(File::create(path).map_err(mesh_error)?);
    output.write_all(MAGIC).map_err(mesh_error)?;
    output
        .write_all(&(mesh.vertices.len() as u32).to_le_bytes())
        .map_err(mesh_error)?;
    output
        .write_all(&(mesh.faces.len() as u32).to_le_bytes())
        .map_err(mesh_error)?;
    for point in &mesh.vertices {
        if !point.iter().all(|value| value.is_finite()) {
            return Err("Blender mesh contains a non-finite position".into());
        }
        for value in point {
            output.write_all(&value.to_le_bytes()).map_err(mesh_error)?;
        }
    }
    for triangle in &mesh.faces {
        for vertex in triangle {
            if *vertex as usize >= mesh.vertices.len() {
                return Err("Blender mesh contains an invalid triangle index".into());
            }
            output
                .write_all(&vertex.to_le_bytes())
                .map_err(mesh_error)?;
        }
    }
    for face in 0..mesh.faces.len() {
        let rgba = mesh.face_colors.get(face).unwrap_or(&DEFAULT_FACE_COLOR);
        output.write_all(rgba).map_err(mesh_error)?;
    }
    output.write_all(&[0; 8]).map_err(mesh_error)?;
    output.flush().map_err(mesh_error)
}

fn read_u32(input: &mut impl Read) -> Result<u32, String> {
    let mut value = [0; 4];
    input.read_exact(&mut value).map_err(mesh_error)?;
    Ok(u32::from_le_bytes(value))
}

fn mesh_error(error: std::io::Error) -> String {
    format!("Cannot transfer Blender mesh data: {error}")
}

fn convert(
    blender: &Path,
    work: &Path,
    operation: &str,
    source: &Path,
    target: &Path,
    extension: &str,
) -> Result<(), String> {
    if !matches!(extension, "fbx" | "usd" | "usda" | "usdc" | "usdz") {
        return Err(format!("Unsupported Blender exchange format: {extension}"));
    }
    let script = work.join("convert.py");
    fs::write(&script, SCRIPT).map_err(|err| format!("Cannot stage Blender script: {err}"))?;
    let mut log = tempfile::tempfile().map_err(|err| format!("Cannot open Blender log: {err}"))?;
    let mut command = Command::new(blender);
    command
        .current_dir(work)
        .args([
            "--background",
            "--factory-startup",
            "--disable-autoexec",
            "--python-exit-code",
            "1",
            "--python",
        ])
        .arg(script)
        .args(["--", operation])
        .arg(source)
        .arg(target)
        .arg(extension)
        .arg(srgb_to_linear(f32::from(DEFAULT_FACE_COLOR[0]) / 255.0).to_string())
        .arg(MAX_FACES.to_string())
        .stdin(Stdio::null())
        .stdout(log_stream(&log)?)
        .stderr(log_stream(&log)?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|err| format!("Cannot start Blender at {}: {err}", blender.display()))?;
    let status = wait_for_conversion(&mut child, CONVERSION_TIMEOUT)?;
    if !status.success()
        || !target
            .metadata()
            .is_ok_and(|file| file.is_file() && file.len() > 0)
    {
        let details = log_tail(&mut log);
        return Err(format!(
            "Blender could not {operation} {} ({status}). {}",
            extension.to_ascii_uppercase(),
            details.trim()
        )
        .trim_end()
        .into());
    }
    Ok(())
}

fn wait_for_conversion(child: &mut Child, timeout: Duration) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    let error = loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(error) => break format!("Cannot wait for Blender conversion: {error}"),
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break format!(
                "Blender conversion timed out after {} seconds",
                timeout.as_secs()
            );
        }
        thread::sleep(STATUS_POLL_INTERVAL.min(remaining));
    };
    let _ = child.kill();
    let _ = child.wait();
    Err(error)
}

fn log_stream(log: &File) -> Result<Stdio, String> {
    log.try_clone()
        .map(Stdio::from)
        .map_err(|err| format!("Cannot open Blender log stream: {err}"))
}

fn log_tail(log: &mut File) -> String {
    let Ok(end) = log.seek(SeekFrom::End(0)) else {
        return String::new();
    };
    let mut bytes = Vec::new();
    if log
        .seek(SeekFrom::Start(end.saturating_sub(LOG_BYTES)))
        .is_ok()
    {
        let _ = log.take(LOG_BYTES).read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn executable() -> Result<PathBuf, String> {
    if let Some(setting) = env::var_os(OVERRIDE).filter(|value| !value.is_empty()) {
        let path = PathBuf::from(setting);
        return checked_program(&path).ok_or_else(|| {
            format!(
                "{OVERRIDE} does not point to a Blender executable: {}",
                path.display()
            )
        });
    }
    let program = if cfg!(windows) {
        "blender.exe"
    } else {
        "blender"
    };
    if let Some(paths) = env::var_os("PATH") {
        if let Some(path) =
            env::split_paths(&paths).find_map(|path| checked_program(&path.join(program)))
        {
            return Ok(path);
        }
    }
    installed_program().ok_or_else(|| {
        "Blender is required for FBX and USD exchange. Install Blender 4.5 or newer, or set CYM_BLENDER to its executable path.".into()
    })
}

fn checked_program(path: &Path) -> Option<PathBuf> {
    let metadata = path.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    std::path::absolute(path).ok()
}

#[cfg(target_os = "macos")]
fn installed_program() -> Option<PathBuf> {
    let app = "Applications/Blender.app/Contents/MacOS/Blender";
    checked_program(&Path::new("/").join(app))
        .or_else(|| checked_program(&PathBuf::from(env::var_os("HOME")?).join(app)))
}

#[cfg(windows)]
fn installed_program() -> Option<PathBuf> {
    ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(env::var_os)
        .find_map(|root| {
            let root = PathBuf::from(root).join("Blender Foundation");
            let mut installs: Vec<_> = fs::read_dir(root)
                .ok()?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect();
            installs.sort_unstable();
            installs
                .into_iter()
                .rev()
                .find_map(|path| checked_program(&path.join("blender.exe")))
        })
}

#[cfg(not(any(target_os = "macos", windows)))]
fn installed_program() -> Option<PathBuf> {
    [
        "/usr/local/bin/blender",
        "/usr/bin/blender",
        "/snap/bin/blender",
    ]
    .into_iter()
    .find_map(|path| checked_program(Path::new(path)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    #[ignore = "requires a local Blender 4.5 or newer"]
    fn roundtrips_coincident_shells_and_shared_color_seams() {
        let work = tempfile::tempdir().unwrap();
        let source = work.path().join("coincident.usda");
        fs::write(
            &source,
            r#"#usda 1.0
(
    metersPerUnit = 0.01
    upAxis = "Z"
)
def Mesh "CoincidentShells"
{
    uniform token subdivisionScheme = "none"
    int[] faceVertexCounts = [3, 3, 3, 3]
    int[] faceVertexIndices = [0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]
    point3f[] points = [(0,0,0), (2,0,0), (2,2,0), (0,2,0), (0,0,0), (2,0,0), (2,2,0), (0,2,0)]
    color3f[] primvars:displayColor = [(1,0,0), (0,1,0), (1,0,0), (0,1,0)] (
        interpolation = "uniform"
    )
    float[] primvars:displayOpacity = [0.75, 1, 0.75, 1] (
        interpolation = "uniform"
    )
}
"#,
        )
        .unwrap();

        let (vertices, faces, face_colors) = import(&source).unwrap();
        check_topology(&vertices, &faces, &face_colors);
        let mut mesh = MeshModel::new();
        mesh.vertices = vertices;
        mesh.faces = faces;
        mesh.face_colors = face_colors;
        for extension in ["fbx", "usd", "usda", "usdc", "usdz"] {
            let target = work.path().join("roundtrip").with_extension(extension);
            export(&mesh, &target, extension).unwrap();
            let (vertices, faces, colors) = import(&target).unwrap();
            check_topology(&vertices, &faces, &colors);
        }
    }

    fn check_topology(vertices: &[[f32; 3]], faces: &[[u32; 3]], colors: &[[u8; 4]]) {
        assert_eq!(vertices.len(), 8);
        assert_eq!(
            vertices
                .iter()
                .map(|point| point.map(f32::to_bits))
                .collect::<HashSet<_>>()
                .len(),
            4
        );
        assert_eq!(faces.len(), 4);
        assert_eq!(
            colors.iter().copied().collect::<HashSet<_>>(),
            HashSet::from([[255, 0, 0, 191], [0, 255, 0, 255]])
        );
        for (index, face) in faces.iter().enumerate() {
            let neighbors = faces
                .iter()
                .enumerate()
                .filter(|(other, candidate)| {
                    *other != index
                        && candidate
                            .iter()
                            .filter(|vertex| face.contains(vertex))
                            .count()
                            == 2
                })
                .map(|(other, _)| other)
                .collect::<Vec<_>>();
            assert_eq!(neighbors.len(), 1);
            assert_ne!(colors[index], colors[neighbors[0]]);
        }
    }

    #[cfg(unix)]
    #[test]
    fn times_out_and_reaps_a_hung_child() {
        let mut child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let error = wait_for_conversion(&mut child, Duration::from_millis(20)).unwrap_err();
        assert!(error.contains("Blender conversion timed out"));
        assert!(child.try_wait().unwrap().is_some());
    }
}
