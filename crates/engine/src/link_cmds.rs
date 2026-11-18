//! MaterialCraft materials as fill layers. A `.mcmat` portable material becomes a fill whose
//! channel sources are its cooked maps (image resources) and whose [`MaterialLink`] keeps the
//! graph's exposed parameters. Changing one re-cooks the maps through `materialcraft-cli` at the
//! texture set's resolution, with the set's baked mesh maps feeding graph inputs of the same name.
//! Both apps use the shared craft-mcmat container and the public CLI.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use texturecraft_core::{BakedMap, Channel, Fill, Layer, LayerId, LayerKind, LinkParam, MaterialLink, Source};
use texturecraft_formats::mcmat::{MCRAFT_EXT, is_mcmat, is_mcraft, validate_mcmat};

use crate::commands::{CommandSpec, bad, boolp, has_layer, layer_param, strp};
use crate::materials::Place;
use crate::{EngineError, Result, Session, cmd};

#[cfg(not(target_arch = "wasm32"))]
mod cook_report;
mod cooked_map;
mod cooked_material;
mod image_input;
mod link_job;
#[cfg(not(target_arch = "wasm32"))]
mod map_file;
mod material_interface;

pub use cooked_map::CookedMap;
pub use cooked_material::CookedMaterial;
pub use image_input::ImageInput;
pub use link_job::LinkJob;
pub use material_interface::MaterialInterface;

/// The undo label of parameter edits and re-cooks (a re-cook merges into the edit before it).
const EDIT: &str = "Edit MaterialCraft material";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "material.link.set",
            "Set material parameter",
            [],
            None,
            "{layer?, id, value (number, [..], bool, \"#rrggbb\" or {t, v}), cook?: true} — sets an exposed input of a MaterialCraft material layer and re-cooks its maps (cook: false only stores the value)",
            has_layer,
            set
        ),
        cmd!(
            "material.link.preset",
            "Apply material preset",
            [],
            None,
            "{layer?, name, cook?: true} — applies a preset of a MaterialCraft material layer",
            has_layer,
            preset
        ),
        cmd!(
            "material.link.cook",
            "Re-cook MaterialCraft material",
            [],
            None,
            "{layer?} — re-cooks a MaterialCraft material layer at its texture set's resolution with the set's baked mesh maps",
            has_layer,
            |s, p| {
                let id = layer_param(s, "material.link.cook", p)?;
                cook(s, id, Map::new())
            }
        ),
        cmd!(
            "material.link.saveSource",
            "Save MaterialCraft package",
            [],
            None,
            "{layer?, path} — writes the MaterialCraft source (.mcmat, .mcraft or .sbs) behind a material layer",
            has_layer,
            save_source
        ),
    ]
}

/// The link of fill layer `id`.
fn link_of(s: &Session, cmd: &str, id: LayerId) -> Result<MaterialLink> {
    match &s.doc.project.layer(id)?.kind {
        LayerKind::Fill(Fill { link: Some(link), .. }) => Ok(link.as_ref().clone()),
        _ => Err(bad(cmd, format!("layer {} is not a MaterialCraft material", id.0))),
    }
}

/// Value types other apps may set (MaterialCraft's portable inputs), with their component count.
fn components(t: &str) -> Option<usize> {
    Some(match t {
        "float" | "int" | "enum" | "bool" => 1,
        "float2" | "int2" => 2,
        "float3" => 3,
        "float4" | "color" => 4,
        _ => return None,
    })
}

fn type_of(v: &Value) -> &str {
    v.get("t").and_then(Value::as_str).unwrap_or("")
}

/// `v` (plain or `{t, v}`) as a typed value for `param`, with finite numbers.
pub fn typed(param: &LinkParam, v: &Value) -> Option<Value> {
    let t = type_of(&param.default);
    let n = components(t)?;
    let payload = if v.get("t").is_some() { v.get("v")? } else { v };
    let num = |x: &Value| x.as_f64().filter(|f| f.is_finite());
    let payload = match (t, payload) {
        ("bool", Value::Bool(b)) => json!(b),
        ("bool", x) => json!(num(x)? != 0.0),
        ("color", Value::String(hex)) => {
            let h = hex.trim_start_matches('#');
            if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let c = |i: usize| Some(f64::from(u8::from_str_radix(h.get(i..i + 2)?, 16).ok()?) / 255.0);
            // Hex colors are sRGB; values are linear.
            let lin = |x: f64| if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) };
            json!([lin(c(0)?), lin(c(2)?), lin(c(4)?), 1.0])
        }
        (_, x) if n == 1 => {
            let f = num(x)?;
            if matches!(t, "int" | "enum") { json!(f.round() as i64) } else { json!(f) }
        }
        (_, Value::Array(a)) if a.len() >= n.min(3) => {
            let mut out: Vec<f64> = a.iter().take(n).map(num).collect::<Option<_>>()?;
            out.resize(n, 1.0);
            if t == "int2" { json!(out.iter().map(|f| f.round() as i64).collect::<Vec<_>>()) } else { json!(out) }
        }
        _ => return None,
    };
    Some(json!({"t": t, "v": payload}))
}

/// `overrides` checked against the link's parameters.
fn checked(cmd: &str, link: &MaterialLink, overrides: &Map<String, Value>) -> Result<Map<String, Value>> {
    overrides
        .iter()
        .map(|(id, v)| {
            let p = link.params.iter().find(|p| p.id == *id).ok_or_else(|| bad(cmd, format!("`{}` has no parameter `{id}`", link.name)))?;
            Ok((id.clone(), typed(p, v).ok_or_else(|| bad(cmd, format!("`{id}` takes a {}", type_of(&p.default))))?))
        })
        .collect()
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "material.link.set";
    let id = layer_param(s, CMD, p)?;
    let pid = strp(p, "id").ok_or_else(|| bad(CMD, "`id` is required"))?;
    let value = p.get("value").ok_or_else(|| bad(CMD, "`value` is required"))?;
    update(s, CMD, id, Map::from_iter([(pid.to_string(), value.clone())]), boolp(p, "cook").unwrap_or(true))
}

fn preset(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "material.link.preset";
    let id = layer_param(s, CMD, p)?;
    let name = strp(p, "name").ok_or_else(|| bad(CMD, "`name` is required"))?;
    let link = link_of(s, CMD, id)?;
    let values = link.presets.iter().find(|pr| pr.name == name).ok_or_else(|| bad(CMD, format!("`{}` has no preset `{name}`", link.name)))?.values.clone();
    // Presets may name inputs the painter can't set; those stay as cooked.
    let values = values.into_iter().filter(|(k, _)| link.params.iter().any(|p| p.id == *k));
    update(s, CMD, id, values, boolp(p, "cook").unwrap_or(true))
}

/// Set parameter values of linked layer `id`; with `cook`, re-cook now (one undo step).
fn update(s: &mut Session, cmd: &str, id: LayerId, overrides: impl IntoIterator<Item = (String, Value)>, cook_now: bool) -> Result<Value> {
    let link = link_of(s, cmd, id)?;
    let values = checked(cmd, &link, &overrides.into_iter().collect::<Map<_, _>>())?;
    if cook_now {
        return cook(s, id, values);
    }
    s.edit(EDIT, |s| {
        if let LayerKind::Fill(Fill { link: Some(link), .. }) = &mut s.doc.project.layer_mut(id)?.kind {
            for p in &mut link.params {
                if let Some(v) = values.get(&p.id) {
                    p.value = v.clone();
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({"layer": id.0, "cooked": false}))
}

fn cook(s: &mut Session, id: LayerId, overrides: Map<String, Value>) -> Result<Value> {
    let job = s.link_job(id, &overrides)?;
    let bytes = job.run()?;
    s.apply_link_cook(id, &bytes, false)?;
    Ok(json!({"layer": id.0, "cooked": true, "size": job.size}))
}

#[cfg(not(target_arch = "wasm32"))]
fn save_source(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "material.link.saveSource";
    let id = layer_param(s, CMD, p)?;
    let path = strp(p, "path").ok_or_else(|| bad(CMD, "`path` is required"))?;
    let link = link_of(s, CMD, id)?;
    let ext = source_extension(&link.package);
    let path = if path.to_ascii_lowercase().ends_with(&format!(".{ext}")) { path.to_string() } else { format!("{path}.{ext}") };
    let bytes = s.link_package(id)?;
    texturecraft_formats::write_atomic(std::path::Path::new(&path), &bytes).map_err(|e| bad(CMD, e.to_string()))?;
    Ok(json!({"path": path}))
}

#[cfg(target_arch = "wasm32")]
fn save_source(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Other("file paths are not available in the browser".into()))
}

/// A MaterialCraft executable: `$<env>`, else next to this program, else `name` on `PATH`.
#[cfg(not(target_arch = "wasm32"))]
pub fn materialcraft_tool(name: &str, env: &str) -> std::path::PathBuf {
    if let Some(p) = std::env::var_os(env).filter(|p| !p.is_empty()) {
        return p.into();
    }
    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join(&exe))).filter(|p| p.is_file()).unwrap_or_else(|| exe.into())
}

/// A command for a MaterialCraft executable that opens no console window.
#[cfg(not(target_arch = "wasm32"))]
fn quiet(program: &std::path::Path) -> std::process::Command {
    #[allow(unused_mut)]
    let mut c = std::process::Command::new(program);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut c, 0x0800_0000); // CREATE_NO_WINDOW
    c
}

/// Open `package` (a published material or authoring package) in the MaterialCraft app (`$MATERIALCRAFT`, next to this
/// program, or on `PATH`).
#[cfg(not(target_arch = "wasm32"))]
pub fn open_in_materialcraft(package: &std::path::Path) -> Result<()> {
    let app = materialcraft_tool("materialcraft", "MATERIALCRAFT");
    quiet(&app).arg(package).spawn().map(drop).map_err(|e| {
        EngineError::Other(format!("MaterialCraft ({}) did not start: {e}. Install MaterialCraft next to TextureCraft or set MATERIALCRAFT", app.display()))
    })
}

/// Source file extensions are chosen from the linked resource name, never an untrusted path.
pub fn source_extension(name: &str) -> &'static str {
    if name.to_ascii_lowercase().ends_with(".mcmat") {
        "mcmat"
    } else if name.to_ascii_lowercase().ends_with(".sbs") {
        "sbs"
    } else {
        MCRAFT_EXT
    }
}

/// Prepare the first bake without borrowing the paint document. Its original archive is kept for future cooks.
fn cook_source(bytes: Vec<u8>, size: u32, baked: Option<&texturecraft_compose::BakedMaps>) -> Result<(LinkJob, CookedMaterial)> {
    let source_ext = if is_mcmat(&bytes) {
        "mcmat"
    } else if is_mcraft(&bytes) {
        MCRAFT_EXT
    } else {
        return Err(EngineError::Other("expected a MaterialCraft material or project archive".into()));
    };

    let job = LinkJob { layer: LayerId(0), package: Arc::new(bytes), source_ext, graph: None, size, values: Map::new(), inputs: Vec::new() };
    let material = job.run_for_import(baked).map_err(|error| {
        // Older archives still need a republishing diagnostic when the desktop CLI is unavailable
        if source_ext == "mcmat"
            && let Err(format) = validate_mcmat(&job.package)
        {
            return EngineError::Other(format.to_string());
        }

        error
    })?;
    Ok((job, material))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn cook_link_file(path: &std::path::Path, size: u32, baked: Option<&texturecraft_compose::BakedMaps>) -> Result<(LinkJob, CookedMaterial)> {
    cook_source(link_job::read_bounded(path, link_job::MAX_COOK_BYTES)?, size, baked)
}

impl Session {
    /// Add a MaterialCraft material (`.mcmat` bytes, or a `.mcraft` package cooked first) as a
    /// fill layer at `place` (one undo step). Returns the new layer.
    pub fn add_linked_material_bytes_at(&mut self, bytes: &[u8], place: Place) -> Result<LayerId> {
        let ts = self.ts();
        let size = self.doc.project.texture_set(ts)?.resolution;
        let (job, material) = cook_source(bytes.to_vec(), size, self.doc.baked.get(&ts))?;
        self.add_cooked_material(&material, job.package, job.source_ext, place)
    }

    /// Install validated CLI output in one undo step. Keep the original source for future variations.
    pub fn add_cooked_material(&mut self, m: &CookedMaterial, source: Arc<Vec<u8>>, source_ext: &str, place: Place) -> Result<LayerId> {
        if m.maps.is_empty() {
            return Err(EngineError::Other(format!("{} has no maps TextureCraft can paint with", m.interface.name)));
        }

        let label = format!("Add {}", m.interface.name);
        let (id, names) = self.edit(&label, |s| {
            let ts = s.ts();
            let stem = texturecraft_formats::sanitize_resource_name(&m.interface.name);
            let ext = match source_ext {
                "mcmat" | "sbs" | "mcraft" => source_ext,
                _ => return Err(EngineError::Other("unknown linked source format".into())),
            };
            let package = unique(s, &format!("{stem}.{ext}"));
            s.doc.resources.insert(package.clone(), source);
            let link = MaterialLink {
                name: m.interface.name.clone(),
                package,
                graph: m.interface.graph.clone(),
                params: m.interface.parameters.iter().filter(|p| components(type_of(&p.default)).is_some()).cloned().collect(),
                presets: m.interface.presets.clone(),
                inputs: m.interface.inputs.iter().map(|i| i.identifier.clone()).collect(),
                size: m.size,
            };
            let mut fill = Fill { link: Some(Box::new(link)), ..Fill::default() };
            let names = put_maps(s, &stem, &m.maps, &mut fill);
            for c in fill.sources.keys() {
                crate::set_cmds::add_channel(s, ts, *c)?;
            }
            let channels: Vec<Channel> = fill.sources.keys().copied().collect();
            let lid = s.doc.project.alloc_id();
            let name = s.doc.project.next_layer_name(ts, &m.interface.name);
            let lid = crate::layer_cmds::insert_selected(s, Layer::new(lid, name, LayerKind::Fill(fill), &channels))?;
            place.apply(s, lid)?;
            s.select_one(ts, lid);
            s.target = crate::session::StackTarget::Content;
            Ok((lid, names))
        })?;
        self.cache_decoded(names, &m.maps);
        Ok(id)
    }

    /// The source package behind linked layer `id`.
    pub fn link_package(&self, id: LayerId) -> Result<Arc<Vec<u8>>> {
        let link = link_of(self, "material.link", id)?;
        self.doc.resources.get(&link.package).cloned().ok_or_else(|| EngineError::Other(format!("the package of {} is missing", link.name)))
    }

    /// The parameter values of linked layer `id`, by id.
    pub fn link_values(&self, id: LayerId) -> Result<Map<String, Value>> {
        Ok(link_of(self, "material.link", id)?.params.iter().map(|p| (p.id.clone(), p.value.clone())).collect())
    }

    /// Prepare a re-cook with current parameter values and matching baked mesh maps
    pub fn link_job(&self, id: LayerId, overrides: &Map<String, Value>) -> Result<LinkJob> {
        let link = link_of(self, "material.link", id)?;
        let (texture_set, _, _) = self.doc.project.locate(id).ok_or_else(|| EngineError::Other(format!("layer {} is not in a stack", id.0)))?;
        let mut values = self.link_values(id)?;
        values.extend(overrides.clone());

        let mut inputs = Vec::new();
        if let Some(baked) = self.doc.baked.get(&texture_set) {
            for identifier in &link.inputs {
                let Some((map, data)) = BakedMap::parse(identifier).and_then(|map| Some((map, baked.maps.get(&map)?))) else { continue };

                let png = texturecraft_formats::mcmat::mesh_map_png(baked.resolution, map.components(), data)
                    .map_err(|error| EngineError::Other(error.to_string()))?;
                inputs.push((identifier.clone(), png));
            }
        }

        // UDIM tiles share one cook at the texture set's base resolution
        let size = self.doc.project.texture_set(texture_set)?.resolution;
        Ok(LinkJob {
            layer: id,
            package: self.link_package(id)?,
            source_ext: source_extension(&link.package),
            graph: Some(link.graph.clone()),
            size,
            values,
            inputs,
        })
    }

    /// Install the maps and exposed values returned by a completed [`LinkJob`].
    /// With `merge`, this folds into the previous undo step when that was an edit of the same kind
    /// (a parameter set while the cook ran).
    pub fn apply_link_cook(&mut self, id: LayerId, material: &CookedMaterial, merge: bool) -> Result<()> {
        let names = self.edit_merging(EDIT, merge, |session| {
            let LayerKind::Fill(mut fill) = session.doc.project.layer(id)?.kind.clone() else {
                return Err(EngineError::Other(format!("layer {} is not a fill", id.0)));
            };

            let link = fill.link.as_mut().ok_or_else(|| EngineError::Other(format!("layer {} is not a MaterialCraft material", id.0)))?;
            for parameter in &mut link.params {
                if let Some(cooked) = material.interface.parameters.iter().find(|candidate| candidate.id == parameter.id) {
                    parameter.value = cooked.value.clone();
                }
            }
            link.size = material.size;
            let stem = texturecraft_formats::sanitize_resource_name(&link.name);
            let old_names: Vec<String> =
                fill.sources.values().filter_map(|source| if let Source::Image { name, .. } = source { Some(name.clone()) } else { None }).collect();
            let names = put_maps(session, &stem, &material.maps, &mut fill);
            session.doc.project.layer_mut(id)?.kind = LayerKind::Fill(fill);

            // Duplicated layers may still use the earlier maps
            let mut used_names = Vec::new();
            for set in 0..session.doc.project.texture_sets.len() {
                for root in session.doc.project.texture_sets.get(set).map(|texture_set| texture_set.stack.clone()).unwrap_or_default() {
                    crate::material_lib::images_used(&session.doc.project, root, &mut used_names, 0);
                }
            }
            for name in old_names.iter().filter(|name| !used_names.contains(name)) {
                session.doc.resources.remove(name);
            }

            Ok(names)
        })?;

        // Reuse the validated maps before synchronization considers decoding missing images
        self.cache_decoded(names, &material.maps);
        self.sync_images();

        Ok(())
    }

    /// Keep the images decoded while validating maps instead of decoding them again.
    fn cache_decoded(&mut self, names: Vec<String>, maps: &[CookedMap]) {
        for (name, map) in names.into_iter().zip(maps) {
            self.images.insert(name, Arc::clone(&map.image));
        }
    }
}

/// A resource name not in use: `name`, else `2_name`, `3_name`…
fn unique(s: &Session, name: &str) -> String {
    (1..).map(|n| if n == 1 { name.to_string() } else { format!("{n}_{name}") }).find(|n| !s.doc.resources.contains_key(n)).unwrap_or_else(|| name.to_string())
}

/// Store `maps` as new resources (names carry a fresh id, so the decoded-image cache never
/// confuses two cooks) and point `fill`'s channels at them. Returns the names, in map order.
fn put_maps(s: &mut Session, stem: &str, maps: &[CookedMap], fill: &mut Fill) -> Vec<String> {
    let cook = s.doc.project.alloc_id().0;
    let mut names = Vec::new();
    for map in maps {
        let channel = map.channel;
        let extension = if map.bytes.starts_with(&[0x76, 0x2f, 0x31, 0x01]) { "exr" } else { "png" };
        let name = unique(s, &format!("{stem}_{}_{cook}.{extension}", channel.id()));
        s.doc.resources.insert(name.clone(), Arc::clone(&map.bytes));

        // Preserve a normal convention the user chose for this layer
        let normal_format = match fill.sources.get(&channel) {
            Some(Source::Image { normal_format, .. }) => *normal_format,
            _ => Default::default(),
        };
        fill.sources.insert(channel, Source::Image { name: name.clone(), normal_format });
        names.push(name);
    }

    names
}

#[cfg(test)]
mod tests {
    use super::typed;
    use serde_json::json;
    use texturecraft_core::layer::LinkParam;

    fn color_param() -> LinkParam {
        let c = json!({"t": "color", "v": [0.0, 0.0, 0.0, 1.0]});
        serde_json::from_value(json!({"id": "c", "label": "C", "value": c, "default": c})).unwrap()
    }

    #[test]
    fn hex_color_rejects_signs_and_extra_digits() {
        let p = color_param();
        assert_eq!(typed(&p, &json!("#aabbccdd")), None);
        assert_eq!(typed(&p, &json!("#+f+f+f")), None);
        assert!(typed(&p, &json!("#ff8000")).is_some());
    }
}
