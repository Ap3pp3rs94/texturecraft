//! The first procedural bake can take as long as a re-cook. Keep the native UI responsive.
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::TextureCraftApp;
use serde_json::{Value, json};
use texturecraft_engine::link_cmds::{CookedMaterial, LinkJob, cook_link_file};
use texturecraft_engine::materials::Place;

pub(super) struct LinkImport {
    mesh: u64,
    set: usize,
    resolution: u32,
    place: Place,
    result: Receiver<texturecraft_engine::Result<(LinkJob, CookedMaterial)>>,
}

pub(super) fn is_procedural(path: &str) -> bool {
    Path::new(path).extension().and_then(|s| s.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("mcmat") || ext.eq_ignore_ascii_case("mcraft"))
}

impl TextureCraftApp {
    pub(super) fn start_link_import(&mut self, path: &str, params: &Value) -> Option<Value> {
        let set = self.session.ts();
        let texture_set = match self.session.doc.project.texture_set(set) {
            Ok(set) => set,
            Err(error) => {
                self.toast(error.to_string(), true);
                return None;
            }
        };

        // The artist can select a different layer while this import cooks
        let place = match Place::from_params(params) {
            Place::Selection => self.session.selected_layer().map_or(Place::Index(texture_set.stack.len()), |target| {
                Place::Above(target, self.session.doc.project.layer(target).is_ok_and(|layer| layer.is_group()))
            }),
            explicit => explicit,
        };
        let resolution = texture_set.resolution;
        let baked = self.session.baked(set).cloned();
        let path = PathBuf::from(path);
        let rx = super::spawn_worker("texturecraft-material-import", move || cook_link_file(&path, resolution, baked.as_ref()));
        self.link_imports.push(LinkImport { mesh: self.session.mesh_id, set, resolution, place, result: rx });
        self.toast("Cooking MaterialCraft material", false);
        Some(json!({"pending":true}))
    }

    pub(super) fn poll_link_imports(&mut self, ctx: &egui::Context) {
        if self.link_imports.is_empty() {
            return;
        }

        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        for job in std::mem::take(&mut self.link_imports) {
            let result = match job.result.try_recv() {
                Err(TryRecvError::Empty) => {
                    self.link_imports.push(job);
                    continue;
                }
                Err(TryRecvError::Disconnected) => Err(texturecraft_engine::EngineError::Other("the material import stopped".into())),
                Ok(result) => result,
            };

            // Replacing a project or resizing its texture set invalidates a pending bake
            if job.mesh != self.session.mesh_id
                || job.set != self.session.ts()
                || !self.session.doc.project.texture_set(job.set).is_ok_and(|set| set.resolution == job.resolution)
            {
                self.toast("Material import canceled because the target texture set changed", true);
                continue;
            }

            if let Place::Above(target, _) = job.place
                && !self.session.doc.project.locate(target).is_some_and(|(set, _, _)| set == job.set)
            {
                self.toast("Material import canceled because the target layer changed", true);
                continue;
            }

            match result.and_then(|(source, cooked)| self.session.add_cooked_material(&cooked, source.package, source.source_ext, job.place)) {
                Ok(_) => self.toast("Material imported", false),
                Err(error) => self.toast(error.to_string(), true),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use texturecraft_engine::{Session, execute};

    #[test]
    fn pending_import_keeps_its_original_layer_position() {
        let Some(cli) = std::env::var_os("MATERIALCRAFT_CLI") else { return };
        let path = std::env::temp_dir().join(format!("texturecraft-pending-import-{}.mcraft", std::process::id()));
        let source = std::process::Command::new(cli).args(["new", "pbr"]).arg(&path).args(["--size", "32"]).output().unwrap();
        assert!(source.status.success());
        let session = Session::new_project("Paint", texturecraft_engine::geom::primitives::cube(1.0), "builtin:cube", None, 128).unwrap();
        let context = eframe::CreationContext::_new_kittest(egui::Context::default());
        let mut app = TextureCraftApp::new(&context, session);
        execute(&mut app.session, "layer.add.paint", &json!({"name":"Drop target"})).unwrap();
        let target = app.session.selected_layer().unwrap();

        app.start_link_import(path.to_str().unwrap(), &Value::Null).unwrap();
        execute(&mut app.session, "layer.add.paint", &json!({"name":"Later selection"})).unwrap();
        let later = app.session.selected_layer().unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        while !app.link_imports.is_empty() && Instant::now() < deadline {
            app.poll_link_imports(&context.egui_ctx);
            std::thread::sleep(Duration::from_millis(10));
        }
        std::fs::remove_file(path).unwrap();

        let imported = app.session.selected_layer().unwrap();
        let stack = &app.session.doc.project.texture_sets[0].stack;
        assert!(stack.ends_with(&[target, imported, later]), "{stack:?}");
    }
}
