use std::fs::{self, File};
use std::path::{Path};
use xml::reader::{XmlEvent, EventReader};
use xml::common::{Position, TextPosition};
use std::env;
use std::result::Result;
use std::process::ExitCode;
use std::str;
use std::io::{BufReader, BufWriter};
use std::sync::{Arc, Mutex};
use std::thread;

mod model;
use model::*;
mod server;
mod lexer;
pub mod snowball;

fn parse_entire_txt_file(file_path: &Path) -> Result<String, ()> {
    fs::read_to_string(file_path).map_err(|err| {
        eprintln!("ERROR: coult not open file {file_path}: {err}", file_path = file_path.display());
    })
}

fn parse_entire_pdf_file(file_path: &Path) -> Result<String, ()> {
    use poppler::Document;
    use std::io::Read;

    let mut content = Vec::new();
    File::open(file_path)
        .and_then(|mut file| file.read_to_end(&mut content))
        .map_err(|err| {
            eprintln!("ERROR: could not read file {file_path}: {err}", file_path = file_path.display());
        })?;

    let pdf = Document::from_data(&content, None).map_err(|err| {
        eprintln!("ERROR: could not read file {file_path}: {err}",
                  file_path = file_path.display());
    })?;

    let mut result = String::new();

    let n = pdf.n_pages();
    for i in 0..n {
        let page = pdf.page(i).expect(&format!("{i} is within the bounds of the range of the page"));
        if let Some(content) = page.text() {
            result.push_str(content.as_str());
            result.push(' ');
        }
    }

    Ok(result)
}

fn parse_entire_xml_file(file_path: &Path) -> Result<String, ()> {
    let file = File::open(file_path).map_err(|err| {
        eprintln!("ERROR: could not open file {file_path}: {err}", file_path = file_path.display());
    })?;
    let er = EventReader::new(BufReader::new(file));
    let mut content = String::new();
    for event in er.into_iter() {
        let event = event.map_err(|err| {
            let TextPosition {row, column} = err.position();
            let msg = err.msg();
            eprintln!("{file_path}:{row}:{column}: ERROR: {msg}", file_path = file_path.display());
        })?;

        if let XmlEvent::Characters(text) = event {
            content.push_str(&text);
            content.push(' ');
        }
    }
    Ok(content)
}

fn parse_entire_file_by_extension(file_path: &Path) -> Result<String, ()> {
    let extension = file_path.extension().ok_or_else(|| {
        eprintln!("ERROR: can't detect file type of {file_path} without extension",
                  file_path = file_path.display());
    })?.to_string_lossy();
    match extension.as_ref() {
        "xhtml" | "xml" => parse_entire_xml_file(file_path),
        // TODO: specialized parser for markdown files
        "txt" | "md" => parse_entire_txt_file(file_path),
        "pdf" => parse_entire_pdf_file(file_path),
        _ => {
            eprintln!("ERROR: can't detect file type of {file_path}: unsupported extension {extension}",
                      file_path = file_path.display(),
                      extension = extension);
            Err(())
        }
    }
}

fn save_model_as_json(model: &Model, index_path: &str) -> Result<(), ()> {
    println!("Saving {index_path}...");

    let index_file = File::create(index_path).map_err(|err| {
        eprintln!("ERROR: could not create index file {index_path}: {err}");
    })?;

    serde_json::to_writer(BufWriter::new(index_file), &model).map_err(|err| {
        eprintln!("ERROR: could not serialize index into file {index_path}: {err}");
    })?;

    Ok(())
}

fn add_folder_to_model(dir_path: &Path, model: Arc<Mutex<Model>>, processed: &mut usize) -> Result<(), ()> {
    let dir = fs::read_dir(dir_path).map_err(|err| {
        eprintln!("ERROR: could not open directory {dir_path} for indexing: {err}",
                  dir_path = dir_path.display());
    })?;

    'next_file: for file in dir {
        let file = file.map_err(|err| {
            eprintln!("ERROR: could not read next file in directory {dir_path} during indexing: {err}",
                      dir_path = dir_path.display());
        })?;

        let file_path = file.path();
        let file_type = file.file_type().map_err(|err| {
            eprintln!("ERROR: could not determine type of file {file_path}: {err}",
                      file_path = file_path.display());
        })?;
        let last_modified = file.metadata().map_err(|err| {
            eprintln!("ERROR: could not get the metadata of file {file_path}: {err}",
                      file_path = file_path.display());
        })?.modified().map_err(|err| {
            eprintln!("ERROR: could not get the last modification date of file {file_path}: {err}",
                      file_path = file_path.display())
        })?;

        if file_type.is_dir() {
            add_folder_to_model(&file_path, Arc::clone(&model), processed)?;
            continue 'next_file;
        }

        // TODO: how does this work with symlinks?

        let mut model = model.lock().unwrap();
        if model.requires_reindexing(&file_path, last_modified) {
            println!("Indexing {:?}...", &file_path);

            let content = match parse_entire_file_by_extension(&file_path) {
                Ok(content) => content.chars().collect::<Vec<_>>(),
                // TODO: still add the skipped files to the model to prevent their reindexing in the future
                Err(()) => continue 'next_file,
            };

            model.add_document(file_path, last_modified, &content);
            *processed += 1;
        }
    }

    Ok(())
}

fn usage(program: &str) {
    eprintln!("Usage: {program} [SUBCOMMAND] [OPTIONS]");
    eprintln!("Subcommands:");
    eprintln!("    serve <folder> [address]       start local HTTP server with Web Interface");
}

fn entry() -> Result<(), ()> {
    let mut args = env::args();
    let program = args.next().expect("path to program is provided");

    let subcommand = args.next().ok_or_else(|| {
        usage(&program);
        eprintln!("ERROR: no subcommand is provided");
    })?;

    match subcommand.as_str() {
        "serve" => {
            let dir_path = args.next().ok_or_else(|| {
                usage(&program);
                eprintln!("ERROR: no directory is provided for {subcommand} subcommand");
            })?;

            // TODO: figure out index_path based on dir_path
            let index_path = "index.json";

            let address = args.next().unwrap_or("127.0.0.1:6969".to_string());

            let exists = Path::new(index_path).try_exists().map_err(|err| {
                eprintln!("ERROR: could not check the existence of file {index_path}: {err}");
            })?;

            let model: Arc<Mutex<Model>>;
            if exists {
                let index_file = File::open(&index_path).map_err(|err| {
                    eprintln!("ERROR: could not open index file {index_path}: {err}");
                })?;

                model = Arc::new(Mutex::new(serde_json::from_reader(index_file).map_err(|err| {
                    eprintln!("ERROR: could not parse index file {index_path}: {err}");
                })?));
            } else {
                model = Arc::new(Mutex::new(Default::default()));
            }

            {
                let model = Arc::clone(&model);
                thread::spawn(move || {
                    let mut processed = 0;
                    // TODO: what should we do in case indexing thread crashes
                    add_folder_to_model(Path::new(&dir_path), Arc::clone(&model), &mut processed).unwrap();
                    if processed > 0 {
                        let model = model.lock().unwrap();
                        save_model_as_json(&model, index_path).unwrap();
                    }
                    println!("Finished indexing");
                });
            }

            server::start(&address, Arc::clone(&model))
        }

        _ => {
            usage(&program);
            eprintln!("ERROR: unknown subcommand {subcommand}");
            Err(())
        }
    }
}

fn main() -> ExitCode {
    match entry() {
        Ok(()) => ExitCode::SUCCESS,
        Err(()) => ExitCode::FAILURE,
    }
}

// TODO: search result must consist of clickable links
// TODO: parse pdf files
// TODO: synonym terms
