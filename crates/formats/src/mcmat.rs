//! Validate a procedural MCMAT and encode caller-supplied mesh maps for MaterialCraft.
use crate::{FormatError, Result};
use std::io::Cursor;

pub const MCMAT_EXT: &str = "mcmat";
pub const MCRAFT_EXT: &str = "mcraft";

/// Check the v3 envelope before passing the original bytes to the CLI. SBS evaluation belongs to MaterialCraft.
pub fn validate_mcmat(bytes: &[u8]) -> Result<()> {
    craft_mcmat::Material::open(bytes, craft_mcmat::Limits::default()).map(drop).map_err(|err| match err {
        craft_mcmat::Error::Version(_) => FormatError::Unsupported(err.to_string()),
        craft_mcmat::Error::Archive(ref archive) if archive.is_limit() => FormatError::TooLarge(err.to_string()),
        _ => FormatError::Malformed { kind: "MaterialCraft material", msg: err.to_string() },
    })
}

/// Whether `bytes` contain a portable material manifest within the ZIP entry limit.
pub fn is_mcmat(bytes: &[u8]) -> bool {
    craft_mcmat::is_material(bytes, 100_000)
}

pub fn is_mcraft(bytes: &[u8]) -> bool {
    let Ok(mut archive) = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: 100_000, max_total_bytes: 0 }) else { return false };
    // A portable material may itself contain source.sbs, so it is not a source archive.
    if archive.has_duplicate_names() || archive.contains("material.json") {
        return false;
    }
    if archive.contains("package.json") {
        return true;
    }
    (0..archive.len()).any(|i| archive.entry_name(i).ok().flatten().is_some_and(|name| !name.contains('/') && name.to_ascii_lowercase().ends_with(".sbs")))
}

/// A baked mesh map (`components` floats per texel, 0..1, rows top first) as a 16-bit PNG
/// (grayscale for one component, RGB for three).
pub fn mesh_map_png(width: u32, components: usize, data: &[f32]) -> Result<Vec<u8>> {
    let per_row = (width as usize).saturating_mul(components);
    let height = data.len().checked_div(per_row).unwrap_or(0) as u32;
    if height == 0 || !matches!(components, 1 | 3) {
        return Err(FormatError::Invalid(format!("mesh map of {} values, {width} wide, {components} components", data.len())));
    }
    let px: Vec<u16> = data.iter().take(per_row * height as usize).map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16).collect();
    let img = if components == 1 {
        image::ImageBuffer::<image::Luma<u16>, _>::from_raw(width, height, px).map(image::DynamicImage::ImageLuma16)
    } else {
        image::ImageBuffer::<image::Rgb<u16>, _>::from_raw(width, height, px).map(image::DynamicImage::ImageRgb16)
    }
    .ok_or_else(|| FormatError::Invalid("mesh map size".into()))?;
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).map_err(|e| FormatError::Invalid(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn recognizes_v3_and_directs_older_authors_to_republish() {
        let source = craft_mcmat::write(b"<package/>", []).unwrap();
        assert!(is_mcmat(&source) && !is_mcraft(&source));
        validate_mcmat(&source).unwrap();
        for version in [1, 2] {
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            zip.start_file("material.json", zip::write::SimpleFileOptions::default()).unwrap();
            write!(zip, r#"{{"version":{version}}}"#).unwrap();
            let err = validate_mcmat(&zip.finish().unwrap().into_inner()).unwrap_err();
            assert!(err.to_string().contains("Republish"));
        }
    }

    #[test]
    fn mesh_maps_encode_as_16_bit() {
        let png = mesh_map_png(2, 1, &[0.0, 0.25, 0.5, 1.0]).unwrap();
        let d = crate::decode_image(&png).unwrap();
        assert_eq!((d.width, d.height), (2, 2));
        assert!((d.rgba[1][0] - 0.25).abs() < 1e-4);
        assert!(mesh_map_png(0, 1, &[]).is_err());
        assert!(mesh_map_png(2, 2, &[0.0; 8]).is_err());
    }
}
