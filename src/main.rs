use std::fs::{self, File};
use std::path::{Path, PathBuf};
use xml::reader::{XmlEvent, EventReader};
use xml::common::{Position, TextPosition};
use std::collections::HashMap;
use std::env;
use std::result::Result;

struct Lexer<'a> {
    content: &'a [char],
}

impl<'a> Lexer<'a> {
    fn new(content: &'a [char]) -> Self {
        Self { content }
    }

    fn trim_left(&mut self) {
        while !self.content.is_empty() && self.content[0].is_whitespace() {
            self.content = &self.content[1..];
        }
    }

    fn chop(&mut self, n: usize) -> &'a [char] {
        let token = &self.content[0..n];
        self.content = &self.content[n..];
        token
    }

    fn chop_while<P>(&mut self, mut predicate: P) -> &'a [char] where P: FnMut(&char) -> bool {
        let mut n = 0;
        while n < self.content.len() && predicate(&self.content[n]) {
            n += 1;
        }
        self.chop(n)
    }

    fn next_token(&mut self) -> Option<&'a [char]> {
        self.trim_left();
        if self.content.is_empty() {
            return None
        }

        if self.content[0].is_numeric() {
            return Some(self.chop_while(|x| x.is_numeric()));
        }

        if self.content[0].is_alphabetic() {
            return Some(self.chop_while(|x| x.is_alphanumeric()));
        }

        return Some(self.chop(1));
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = &'a [char];

    fn next(&mut self) -> Option<Self::Item> {
        self.next_token()
    }
}

fn parse_entire_xml_file(file_path: &Path) -> Result<String, ()> {
    let file = File::open(file_path).map_err(|err| {
        eprintln!("ERROR: could not open file {file_path}: {err}", file_path = file_path.display());
    })?;
    let er = EventReader::new(file);
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

type TermFreq = HashMap::<String, usize>;
type TermFreqIndex = HashMap<PathBuf, TermFreq>;

fn check_index(index_path: &str) -> Result<(), ()> {
    println!("Reading {index_path} index file...");

    let index_file = File::open(index_path).map_err(|err| {
        eprintln!("ERROR: could not open index file {index_path}: {err}");
    })?;

    let tf_index: TermFreqIndex = serde_json::from_reader(index_file).map_err(|err| {
        eprintln!("ERROR: could not parse index file {index_path}: {err}");
    })?;

    println!("{index_path} contains {count} files", count = tf_index.len());

    Ok(())
}

fn save_tf_index(tf_index: &TermFreqIndex, index_path: &str) -> Result<(), ()> {
    println!("Saving {index_path}...");

    let index_file = File::create(index_path).map_err(|err| {
        eprintln!("ERROR: could not create index file {index_path}: {err}");
    })?;

    serde_json::to_writer(index_file, &tf_index).map_err(|err| {
        eprintln!("ERROR: could not serialize index into file {index_path}: {err}");
    })?;

    Ok(())
}

fn tf_index_of_folder(dir_path: &str) -> Result<TermFreqIndex, ()> {
    let dir = fs::read_dir(dir_path).map_err(|err| {
        eprintln!("ERROR: could not open directory {dir_path} for indexing: {err}");
    })?;
    let mut tf_index = TermFreqIndex::new();

    'next_file: for file in dir {
        let file_path = file.map_err(|err| {
            eprintln!("ERROR: could not read next file in directory {dir_path} during indexing: {err}");
        })?.path();

        println!("Indexing {:?}...", &file_path);

        let content = match parse_entire_xml_file(&file_path) {
            Ok(content) => content.chars().collect::<Vec<_>>(),
            Err(()) => continue 'next_file,
        };

        let mut tf = TermFreq::new();

        for token in Lexer::new(&content) {
            let term = token.iter().map(|x| x.to_ascii_uppercase()).collect::<String>();
            if let Some(freq) = tf.get_mut(&term) {
                *freq += 1;
            } else {
                tf.insert(term, 1);
            }
        }

        let mut stats = tf.iter().collect::<Vec<_>>();
        stats.sort_by_key(|(_, f)| *f);
        stats.reverse();

        tf_index.insert(file_path, tf);
    }

    Ok(tf_index)
}

fn main() -> Result<(), ()> {
    let mut args = env::args();
    let _program = args.next().expect("path to program is provided");

    let subcommand = args.next().ok_or_else(|| {
        println!("ERROR: no subcommand is provided");
    })?;

    match subcommand.as_str() {
        "index" => {
            let dir_path = args.next().ok_or_else(|| {
                println!("ERROR: no directory is provided for {subcommand} subcommand");
            })?;

            let tf_index = tf_index_of_folder(&dir_path)?;
            save_tf_index(&tf_index, "index.json")?;
        },
        "search" => {
            let index_path = args.next().ok_or_else(|| {
                println!("ERROR: no path to index is provided for {subcommand} subcommand");
            })?;

            check_index(&index_path)?;
        }
        _ => {
            println!("ERROR: unknown subcommand {subcommand}");
            return Err(());
        }
    }

    Ok(())
}
