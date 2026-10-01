//! Parallel, bounded-memory BMP->PNG and strict decoded-pixel comparisons.
//! No Excel/COM, no screenshots of its own, no implicit white->alpha conversion.
use anyhow::{bail, ensure, Context, Result};
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder, ImageReader, Limits, Rgba, RgbaImage};
use rayon::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Deserialize)]
struct Manifest {
    schema: String,
    captures: Vec<Capture>,
}
#[derive(Debug, Deserialize)]
struct Capture {
    id: String,
    #[serde(default)]
    original: Option<String>,
    #[serde(default)]
    shrunk: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    error: Option<String>,
}
struct Options {
    manifest: PathBuf,
    workers: usize,
    compare: bool,
    max_pixels: u64,
}
fn options() -> Result<Options> {
    let mut manifest = None;
    let mut workers = 4;
    let mut compare = false;
    let mut max_pixels = 32_000_000;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manifest" => manifest = Some(PathBuf::from(args.next().context("missing --manifest value")?)),
            "--workers" => workers = args.next().context("missing --workers")?.parse()?,
            "--max-pixels" => max_pixels = args.next().context("missing --max-pixels")?.parse()?,
            "--compare" => compare = true,
            "--help" | "-h" => {
                println!("excel-lab-images --manifest captures.json [--workers 4] [--max-pixels 32000000] [--compare]");
                std::process::exit(0);
            }
            _ => bail!("unknown argument: {arg}"),
        }
    }
    ensure!((1..=32).contains(&workers), "workers must be 1..32");
    ensure!((1..=256_000_000).contains(&max_pixels), "invalid max_pixels");
    Ok(Options { manifest: manifest.context("--manifest is required")?, workers, compare, max_pixels })
}
fn safe_relative(root: &Path, relative: &str) -> Result<PathBuf> {
    // Use the same forward-slash manifest on Windows and WSL. Reject Windows
    // drive/UNC syntax even when this binary happens to run on Linux.
    ensure!(!relative.contains('\\') && !relative.contains(':'), "invalid relative image path");
    let path = Path::new(relative);
    ensure!(!relative.is_empty() && path.components().all(|c| matches!(c, Component::Normal(_))), "non-relative image path");
    let resolved = root.join(path).canonicalize()?;
    ensure!(resolved.starts_with(root) && resolved.is_file(), "image escapes artifact root");
    Ok(resolved)
}
fn load(path: &Path, max_pixels: u64) -> Result<RgbaImage> {
    let (w, h) = ImageReader::open(path)?.with_guessed_format()?.into_dimensions()?;
    ensure!(w > 0 && h > 0 && w < 32766 && h < 32766, "invalid/truncated image dimensions {w}x{h}");
    ensure!(u64::from(w) * u64::from(h) <= max_pixels, "image exceeds max_pixels");
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(32765);
    limits.max_image_height = Some(32765);
    limits.max_alloc = Some(max_pixels.saturating_mul(16));
    reader.limits(limits);
    Ok(reader.decode()?.into_rgba8())
}
fn png(path: &Path, image: &RgbaImage) -> Result<()> {
    ensure!(!path.exists(), "output exists: {}", path.display());
    let temporary = path.with_extension("png.partial");
    let file = OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let result = (|| -> Result<()> {
        let mut writer = BufWriter::with_capacity(1024 * 1024, file);
        PngEncoder::new_with_quality(&mut writer, CompressionType::Fast, FilterType::Adaptive)
            .write_image(image.as_raw(), image.width(), image.height(), ExtendedColorType::Rgba8)?;
        writer.flush()?;
        drop(writer);
        ensure!(!path.exists(), "output appeared during conversion");
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_file(&temporary); }
    result
}
fn compare_pixels(a: &RgbaImage, b: &RgbaImage) -> (Value, Option<RgbaImage>) {
    if a.dimensions() != b.dimensions() {
        return (json!({"equal":false,"reason":"dimensions","original_size":a.dimensions(),"shrunk_size":b.dimensions()}), None);
    }
    let mut count = 0_u64;
    let mut first = None;
    // Allocate a diff only for changed images; the overwhelmingly common equal
    // case is one tight byte-buffer comparison and needs no second output.
    if a.as_raw() == b.as_raw() {
        return (json!({"equal":true,"changed_pixels":0,"method":"strict-RGBA8"}), None);
    }
    let mut diff = RgbaImage::new(a.width(), a.height());
    for (index, (left, right)) in a.as_raw().chunks_exact(4).zip(b.as_raw().chunks_exact(4)).enumerate() {
        if left != right {
            count += 1;
            let x = (index as u64 % u64::from(a.width())) as u32;
            let y = (index as u64 / u64::from(a.width())) as u32;
            first.get_or_insert((x, y));
            diff.put_pixel(x, y, Rgba([255, 0, 80, 255]));
        }
    }
    (json!({"equal":false,"changed_pixels":count,"first_difference":first,"method":"strict-RGBA8"}), Some(diff))
}
fn atomic_report(path: &Path, report: &Value) -> Result<()> {
    ensure!(!path.exists(), "report already exists: {}", path.display());
    let temp = path.with_extension("json.partial");
    let mut file = OpenOptions::new().create_new(true).write(true).open(&temp)?;
    serde_json::to_writer_pretty(&mut file, report)?;
    file.write_all(b"\n")?;
    file.flush()?;
    drop(file);
    fs::rename(temp, path)?;
    Ok(())
}
fn run(options: Options) -> Result<()> {
    let manifest_path = options.manifest.canonicalize()?;
    let root = manifest_path.parent().context("no manifest parent")?.to_path_buf();
    let bytes = fs::read(&manifest_path)?;
    let manifest: Manifest = serde_json::from_slice(bytes.strip_prefix(&[0xef,0xbb,0xbf]).unwrap_or(&bytes))?;
    ensure!(manifest.schema == "excel-lab-captures-v1", "unsupported capture schema");
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    for capture in &manifest.captures {
        ensure!(!capture.id.is_empty() && capture.id.bytes().all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c)), "unsafe capture id");
        ensure!(ids.insert(capture.id.clone()), "duplicate capture id");
        if capture.status.as_deref() != Some("failed") {
            names.insert(capture.original.clone().context("capture missing original")?);
            if let Some(right) = &capture.shrunk { names.insert(right.clone()); }
        }
    }
    let pool = rayon::ThreadPoolBuilder::new().num_threads(options.workers).build()?;
    let jobs: Vec<_> = names.into_iter().collect();
    let started = Instant::now();
    let conversion_results: Vec<(String, Result<(PathBuf, Value)>)> = pool.install(|| jobs.par_iter().map(|name| {
        let result = (|| -> Result<(PathBuf, Value)> {
            let source = safe_relative(&root, name)?;
            let t = Instant::now();
            let is_bmp = source.extension().is_some_and(|e| e.eq_ignore_ascii_case("bmp"));
            let output;
            let mut info = json!({"source":name,"converted":is_bmp});
            if is_bmp {
                let pixels = load(&source, options.max_pixels)?;
                output = source.with_extension("converted.png");
                png(&output, &pixels)?;
                info["width"] = json!(pixels.width()); info["height"] = json!(pixels.height());
                info["png"] = json!(output.strip_prefix(&root)?.to_string_lossy().replace('\\',"/"));
            } else { output = source; }
            info["milliseconds"] = json!(t.elapsed().as_secs_f64()*1000.0);
            println!("[images] {} converted={} ms={:.1}", name, is_bmp, t.elapsed().as_secs_f64()*1000.0);
            Ok((output, info))
        })();
        (name.clone(), result)
    }).collect());
    let mut resolved = BTreeMap::new();
    let mut conversions = Vec::new();
    let mut errors = Vec::new();
    for (name, result) in conversion_results {
        match result {
            Ok((path, info)) => { resolved.insert(name, path); conversions.push(info); }
            Err(e) => errors.push(json!({"source":name,"error":format!("{e:#}")})),
        }
    }
    let comparison_results: Vec<Value> = pool.install(|| manifest.captures.par_iter().map(|capture| {
        if capture.status.as_deref() == Some("failed") {
            return json!({"id":capture.id,"status":"failed","error":capture.error});
        }
        if !options.compare { return json!({"id":capture.id,"status":"not-requested"}); }
        let Some(right) = &capture.shrunk else { return json!({"id":capture.id,"status":"single-workbook"}); };
        let result = (|| -> Result<Value> {
            let t = Instant::now();
            let left_name = capture.original.as_ref().context("missing original")?;
            let a = load(resolved.get(left_name).context("original conversion failed")?, options.max_pixels)?;
            let b = load(resolved.get(right).context("shrunk conversion failed")?, options.max_pixels)?;
            let (mut info, diff) = compare_pixels(&a, &b);
            info["id"] = json!(capture.id); info["status"] = json!("compared");
            if let Some(diff) = diff {
                let path = root.join("images").join(format!("{}-pixel-diff.png",capture.id));
                png(&path, &diff)?;
                info["diff_png"] = json!(path.strip_prefix(&root)?.to_string_lossy().replace('\\',"/"));
            }
            info["milliseconds"] = json!(t.elapsed().as_secs_f64()*1000.0);
            println!("[images] compare {} equal={} ms={:.1}",capture.id,info["equal"],t.elapsed().as_secs_f64()*1000.0);
            Ok(info)
        })();
        result.unwrap_or_else(|e| json!({"id":capture.id,"status":"failed","error":format!("{e:#}")}))
    }).collect());
    let failed = comparison_results.iter().filter(|v| v["status"] == "failed").count() + errors.len();
    let changed = comparison_results.iter().filter(|v| v["equal"] == false).count();
    let report = json!({"schema":"excel-lab-images-v1","processing_complete":true,"failed_jobs":failed,"changed_captures":changed,
        "workers":options.workers,"max_pixels_per_image":options.max_pixels,"png_encoding":"Fast/Adaptive; no white-to-alpha rewriting",
        "total_seconds":started.elapsed().as_secs_f64(),"conversions":conversions,"errors":errors,"comparisons":comparison_results});
    let output = root.join("image-report.json");
    atomic_report(&output,&report)?;
    println!("[images] report: {}",output.display());
    ensure!(failed == 0,"{failed} image jobs failed; inspect image-report.json");
    Ok(())
}
fn main() {
    if let Err(error) = options().and_then(run) {
        eprintln!("ERROR: {error:#}");
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime,UNIX_EPOCH};
    fn temp() -> PathBuf {
        let p=std::env::temp_dir().join(format!("xl-lab-images-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&p).unwrap(); p
    }
    #[test] fn equal_pixels_need_no_diff() {
        let a=RgbaImage::from_pixel(2,3,Rgba([1,2,3,255])); let (info,diff)=compare_pixels(&a,&a);
        assert_eq!(info["equal"],true); assert!(diff.is_none());
    }
    #[test] fn alpha_is_part_of_comparison() {
        let a=RgbaImage::from_pixel(2,2,Rgba([255,255,255,0])); let b=RgbaImage::from_pixel(2,2,Rgba([255,255,255,255]));
        assert_eq!(compare_pixels(&a,&b).0["changed_pixels"],4);
    }
    #[test] fn detects_last_pixel_and_location() {
        let a=RgbaImage::new(3,2); let mut b=a.clone(); b.put_pixel(2,1,Rgba([1,0,0,0]));
        let info=compare_pixels(&a,&b).0; assert_eq!(info["changed_pixels"],1); assert_eq!(info["first_difference"],json!([2,1]));
    }
    #[test] fn different_dimensions_are_never_equal() { assert_eq!(compare_pixels(&RgbaImage::new(2,2),&RgbaImage::new(3,2)).0["equal"],false); }
    #[test] fn png_preserves_rgba_and_refuses_overwrite() {
        let dir=temp(); let path=dir.join("x.png"); let a=RgbaImage::from_pixel(3,3,Rgba([10,20,30,127]));
        png(&path,&a).unwrap(); assert_eq!(load(&path,100).unwrap(),a); assert!(png(&path,&a).is_err()); fs::remove_dir_all(dir).unwrap();
    }
    #[test] fn bitmap_roundtrip_to_png() {
        let dir=temp(); let path=dir.join("x.bmp"); let a=image::RgbImage::from_pixel(3,3,image::Rgb([10,20,30]));
        a.save(&path).unwrap(); let data=load(&path,100).unwrap(); png(&dir.join("x.png"),&data).unwrap();
        assert_eq!(load(&dir.join("x.png"),100).unwrap(),data); fs::remove_dir_all(dir).unwrap();
    }
    #[test] fn path_traversal_rejected_on_all_platforms() {
        let dir=temp().canonicalize().unwrap();
        for p in ["../x","/x","C:\\x","\\\\host\\share","images/../../x",""] { assert!(safe_relative(&dir,p).is_err()); }
        fs::remove_dir_all(dir).unwrap();
    }
    #[test] fn pixel_budget_is_enforced() {
        let dir=temp(); let p=dir.join("x.png"); png(&p,&RgbaImage::new(11,10)).unwrap(); assert!(load(&p,100).is_err()); fs::remove_dir_all(dir).unwrap();
    }
}
