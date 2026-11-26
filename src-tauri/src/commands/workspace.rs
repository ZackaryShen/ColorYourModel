use std::collections::HashSet;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::MutexGuard;

use crate::export::source_presets::SourcePresets;
use crate::mesh::model::{MeshDataDto, MeshModel};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabInfo {
    pub id: u64,
    pub name: String,
    pub face_count: u32,
    pub dirty: bool,
}

#[derive(Clone)]
pub struct SourcePlate {
    pub path: PathBuf,
    pub index: usize,
    pub count: u32,
}

pub struct Tab {
    pub id: u64,
    pub name: String,
    pub mesh: Option<MeshModel>,
    pub source: Option<SourcePlate>,
    pub dirty: bool,
    pub busy: bool,
    pub face_count: u32,
    pub hidden_labels: HashSet<u32>,
    pub engineering_state: Option<String>,
    pub history_dropped: bool,
    pub color_histogram: Option<crate::export::gradient::ColorHistograms>,
}

pub struct Workspace {
    pub tabs: Vec<Tab>,
    pub active: Option<u64>,
    next_id: u64,
    pub source_presets: Option<Result<SourcePresets, String>>,
    pub project_path: Option<PathBuf>,
    pub(super) source_layout: Option<crate::threemf::ProjectLayout>,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active: None,
            next_id: 1,
            source_presets: None,
            project_path: None,
            source_layout: None,
        }
    }

    pub fn active_id(&self) -> Option<u64> {
        self.active
    }
    pub fn index(&self, id: u64) -> Result<usize, String> {
        self.tabs
            .iter()
            .position(|t| t.id == id)
            .ok_or_else(|| "Tab no longer exists".into())
    }
    pub fn validate_active(&self, id: u64) -> Result<usize, String> {
        let index = self.index(id)?;
        if self.active != Some(id) {
            return Err("Tab is no longer active; operation was cancelled".into());
        }
        Ok(index)
    }
    pub fn ensure_idle(&self) -> Result<(), String> {
        if self.tabs.iter().any(|t| t.busy) {
            Err("A model operation is still running".into())
        } else {
            Ok(())
        }
    }
    pub fn infos(&self) -> Vec<TabInfo> {
        self.tabs
            .iter()
            .map(|t| TabInfo {
                id: t.id,
                name: t.name.clone(),
                face_count: t
                    .mesh
                    .as_ref()
                    .map(|m| m.faces.len() as u32)
                    .unwrap_or(t.face_count),
                dirty: t.dirty,
            })
            .collect()
    }
    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn append(&mut self, mesh: MeshModel, name: String) -> Result<u64, String> {
        self.ensure_idle()?;
        let id = self.allocate_id();
        let face_count = mesh.faces.len() as u32;
        self.tabs.push(Tab {
            id,
            name,
            mesh: Some(mesh),
            source: None,
            dirty: true,
            busy: false,
            face_count,
            hidden_labels: HashSet::new(),
            engineering_state: None,
            history_dropped: false,
            color_histogram: None,
        });
        self.active = Some(id);
        self.evict_clean_inactive();
        Ok(id)
    }
    pub fn open_project(
        &mut self,
        path: &Path,
        progress: &dyn Fn(f32, &str),
    ) -> Result<(), String> {
        self.ensure_idle()?;
        // Build the replacement before changing the current workspace.
        let (index, mut plate, layout) = crate::threemf::open_active_plate(path, progress)?;
        let active_index = index.active_index;
        let count = index.plates.len() as u32;
        let presets = match SourcePresets::read(path) {
            Ok(Some(s)) => Some(Ok(s)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        };
        plate.mesh.source_path = Some(path.to_path_buf());
        plate.mesh.source_plate_count = count;
        plate.mesh.source_presets = presets.clone();
        let mut resident = Some(plate.mesh);
        let mut tabs = Vec::new();
        let mut active = None;
        for (i, reference) in index.plates.into_iter().enumerate() {
            let id = self.allocate_id();
            if i == active_index {
                active = Some(id);
            }
            let mesh = if i == active_index {
                resident.take()
            } else {
                None
            };
            let face_count = mesh
                .as_ref()
                .map(|m| m.faces.len() as u32)
                .unwrap_or(reference.face_count);
            tabs.push(Tab {
                id,
                name: reference.name,
                mesh,
                source: Some(SourcePlate {
                    path: path.to_path_buf(),
                    index: i,
                    count,
                }),
                dirty: false,
                busy: false,
                face_count,
                hidden_labels: HashSet::new(),
                engineering_state: if i == active_index {
                    Some(plate.engineering_state.as_str().into())
                } else {
                    None
                },
                history_dropped: i == active_index && plate.history_dropped,
                color_histogram: None,
            });
        }
        self.tabs = tabs;
        self.active = active;
        self.source_presets = presets;
        self.source_layout = layout;
        self.project_path = (plate.engineering_state == crate::threemf::EngineeringState::Restored)
            .then(|| path.to_path_buf());
        Ok(())
    }
    pub fn ensure_resident(&mut self, index: usize) -> Result<(), String> {
        self.ensure_resident_with_progress(index, &|_, _| {})
    }

    fn ensure_resident_with_progress(
        &mut self,
        index: usize,
        progress: &dyn Fn(f32, &str),
    ) -> Result<(), String> {
        if self.tabs[index].mesh.is_some() {
            return Ok(());
        }
        if self.tabs[index].busy {
            return Err("A model operation is still running".into());
        }
        let source = self.tabs[index]
            .source
            .clone()
            .ok_or("Tab has no resident or source model")?;
        let mut plate = match self.source_layout.as_ref() {
            Some(layout) if layout.applies_to(&source.path) => {
                layout.load(&source.path, source.index, progress)?
            }
            _ => crate::threemf::load_plate(&source.path, source.index, progress)?,
        };
        plate.mesh.source_path = Some(source.path);
        plate.mesh.source_plate_count = source.count;
        plate.mesh.source_presets = self.source_presets.clone();
        plate.mesh.hidden_labels = self.tabs[index].hidden_labels.clone();
        self.tabs[index].face_count = plate.mesh.faces.len() as u32;
        self.tabs[index].engineering_state = Some(plate.engineering_state.as_str().into());
        self.tabs[index].history_dropped = plate.history_dropped;
        self.tabs[index].color_histogram = None;
        self.tabs[index].mesh = Some(plate.mesh);
        Ok(())
    }
    pub fn ensure_all_resident(&mut self) -> Result<(), String> {
        self.ensure_idle()?;
        for i in 0..self.tabs.len() {
            self.ensure_resident(i)?;
        }
        Ok(())
    }
    pub fn evict_clean_inactive(&mut self) {
        for tab in &mut self.tabs {
            if self.active != Some(tab.id) && !tab.dirty && !tab.busy && tab.source.is_some() {
                if let Some(mesh) = tab.mesh.take() {
                    tab.hidden_labels = mesh.hidden_labels;
                    tab.face_count = mesh.faces.len() as u32;
                }
            }
        }
    }
    pub fn switch(&mut self, id: u64) -> Result<(), String> {
        self.switch_with_progress(id, &|_, _| {})
    }

    pub fn switch_with_progress(
        &mut self,
        id: u64,
        progress: &dyn Fn(f32, &str),
    ) -> Result<(), String> {
        self.ensure_idle()?;
        let index = self.index(id)?;
        self.ensure_resident_with_progress(index, progress)?;
        self.active = Some(id);
        self.evict_clean_inactive();
        Ok(())
    }
    pub fn close(&mut self, id: u64) -> Result<(), String> {
        self.ensure_idle()?;
        let index = self.index(id)?;
        if self.active == Some(id) && self.tabs.len() > 1 {
            let target = if index + 1 < self.tabs.len() {
                index + 1
            } else {
                index - 1
            };
            // A missing source cannot cause the current edited tab to vanish.
            self.ensure_resident(target)?;
            self.active = Some(self.tabs[target].id);
        } else if self.tabs.len() == 1 {
            self.active = None;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.project_path = None;
            self.source_layout = None;
            self.source_presets = None;
        }
        // Plate removal changes the project even when no mesh was painted.
        if let Some(tab) = self.tabs.first_mut() {
            tab.dirty = true;
        }
        self.evict_clean_inactive();
        Ok(())
    }
    pub fn dto(&self) -> Result<MeshDataDto, String> {
        let id = self.active.ok_or("No tab is open")?;
        let index = self.index(id)?;
        let tab = &self.tabs[index];
        let mut dto = tab
            .mesh
            .as_ref()
            .ok_or("A model operation is still running")?
            .to_dto();
        dto.tab_id = id;
        dto.tabs = self.infos();
        dto.plate_index = index as u32 + 1;
        dto.plate_count = self.tabs.len() as u32;
        dto.engineering_state = tab.engineering_state.clone();
        dto.history_dropped = tab.history_dropped;
        Ok(dto)
    }
    pub fn rename(&mut self, id: u64, name: String) -> Result<(), String> {
        self.ensure_idle()?;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 128 {
            return Err("Plate name must contain 1 to 128 characters".into());
        }
        let index = self.index(id)?;
        if self.tabs[index].name != name {
            self.ensure_resident(index)?;
            self.tabs[index].name = name.into();
            self.tabs[index].dirty = true;
        }
        Ok(())
    }
}

pub struct MeshGuard<'a> {
    pub workspace: MutexGuard<'a, Workspace>,
    index: usize,
}
impl<'a> MeshGuard<'a> {
    pub fn new(workspace: MutexGuard<'a, Workspace>, id: u64) -> Result<Self, String> {
        let index = workspace.validate_active(id)?;
        Ok(Self { workspace, index })
    }
    pub fn commit_if(&mut self, changed: bool) {
        if changed {
            self.workspace.tabs[self.index].dirty = true;
            self.workspace.tabs[self.index].color_histogram = None;
        }
    }
}
impl Deref for MeshGuard<'_> {
    type Target = Option<MeshModel>;
    fn deref(&self) -> &Self::Target {
        &self.workspace.tabs[self.index].mesh
    }
}
impl DerefMut for MeshGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.workspace.tabs[self.index].mesh
    }
}
impl Drop for MeshGuard<'_> {
    fn drop(&mut self) {
        self.workspace.tabs[self.index].busy = self.workspace.tabs[self.index].mesh.is_none();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stl_append_keeps_both_models_and_ids_are_never_reused() {
        let mut ws = Workspace::new();
        let a = ws.append(MeshModel::new(), "A".into()).unwrap();
        let b = ws.append(MeshModel::new(), "B".into()).unwrap();
        assert_ne!(a, b);
        assert_eq!(ws.active_id(), Some(b));
        ws.close(b).unwrap();
        assert_eq!(ws.active_id(), Some(a));
        let c = ws.append(MeshModel::new(), "C".into()).unwrap();
        assert!(c > b);
    }
    #[test]
    fn closing_last_tab_clears_the_workspace() {
        let mut ws = Workspace::new();
        let a = ws.append(MeshModel::new(), "A".into()).unwrap();
        ws.project_path = Some(PathBuf::from("previous.3mf"));
        ws.source_presets = Some(Err("previous source settings".into()));
        ws.close(a).unwrap();
        assert!(ws.project_path.is_none());
        assert!(ws.source_presets.is_none());
        assert_eq!(ws.active_id(), None);
        assert!(ws.close(a).is_err());
    }
    #[test]
    fn stale_tab_operation_cannot_reach_the_new_active_model() {
        let mut ws = Workspace::new();
        let a = ws.append(MeshModel::new(), "A".into()).unwrap();
        let b = ws.append(MeshModel::new(), "B".into()).unwrap();
        assert!(ws.validate_active(a).is_err());
        assert_eq!(ws.validate_active(b).unwrap(), 1);
    }
    #[test]
    fn per_tab_names_and_hidden_state_are_isolated() {
        let mut ws = Workspace::new();
        let mut mesh = MeshModel::new();
        mesh.hidden_labels.insert(7);
        let a = ws.append(mesh, "A".into()).unwrap();
        let b = ws.append(MeshModel::new(), "B".into()).unwrap();
        ws.rename(a, "Changed".into()).unwrap();
        ws.switch(a).unwrap();
        assert!(ws.tabs[0].mesh.as_ref().unwrap().hidden_labels.contains(&7));
        ws.switch(b).unwrap();
        assert!(ws.tabs[1].mesh.as_ref().unwrap().hidden_labels.is_empty());
        assert_eq!(ws.tabs[1].name, "B");
    }
    #[test]
    fn a_busy_operation_prevents_switch_close_or_replacement() {
        let mut ws = Workspace::new();
        let a = ws.append(MeshModel::new(), "A".into()).unwrap();
        ws.tabs[0].busy = true;
        assert!(ws.close(a).is_err());
        assert!(ws.switch(a).is_err());
        assert!(ws
            .open_project(Path::new("absent.3mf"), &|_, _| {})
            .is_err());
        assert_eq!(ws.tabs.len(), 1);
    }
    #[test]
    fn failed_project_open_keeps_the_current_model() {
        let mut ws = Workspace::new();
        let a = ws.append(MeshModel::new(), "A".into()).unwrap();
        assert!(ws
            .open_project(Path::new("absent.3mf"), &|_, _| {})
            .is_err());
        assert_eq!(ws.active_id(), Some(a));
        assert_eq!(ws.tabs[0].name, "A");
    }
    fn sample_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("examples/3plate_sample.3mf")
    }
    #[test]
    fn multplate_sources_are_lazy_and_loading_failure_keeps_active_edits() {
        let mut ws = Workspace::new();
        ws.open_project(&sample_path(), &|_, _| {}).unwrap();
        assert_eq!(ws.tabs.len(), 3);
        assert_eq!(ws.tabs.iter().filter(|t| t.mesh.is_some()).count(), 1);
        assert!(ws.infos().iter().all(|t| t.face_count > 0));
        let first = ws.active.unwrap();
        let second = ws.tabs[1].id;
        ws.switch(second).unwrap();
        assert!(ws.tabs[0].mesh.is_none());
        assert!(ws.tabs[1].mesh.is_some());
        ws.tabs[0].source.as_mut().unwrap().path = PathBuf::from("missing-source.3mf");
        ws.tabs[1].dirty = true;
        assert!(ws.switch(first).is_err());
        assert_eq!(ws.active, Some(second));
        assert!(ws.tabs[1].mesh.is_some());
    }
    #[test]
    fn edited_tab_and_history_remain_resident_when_switching() {
        let state = super::super::mesh::AppState::new();
        {
            let mut ws = state.workspace.lock().unwrap();
            ws.open_project(&sample_path(), &|_, _| {}).unwrap();
        }
        let first = state.workspace.lock().unwrap().active.unwrap();
        {
            let mut guard = state.lock_mesh(first).unwrap();
            guard
                .as_mut()
                .unwrap()
                .segment_names
                .insert(0, "edited".into());
            guard.commit_if(true);
        }
        let second = state.workspace.lock().unwrap().tabs[1].id;
        {
            let mut ws = state.workspace.lock().unwrap();
            ws.switch(second).unwrap();
            assert!(ws.tabs[0].dirty);
            assert!(ws.tabs[0].mesh.is_some());
            ws.switch(first).unwrap();
        }
        assert_eq!(
            state
                .lock_mesh(first)
                .unwrap()
                .as_ref()
                .unwrap()
                .segment_names[&0],
            "edited"
        );
        assert!(state.lock_mesh(second).is_err());
    }
    #[test]
    fn workspace_replacement_never_reuses_previous_ids() {
        let mut ws = Workspace::new();
        let before = ws.append(MeshModel::new(), "old".into()).unwrap();
        ws.open_project(&sample_path(), &|_, _| {}).unwrap();
        assert!(ws.tabs.iter().all(|t| t.id > before));
        assert!(ws.validate_active(before).is_err());
    }
    #[test]
    fn closing_a_plate_marks_the_remaining_project_dirty() {
        let mut ws = Workspace::new();
        ws.open_project(&sample_path(), &|_, _| {}).unwrap();
        let removed = ws.tabs[2].id;
        ws.close(removed).unwrap();
        assert!(ws.infos().iter().any(|t| t.dirty));
        assert_eq!(ws.tabs.len(), 2);
    }

    #[test]
    fn empty_declared_plate_restores_active_name_and_has_finite_bounds() {
        use crate::export::project_state::{
            encode_project_json, fingerprint_of, PlateStateHeader, ProjectState,
        };
        use crate::threemf::test_support::{
            mesh_object, model_settings, model_xml, plate, project_settings, Fixture,
        };
        let object = mesh_object(
            1,
            "object",
            &[[20.0, 20.0, 0.0], [21.0, 20.0, 0.0], [20.0, 21.0, 0.0]],
            &[[0, 1, 2]],
            &[None],
        );
        let header = ProjectState {
            unit: "millimeter".into(),
            active_plate: 2,
            paint_sources_complete: false,
            plates: vec![PlateStateHeader {
                plater_id: 2,
                name: "Saved empty plate".into(),
                object_ids: vec![],
                fingerprint: fingerprint_of(&[], &[]),
                history_truncated: false,
                placement_offset: [0.0; 3],
            }],
        };
        let fixture = Fixture::new(&[
            (
                "3D/3dmodel.model",
                model_xml("millimeter", &object, r#"<item objectid="1"/>"#),
            ),
            (
                "Metadata/model_settings.config",
                model_settings("", &(plate(1, "A", &[1]) + &plate(2, "Empty", &[]))),
            ),
            (
                "Metadata/project_settings.config",
                project_settings(&["#FFFFFF"], Some((100.0, 100.0))),
            ),
            (
                "Metadata/cym/project.json",
                encode_project_json(&header).into_bytes(),
            ),
        ]);
        let mut ws = Workspace::new();
        ws.open_project(fixture.path(), &|_, _| {}).unwrap();
        assert_eq!(ws.active, Some(ws.tabs[1].id));
        assert_eq!(ws.tabs[1].name, "Saved empty plate");
        let dto = ws.dto().unwrap();
        assert_eq!(dto.face_count, 0);
        assert!(dto
            .bbox
            .min
            .iter()
            .chain(&dto.bbox.max)
            .all(|v| v.is_finite()));
        ws.switch(ws.tabs[0].id).unwrap();
        ws.switch(ws.tabs[1].id).unwrap();
        assert_eq!(ws.dto().unwrap().face_count, 0);
    }
    #[test]
    fn observational_mutable_access_and_cancelled_lasso_keep_clean_source_evictable() {
        let state = super::super::mesh::AppState::new();
        {
            state
                .workspace
                .lock()
                .unwrap()
                .open_project(&sample_path(), &|_, _| {})
                .unwrap();
        }
        let id = state.workspace.lock().unwrap().active.unwrap();
        {
            let mut guard = state.lock_mesh(id).unwrap();
            let mesh = guard.as_mut().unwrap();
            let point = mesh.vertices[0];
            let _ = crate::segment::manual::snap_point_to_vertex_on_face(mesh, &point, 0);
        }
        let mut ws = state.workspace.lock().unwrap();
        assert!(!ws.tabs[0].dirty);
        let other = ws.tabs[1].id;
        ws.switch(other).unwrap();
        assert!(ws.tabs[0].mesh.is_none());
    }
    #[test]
    fn painting_the_existing_colour_is_a_no_op() {
        let mut mesh = MeshModel::new();
        mesh.vertices = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        mesh.faces = vec![[0, 1, 2]];
        mesh.face_colors = vec![[255, 0, 0, 255]];
        mesh.segment_labels = vec![0];
        let result = crate::commands::paint::apply_fill_segment(&mut mesh, 0, [255, 0, 0, 255]);
        assert!(result.updated_faces.is_empty());
        assert!(!mesh.history.can_undo());
    }
    #[test]
    fn failed_or_empty_edits_and_empty_history_keep_a_clean_tab_clean() {
        let state = super::super::mesh::AppState::new();
        state
            .workspace
            .lock()
            .unwrap()
            .open_project(&sample_path(), &|_, _| {})
            .unwrap();
        let id = state.workspace.lock().unwrap().active.unwrap();
        {
            let mut guard = state.lock_mesh(id).unwrap();
            let mesh = guard.as_mut().unwrap();
            assert!(mesh.rename_segment(u32::MAX, "invalid").is_err());
            assert!(crate::segment::split::split_segment(
                mesh,
                u32::MAX,
                &crate::segment::split::SplitMethod::Plane {
                    normal: [0.0, 0.0, 1.0],
                    point: [0.0; 3]
                }
            )
            .is_err());
            let empty =
                crate::commands::paint::apply_fill_segment(mesh, u32::MAX, [255, 0, 0, 255]);
            assert!(empty.updated_faces.is_empty());
            assert!(mesh
                .history
                .undo(&mut mesh.face_colors, &mut mesh.segment_labels)
                .is_none());
            assert!(mesh
                .history
                .redo(&mut mesh.face_colors, &mut mesh.segment_labels)
                .is_none());
            assert!(mesh
                .history
                .jump_to(0, &mut mesh.face_colors, &mut mesh.segment_labels)
                .is_none());
            guard.commit_if(false);
        }
        assert!(!state.workspace.lock().unwrap().tabs[0].dirty);
    }
}
