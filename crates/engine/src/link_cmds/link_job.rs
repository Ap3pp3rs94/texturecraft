use std::sync::Arc;

use super::{CookedMaterial, EngineError, Result};
use serde_json::{Map, Value};
use texturecraft_core::LayerId;

#[cfg(not(target_arch = "wasm32"))]
use {
    super::{CookedMap, MaterialInterface, cook_report::CookReport, materialcraft_tool, quiet},
    std::{io::Read, path::Path},
    texturecraft_compose::BakedMaps,
    texturecraft_core::{BakedMap, Channel},
};

#[cfg(not(target_arch = "wasm32"))]
pub(super) const MAX_COOK_BYTES: u64 = 1 << 30;

/// A re-cook of a linked layer, to run anywhere (a worker thread) and hand back to
/// [`crate::Session::apply_link_cook`].
#[derive(Clone, Debug)]
pub struct LinkJob {
    pub layer: LayerId,
    pub package: Arc<Vec<u8>>,
    /// The source package extension (`mcmat`, `mcraft` or `sbs`).
    pub source_ext: &'static str,
    /// The named graph in a linked material; direct package imports use its default.
    pub graph: Option<String>,
    /// Pixels across (the texture set's resolution).
    pub size: u32,
    /// Every parameter's value, by id, in MaterialCraft's typed form.
    pub values: Map<String, Value>,
    /// Graph inputs fed from baked mesh maps: identifier → 16-bit PNG.
    pub inputs: Vec<(String, Vec<u8>)>,
}

#[cfg(not(target_arch = "wasm32"))]
fn io(error: impl std::fmt::Display) -> EngineError {
    EngineError::Other(error.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).map_err(io)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(io(format!("invalid or oversized cook output: {}", path.display())));
    }

    let mut bytes = Vec::new();
    std::fs::File::open(path).map_err(io)?.take(limit.saturating_add(1)).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > limit {
        return Err(io("cook output exceeds the remaining budget"));
    }

    Ok(bytes)
}

#[cfg(not(target_arch = "wasm32"))]
fn invoke(mut command: std::process::Command, dir: &Path) -> Result<Vec<u8>> {
    const MAX_DIAGNOSTIC_BYTES: u64 = 1 << 20;
    const MAX_REPORT_BYTES: u64 = 8 << 20;
    let stdout = dir.join("reply.json");
    let stderr = dir.join("diagnostic.txt");
    let status = command
        .stdout(std::fs::File::create(&stdout).map_err(io)?)
        .stderr(std::fs::File::create(&stderr).map_err(io)?)
        .status()
        .map_err(|e| io(format!("could not start materialcraft-cli: {e}. Install MaterialCraft or set MATERIALCRAFT_CLI")))?;
    if !status.success() {
        return Err(io(format!(
            "MaterialCraft could not cook the material: {}",
            String::from_utf8_lossy(&read_bounded(&stderr, MAX_DIAGNOSTIC_BYTES)?).trim()
        )));
    }

    read_bounded(&stdout, MAX_REPORT_BYTES)
}

#[cfg(not(target_arch = "wasm32"))]
impl LinkJob {
    pub fn run(&self) -> Result<CookedMaterial> {
        self.run_for_import(None)
    }

    pub(super) fn run_for_import(&self, baked: Option<&BakedMaps>) -> Result<CookedMaterial> {
        let cli = materialcraft_tool("materialcraft-cli", "MATERIALCRAFT_CLI");
        let temp = tempfile::Builder::new().prefix("texturecraft-material-").tempdir().map_err(io)?;
        let dir = temp.path();
        let ext = match self.source_ext {
            "mcmat" | "sbs" | "mcraft" => self.source_ext,
            _ => return Err(io("unknown linked source format")),
        };

        let source = dir.join(format!("source.{ext}"));
        std::fs::write(&source, &*self.package).map_err(io)?;

        let out = dir.join("maps");
        let mut export = quiet(&cli);
        export.arg("export").arg(&source).arg(&out).args(["--profile", "maps", "--size"]).arg(format!("{}x{}", self.size, self.size));
        if self.source_ext != "mcmat"
            && let Some(graph) = &self.graph
        {
            export.arg("--graph").arg(graph);
        }
        for (id, value) in &self.values {
            export.arg("--set").arg(format!("{id}={value}"));
        }

        let add_input = |export: &mut std::process::Command, index: usize, id: &str, data: &[u8]| -> Result<()> {
            let file = dir.join(format!("input{index}.png"));
            std::fs::write(&file, data).map_err(io)?;
            export.arg("--input").arg(format!("{id}={}", file.display()));
            Ok(())
        };
        for (i, (id, bytes)) in self.inputs.iter().enumerate() {
            add_input(&mut export, i, id, bytes)?;
        }

        if let Some(bakes) = baked.filter(|bakes| !bakes.maps.is_empty()) {
            let mut inspect = quiet(&cli);
            if self.source_ext == "mcmat" {
                inspect.arg("info").arg(&source);
            } else {
                inspect.arg("run").arg(&source).arg("material.info");
            }
            let interface: MaterialInterface = serde_json::from_slice(&invoke(inspect, dir)?).map_err(io)?;
            for (i, input) in interface.inputs.iter().enumerate() {
                if let Some(map) = BakedMap::parse(&input.identifier)
                    && let Some(data) = bakes.maps.get(&map)
                {
                    let png = texturecraft_formats::mcmat::mesh_map_png(bakes.resolution, map.components(), data).map_err(io)?;
                    add_input(&mut export, self.inputs.len() + i, &input.identifier, &png)?;
                }
            }
        }

        let reply: CookReport = serde_json::from_slice(&invoke(export, dir)?).map_err(io)?;
        let mut remaining_bytes = MAX_COOK_BYTES;
        let mut remaining_texels = CookedMap::MAX_TOTAL_TEXELS;
        let mut maps = Vec::<CookedMap>::new();
        let mut size = 0;
        for map in reply.maps {
            let Some(channel) = Channel::ALL.into_iter().find(|channel| channel.id() == map.usage) else { continue };
            if maps.iter().any(|map| map.channel == channel) {
                continue;
            }

            let path = Path::new(&map.file);
            if path.file_name().and_then(|p| p.to_str()) != Some(&map.file) || map.file.contains(['\\', ':']) {
                return Err(io("invalid map filename in MaterialCraft's export report"));
            }

            let bytes = read_bounded(&out.join(path), remaining_bytes.min(crate::resource_cmds::MAX_BYTES as u64))?;
            remaining_bytes -= bytes.len() as u64;
            let decoded = CookedMap::decode(channel, bytes, &mut remaining_texels)?;
            if [decoded.image.width, decoded.image.height] != map.size {
                return Err(io(format!("{} map dimensions disagree with the export report", map.usage)));
            }

            size = size.max(decoded.image.width).max(decoded.image.height);
            maps.push(decoded);
        }

        if maps.is_empty() {
            return Err(io("MaterialCraft returned no paintable map usages"));
        }

        Ok(CookedMaterial { interface: reply.material, maps, size })
    }
}

#[cfg(target_arch = "wasm32")]
impl LinkJob {
    pub(super) fn run_for_import(&self, _: Option<&texturecraft_compose::BakedMaps>) -> Result<CookedMaterial> {
        self.run()
    }

    pub fn run(&self) -> Result<CookedMaterial> {
        Err(EngineError::Other("cooking a procedural material requires materialcraft-cli on a desktop".into()))
    }
}
