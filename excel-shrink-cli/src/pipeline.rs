use rayon::prelude::*;
use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use threadpool::ThreadPool;
use tracing::{error, info, warn};
use zip::{ZipArchive, ZipWriter};

use excel_shrink_core::models::{CleanSheetParameters, CleanSheetParametersBuilder};
use excel_shrink_core::sheet::clean_sheet_data_streamwise;

struct WriteJob {
    filename: String,
    data: Vec<u8>,
    compress_method: zip::CompressionMethod,
}

pub fn process_files(
    input_files: Vec<PathBuf>,
    output_dir: PathBuf,
    chunk_size: usize,
    disable_mp: bool,
    force_overwrite: bool,
) -> anyhow::Result<()> {
    if !output_dir.exists() {
        std::fs::create_dir_all(&output_dir)?;
    }

    let writer_pool = ThreadPool::new(16);

    let process_file = |input: &PathBuf| {
        info!("Processing: {:?}", input);
        let out_path = output_dir.join(input.file_name().unwrap());

        if out_path.exists() && !force_overwrite {
            warn!("Output file {:?} already exists. Skipping.", out_path);
            return;
        }

        let (tx, rx) = mpsc::channel::<WriteJob>();

        // Start Writer Task (Asynchroner I/O-Writer)
        writer_pool.execute(move || {
            let out_file = File::create(&out_path).unwrap();
            let mut zip_writer = ZipWriter::new(out_file);
            let options = zip::write::FileOptions::default();

            for job in rx {
                let opts = options.compression_method(job.compress_method);
                if let Err(e) = zip_writer.start_file(&job.filename, opts) {
                    error!("Failed to write to zip: {}", e);
                    continue;
                }
                zip_writer.write_all(&job.data).unwrap();
            }
            zip_writer.finish().unwrap();
            info!("Finished writing {:?}", out_path);
        });

        // OPTIMIERUNG 1: Gesamte Datei einmalig sequenziell in den RAM laden.
        // Das verhindert I/O-Wait und NTFS-Locking-Probleme unter WSL.
        let mut file_bytes = Vec::new();
        {
            let mut f = match File::open(input) {
                Ok(f) => f,
                Err(e) => {
                    error!("Failed to open file {:?}: {}", input, e);
                    return;
                }
            };
            if let Err(e) = f.read_to_end(&mut file_bytes) {
                error!("Failed to read file {:?}: {}", input, e);
                return;
            }
        }

        // OPTIMIERUNG 2: ZIP-Archiv rein im Arbeitsspeicher parsen (Cursor)
        let mut archive = match ZipArchive::new(Cursor::new(file_bytes)) {
            Ok(arc) => arc,
            Err(e) => {
                error!("Failed to parse ZIP archive {:?}: {}", input, e);
                return;
            }
        };

        let mut sheet_params = Vec::new();
        let mut other_files = Vec::new();

        for i in 0..archive.len() {
            let file = archive.by_index(i).unwrap();
            let name = file.name().to_string();

            if name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml") {
                sheet_params.push(
                    CleanSheetParametersBuilder::default()
                        .xlsx_file_path(input.to_string_lossy().to_string())
                        .zip_info_filename(name)
                        .compress_method(file.compression())
                        .build()
                        .unwrap(),
                );
            } else {
                let mut buf = Vec::new();
                let method = file.compression();
                drop(file);
                
                let mut re_opened = archive.by_index(i).unwrap();
                re_opened.read_to_end(&mut buf).unwrap();
                other_files.push(WriteJob {
                    filename: name,
                    data: buf,
                    compress_method: method,
                });
            }
        }

        // Nicht-Sheet-Dateien direkt an den Writer senden
        for job in other_files {
            tx.send(job).unwrap();
        }

        // OPTIMIERUNG 3: Alle Sheets sequenziell im Hauptthread der Datei in den RAM extrahieren
        let mut sheets_data = Vec::with_capacity(sheet_params.len());
        for params in sheet_params {
            if let Ok(mut sheet_file) = archive.by_name(&params.zip_info_filename) {
                let mut buf = Vec::new();
                sheet_file.read_to_end(&mut buf).unwrap();
                sheets_data.push((params, buf));
            }
        }

        // Hilfsfunktion zur Bereinigung (rein CPU-gebunden im RAM)
        let process_sheet = |params: CleanSheetParameters, data: Vec<u8>| {
            match clean_sheet_data_streamwise(Cursor::new(data), &params, chunk_size) {
                Ok(cleaned_data) => {
                    tx.send(WriteJob {
                        filename: params.zip_info_filename.clone(),
                        data: cleaned_data,
                        compress_method: params.compress_method,
                    }).unwrap();
                }
                Err(e) => {
                    error!("Error processing {}: {}", params.zip_info_filename, e);
                }
            }
        };

        // OPTIMIERUNG 4: Reine CPU-Verarbeitung parallel im RAM starten
        if disable_mp {
            for (params, data) in sheets_data {
                process_sheet(params, data);
            }
        } else {
            sheets_data.into_par_iter().for_each(|(params, data)| {
                process_sheet(params, data);
            });
        }
    };

    if disable_mp {
        input_files.iter().for_each(process_file);
    } else {
        input_files.par_iter().for_each(process_file);
    }

    writer_pool.join();
    Ok(())
}