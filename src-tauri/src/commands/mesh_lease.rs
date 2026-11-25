use super::mesh::AppState;
use crate::mesh::model::MeshModel;
use std::ops::{Deref, DerefMut};

/// A background computation owns the model until scope exit, including errors.
pub struct MeshLease<'a> {
    state: &'a AppState,
    tab_id: u64,
    mesh: Option<MeshModel>,
    was_dirty: bool,
    committed: bool,
}
impl AppState {
    pub fn take_mesh(&self, tab_id: u64) -> Result<MeshLease<'_>, String> {
        let mut guard = self.lock_mesh(tab_id)?;
        let was_dirty = guard.workspace.tabs[guard.workspace.index(tab_id)?].dirty;
        let mesh = guard.take().ok_or("No mesh loaded")?;
        drop(guard);
        Ok(MeshLease {
            state: self,
            tab_id,
            mesh: Some(mesh),
            was_dirty,
            committed: false,
        })
    }
}
impl MeshLease<'_> {
    pub fn commit(&mut self) {
        self.committed = true;
    }
}
impl Deref for MeshLease<'_> {
    type Target = MeshModel;
    fn deref(&self) -> &MeshModel {
        self.mesh.as_ref().expect("leased model")
    }
}
impl DerefMut for MeshLease<'_> {
    fn deref_mut(&mut self) -> &mut MeshModel {
        self.mesh.as_mut().expect("leased model")
    }
}
impl Drop for MeshLease<'_> {
    fn drop(&mut self) {
        if let Ok(mut workspace) = self.state.workspace.lock() {
            if let Ok(index) = workspace.index(self.tab_id) {
                workspace.tabs[index].mesh = self.mesh.take();
                workspace.tabs[index].busy = false;
                workspace.tabs[index].dirty =
                    self.was_dirty || self.committed || std::thread::panicking();
                if self.committed || std::thread::panicking() {
                    workspace.tabs[index].color_histogram = None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::commands::mesh::AppState;
    use crate::mesh::model::MeshModel;
    #[test]
    fn unwind_also_restores_the_leased_model() {
        let state = AppState::new();
        let id = state
            .workspace
            .lock()
            .unwrap()
            .append(MeshModel::new(), "A".into())
            .unwrap();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _mesh = state.take_mesh(id).unwrap();
            panic!("compute failed");
        }));
        assert!(caught.is_err());
        let ws = state.workspace.lock().unwrap();
        assert!(ws.ensure_idle().is_ok());
        assert!(ws.tabs[0].mesh.is_some());
    }
    #[test]
    fn failed_background_operation_returns_model_and_releases_busy_workspace() {
        let state = AppState::new();
        let id = state
            .workspace
            .lock()
            .unwrap()
            .append(MeshModel::new(), "A".into())
            .unwrap();
        state.workspace.lock().unwrap().tabs[0].dirty = false;
        let run = || -> Result<(), String> {
            let _mesh = state.take_mesh(id)?;
            Err::<(), String>("no further subdivision".into())?;
            Ok(())
        };
        assert!(run().is_err());
        let ws = state.workspace.lock().unwrap();
        assert!(
            ws.ensure_idle().is_ok(),
            "failed operations must not freeze every tab"
        );
        assert!(ws.tabs[0].mesh.is_some());
        assert!(!ws.tabs[0].dirty);
    }
}
