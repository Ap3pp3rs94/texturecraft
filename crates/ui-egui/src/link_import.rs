//! The first procedural bake can take as long as a re-cook. Keep the native UI responsive.
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::TextureCraftApp;
use serde_json::{Value, json};
use texturecraft_engine::link_cmds::{CookedMaterial, LinkJob, cook_link_file};
use texturecraft_engine::materials::Place;

pub(super) struct Import {
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
        let resolution = match self.session.doc.project.texture_set(set) {
            Ok(set) => set.resolution,
            Err(error) => {
                self.toast(error.to_string(), true);
                return None;
            }
        };
        let baked = self.session.baked(set).cloned();
        let path = PathBuf::from(path);
        let rx = super::spawn_worker("texturecraft-material-import", move || cook_link_file(&path, resolution, baked.as_ref()));
        self.link_imports.push(Import { mesh: self.session.mesh_id, set, resolution, place: Place::from_params(params), result: rx });
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
            // Replacing a project or resizing its texture set invalidates the target captured at drop time.
            if job.mesh != self.session.mesh_id
                || job.set != self.session.ts()
                || !self.session.doc.project.texture_set(job.set).is_ok_and(|set| set.resolution == job.resolution)
            {
                self.toast("Material import canceled because the target texture set changed", true);
                continue;
            }
            match result.and_then(|(source, cooked)| self.session.add_cooked_material(&cooked, source.package, source.source_ext, job.place)) {
                Ok(_) => self.toast("Material imported", false),
                Err(error) => self.toast(error.to_string(), true),
            }
        }
    }
}
