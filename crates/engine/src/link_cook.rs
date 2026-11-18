//! Cook a procedural source in a private directory. Only declared map files enter the paint document.
use super::{CookedMaterial, EngineError, LinkJob, MaterialInterface, Result, materialcraft_tool, quiet};
use serde::Deserialize;
use std::io::Read;
use std::path::Path;
use texturecraft_compose::BakedMaps;
use texturecraft_core::{BakedMap, Channel};

fn io(error: impl std::fmt::Display) -> EngineError {
    EngineError::Other(error.to_string())
}

pub(super) fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).map_err(io)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(io(format!("invalid or oversized cook output: {}", path.display())));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path).map_err(io)?.take(limit + 1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > limit {
        return Err(io("cook output exceeds the remaining budget"));
    }
    Ok(bytes)
}

fn invoke(mut command: std::process::Command, dir: &Path) -> Result<Vec<u8>> {
    let stdout = dir.join("reply.json");
    let stderr = dir.join("diagnostic.txt");
    let status = command
        .stdout(std::fs::File::create(&stdout).map_err(io)?)
        .stderr(std::fs::File::create(&stderr).map_err(io)?)
        .status()
        .map_err(|e| io(format!("could not start materialcraft-cli: {e}. Install MaterialCraft or set MATERIALCRAFT_CLI")))?;
    if !status.success() {
        return Err(io(format!("MaterialCraft could not cook the material: {}", String::from_utf8_lossy(&read_bounded(&stderr, 1 << 20)?).trim())));
    }
    read_bounded(&stdout, 8 << 20)
}

#[derive(Deserialize)]
struct Reply {
    material: MaterialInterface,
    maps: Vec<MapFile>,
}
#[derive(Deserialize)]
struct MapFile {
    usage: String,
    file: String,
    size: [u32; 2],
}

impl LinkJob {
    pub fn run(&self) -> Result<CookedMaterial> {
        self.run_for_import(None)
    }

    pub(super) fn run_for_import(&self, baked: Option<&BakedMaps>) -> Result<CookedMaterial> {
        let cli = materialcraft_tool("materialcraft-cli", "MATERIALCRAFT_CLI");
        self.run_with_cli(&cli, baked)
    }

    fn run_with_cli(&self, cli: &Path, baked: Option<&BakedMaps>) -> Result<CookedMaterial> {
        let temp = tempfile::Builder::new().prefix("texturecraft-material-").tempdir().map_err(io)?;
        let dir = temp.path();
        let ext = match self.source_ext {
            "mcmat" | "sbs" | "mcraft" => self.source_ext,
            _ => return Err(io("unknown linked source format")),
        };
        let source = dir.join(format!("source.{ext}"));
        std::fs::write(&source, &*self.package).map_err(io)?;
        let out = dir.join("maps");
        let mut export = quiet(cli);
        export.arg("export").arg(&source).arg(&out).args(["--profile", "maps", "--size"]).arg(self.size.to_string());
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
        if let Some(bakes) = baked {
            let mut inspect = quiet(cli);
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
        let reply: Reply = serde_json::from_slice(&invoke(export, dir)?).map_err(io)?;
        let mut budget = 1u64 << 30;
        let mut maps = Vec::new();
        let mut size = 0;
        for map in reply.maps {
            if !Channel::ALL.iter().any(|c| c.id() == map.usage) || maps.iter().any(|(usage, _)| usage == &map.usage) {
                continue;
            }
            let path = Path::new(&map.file);
            if path.file_name().and_then(|p| p.to_str()) != Some(&map.file) || map.file.contains(['\\', ':']) {
                return Err(io("invalid map filename in MaterialCraft's export report"));
            }
            let data = read_bounded(&out.join(path), budget.min(512 << 20))?;
            budget -= data.len() as u64;
            size = size.max(map.size[0]).max(map.size[1]);
            maps.push((map.usage, data));
        }
        if maps.is_empty() {
            return Err(io("MaterialCraft returned no paintable map usages"));
        }
        Ok(CookedMaterial { interface: reply.material, maps, size })
    }
}
