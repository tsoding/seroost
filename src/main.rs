use std::fs::{self, File};
use std::path::Path;
use xml::reader::{XmlEvent, EventReader};
use xml::common::{Position, TextPosition};
use std::env;
use std::result::Result;
use std::process::ExitCode;
use std::str;
use std::io::{BufReader, BufWriter};

mod model;
use model::*;
mod server;

fn parse_entire_txt_file(file_path: &Path) -> Result<String, ()> {
    fs::read_to_string(file_path).map_err(|err| {
        eprintln!("ERROR: coult not open file {file_path}: {err}", file_path = file_path.display());
    })
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
        _ => {
            eprintln!("ERROR: can't detect file type of {file_path}: unsupported extension {extension}",
                      file_path = file_path.display(),
                      extension = extension);
            Err(())
        }
    }
}

fn save_model_as_json(model: &InMemoryModel, index_path: &str) -> Result<(), ()> {
    println!("Saving {index_path}...");

    let index_file = File::create(index_path).map_err(|err| {
        eprintln!("ERROR: could not create index file {index_path}: {err}");
    })?;

    serde_json::to_writer(BufWriter::new(index_file), &model).map_err(|err| {
        eprintln!("ERROR: could not serialize index into file {index_path}: {err}");
    })?;

    Ok(())
}

fn add_folder_to_model(dir_path: &Path, model: &mut dyn Model, skipped: &mut usize) -> Result<(), ()> {
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

        if file_type.is_dir() {
            add_folder_to_model(&file_path, model, skipped)?;
            continue 'next_file;
        }

        // TODO: how does this work with symlinks?

        println!("Indexing {:?}...", &file_path);

        let content = match parse_entire_file_by_extension(&file_path) {
            Ok(content) => content.chars().collect::<Vec<_>>(),
            Err(()) => {
                *skipped += 1;
                continue 'next_file;
            }
        };

        model.add_document(file_path, &content)?;
    }

    Ok(())
}

fn usage(program: &str) {
    eprintln!("Usage: {program} [SUBCOMMAND] [OPTIONS]");
    eprintln!("Subcommands:");
    eprintln!("    index <folder>                  index the <folder> and save the index to index.json file");
    eprintln!("    search <index-file> <query>     search <query> within the <index-file>");
    eprintln!("    serve <index-file> [address]    start local HTTP server with Web Interface");
}

fn entry() -> Result<(), ()> {
    let mut args = env::args();
    let program = args.next().expect("path to program is provided");

    let mut subcommand = None;
    let mut use_sqlite_mode = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sqlite" => use_sqlite_mode = true,
            _ => {
                subcommand = Some(arg);
                break
            }
        }
    }

    let subcommand = subcommand.ok_or_else(|| {
        usage(&program);
        eprintln!("ERROR: no subcommand is provided");
    })?;

    match subcommand.as_str() {
        "index" => {
            let dir_path = args.next().ok_or_else(|| {
                usage(&program);
                eprintln!("ERROR: no directory is provided for {subcommand} subcommand");
            })?;

            let mut skipped = 0;

            if use_sqlite_mode {
                let index_path = "index.db";

                if let Err(err) = fs::remove_file(index_path) {
                    if err.kind() != std::io::ErrorKind::NotFound {
                        eprintln!("ERROR: could not delete file {index_path}: {err}");
                        return Err(())
                    }
                }

                let mut model = SqliteModel::open(Path::new(index_path))?;
                model.begin()?;
                add_folder_to_model(Path::new(&dir_path), &mut model, &mut skipped)?;
                // TODO: implement a special transaction object that implements Drop trait and commits the transaction when it goes out of scope
                model.commit()?;
            } else {
                let index_path = "index.json";
                let mut model = Default::default();
                add_folder_to_model(Path::new(&dir_path), &mut model, &mut skipped)?;
                save_model_as_json(&model, index_path)?;
            }

            println!("Skipped {skipped} files.");
            Ok(())
        },
        "search" => {
            let index_path = args.next().ok_or_else(|| {
                usage(&program);
                eprintln!("ERROR: no path to index is provided for {subcommand} subcommand");
            })?;

            let prompt = args.next().ok_or_else(|| {
                usage(&program);
                eprintln!("ERROR: no search query is provided {subcommand} subcommand");
            })?.chars().collect::<Vec<_>>();

            if use_sqlite_mode {
                let model = SqliteModel::open(Path::new(&index_path))?;

                for (path, rank) in model.search_query(&prompt)?.iter().take(20) {
                    println!("{path} {rank}", path = path.display());
                }
            } else {
                let index_file = File::open(&index_path).map_err(|err| {
                    eprintln!("ERROR: could not open index file {index_path}: {err}");
                })?;

                let model = serde_json::from_reader::<_, InMemoryModel>(index_file).map_err(|err| {
                    eprintln!("ERROR: could not parse index file {index_path}: {err}");
                })?;

                for (path, rank) in model.search_query(&prompt)?.iter().take(20) {
                    println!("{path} {rank}", path = path.display());
                }
            }

            Ok(())
        }
        "serve" => {
            let index_path = args.next().ok_or_else(|| {
                usage(&program);
                eprintln!("ERROR: no path to index is provided for {subcommand} subcommand");
            })?;

            let address = args.next().unwrap_or("127.0.0.1:6969".to_string());

            if use_sqlite_mode {
                let model = SqliteModel::open(Path::new(&index_path))?;

                server::start(&address, &model)
            } else {
                let index_file = File::open(&index_path).map_err(|err| {
                    eprintln!("ERROR: could not open index file {index_path}: {err}");
                })?;

                let model: InMemoryModel = serde_json::from_reader(index_file).map_err(|err| {
                    eprintln!("ERROR: could not parse index file {index_path}: {err}");
                })?;

                server::start(&address, &model)
            }
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
